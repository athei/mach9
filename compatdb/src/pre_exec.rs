//! Resolve a child's app name without applying any of its runtime settings.

use std::{
    ffi::{CStr, OsStr},
    mem::MaybeUninit,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use compatdb_core::{
    Table, VersionInfo,
    runtime::{basename_of, steam_app, steam_app_id},
};

use crate::{RULE_SOURCES, load_table, ntdll::Ntdll, version};

/// Look up a name before forking a Windows child. Use the target's
/// environment: a game launched by Steam can have a
/// `SteamAppId` that the Steam client itself does not. This lookup must not
/// apply the target's settings to the process asking for its name.
///
/// Counts are UTF-16 code units, not bytes. The counted DOS image path
/// excludes NUL; the environment includes its final empty entry (an empty
/// environment may also be passed as null with length zero).
///
/// Return the number of UTF-8 bytes written, excluding the terminating NUL,
/// or zero without writing on no match, invalid input, or insufficient space.
/// The result is display text, not a path component.
///
/// # Safety
/// Nonempty input regions must be readable, initialized, aligned and valid for
/// their counts. The output must be writable for its count and not overlap
/// either input.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compatdb_resolve_display_name(
    image: *const u16,
    image_len: usize,
    env: *const u16,
    env_len: usize,
    out: *mut libc::c_char,
    out_size: usize,
) -> usize {
    if image_len == 0
        || image.is_null()
        || !image.is_aligned()
        || image_len > isize::MAX as usize / size_of::<u16>()
        || env_len > isize::MAX as usize / size_of::<u16>()
        || (env_len != 0 && (env.is_null() || !env.is_aligned()))
        || out.is_null()
        || out_size == 0
        || out_size > isize::MAX as usize
    {
        return 0;
    }
    // SAFETY: the caller provides valid regions; checked the slice size,
    // nullness and alignment above. A zero-length env need not have a pointer.
    let (image, env) = unsafe {
        (
            std::slice::from_raw_parts(image, image_len),
            if env_len == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(env, env_len)
            },
        )
    };
    if image.contains(&0) || !valid_env(env) {
        return 0;
    }
    let exe = basename_of(image);
    if exe.is_empty() {
        return 0;
    }

    // SAFETY: output region and its bound were checked above.
    let output = unsafe { std::slice::from_raw_parts_mut(out.cast(), out_size) };
    lookup(&exe, steam_app(env), || image_version(image), output)
}

/// Look up a direct Unix launch before Wine has a PEB, a Windows environment,
/// or a server connection. Never call ntdll path conversion from this entry point.
///
/// # Safety
/// `image` (argv[1]) and `prefix` are NUL-terminated Unix strings. `appid` is
/// either NULL or the NUL-terminated value of `SteamAppId` from the Unix launch
/// environment. `out` must be writable for `out_size` bytes, disjoint from the
/// inputs. Returns UTF-8 bytes excluding NUL, or zero without writing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compatdb_resolve_initial_display_name(
    image: *const libc::c_char,
    prefix: *const libc::c_char,
    appid: *const libc::c_char,
    out: *mut libc::c_char,
    out_size: usize,
) -> usize {
    if image.is_null()
        || prefix.is_null()
        || out.is_null()
        || out_size == 0
        || out_size > isize::MAX as usize
    {
        return 0;
    }
    // SAFETY: the caller guarantees NUL termination and writable disjoint output.
    let (image, prefix, appid, output) = unsafe {
        (
            CStr::from_ptr(image).to_bytes(),
            CStr::from_ptr(prefix).to_bytes(),
            (!appid.is_null()).then(|| CStr::from_ptr(appid)),
            std::slice::from_raw_parts_mut(out.cast(), out_size),
        )
    };
    let Some(exe) = image
        .rsplit(|c| matches!(c, b'/' | b'\\'))
        .next()
        .filter(|s| !s.is_empty())
    else {
        return 0;
    };
    let Ok(exe) = std::str::from_utf8(exe) else {
        return 0;
    };
    let appid = appid.and_then(|s| s.to_str().ok()).and_then(steam_app_id);
    lookup(
        exe,
        appid,
        || version::read_file(&initial_image_path(image, prefix)?),
        output,
    )
}

// Resolve only paths whose host spelling is known without querying Wine.
// Don't guess fingerprints for drive-relative, device or UNC paths.
fn initial_image_path(image: &[u8], prefix: &[u8]) -> Option<PathBuf> {
    if image.starts_with(b"/") {
        return Some(PathBuf::from(OsStr::from_bytes(image)));
    }
    let normalized: Vec<_> = image
        .iter()
        .map(|&b| if b == b'\\' { b'/' } else { b })
        .collect();
    match normalized.as_slice() {
        [drive, b':', b'/', rest @ ..]
            if drive.is_ascii_alphabetic() && !rest.starts_with(b"/") =>
        {
            Some(
                Path::new(OsStr::from_bytes(prefix))
                    .join("dosdevices")
                    .join(OsStr::from_bytes(&[drive.to_ascii_lowercase(), b':']))
                    .join(OsStr::from_bytes(rest)),
            )
        }
        [b'/', ..] | [_, b':', ..] => None,
        _ => Some(PathBuf::from(OsStr::from_bytes(&normalized))),
    }
}

fn lookup(
    exe: &str,
    steam: Option<u32>,
    image_version: impl FnOnce() -> Option<VersionInfo>,
    output: &mut [MaybeUninit<u8>],
) -> usize {
    let Some(sources) = RULE_SOURCES.get() else {
        return 0;
    };
    let (table, _) = load_table(sources);
    let Some(resolution) = resolve(&table, exe, steam, image_version) else {
        return 0;
    };
    let Some(name) = resolution
        .display_name()
        .filter(|name| name.len() < output.len())
    else {
        return 0;
    };
    let Some((bytes, tail)) = output.split_at_mut_checked(name.len()) else {
        return 0;
    };
    let Some(nul) = tail.first_mut() else {
        return 0;
    };
    bytes.write_copy_of_slice(name.as_bytes());
    nul.write(0);
    name.len()
}

fn valid_env(env: &[u16]) -> bool {
    match env {
        [] | [0] | [0, 0] => true,
        [entries @ .., 0, 0] => {
            entries.first() != Some(&0)
                && entries.last() != Some(&0)
                && !entries.windows(2).any(|pair| pair == [0, 0])
        }
        _ => false,
    }
}

fn resolve(
    table: &Table,
    exe: &str,
    steam: Option<u32>,
    image_version: impl FnOnce() -> Option<VersionInfo>,
) -> Option<compatdb_core::Resolution> {
    let mut naming_rules = table
        .rules()
        .iter()
        .filter(|rule| !rule.display_name().is_unset() && rule.exe().matches(exe, steam));
    let first = naming_rules.next()?;
    let version =
        if first.has_fingerprint() || naming_rules.any(compatdb_core::Rule::has_fingerprint) {
            image_version()
        } else {
            None
        };
    Some(table.resolve_with_optional_version(exe, version.as_ref(), steam))
}

fn image_version(image: &[u16]) -> Option<VersionInfo> {
    let path = Ntdll::resolve()?.unix_file_name(image).ok()?;
    version::read_file(Path::new(OsStr::from_bytes(&path)))
}

#[cfg(test)]
mod tests;
