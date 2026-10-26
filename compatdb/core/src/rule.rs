//! A rule: the processes it matches and the settings it applies to them.

use crate::{D3d9, Dxgi, OpenGl, Scalar, ScalarValue, VersionInfo, Wined3d};
#[cfg(doc)]
use crate::{ENV_VAR, Origin, Table};

/// The exe pattern that matches every process.
pub const ANY_EXE: &str = "*";

/// Where a rule comes from. It decides how the log names the rule's values
/// (see [`Origin`]) and whether its `env` entries are defaults (see
/// [`Rule::env`]). Never serialized.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// Compiled into `compatdb.so`, which is what [`Rule::default`] gives.
    #[default]
    BuiltIn,
    /// Line `n` of [`ENV_VAR`], counted from 1 as [`Table::parse`] reads
    /// the value, blank lines and a legacy header included.
    Line(usize),
}

/// One rule of the table: what it matches, and the settings it applies to a
/// process that matches. A rule has no name; `exe` is its only required
/// matcher, and everything else is optional.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Image basename to match, case-insensitively, or [`ANY_EXE`] for every
    /// process.
    pub exe: String,
    /// Substring the image's version `CompanyName` must contain.
    pub company: Option<String>,
    /// Substring the image's version `ProductName` must contain.
    pub product: Option<String>,
    /// Substring the image's version `OriginalFilename` must contain.
    pub original_filename: Option<String>,
    /// The DXGI-family implementation to load.
    pub dxgi: Option<Dxgi>,
    /// The D3D9 implementation to load.
    pub d3d9: Option<D3d9>,
    /// The renderer of wined3d, for every wined3d user in the process.
    pub wined3d: Option<Wined3d>,
    /// Which OpenGL the Mac driver gives the process. Unset leaves the
    /// decision to a rule folded earlier; with none, the Mac driver gives
    /// Apple's OpenGL, as upstream Wine does.
    pub opengl: Option<OpenGl>,
    /// Whether the process is DPI-aware (system-aware) or unaware. Unset
    /// leaves the decision to a rule folded earlier, and with none to wine
    /// (the registry and the manifest).
    pub dpi_aware: Option<bool>,
    /// Whether a 32-bit process gets the 4 GB address space whatever its
    /// executable's `IMAGE_FILE_LARGE_ADDRESS_AWARE` flag says. `false` does
    /// not take the space away from an executable that has the flag; it only
    /// stops forcing it. Has no effect on a 64-bit process.
    pub large_address_aware: Option<bool>,
    /// Whether the Mac driver takes the mouse movement of raw input
    /// (`WM_INPUT`) and `DirectInput` from `GameController`'s `GCMouse`, in the
    /// device's own counts, instead of from the cursor's movement, which macOS
    /// accelerates and scales for retina. Unset leaves the decision to a rule
    /// folded earlier; with none, the Mac driver keeps upstream Wine's
    /// behaviour, the cursor's movement.
    pub gcmouse: Option<bool>,
    /// Whether no-execute stays permanently on in a 32-bit process, even when
    /// its executable or a DLL lacks the `NX_COMPAT` flag, for which Wine
    /// turns it off. `false` is the same as unset: Wine's behaviour. Has no
    /// effect on a 64-bit process, which always has no-execute on. Unlike an
    /// `env` entry, the value is the process's own: the processes it starts
    /// get their own rules' value.
    pub force_nx: Option<bool>,
    /// The keys of the scalar settings (those of [`Scalar::ALL`]) this rule
    /// sets to `default`: from this rule on the setting is unset again, so
    /// upstream Wine decides unless a rule folded later sets it. A key is
    /// either here or has a value in its field, never both.
    pub resets: Vec<&'static str>,
    /// `WINEDLLOVERRIDES` elements (each a `names=order` string). An entry
    /// whose order is `default` sets no load order: it removes the entries
    /// that the rules folded before it gave its modules, so the registry and
    /// `WINEDLLOVERRIDES` decide for them again.
    pub dll_overrides: Vec<String>,
    /// Text appended to the command line, unless already present.
    pub arguments: Vec<String>,
    /// When set, the arguments that replace the list the rules folded before
    /// this one accumulated (see [`Table::resolve`]); `Some` with no entries
    /// clears it. This rule's own [`Rule::arguments`] and those of the rules
    /// folded after it still append.
    pub arguments_override: Option<Vec<String>>,
    /// Environment entries; an empty value removes the variable. The library
    /// sets them in the process's environment block and unix environment
    /// when it loads, before any PE code runs. The entries of a built-in
    /// rule that matches every process (`exe` [`ANY_EXE`], no fingerprint)
    /// are defaults: such an entry sets its variable only where the process
    /// does not have it, so a value the caller exported wins. Every other
    /// entry replaces the variable.
    pub env: Vec<(String, String)>,
    /// `WINEDEBUG` channels for this process. Applied by the launcher through
    /// wine's own `WINEDEBUG=<exe>:` prefix, not by the library, because the
    /// channels are parsed before it loads; never serialized.
    pub debug_channels: Vec<String>,
    /// Where the rule comes from. Never serialized: [`Table::parse`] sets
    /// it to the line it read the rule from.
    pub source: Source,
}

/// The value of a scalar setting that resets it (see [`Rule::resets`]), and
/// of a `dll_overrides` entry that removes the overrides of its modules.
///
/// Like every word of the format it is matched ASCII-case-insensitively and
/// written in lowercase.
pub const DEFAULT_VALUE: &str = "default";

impl Default for Rule {
    fn default() -> Self {
        Self {
            exe: String::new(),
            company: None,
            product: None,
            original_filename: None,
            dxgi: None,
            d3d9: None,
            wined3d: None,
            opengl: None,
            dpi_aware: None,
            large_address_aware: None,
            gcmouse: None,
            force_nx: None,
            resets: Vec::new(),
            dll_overrides: Vec::new(),
            arguments: Vec::new(),
            arguments_override: None,
            env: Vec::new(),
            debug_channels: Vec::new(),
            source: Source::BuiltIn,
        }
    }
}

impl Rule {
    /// The value this rule sets for the scalar setting `scalar`, or `None`
    /// when it sets none.
    #[must_use]
    pub const fn scalar(&self, scalar: Scalar) -> Option<ScalarValue> {
        match scalar {
            Scalar::Dxgi => match self.dxgi {
                Some(value) => Some(ScalarValue::Dxgi(value)),
                None => None,
            },
            Scalar::D3d9 => match self.d3d9 {
                Some(value) => Some(ScalarValue::D3d9(value)),
                None => None,
            },
            Scalar::Wined3d => match self.wined3d {
                Some(value) => Some(ScalarValue::Wined3d(value)),
                None => None,
            },
            Scalar::OpenGl => match self.opengl {
                Some(value) => Some(ScalarValue::OpenGl(value)),
                None => None,
            },
            Scalar::DpiAware => bool_value(self.dpi_aware),
            Scalar::LargeAddressAware => bool_value(self.large_address_aware),
            Scalar::Gcmouse => bool_value(self.gcmouse),
            Scalar::ForceNx => bool_value(self.force_nx),
        }
    }

    /// Set the scalar setting that `value` is a value of, `scalar`, and
    /// take it out of [`Rule::resets`]; `None` resets it to `default`. A
    /// value of another setting leaves the rule as it is.
    pub(crate) fn put_scalar(&mut self, scalar: Scalar, value: Option<ScalarValue>) {
        match (scalar, value) {
            (Scalar::Dxgi, Some(ScalarValue::Dxgi(value))) => self.dxgi = Some(value),
            (Scalar::D3d9, Some(ScalarValue::D3d9(value))) => self.d3d9 = Some(value),
            (Scalar::Wined3d, Some(ScalarValue::Wined3d(value))) => self.wined3d = Some(value),
            (Scalar::OpenGl, Some(ScalarValue::OpenGl(value))) => self.opengl = Some(value),
            (Scalar::DpiAware, Some(ScalarValue::Bool(on))) => self.dpi_aware = Some(on),
            (Scalar::LargeAddressAware, Some(ScalarValue::Bool(on))) => {
                self.large_address_aware = Some(on);
            }
            (Scalar::Gcmouse, Some(ScalarValue::Bool(on))) => self.gcmouse = Some(on),
            (Scalar::ForceNx, Some(ScalarValue::Bool(on))) => self.force_nx = Some(on),
            (_, Some(_)) => return,
            (Scalar::Dxgi, None) => self.dxgi = None,
            (Scalar::D3d9, None) => self.d3d9 = None,
            (Scalar::Wined3d, None) => self.wined3d = None,
            (Scalar::OpenGl, None) => self.opengl = None,
            (Scalar::DpiAware, None) => self.dpi_aware = None,
            (Scalar::LargeAddressAware, None) => self.large_address_aware = None,
            (Scalar::Gcmouse, None) => self.gcmouse = None,
            (Scalar::ForceNx, None) => self.force_nx = None,
        }
        let key = scalar.as_str();
        self.resets.retain(|k| *k != key);
        if value.is_none() {
            self.resets.push(key);
        }
    }

    /// Whether this rule applies to a process with the given image basename
    /// and version resource. `exe` matches the basename case-insensitively, or
    /// every basename when it is [`ANY_EXE`]; a fingerprint field, when
    /// present, is an additional case-insensitive substring constraint on the
    /// corresponding version string (so a rule with none matches on the
    /// basename alone).
    #[must_use]
    pub fn matches(&self, exe: &str, version: &VersionInfo) -> bool {
        (self.exe == ANY_EXE || exe.eq_ignore_ascii_case(&self.exe))
            && field_matches(self.company.as_deref(), &version.company)
            && field_matches(self.product.as_deref(), &version.product)
            && field_matches(
                self.original_filename.as_deref(),
                &version.original_filename,
            )
    }

    /// How specific this rule is, which is the order [`Table::resolve`] folds
    /// the matching rules of a layer in: a wildcard rule first (a launch-wide
    /// default), then one naming an executable, then one that also pins a
    /// version fingerprint. Within a layer a more specific rule therefore
    /// wins a scalar field.
    #[must_use]
    pub fn specificity(&self) -> u8 {
        let fingerprinted =
            self.company.is_some() || self.product.is_some() || self.original_filename.is_some();
        match (self.exe == ANY_EXE, fingerprinted) {
            (true, false) => 0,
            (true, true) => 1,
            (false, false) => 2,
            (false, true) => 3,
        }
    }

    /// Whether this rule matches on its executable alone. Such a rule catches
    /// every program with that file name, which is worth a warning for all but
    /// the few basenames that are unique in practice.
    #[must_use]
    pub fn is_unfingerprinted(&self) -> bool {
        self.specificity() == 2
    }

    /// Whether two rules would match exactly the same processes, which can only
    /// be a mistake in a hand-written table.
    #[must_use]
    pub fn matches_the_same_as(&self, other: &Self) -> bool {
        self.exe.eq_ignore_ascii_case(&other.exe)
            && self.company == other.company
            && self.product == other.product
            && self.original_filename == other.original_filename
    }

    /// Whether this rule's `env` entries are defaults, which leave a variable
    /// the process already has alone: those of a built-in rule that matches
    /// every process (see [`Rule::env`]).
    #[must_use]
    pub fn env_is_default(&self) -> bool {
        self.source == Source::BuiltIn && self.specificity() == 0
    }

    /// The matchers of this rule as the log prints them: the `exe`, then the
    /// fingerprint values it pins in parentheses, for example
    /// `WoW.exe (Blizzard Entertainment, World of Warcraft, WoW.exe)`.
    #[must_use]
    pub fn matchers(&self) -> String {
        let pins: Vec<&str> = [&self.company, &self.product, &self.original_filename]
            .into_iter()
            .filter_map(Option::as_deref)
            .collect();
        if pins.is_empty() {
            self.exe.clone()
        } else {
            format!("{} ({})", self.exe, pins.join(", "))
        }
    }
}

/// The value of a true-or-false setting a rule sets.
const fn bool_value(value: Option<bool>) -> Option<ScalarValue> {
    match value {
        Some(on) => Some(ScalarValue::Bool(on)),
        None => None,
    }
}

/// A fingerprint constraint against an actual version string: an absent
/// constraint matches anything, a present one must be an ASCII-case-insensitive
/// substring.
fn field_matches(constraint: Option<&str>, actual: &str) -> bool {
    constraint.is_none_or(|want| {
        actual
            .to_ascii_lowercase()
            .contains(&want.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests;
