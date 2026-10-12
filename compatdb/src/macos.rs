//! The host's macOS version, which decides whether `gptk` can load.
//!
//! `D3DMetal.framework`, which the bundle ships, carries its own
//! `libdxccontainer.dylib`. That library is built for macOS 26.4 at the
//! earliest (its minos) and `D3DMetal` links it non-weak, so on an older
//! release the DXGI modules of the `gptk` tree fail to load. The library reads
//! the version once per process and passes the answer to [`Dxgi::effective`],
//! which then picks `dxmt` instead.
//!
//! [`Dxgi::effective`]: compatdb_table::Dxgi::effective

use std::{
    ffi::{CStr, c_int, c_void},
    ptr,
    sync::OnceLock,
};

use crate::log;

/// A macOS or Darwin release, as far as the check needs it: major and minor.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
}

impl Version {
    /// Parse a version such as `15.7.1`, `26.4` or `25.4.0`. A missing minor
    /// reads as 0 and a patch level is ignored. `None` when the text is not a
    /// version.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = match parts.next() {
            Some(minor) => minor.parse().ok()?,
            None => 0,
        };
        Some(Self { major, minor })
    }
}

/// The first macOS release `D3DMetal` loads on: the minos of the
/// `libdxccontainer.dylib` in the pinned GPTK image. Must match `GPTK_FLOOR`
/// in bundle-wine.sh and move with the GPTK pin in redist.env, together with
/// [`GPTK_MIN_TEXT`] and [`GPTK_MIN_DARWIN`].
pub const GPTK_MIN: Version = Version {
    major: 26,
    minor: 4,
};

/// [`GPTK_MIN`] as the log prints it.
pub const GPTK_MIN_TEXT: &str = "26.4";

/// The Darwin release of [`GPTK_MIN`], for when only `kern.osrelease` can be
/// trusted.
pub const GPTK_MIN_DARWIN: Version = Version {
    major: 25,
    minor: 4,
};

/// What the library knows about the host.
#[derive(Debug, PartialEq, Eq)]
pub struct Host {
    /// The release the decision was made on, as the log prints it
    /// (`macOS 15.7.1`, `Darwin 25.3.0`); empty when none could be read.
    pub name: String,
    /// Whether `gptk` can load here.
    pub gptk_loads: bool,
    /// The line to log about how the release was found. `None` in the normal
    /// case, where `kern.osproductversion` is read and taken as it is.
    pub note: Option<String>,
}

/// Decide whether `gptk` can load from the three sysctls: `product`
/// (`kern.osproductversion`), `compat` (`kern.system_version_compat` is
/// nonzero) and `release` (`kern.osrelease`), each `None` when it could not
/// be read.
///
/// The product version is the one to go by, but macOS shims it for a process
/// whose main binary was built against an old SDK or that runs with
/// `SYSTEM_VERSION_COMPAT=1` (which a rule's env could set before exec): it
/// then reads 10.16 or 16.0 instead of the real release, which would drop
/// `gptk` on a host that has it. In that case, or when the product version
/// cannot be read or parsed, the Darwin release decides instead. It is not
/// the first choice because its minor did not track the macOS minor on every
/// release (11 to 13), though it does where the threshold sits. When neither
/// is usable, `gptk` is assumed to load: that is what the library did before
/// it checked, and a failed check should not take `gptk` away from a host
/// that has it.
pub fn decide(product: Option<&str>, compat: bool, release: Option<&str>) -> Host {
    let parsed = product.and_then(Version::parse);
    let problem = match (product, parsed) {
        (Some(text), Some(v)) if compat || v.major == 10 || v.major == 16 => {
            format!("kern.osproductversion {text:?} is a compatibility value")
        }
        (Some(text), Some(v)) => {
            return Host {
                name: format!("macOS {text}"),
                gptk_loads: v >= GPTK_MIN,
                note: None,
            };
        }
        (Some(text), None) => format!("kern.osproductversion {text:?} not understood"),
        (None, _) => "cannot read kern.osproductversion".to_owned(),
    };
    match (release, release.and_then(Version::parse)) {
        (Some(text), Some(v)) => Host {
            name: format!("Darwin {text}"),
            gptk_loads: v >= GPTK_MIN_DARWIN,
            note: Some(format!("{problem}, using kern.osrelease {text:?}")),
        },
        (Some(text), None) => Host {
            name: String::new(),
            gptk_loads: true,
            note: Some(format!(
                "{problem} and kern.osrelease {text:?} not understood, assuming gptk loads"
            )),
        },
        (None, _) => Host {
            name: String::new(),
            gptk_loads: true,
            note: Some(format!(
                "{problem} and cannot read kern.osrelease, assuming gptk loads"
            )),
        },
    }
}

/// The host, read on the first call in the process. Logs one line then when
/// the product version is not taken as it is.
pub fn host() -> &'static Host {
    static HOST: OnceLock<Host> = OnceLock::new();
    HOST.get_or_init(|| {
        let product = sysctl_string(c"kern.osproductversion");
        let compat = sysctl_int(c"kern.system_version_compat").is_some_and(|v| v != 0);
        let release = sysctl_string(c"kern.osrelease");
        let host = decide(product.as_deref(), compat, release.as_deref());
        if let Some(note) = &host.note {
            log::line(note);
        }
        host
    })
}

/// A string sysctl such as `kern.osproductversion`, or `None` when the call
/// fails or the value is not UTF-8.
fn sysctl_string(name: &CStr) -> Option<String> {
    let mut buf = [0_u8; 64];
    let mut len = buf.len();
    // SAFETY: `name` is a NUL-terminated string; `buf` and `len` describe a
    // writable buffer and its size, which sysctl writes at most `len` bytes
    // into and then updates `len`; the null new value sets nothing.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast::<c_void>(),
            &raw mut len,
            ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    let value = buf.get(..len)?.split(|&b| b == 0).next()?;
    std::str::from_utf8(value).ok().map(str::to_owned)
}

/// An integer sysctl such as `kern.system_version_compat`, or `None` when the
/// call fails or the value is not an `int`.
fn sysctl_int(name: &CStr) -> Option<c_int> {
    let mut value: c_int = 0;
    let mut len = size_of::<c_int>();
    // SAFETY: `name` is a NUL-terminated string; `value` and `len` describe a
    // writable `int` and its size; the null new value sets nothing.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&raw mut value).cast::<c_void>(),
            &raw mut len,
            ptr::null_mut(),
            0,
        )
    };
    (rc == 0 && len == size_of::<c_int>()).then_some(value)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Whether gptk loads on macOS `product`, with no compat flag and no
    /// Darwin release to fall back to.
    fn loads_on(product: &str) -> bool {
        decide(Some(product), false, None).gptk_loads
    }

    #[test]
    fn gptk_needs_macos_26_4() {
        for old in ["15.0", "15.7.1", "26.3", "26.3.9", "15"] {
            assert!(!loads_on(old), "{old}");
        }
        for new in ["26.4", "26.4.1", "26.10", "27.0", "27"] {
            assert!(loads_on(new), "{new}");
        }
    }

    #[test]
    fn a_version_that_does_not_parse_counts_as_one_gptk_loads_on() {
        for bad in ["", "garbage", "26.x", ".4", "26..4"] {
            assert!(Version::parse(bad).is_none(), "{bad:?}");
            assert!(loads_on(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_version_parses_major_and_minor_only() {
        assert_eq!(
            Version::parse("15.7.1"),
            Some(Version {
                major: 15,
                minor: 7
            })
        );
        assert_eq!(
            Version::parse("26"),
            Some(Version {
                major: 26,
                minor: 0
            })
        );
        assert_eq!(Version::parse(GPTK_MIN_TEXT), Some(GPTK_MIN));
    }

    #[test]
    fn the_product_version_decides_quietly_when_it_is_real() {
        // The Darwin release is ignored here, even when it disagrees.
        assert_eq!(
            decide(Some("15.7.1"), false, Some("25.4.0")),
            Host {
                name: "macOS 15.7.1".into(),
                gptk_loads: false,
                note: None,
            }
        );
        assert_eq!(
            decide(Some("26.4"), false, None),
            Host {
                name: "macOS 26.4".into(),
                gptk_loads: true,
                note: None,
            }
        );
        assert!(!decide(Some("26.3"), false, Some("25.3.0")).gptk_loads);
        assert!(decide(Some("27.0"), false, Some("27.0.0")).gptk_loads);
    }

    #[test]
    fn a_shimmed_product_version_defers_to_the_darwin_release() {
        // The compat flag, whatever the product version reads.
        for product in ["16.0", "10.16", "26.4"] {
            let host = decide(Some(product), true, Some("25.4.0"));
            assert!(host.gptk_loads, "{product}");
            assert_eq!(host.name, "Darwin 25.4.0");
            assert_eq!(
                host.note.as_deref(),
                Some(
                    format!(
                        "kern.osproductversion {product:?} is a compatibility value, \
                         using kern.osrelease \"25.4.0\""
                    )
                    .as_str()
                )
            );
        }
        // 10.x and 16.x read as shimmed even without the flag.
        assert!(decide(Some("16.0"), false, Some("25.4.0")).gptk_loads);
        assert!(!decide(Some("16.0"), false, Some("25.3.0")).gptk_loads);
        assert!(!decide(Some("10.16"), false, Some("24.6.0")).gptk_loads);
        // The Darwin threshold.
        for (release, loads) in [
            ("24.6.0", false),
            ("25.0.0", false),
            ("25.3.9", false),
            ("25.4.0", true),
            ("25.10.0", true),
            ("26.0.0", true),
            ("27.0.0", true),
        ] {
            assert_eq!(
                decide(Some("16.0"), true, Some(release)).gptk_loads,
                loads,
                "{release}"
            );
        }
    }

    #[test]
    fn an_unusable_product_version_defers_to_the_darwin_release() {
        let host = decide(None, false, Some("24.6.0"));
        assert!(!host.gptk_loads);
        assert_eq!(
            host.note.as_deref(),
            Some("cannot read kern.osproductversion, using kern.osrelease \"24.6.0\"")
        );
        let host = decide(Some("garbage"), false, Some("25.4.0"));
        assert!(host.gptk_loads);
        assert_eq!(
            host.note.as_deref(),
            Some(
                "kern.osproductversion \"garbage\" not understood, using kern.osrelease \"25.4.0\""
            )
        );
    }

    #[test]
    fn nothing_usable_assumes_gptk_loads() {
        let host = decide(None, false, None);
        assert_eq!(
            host,
            Host {
                name: String::new(),
                gptk_loads: true,
                note: Some(
                    "cannot read kern.osproductversion and cannot read kern.osrelease, \
                     assuming gptk loads"
                        .into()
                ),
            }
        );
        let host = decide(Some("16.0"), true, Some("garbage"));
        assert!(host.gptk_loads);
        assert_eq!(
            host.note.as_deref(),
            Some(
                "kern.osproductversion \"16.0\" is a compatibility value and \
                 kern.osrelease \"garbage\" not understood, assuming gptk loads"
            )
        );
        assert!(decide(Some(""), false, Some("")).gptk_loads);
    }

    #[test]
    fn the_host_sysctls_read_and_parse() {
        // The tests run on macOS, so the sysctls must answer.
        let product = sysctl_string(c"kern.osproductversion").unwrap();
        assert!(Version::parse(&product).is_some(), "{product:?}");
        let release = sysctl_string(c"kern.osrelease").unwrap();
        assert!(Version::parse(&release).is_some(), "{release:?}");
        assert!(sysctl_int(c"kern.system_version_compat").is_some());
        assert_ne!(host().name, "");
    }
}
