//! The table of per-process rules behind `compatdb.so`.
//!
//! A [`Rule`] says which processes it matches and which settings it applies to
//! them: the Direct3D implementations to load (one for D3D9, one for the DXGI
//! family) and the renderer of wined3d, DLL load-order overrides, the DPI
//! awareness, large address awareness, whether the Mac driver reads raw mouse
//! input from `GameController`, command-line switches and environment entries.
//!
//! Shared by `compatdb.so`, the unix library wine's ntdll loads into every
//! process (it holds the built-in rules, parses [`ENV_VAR`], overlays the
//! result onto them and applies whatever matches the process), and by any
//! launcher that wants to feed it extra rules through that variable.
//!
//! Everything here is pure: it works on `&str`, `&[u16]` (UTF-16 as wine hands
//! it over) and owned values, with no unsafe and no I/O, so the library's
//! decision logic is all unit-tested on the host and the `.so` itself only has
//! to move bytes in and out of the process.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::fmt::Write as _;

/// The environment variable `compatdb.so` reads extra rules from. Wine itself
/// never looks at it; it is only the channel from whatever started the process
/// tree to the library.
pub const ENV_VAR: &str = "WINE_COMPATDB";

/// Whether `line` is a version header of the old format, `v=` and nothing
/// but ASCII digits, which launchers wrote as the first line of a table.
///
/// [`Table::parse`] skips such a first line. It can never be a rule: a rule
/// needs a `name` field, and this line has only `v`.
fn is_legacy_header(line: &str) -> bool {
    line.strip_prefix("v=")
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// The parts of a PE version resource the database can match on. Vendor-set at
/// link time, so stable across install location and patches.
///
/// A field is an empty string when the image carries no version resource or
/// the resource leaves that field empty.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct VersionInfo {
    pub company: String,
    pub product: String,
    pub original_filename: String,
}

impl VersionInfo {
    /// Extract the standard fields from a `VS_VERSIONINFO` blob (the bytes of an
    /// `RT_VERSION` resource, as UTF-16 units). The blob is walked as the tree
    /// it is: the root, its `StringFileInfo` children, their string tables and
    /// the `String` entries in them. A field takes the value of the first entry
    /// with its key, in table order; an entry whose value is empty, or a key no
    /// table has, leaves the field empty, so a rule that pins it cannot match.
    #[must_use]
    pub fn from_resource(blob: &[u16]) -> Self {
        Self {
            company: string_value(blob, "CompanyName"),
            product: string_value(blob, "ProductName"),
            original_filename: string_value(blob, "OriginalFilename"),
        }
    }
}

/// One node of a `VS_VERSIONINFO` tree. Every node, the root, the containers
/// and the `String` entries alike, is `wLength` (bytes, children included),
/// `wValueLength`, `wType`, a NUL-terminated UTF-16 key, padding to a 32-bit
/// boundary, the value, padding again, and then the children.
struct VersionNode<'a> {
    key: &'a [u16],
    /// From the start of the value to the end of the node.
    value: &'a [u16],
    /// The children, as the units that hold them.
    children: &'a [u16],
}

impl<'a> VersionNode<'a> {
    /// Parse the node at the start of `units` and return it with the units
    /// after it, where its next sibling starts. `units` has to start on a
    /// 32-bit boundary, which every node does. A node whose length is too
    /// short for its header or whose key is not NUL-terminated ends the walk;
    /// one that claims more than `units` holds is cut off at the end.
    fn parse(units: &'a [u16]) -> Option<(Self, &'a [u16])> {
        let length = usize::from(*units.first()?).div_ceil(2);
        let value_length = usize::from(*units.get(1)?);
        // Any nonzero wType is text, as in Wine's VerQueryValue.
        let text = *units.get(2)? != 0;
        if length < 3 {
            return None;
        }
        let node = units.get(..length).unwrap_or(units);
        let key_text = node.get(3..)?;
        let key_end = key_text.iter().position(|&u| u == 0)?;
        let key = key_text.get(..key_end)?;
        // The key starts 3 units in, and its NUL is one more.
        let value_at = key_end.checked_add(4)?.checked_next_multiple_of(2)?;
        let value = node.get(value_at..).unwrap_or_default();
        // wValueLength counts characters for a text value and bytes for a
        // binary one; the children start at the next 32-bit boundary after it.
        let value_units = if text {
            value_length
        } else {
            value_length.div_ceil(2)
        };
        let children_at = value_at
            .checked_add(value_units)?
            .checked_next_multiple_of(2)?;
        let children = node.get(children_at..).unwrap_or_default();
        let next = length.checked_next_multiple_of(2)?;
        let rest = units.get(next..).unwrap_or_default();
        Some((
            Self {
                key,
                value,
                children,
            },
            rest,
        ))
    }

    /// The nodes in a run of sibling units, in order.
    fn siblings(mut units: &'a [u16]) -> impl Iterator<Item = Self> {
        std::iter::from_fn(move || {
            let (node, rest) = Self::parse(units)?;
            units = rest;
            Some(node)
        })
    }

    /// Whether the key is `want`, compared ASCII-case-insensitively as
    /// `VerQueryValue` does.
    fn key_is(&self, want: &str) -> bool {
        self.key
            .iter()
            .map(|&u| to_lower_u16(u))
            .eq(want.encode_utf16().map(to_lower_u16))
    }
}

/// The value of the first `String` entry with this key in any string table of
/// the blob, empty when there is none or its value is empty.
///
/// The value is read up to its NUL within the entry rather than by
/// `wValueLength`, which resource compilers do not all fill in the same way.
fn string_value(blob: &[u16], key: &str) -> String {
    let Some((root, _)) = VersionNode::parse(blob) else {
        return String::new();
    };
    if !root.key_is("VS_VERSION_INFO") {
        return String::new();
    }
    VersionNode::siblings(root.children)
        .filter(|info| info.key_is("StringFileInfo"))
        .flat_map(|info| VersionNode::siblings(info.children))
        .flat_map(|table| VersionNode::siblings(table.children))
        .find(|entry| entry.key_is(key))
        .and_then(|entry| entry.value.split(|&u| u == 0).next())
        .map(String::from_utf16_lossy)
        .unwrap_or_default()
}

/// Which implementation of the DXGI family a process loads.
///
/// D3D10, D3D10.1, D3D11 and D3D12 travel together because they all create
/// their device through one `dxgi.dll`, and each implementation's modules only
/// work with its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dxgi {
    /// DXMT, the default for every program.
    Dxmt,
    /// Wine's own wined3d.
    Wined3d,
}

impl Dxgi {
    /// Parse a wire/TOML value, case-insensitively.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "dxmt" => Some(Self::Dxmt),
            "wined3d" => Some(Self::Wined3d),
            _ => None,
        }
    }

    /// The wire/TOML spelling, which is also the `lib/wine/dxgi/<name>` tree.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dxmt => "dxmt",
            Self::Wined3d => "wined3d",
        }
    }

    /// What a process gets when no rule names an implementation: DXMT, for
    /// every arch, a native ARM64 program included. Either tree serves every
    /// arch: the 64-bit modules of both are ARM64X, so a native ARM64 program
    /// loads their ARM64 half and an x64 program, which runs as ARM64EC,
    /// their ARM64EC half; an i386 program gets their i386 builds under
    /// emulation.
    #[must_use]
    pub const fn default_for(_arch: Arch) -> Self {
        Self::Dxmt
    }

    /// The implementation a process loads: the requested one, or the default
    /// for the arch when nothing was requested.
    #[must_use]
    pub const fn effective(requested: Option<Self>, arch: Arch) -> Self {
        match requested {
            Some(want) => want,
            None => Self::default_for(arch),
        }
    }
}

/// Which D3D9 implementation a process loads. Independent of [`Dxgi`]: neither
/// implementation's `d3d9.dll` touches `dxgi.dll`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum D3d9 {
    /// mtld3d, the default for every program.
    Mtld3d,
    /// Wine's own wined3d.
    Wined3d,
}

impl D3d9 {
    /// Parse a wire/TOML value, case-insensitively.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "mtld3d" => Some(Self::Mtld3d),
            "wined3d" => Some(Self::Wined3d),
            _ => None,
        }
    }

    /// The wire/TOML spelling, which is also the `lib/wine/d3d9/<name>` tree.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mtld3d => "mtld3d",
            Self::Wined3d => "wined3d",
        }
    }

    /// What a process gets when no rule names an implementation: mtld3d, for
    /// every arch, a native ARM64 program included. Either tree serves every
    /// arch: the 64-bit modules of both, mtld3d's release build and Wine's
    /// own `d3d9.dll`, are ARM64X, so a native ARM64 program loads their
    /// ARM64 half and an x64 program, which runs as ARM64EC, their ARM64EC
    /// half; an i386 program gets their i386 builds under emulation.
    #[must_use]
    pub const fn default_for(_arch: Arch) -> Self {
        Self::Mtld3d
    }

    /// The implementation a process loads: the requested one, or the default
    /// for the arch when nothing was requested.
    #[must_use]
    pub const fn effective(requested: Option<Self>, arch: Arch) -> Self {
        match requested {
            Some(want) => want,
            None => Self::default_for(arch),
        }
    }
}

/// Which renderer wined3d, Wine's own Direct3D, draws with in a process.
///
/// One `wined3d.dll` serves every wined3d user in a process (the wined3d trees
/// of [`Dxgi`] and [`D3d9`], D3D8 and `DirectDraw`), and it reads its settings
/// once, so this is a setting of the process, not of a family. It chooses no
/// tree: [`Dxgi`] and [`D3d9`] still decide whether wined3d is used at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wined3d {
    /// OpenGL, wined3d's own default.
    Gl,
    /// Vulkan, on the bundle's `KosmicKrisp`.
    Vulkan,
}

impl Wined3d {
    /// The environment variable wined3d reads its settings from, a list of
    /// `name=value` options.
    pub const CONFIG_VAR: &'static str = "WINE_D3D_CONFIG";

    /// Parse a wire/TOML value, case-insensitively.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "gl" => Some(Self::Gl),
            "vulkan" => Some(Self::Vulkan),
            _ => None,
        }
    }

    /// The wire/TOML spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gl => "gl",
            Self::Vulkan => "vulkan",
        }
    }

    /// The `WINE_D3D_CONFIG` value for this renderer on top of `config`, the
    /// value the rules' own `env` entries leave the variable at (`None` when
    /// they do not set it, or remove it). The renderer comes first and the
    /// other options of `config` are kept, while a `renderer` option there is
    /// dropped: this setting decides the renderer. OpenGL is set as
    /// explicitly as Vulkan, so a process that asks for it does not inherit
    /// Vulkan from the process that started it.
    #[must_use]
    pub fn config(self, config: Option<&str>) -> String {
        let mut options = vec![format!("renderer={}", self.as_str())];
        options.extend(
            config
                .unwrap_or("")
                .split([',', ';'])
                .filter(|o| !o.is_empty())
                .filter(|o| o.split('=').next() != Some("renderer"))
                .map(String::from),
        );
        options.join(",")
    }

    /// The `renderer` option of a `WINE_D3D_CONFIG` value, the first one as
    /// wined3d reads it, or `None` when the value has none.
    #[must_use]
    pub fn renderer_in(config: &str) -> Option<&str> {
        config
            .split([',', ';'])
            .find_map(|o| o.strip_prefix("renderer="))
    }
}

/// The architecture of the program a process runs, which is that of its main
/// executable.
///
/// Wine itself, the unix side with `ntdll.so` and this library, is built for
/// arm64 and runs natively; x86 code is emulated inside the process.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arch {
    /// A 64-bit x86 program. It runs in an ARM64EC process with its x86 code
    /// emulated.
    X86_64,
    /// A 32-bit x86 program, run through wow64 with its x86 code emulated.
    I386,
    /// A native ARM64 program.
    Aarch64,
}

impl Arch {
    /// The `<arch>-windows` builtin directory name. The builtins of an
    /// `x86_64` process load from `aarch64-windows`, as upstream Wine does on
    /// an arm64 host: they are ARM64X modules that serve the process's
    /// ARM64EC side.
    #[must_use]
    pub const fn pe_dir(self) -> &'static str {
        match self {
            Self::I386 => "i386-windows",
            Self::X86_64 | Self::Aarch64 => "aarch64-windows",
        }
    }

    /// The arch of the current process. `wow64` says whether it is a wow64
    /// (32-bit) process; `machine` is the `Machine` field of its main image's
    /// COFF header, when it could be read.
    ///
    /// The bundle only runs x86 programs under wow64, so a wow64 process is
    /// i386. A 64-bit process is always native from Wine's side (ARM64EC for
    /// an `x86_64` program), so the main image decides:
    /// `IMAGE_FILE_MACHINE_ARM64` is a native ARM64 program, and anything
    /// else, an unreadable header included, counts as `x86_64`. An ARM64EC
    /// executable carries the AMD64 machine in its header and so also counts
    /// as `x86_64`, which is what it presents itself as.
    #[must_use]
    pub const fn of_process(wow64: bool, machine: Option<u16>) -> Self {
        match (wow64, machine) {
            (true, _) => Self::I386,
            (false, Some(IMAGE_FILE_MACHINE_ARM64)) => Self::Aarch64,
            _ => Self::X86_64,
        }
    }
}

/// The COFF `Machine` value of an ARM64 image.
pub const IMAGE_FILE_MACHINE_ARM64: u16 = 0xaa64;

/// The exe pattern that matches every process.
pub const ANY_EXE: &str = "*";

/// One rule of the table: what it matches, and the settings it applies to a
/// process that matches. `name` is its identity and `exe` its only required
/// matcher; everything else is optional.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The rule's name, unique within a table. Carries no meaning beyond
    /// identity: it is what a game file names to extend, override or drop a
    /// rule, and what the log prints. Matched case-insensitively.
    pub name: String,
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
    /// Whether the process is DPI-aware (system-aware) or unaware. Unset
    /// leaves the decision to a less specific rule, and with none to wine
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
    /// accelerates and scales for retina. Unset leaves the decision to a less
    /// specific rule; with none, the Mac driver keeps upstream Wine's
    /// behaviour, the cursor's movement.
    pub gcmouse: Option<bool>,
    /// `WINEDLLOVERRIDES` elements (each a `names=order` string).
    pub dll_overrides: Vec<String>,
    /// Text appended to the command line, unless already present.
    pub arguments: Vec<String>,
    /// Environment entries; an empty value removes the variable. The library
    /// sets them in the process's environment block and unix environment
    /// when it loads, before any PE code runs.
    pub env: Vec<(String, String)>,
    /// `WINEDEBUG` channels for this process. Applied by the launcher through
    /// wine's own `WINEDEBUG=<exe>:` prefix, not by the library, because the
    /// channels are parsed before it loads; never serialized.
    pub debug_channels: Vec<String>,
    /// False marks an override that drops the rule of this name. Default true;
    /// only travels on the wire when false.
    pub enabled: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            name: String::new(),
            exe: String::new(),
            company: None,
            product: None,
            original_filename: None,
            dxgi: None,
            d3d9: None,
            wined3d: None,
            dpi_aware: None,
            large_address_aware: None,
            gcmouse: None,
            dll_overrides: Vec::new(),
            arguments: Vec::new(),
            env: Vec::new(),
            debug_channels: Vec::new(),
            enabled: true,
        }
    }
}

impl Rule {
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
    /// matching rules in: a wildcard rule first (a launch-wide default), then
    /// one naming an executable, then one that also pins a version
    /// fingerprint. A more specific rule therefore wins a scalar field.
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

    /// Lay an override on top of this rule: a set scalar field wins, lists are
    /// appended (this rule's own entries first), env entries are appended (a
    /// later duplicate name wins when the block is merged).
    fn overlay(&mut self, over: Self) {
        if over.company.is_some() {
            self.company = over.company;
        }
        if over.product.is_some() {
            self.product = over.product;
        }
        if over.original_filename.is_some() {
            self.original_filename = over.original_filename;
        }
        if over.dxgi.is_some() {
            self.dxgi = over.dxgi;
        }
        if over.d3d9.is_some() {
            self.d3d9 = over.d3d9;
        }
        if over.wined3d.is_some() {
            self.wined3d = over.wined3d;
        }
        if over.dpi_aware.is_some() {
            self.dpi_aware = over.dpi_aware;
        }
        if over.large_address_aware.is_some() {
            self.large_address_aware = over.large_address_aware;
        }
        if over.gcmouse.is_some() {
            self.gcmouse = over.gcmouse;
        }
        if !over.exe.is_empty() {
            self.exe = over.exe;
        }
        self.dll_overrides.extend(over.dll_overrides);
        self.arguments.extend(over.arguments);
        self.env.extend(over.env);
        self.debug_channels.extend(over.debug_channels);
    }
}

/// The whole database: the built-in rules plus whatever a game file added,
/// in the order they were declared.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Table {
    pub rules: Vec<Rule>,
}

/// The settings that apply to one process.
///
/// Every matching rule folded in specificity order, so a scalar holds the most
/// specific rule's value and the lists accumulate. Also names the rules that
/// matched, for the log.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub dxgi: Option<Dxgi>,
    pub d3d9: Option<D3d9>,
    pub wined3d: Option<Wined3d>,
    pub dpi_aware: Option<bool>,
    pub large_address_aware: Option<bool>,
    pub gcmouse: Option<bool>,
    pub dll_overrides: Vec<String>,
    pub arguments: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Names of the rules that matched, in the order they were folded.
    pub matched: Vec<String>,
}

impl Resolution {
    /// Whether there is nothing to apply.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.dxgi.is_none()
            && self.d3d9.is_none()
            && self.wined3d.is_none()
            && self.dpi_aware.is_none()
            && self.large_address_aware.is_none()
            && self.gcmouse.is_none()
            && self.dll_overrides.is_empty()
            && self.arguments.is_empty()
            && self.env.is_empty()
    }

    /// The value the rules' own `env` entries leave `WINE_D3D_CONFIG` at:
    /// the last entry for it, `Some("")` when that one removes it, and `None`
    /// when no rule sets it.
    #[must_use]
    pub fn d3d_config(&self) -> Option<&str> {
        self.env
            .iter()
            .rev()
            .find(|(name, _)| name.eq_ignore_ascii_case(Wined3d::CONFIG_VAR))
            .map(|(_, value)| value.as_str())
    }

    /// When a rule sets [`Resolution::wined3d`], that renderer, with the
    /// `WINE_D3D_CONFIG` that names it folded into [`Resolution::env`]. That
    /// entry goes after the rules' own entries, so it is the value the
    /// variable ends up with when the env is applied, and it keeps the other
    /// options a rule's `WINE_D3D_CONFIG` set (see [`Wined3d::config`]).
    /// `None`, with nothing changed, when no rule sets it: then the process
    /// keeps whatever `WINE_D3D_CONFIG` it has, from a rule's `env` or
    /// inherited, and without one wined3d uses OpenGL. Call it once, after
    /// [`Table::resolve`].
    pub fn apply_wined3d(&mut self) -> Option<Wined3d> {
        let wined3d = self.wined3d?;
        let value = wined3d.config(self.d3d_config().filter(|value| !value.is_empty()));
        self.env.push((Wined3d::CONFIG_VAR.to_string(), value));
        Some(wined3d)
    }
}

impl Table {
    /// Fold every matching rule into one [`Resolution`]. `exe` is the image
    /// basename (see [`basename_of`]); `version` is the image's version
    /// resource, for the optional fingerprint constraints.
    ///
    /// Rules are folded least specific first (see [`Rule::specificity`]), and
    /// within a tier in table order, which puts the built-ins before a game's.
    /// So a scalar field ends up holding the most specific rule's value and a
    /// rule naming a process always beats a launch-wide default, while the
    /// lists accumulate across every rule that matched.
    ///
    /// The sort has to stay stable: rules of equal specificity fold in table
    /// order, which is what lets a launcher's `*` rule, appended after the
    /// built-in `*` rules, win a scalar over them and add its list entries
    /// after theirs.
    ///
    /// A rule whose executable matches but whose fingerprint does not simply
    /// does not apply; nothing falls back to it, and the rules that do match
    /// still apply.
    #[must_use]
    pub fn resolve(&self, exe: &str, version: &VersionInfo) -> Resolution {
        let mut matching: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|rule| rule.matches(exe, version))
            .collect();
        matching.sort_by_key(|rule| rule.specificity());
        let mut r = Resolution::default();
        for rule in matching {
            if let Some(dxgi) = rule.dxgi {
                r.dxgi = Some(dxgi);
            }
            if let Some(d3d9) = rule.d3d9 {
                r.d3d9 = Some(d3d9);
            }
            if let Some(wined3d) = rule.wined3d {
                r.wined3d = Some(wined3d);
            }
            if let Some(aware) = rule.dpi_aware {
                r.dpi_aware = Some(aware);
            }
            if let Some(laa) = rule.large_address_aware {
                r.large_address_aware = Some(laa);
            }
            if let Some(gcmouse) = rule.gcmouse {
                r.gcmouse = Some(gcmouse);
            }
            r.dll_overrides.extend(rule.dll_overrides.iter().cloned());
            r.arguments.extend(rule.arguments.iter().cloned());
            r.env.extend(rule.env.iter().cloned());
            r.matched.push(rule.name.clone());
        }
        r
    }

    /// Lay a set of overrides on top of this table, matching by rule name.
    ///
    /// An override naming a rule that is already there is merged into it field
    /// by field (a set scalar wins, lists append), one naming a rule that is
    /// not is appended, and one marked `enabled = false` removes the rule of
    /// that name. Matching by name rather than by executable is what lets two
    /// rules name the same executable with different fingerprints, and what
    /// keeps a new rule from inheriting an unrelated rule's fingerprint.
    ///
    /// An override that names no rule here may only add a new one, so it has to
    /// bring an `exe` of its own. One that does not (an `enabled = false` for a
    /// rule that is gone, or a rule whose only home was a misspelled name) can
    /// never match a process, so it is dropped and its name returned for the
    /// caller to warn about.
    pub fn overlay(&mut self, overrides: Self) -> Vec<String> {
        let mut unmatched = Vec::new();
        for over in overrides.rules {
            let pos = self
                .rules
                .iter()
                .position(|rule| rule.name.eq_ignore_ascii_case(&over.name));
            match pos {
                Some(i) if !over.enabled => {
                    self.rules.remove(i);
                }
                Some(i) => {
                    if let Some(existing) = self.rules.get_mut(i) {
                        existing.overlay(over);
                    }
                }
                None if over.enabled && !over.exe.is_empty() => self.rules.push(over),
                None => unmatched.push(over.name),
            }
        }
        unmatched
    }

    /// Every pair of rules that matches exactly the same processes, by name.
    /// A duplicate can only be a mistake, so the launcher warns about it.
    ///
    /// Plain wildcard rules (`exe = *` with no fingerprint) are left out: they
    /// are defaults that are meant to stack, one per setting, so several of
    /// them matching every process is not a mistake.
    #[must_use]
    pub fn duplicate_matchers(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        for (i, rule) in self.rules.iter().enumerate() {
            if rule.specificity() == 0 {
                continue;
            }
            for other in self.rules.iter().skip(i.saturating_add(1)) {
                if rule.matches_the_same_as(other) {
                    pairs.push((rule.name.clone(), other.name.clone()));
                }
            }
        }
        pairs
    }

    /// Serialize the table into the value of [`ENV_VAR`]: one record per
    /// rule, one per line.
    #[must_use]
    pub fn to_env_value(&self) -> String {
        self.rules
            .iter()
            .map(rule_to_record)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Parse the value of [`ENV_VAR`]. Lenient, since the `.so` must never
    /// abort a process: a field with an unknown key or a value its key does
    /// not take is ignored with a diagnostic naming the rule and the field,
    /// and the rest of its record still applies. Only a record without a
    /// `name` is skipped, since an override cannot say which rule it is for
    /// without one. A first line that is a version header of the old format
    /// (`v=<digits>`) is skipped too, so a launcher that still writes one
    /// keeps working. Returns the table and the diagnostics (empty when
    /// clean).
    #[must_use]
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut lines = text.lines().peekable();
        lines.next_if(|line| is_legacy_header(line));
        let mut diagnostics = Vec::new();
        let mut rules = Vec::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            match record_to_rule(line) {
                Ok((rule, ignored)) => {
                    diagnostics.extend(
                        ignored
                            .into_iter()
                            .map(|field| format!("rule {:?}: ignoring {field}", rule.name)),
                    );
                    rules.push(rule);
                }
                Err(reason) => diagnostics.push(format!("skipping record {line:?}: {reason}")),
            }
        }
        (Self { rules }, diagnostics)
    }
}

/// The basename of a DOS image path (`...\Foo\Bar.exe` -> `Bar.exe`), decoded
/// lossily from UTF-16.
#[must_use]
pub fn basename_of(image_path: &[u16]) -> String {
    let start = image_path
        .iter()
        .rposition(|&u| u == u16::from(b'\\') || u == u16::from(b'/'))
        .map_or(0, |i| i.saturating_add(1));
    String::from_utf16_lossy(image_path.get(start..).unwrap_or(&[]))
}

/// Append each text to the command line unless it is already present.
///
/// The substring test is case-sensitive, the rule that leaves Chromium
/// `--type=` children, which inherit their parent's switches, untouched.
/// Returns `None` when nothing changed.
#[must_use]
pub fn append_cmdline(cmdline: &[u16], appends: &[String]) -> Option<Vec<u16>> {
    let mut cur = cmdline.to_vec();
    let mut changed = false;
    for text in appends {
        if text.is_empty() || contains_cs(&cur, text) {
            continue;
        }
        cur.push(u16::from(b' '));
        cur.extend(text.encode_utf16());
        changed = true;
    }
    changed.then_some(cur)
}

/// The value of the variable `name` in a `NAME=value\0...\0\0` block, the
/// first entry for it, matched case-insensitively and decoded lossily; `None`
/// when the block has none.
#[must_use]
pub fn env_value(block: &[u16], name: &str) -> Option<String> {
    let seg = split_env(block)
        .into_iter()
        .find(|seg| env_name_matches(seg, name))?;
    let value = seg
        .iter()
        .position(|&u| u == u16::from(b'='))
        .and_then(|i| seg.get(i.saturating_add(1)..))
        .unwrap_or(&[]);
    Some(String::from_utf16_lossy(value))
}

/// Merge environment entries into a `NAME=value\0...\0\0` block.
///
/// A name is matched case-insensitively; a non-empty value replaces in place
/// or appends, an empty value removes. Drive entries (`=C:=...`) are left
/// alone. Returns `None` when nothing changed.
#[must_use]
pub fn merge_env(block: &[u16], entries: &[(String, String)]) -> Option<Vec<u16>> {
    let mut segs = split_env(block);
    let mut changed = false;
    for (name, value) in entries {
        let found = segs.iter().position(|seg| env_name_matches(seg, name));
        if value.is_empty() {
            if let Some(i) = found {
                segs.remove(i);
                changed = true;
            }
            continue;
        }
        let mut new_seg: Vec<u16> = name.encode_utf16().collect();
        new_seg.push(u16::from(b'='));
        new_seg.extend(value.encode_utf16());
        match found {
            Some(i) if segs.get(i) == Some(&new_seg) => {}
            Some(i) => {
                if let Some(slot) = segs.get_mut(i) {
                    *slot = new_seg;
                    changed = true;
                }
            }
            None => {
                segs.push(new_seg);
                changed = true;
            }
        }
    }
    if !changed {
        return None;
    }
    let mut out = Vec::new();
    for seg in &segs {
        out.extend_from_slice(seg);
        out.push(0);
    }
    out.push(0);
    Some(out)
}

// ── Wire format helpers ──────────────────────────────────────────────────

/// Serialize one rule. `debug_channels` is deliberately left out: whoever
/// starts wine applies those through `WINEDEBUG`, and the library would see
/// them too late.
///
/// A library that does not know a key added later ignores that field with
/// a diagnostic and applies the rest of the record. That is safe only for a
/// setting, so the matcher keys (`exe`, `company`, `product`,
/// `original_filename`) are a closed set and a new key is only ever a
/// setting: a library that ignored an unknown matcher would apply the rule
/// to more processes than it names.
fn rule_to_record(rule: &Rule) -> String {
    let mut fields: Vec<String> = vec![field("name", &rule.name)];
    if !rule.enabled {
        // A disabled rule is a directive to drop the rule of that name, so it
        // travels as the name and nothing else.
        fields.push(String::from("enabled=false"));
        return fields.join(";");
    }
    fields.push(field("exe", &rule.exe));
    if let Some(v) = &rule.company {
        fields.push(field("company", v));
    }
    if let Some(v) = &rule.product {
        fields.push(field("product", v));
    }
    if let Some(v) = &rule.original_filename {
        fields.push(field("original_filename", v));
    }
    if let Some(dxgi) = rule.dxgi {
        fields.push(field("dxgi", dxgi.as_str()));
    }
    if let Some(d3d9) = rule.d3d9 {
        fields.push(field("d3d9", d3d9.as_str()));
    }
    if let Some(wined3d) = rule.wined3d {
        fields.push(field("wined3d", wined3d.as_str()));
    }
    for (key, value) in [
        ("dpi_aware", rule.dpi_aware),
        ("large_address_aware", rule.large_address_aware),
        ("gcmouse", rule.gcmouse),
    ] {
        if let Some(value) = value {
            fields.push(field(key, if value { "true" } else { "false" }));
        }
    }
    for over in &rule.dll_overrides {
        fields.push(field("dll_overrides", over));
    }
    for text in &rule.arguments {
        fields.push(field("arguments", text));
    }
    for (name, value) in &rule.env {
        fields.push(format!("env={}={}", encode(name), encode(value)));
    }
    fields.join(";")
}

/// Read one record into a rule. A field that cannot be read (an unknown key,
/// a value its key does not take, no `=`) is left out, and what was wrong
/// with it is returned next to the rule for the caller to report; the other
/// fields still apply. Only a record without a `name` fails as a whole.
///
/// The matcher keys are a closed set (see [`rule_to_record`]), so an unknown
/// key is a setting this library does not know, or a typo. A misspelt
/// matcher is ignored like any other unknown key, with the warning, and the
/// rule then matches more processes than it meant to.
fn record_to_rule(record: &str) -> Result<(Rule, Vec<String>), String> {
    let mut rule = Rule::default();
    let mut have_name = false;
    let mut ignored = Vec::new();
    for raw in record.split(';') {
        if raw.is_empty() {
            continue;
        }
        let Some((key, value)) = raw.split_once('=') else {
            ignored.push(format!("field {raw:?} without '='"));
            continue;
        };
        if key == "name" {
            rule.name = decode(value);
            have_name = true;
        } else if let Err(reason) = set_field(&mut rule, key, value) {
            ignored.push(reason);
        }
    }
    if !have_name {
        return Err(String::from("no name field"));
    }
    // `exe` may legitimately be absent: an override that extends or drops a
    // rule declared elsewhere carries only the fields it changes. A rule that
    // ends up with no exe at all matches nothing, which `overlay` reports.
    Ok((rule, ignored))
}

/// Set the field `key` of `rule` from its wire `value`, or say why it
/// cannot be set, leaving the rule as it was.
fn set_field(rule: &mut Rule, key: &str, value: &str) -> Result<(), String> {
    match key {
        "exe" => rule.exe = decode(value),
        "company" => rule.company = Some(decode(value)),
        "product" => rule.product = Some(decode(value)),
        "original_filename" => rule.original_filename = Some(decode(value)),
        // A value other than `true` or `false` is ignored like any bad
        // boolean, so the rule stays enabled: a typo never drops a rule.
        "enabled" => rule.enabled = parse_bool(key, value)?,
        "dxgi" => rule.dxgi = Some(parse_value(key, value, wire_dxgi)?),
        "d3d9" => rule.d3d9 = Some(parse_value(key, value, D3d9::parse)?),
        "wined3d" => rule.wined3d = Some(parse_value(key, value, Wined3d::parse)?),
        "dpi_aware" => rule.dpi_aware = Some(parse_bool(key, value)?),
        "large_address_aware" => rule.large_address_aware = Some(parse_bool(key, value)?),
        "gcmouse" => rule.gcmouse = Some(parse_bool(key, value)?),
        // The x86_64 Wine's switch for the x87sidecar, which ran under
        // Rosetta. A launcher written for that Wine may still send it, so
        // the key is accepted, whatever its value, and ignored without a
        // diagnostic.
        "x87_sidecar" => {}
        "dll_overrides" => rule.dll_overrides.push(decode(value)),
        "arguments" => rule.arguments.push(decode(value)),
        "env" => {
            let (name, val) = value.split_once('=').unwrap_or((value, ""));
            rule.env.push((decode(name), decode(val)));
        }
        other => return Err(format!("unknown key {other:?}")),
    }
    Ok(())
}

/// A field's value read by `parse`, which knows the values its key takes.
fn parse_value<T>(key: &str, value: &str, parse: fn(&str) -> Option<T>) -> Result<T, String> {
    let decoded = decode(value);
    parse(&decoded).ok_or_else(|| format!("unknown {key} value {decoded:?}"))
}

/// A `dxgi` value on the wire, case-insensitively. `gptk`, Apple's `D3DMetal`,
/// which only the `x86_64` Wine shipped, reads as `dxmt`: that is what the
/// arm64 library has always loaded in its place, so a table written for
/// both Wines keeps its meaning. [`Dxgi::effective`] still gives a native
/// ARM64 program wined3d.
fn wire_dxgi(value: &str) -> Option<Dxgi> {
    if value.eq_ignore_ascii_case("gptk") {
        Some(Dxgi::Dxmt)
    } else {
        Dxgi::parse(value)
    }
}

/// A boolean field's value: exactly `true` or `false`, case-sensitive, so a
/// typo leaves the field out instead of quietly meaning one or the other.
fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    let decoded = decode(value);
    match decoded.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("unknown {key} value {decoded:?}")),
    }
}

fn field(key: &str, value: &str) -> String {
    format!("{key}={}", encode(value))
}

/// Percent-escape the characters that would collide with the wire format
/// (`%`, `;`, and CR/LF) plus any other control character. Everything else,
/// including `=`, `,`, spaces and non-ASCII, passes through.
fn encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    encode_into(raw, &mut out);
    out
}

fn encode_into(raw: &str, out: &mut String) {
    for c in raw.chars() {
        match c {
            '%' => out.push_str("%25"),
            ';' => out.push_str("%3B"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "%{:02X}", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// Reverse [`encode_into`]. Lenient: a `%` not followed by two hex digits is
/// kept literally, so decoding can never fail.
fn decode(encoded: &str) -> String {
    let mut out = String::with_capacity(encoded.len());
    let mut chars = encoded.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let rest = chars.as_str();
        match rest
            .get(..2)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            Some(byte) => {
                out.push(byte as char);
                let _ = chars.next();
                let _ = chars.next();
            }
            None => out.push('%'),
        }
    }
    out
}

// ── UTF-16 helpers ───────────────────────────────────────────────────────

const fn to_lower_u16(u: u16) -> u16 {
    if u >= b'A' as u16 && u <= b'Z' as u16 {
        u | 0x20
    } else {
        u
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

/// Case-sensitive substring test.
fn contains_cs(haystack: &[u16], needle: &str) -> bool {
    let n: Vec<u16> = needle.encode_utf16().collect();
    if n.is_empty() {
        return true;
    }
    if haystack.len() < n.len() {
        return false;
    }
    haystack.windows(n.len()).any(|w| w == n.as_slice())
}

/// Split an environment block into its `NAME=value` segments, dropping the
/// terminating NULs.
fn split_env(block: &[u16]) -> Vec<Vec<u16>> {
    let mut segs = Vec::new();
    let mut cur = Vec::new();
    for &u in block {
        if u == 0 {
            if cur.is_empty() {
                break;
            }
            segs.push(std::mem::take(&mut cur));
        } else {
            cur.push(u);
        }
    }
    segs
}

/// Whether an env segment's name equals `name` case-insensitively, skipping
/// the `=C:` drive entries.
fn env_name_matches(seg: &[u16], name: &str) -> bool {
    if seg.first() == Some(&u16::from(b'=')) {
        return false;
    }
    let seg_name = seg.split(|&u| u == u16::from(b'=')).next().unwrap_or(seg);
    let want: Vec<u16> = name.encode_utf16().map(to_lower_u16).collect();
    seg_name.len() == want.len()
        && seg_name
            .iter()
            .map(|&u| to_lower_u16(u))
            .eq(want.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a test returns, so a missing value fails it through `?` rather
    /// than a panic in the test body.
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn u16s(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn env_block(entries: &[&str]) -> Vec<u16> {
        let mut out = Vec::new();
        for e in entries {
            out.extend(e.encode_utf16());
            out.push(0);
        }
        out.push(0);
        out
    }

    #[test]
    fn encode_decode_round_trips_specials() {
        for raw in [
            "plain",
            "a;b",
            "a%b",
            "a=b,c d",
            "line\nfeed\r",
            "tab\there",
            "café ünïcode",
            "",
        ] {
            assert_eq!(decode(&encode(raw)), raw, "round trip of {raw:?}");
        }
    }

    #[test]
    fn encode_escapes_only_the_dangerous_characters() {
        assert_eq!(encode("d3d12,vulkan-1="), "d3d12,vulkan-1=");
        assert_eq!(encode("a;b"), "a%3Bb");
        assert_eq!(encode("100%"), "100%25");
    }

    #[test]
    fn decode_is_lenient_about_bad_escapes() {
        assert_eq!(decode("50%"), "50%");
        assert_eq!(decode("%zz"), "%zz");
        assert_eq!(decode("%2"), "%2");
    }

    #[test]
    fn table_round_trips_through_the_env_value() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "rockstar-launcher".into(),
                    exe: "Launcher.exe".into(),
                    company: Some("Rockstar Games".into()),
                    product: Some("Rockstar Games Launcher".into()),
                    dxgi: Some(Dxgi::Wined3d),
                    ..Rule::default()
                },
                Rule {
                    name: "social-club-ui".into(),
                    exe: "SocialClubHelper.exe".into(),
                    arguments: vec!["--in-process-gpu --disable-gpu".into()],
                    original_filename: Some("SocialClubHelper.exe".into()),
                    ..Rule::default()
                },
                Rule {
                    name: "wow".into(),
                    exe: "WoW.exe".into(),
                    d3d9: Some(D3d9::Wined3d),
                    dll_overrides: vec!["d3d12,vulkan-1=".into()],
                    env: vec![("DXMT_CONFIG".into(), "d3d11.maxFrameRate=60".into())],
                    ..Rule::default()
                },
            ],
        };
        let (parsed, diagnostics) = Table::parse(&table.to_env_value());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(parsed, table);
    }

    #[test]
    fn env_field_survives_an_equals_in_the_value() {
        let table = Table {
            rules: vec![Rule {
                name: "x".into(),
                exe: "x.exe".into(),
                env: vec![("A".into(), "b=c;d".into())],
                ..Rule::default()
            }],
        };
        let (parsed, _) = Table::parse(&table.to_env_value());
        assert_eq!(parsed, table);
    }

    #[test]
    fn a_legacy_header_on_the_first_line_is_skipped() {
        // What a launcher written for the versioned format sends: the same
        // records mean the same with any version header in front, or none.
        let records = "name=rockstar-launcher;dxgi=dxmt\n\
                       name=wow;exe=WoW.exe;company=Blizzard;d3d9=wined3d;wined3d=vulkan;env=A=1\n\
                       name=dpi-aware;enabled=false";
        let (bare, diagnostics) = Table::parse(records);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(bare.rules.len(), 3);
        for header in ["v=5", "v=6", "v=7", "v=10"] {
            let (table, diagnostics) = Table::parse(&format!("{header}\n{records}"));
            assert!(diagnostics.is_empty(), "{header}: {diagnostics:?}");
            assert_eq!(table, bare, "{header}");
        }
    }

    #[test]
    fn only_a_first_line_of_v_and_digits_is_a_legacy_header() {
        for header in ["v=6", "v=0", "v=123"] {
            assert!(is_legacy_header(header), "{header}");
        }
        for other in [
            "v=",
            "v=6a",
            "v=-1",
            "V=6",
            " v=6",
            "v=6;name=x",
            "name=x;v=6",
        ] {
            assert!(!is_legacy_header(other), "{other}");
        }
        // Anything else on the first line is a record: one with a name is a
        // rule, and one without, a legacy header that looks almost right
        // included, is skipped with a diagnostic.
        let (table, diagnostics) = Table::parse("name=x;exe=x.exe\nname=y;exe=y.exe");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(table.rules.len(), 2);
        let (table, diagnostics) = Table::parse("v=6a\nname=x;exe=x.exe");
        assert_eq!(table.rules.len(), 1);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        // Only the first line can be one; a later one is a record without a
        // name.
        let (table, diagnostics) = Table::parse("name=x;exe=x.exe\nv=6");
        assert_eq!(table.rules.len(), 1);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn the_env_value_is_the_records_alone() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "game".into(),
                    exe: "game.exe".into(),
                    dxgi: Some(Dxgi::Wined3d),
                    wined3d: Some(Wined3d::Vulkan),
                    env: vec![("A".into(), "1".into())],
                    ..Rule::default()
                },
                // A disabled rule carries nothing but its name.
                Rule {
                    name: "dropped".into(),
                    gcmouse: Some(false),
                    enabled: false,
                    ..Rule::default()
                },
            ],
        };
        let value = table.to_env_value();
        assert_eq!(
            value,
            "name=game;exe=game.exe;dxgi=wined3d;wined3d=vulkan;env=A=1\n\
             name=dropped;enabled=false"
        );
        let (parsed, diagnostics) = Table::parse(&value);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(parsed.rules.first(), table.rules.first());
        assert_eq!(parsed.to_env_value(), value);
        assert_eq!(Table::default().to_env_value(), "");
        assert_eq!(Table::parse(""), (Table::default(), Vec::new()));
    }

    #[test]
    fn parse_skips_a_record_without_a_name_but_keeps_the_rest() {
        let (table, diagnostics) = Table::parse("exe=no-name.exe\nname=ok;exe=ok.exe");
        assert_eq!(table.rules.len(), 1);
        assert_eq!(table.rules.first().map(|r| r.exe.as_str()), Some("ok.exe"));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn an_override_carrying_only_the_changed_fields_needs_no_exe() {
        // What a game file writes to extend a built-in rule: the name it is
        // addressing plus the one field it changes.
        let (over, diagnostics) = Table::parse("name=rockstar-launcher;dxgi=dxmt");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(over.rules.len(), 1);
        assert_eq!(over.rules.first().map(|r| r.exe.as_str()), Some(""));
    }

    #[test]
    fn overlay_drops_an_override_that_could_never_match() {
        // A misspelled name with no exe of its own: it neither extends a rule
        // nor stands on its own, so it is reported rather than kept.
        let mut base = Table::default();
        let unmatched = base.overlay(Table {
            rules: vec![Rule {
                name: "typo".into(),
                dxgi: Some(Dxgi::Dxmt),
                ..Rule::default()
            }],
        });
        assert_eq!(unmatched, vec!["typo".to_string()]);
        assert_eq!(base.rules, []);
    }

    #[test]
    fn a_field_that_cannot_be_read_is_ignored_and_the_rest_applies() -> TestResult {
        // An unknown key, a value its key does not take and a field without
        // '=' each cost only that field, with one diagnostic naming the rule
        // and the field. The record before and after it still applies.
        let want = Rule {
            name: "x".into(),
            exe: "x.exe".into(),
            d3d9: Some(D3d9::Wined3d),
            env: vec![("A".into(), "1".into())],
            ..Rule::default()
        };
        for (field, said) in [
            ("bogus=1", "unknown key \"bogus\""),
            ("dxgi=vulkan", "unknown dxgi value \"vulkan\""),
            ("d3d9=gptk", "unknown d3d9 value \"gptk\""),
            ("wined3d=opengl", "unknown wined3d value \"opengl\""),
            ("dpi_aware=yes", "unknown dpi_aware value \"yes\""),
            // A bad `enabled` leaves the rule enabled.
            ("enabled=False", "unknown enabled value \"False\""),
            ("enabled=no", "unknown enabled value \"no\""),
            ("bogus", "field \"bogus\" without '='"),
        ] {
            let text = format!("name=x;{field};exe=x.exe;d3d9=wined3d;env=A=1\nname=ok;exe=ok.exe");
            let (table, diagnostics) = Table::parse(&text);
            assert_eq!(table.rules.first(), Some(&want), "{field}");
            assert_eq!(
                table.rules.get(1).map(|r| r.name.as_str()),
                Some("ok"),
                "{field}"
            );
            let diagnostic = diagnostics.first().ok_or("no diagnostic")?;
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            assert_eq!(diagnostic, &format!("rule \"x\": ignoring {said}"));
        }
        Ok(())
    }

    #[test]
    fn dpi_aware_round_trips_in_both_values_and_is_absent_when_unset() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "aware".into(),
                    exe: ANY_EXE.into(),
                    dpi_aware: Some(true),
                    ..Rule::default()
                },
                Rule {
                    name: "unaware".into(),
                    exe: "old.exe".into(),
                    d3d9: Some(D3d9::Wined3d),
                    dpi_aware: Some(false),
                    dll_overrides: vec!["a=".into()],
                    ..Rule::default()
                },
                Rule {
                    name: "unset".into(),
                    exe: "other.exe".into(),
                    ..Rule::default()
                },
            ],
        };
        let text = table.to_env_value();
        assert_eq!(
            text,
            "name=aware;exe=*;dpi_aware=true\n\
             name=unaware;exe=old.exe;d3d9=wined3d;dpi_aware=false;dll_overrides=a=\n\
             name=unset;exe=other.exe"
        );
        let (parsed, diagnostics) = Table::parse(&text);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(parsed, table);
    }

    #[test]
    fn a_bad_dpi_aware_value_is_ignored() {
        for bad in ["yes", "1", "TRUE", ""] {
            let (table, diagnostics) = Table::parse(&format!(
                "name=x;exe=x.exe;dpi_aware={bad}\nname=ok;exe=ok.exe"
            ));
            assert_eq!(table.rules.len(), 2, "dpi_aware={bad:?}");
            assert_eq!(table.rules.first().map(|r| r.dpi_aware), Some(None));
            assert_eq!(diagnostics.len(), 1, "dpi_aware={bad:?}");
        }
    }

    #[test]
    fn dpi_aware_resolves_to_the_most_specific_rule_and_unset_inherits() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "pinned".into(),
                    exe: "game.exe".into(),
                    company: Some("Acme".into()),
                    dpi_aware: Some(true),
                    ..Rule::default()
                },
                Rule {
                    name: "named".into(),
                    exe: "game.exe".into(),
                    dpi_aware: Some(false),
                    ..Rule::default()
                },
                Rule {
                    name: "defaults".into(),
                    exe: ANY_EXE.into(),
                    dpi_aware: Some(true),
                    ..Rule::default()
                },
                Rule {
                    name: "quiet".into(),
                    exe: "quiet.exe".into(),
                    arguments: vec!["-x".into()],
                    ..Rule::default()
                },
            ],
        };
        let acme = VersionInfo {
            company: "Acme".into(),
            ..VersionInfo::default()
        };
        // Only the wildcard matches.
        let other = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(other.dpi_aware, Some(true));
        assert!(!other.is_empty());
        // The basename rule beats the wildcard.
        assert_eq!(
            table.resolve("game.exe", &VersionInfo::default()).dpi_aware,
            Some(false)
        );
        // The fingerprinted rule beats both.
        let pinned = table.resolve("game.exe", &acme);
        assert_eq!(pinned.dpi_aware, Some(true));
        assert_eq!(
            pinned.matched,
            vec![
                "defaults".to_string(),
                "named".to_string(),
                "pinned".to_string()
            ]
        );
        // A more specific rule that leaves it unset inherits the wildcard's.
        let quiet = table.resolve("quiet.exe", &VersionInfo::default());
        assert_eq!(quiet.dpi_aware, Some(true));
        assert_eq!(
            quiet.matched,
            vec!["defaults".to_string(), "quiet".to_string()]
        );
        // With no rule setting it, the resolution leaves it to wine.
        assert_eq!(Table::default().resolve("x.exe", &acme).dpi_aware, None);
    }

    #[test]
    fn overlay_by_name_replaces_dpi_aware_only_when_set() {
        let mut base = Table {
            rules: vec![Rule {
                name: "defaults".into(),
                exe: ANY_EXE.into(),
                dpi_aware: Some(true),
                ..Rule::default()
            }],
        };
        // An override that does not mention it leaves the value alone.
        let (over, diagnostics) = Table::parse("name=defaults;dll_overrides=a=");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(base.overlay(over), [] as [String; 0]);
        assert_eq!(base.rules.first().map(|r| r.dpi_aware), Some(Some(true)));
        // One that sets it replaces it.
        let (over, diagnostics) = Table::parse("name=defaults;dpi_aware=false");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(base.overlay(over), [] as [String; 0]);
        assert_eq!(base.rules.first().map(|r| r.dpi_aware), Some(Some(false)));
        assert_eq!(
            base.resolve("x.exe", &VersionInfo::default()).dpi_aware,
            Some(false)
        );
    }

    #[test]
    fn large_address_aware_round_trips_and_is_absent_when_unset() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "defaults".into(),
                    exe: ANY_EXE.into(),
                    large_address_aware: Some(true),
                    ..Rule::default()
                },
                Rule {
                    name: "old".into(),
                    exe: "old.exe".into(),
                    dpi_aware: Some(false),
                    large_address_aware: Some(false),
                    ..Rule::default()
                },
                Rule {
                    name: "unset".into(),
                    exe: "other.exe".into(),
                    ..Rule::default()
                },
            ],
        };
        let text = table.to_env_value();
        assert_eq!(
            text,
            "name=defaults;exe=*;large_address_aware=true\n\
             name=old;exe=old.exe;dpi_aware=false;large_address_aware=false\n\
             name=unset;exe=other.exe"
        );
        let (parsed, diagnostics) = Table::parse(&text);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(parsed, table);
    }

    #[test]
    fn a_bad_large_address_aware_value_is_ignored() {
        let key = "large_address_aware";
        for bad in ["yes", "1", "False", ""] {
            let (table, diagnostics) = Table::parse(&format!(
                "name=x;exe=x.exe;dpi_aware=false;{key}={bad}\nname=ok;exe=ok.exe"
            ));
            assert_eq!(table.rules.len(), 2, "{key}={bad:?}");
            let x = table.rules.first();
            assert_eq!(x.map(|r| r.large_address_aware), Some(None));
            assert_eq!(x.map(|r| r.dpi_aware), Some(Some(false)));
            assert_eq!(diagnostics.len(), 1, "{key}={bad:?}");
            assert!(
                diagnostics.first().is_some_and(|d| d.contains(key)),
                "{diagnostics:?}"
            );
        }
    }

    #[test]
    fn large_address_aware_resolves_to_the_most_specific_rule() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "pinned".into(),
                    exe: "game.exe".into(),
                    company: Some("Acme".into()),
                    dpi_aware: Some(false),
                    ..Rule::default()
                },
                Rule {
                    name: "named".into(),
                    exe: "game.exe".into(),
                    large_address_aware: Some(false),
                    ..Rule::default()
                },
                Rule {
                    name: "defaults".into(),
                    exe: ANY_EXE.into(),
                    large_address_aware: Some(true),
                    ..Rule::default()
                },
            ],
        };
        let acme = VersionInfo {
            company: "Acme".into(),
            ..VersionInfo::default()
        };
        // Only the wildcard matches.
        let other = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(other.large_address_aware, Some(true));
        // The basename rule beats the wildcard.
        let named = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(named.large_address_aware, Some(false));
        // A fingerprinted rule that leaves it unset inherits it from the next
        // most specific rule.
        let pinned = table.resolve("game.exe", &acme);
        assert_eq!(pinned.dpi_aware, Some(false));
        assert_eq!(pinned.large_address_aware, Some(false));
        // With no rule setting it, there is no opinion.
        let none = Table::default().resolve("x.exe", &acme);
        assert_eq!(none.large_address_aware, None);
        assert!(none.is_empty());
        // It alone makes a resolution non-empty.
        let table = Table {
            rules: vec![Rule {
                name: "laa".into(),
                exe: ANY_EXE.into(),
                large_address_aware: Some(false),
                ..Rule::default()
            }],
        };
        assert!(!table.resolve("x.exe", &acme).is_empty());
    }

    #[test]
    fn overlay_by_name_replaces_large_address_aware_only_when_set() -> TestResult {
        let mut base = Table {
            rules: vec![Rule {
                name: "defaults".into(),
                exe: ANY_EXE.into(),
                large_address_aware: Some(true),
                ..Rule::default()
            }],
        };
        let (over, diagnostics) = Table::parse("name=defaults;dll_overrides=a=");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(base.overlay(over), [] as [String; 0]);
        let rule = base.rules.first().ok_or("no rule")?;
        assert_eq!(rule.large_address_aware, Some(true));
        let (over, diagnostics) = Table::parse("name=defaults;large_address_aware=false");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(base.overlay(over), [] as [String; 0]);
        let rule = base.rules.first().ok_or("no rule")?;
        assert_eq!(rule.large_address_aware, Some(false));
        Ok(())
    }

    #[test]
    fn gcmouse_round_trips_and_is_absent_when_unset() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "defaults".into(),
                    exe: ANY_EXE.into(),
                    gcmouse: Some(true),
                    ..Rule::default()
                },
                Rule {
                    name: "old".into(),
                    exe: "old.exe".into(),
                    large_address_aware: Some(false),
                    gcmouse: Some(false),
                    ..Rule::default()
                },
                Rule {
                    name: "unset".into(),
                    exe: "other.exe".into(),
                    ..Rule::default()
                },
            ],
        };
        let text = table.to_env_value();
        assert_eq!(
            text,
            "name=defaults;exe=*;gcmouse=true\n\
             name=old;exe=old.exe;large_address_aware=false;gcmouse=false\n\
             name=unset;exe=other.exe"
        );
        let (parsed, diagnostics) = Table::parse(&text);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(parsed, table);
    }

    #[test]
    fn a_bad_gcmouse_value_is_ignored() {
        let key = "gcmouse";
        for bad in ["yes", "1", "False", ""] {
            let (table, diagnostics) = Table::parse(&format!(
                "name=x;exe=x.exe;dpi_aware=false;{key}={bad}\nname=ok;exe=ok.exe"
            ));
            assert_eq!(table.rules.len(), 2, "{key}={bad:?}");
            let x = table.rules.first();
            assert_eq!(x.map(|r| r.gcmouse), Some(None));
            assert_eq!(x.map(|r| r.dpi_aware), Some(Some(false)));
            assert_eq!(diagnostics.len(), 1, "{key}={bad:?}");
            assert!(
                diagnostics.first().is_some_and(|d| d.contains(key)),
                "{diagnostics:?}"
            );
        }
    }

    #[test]
    fn gcmouse_resolves_to_the_most_specific_rule() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "pinned".into(),
                    exe: "game.exe".into(),
                    company: Some("Acme".into()),
                    dpi_aware: Some(false),
                    ..Rule::default()
                },
                Rule {
                    name: "named".into(),
                    exe: "game.exe".into(),
                    gcmouse: Some(false),
                    ..Rule::default()
                },
                Rule {
                    name: "defaults".into(),
                    exe: ANY_EXE.into(),
                    gcmouse: Some(true),
                    ..Rule::default()
                },
            ],
        };
        let acme = VersionInfo {
            company: "Acme".into(),
            ..VersionInfo::default()
        };
        // Only the wildcard matches.
        let other = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(other.gcmouse, Some(true));
        // The basename rule beats the wildcard.
        let named = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(named.gcmouse, Some(false));
        // A fingerprinted rule that leaves it unset inherits it from the next
        // most specific rule.
        let pinned = table.resolve("game.exe", &acme);
        assert_eq!(pinned.dpi_aware, Some(false));
        assert_eq!(pinned.gcmouse, Some(false));
        // With no rule setting it, there is no opinion.
        let none = Table::default().resolve("x.exe", &acme);
        assert_eq!(none.gcmouse, None);
        assert!(none.is_empty());
        // It alone makes a resolution non-empty.
        let table = Table {
            rules: vec![Rule {
                name: "gcmouse".into(),
                exe: ANY_EXE.into(),
                gcmouse: Some(false),
                ..Rule::default()
            }],
        };
        assert!(!table.resolve("x.exe", &acme).is_empty());
    }

    #[test]
    fn overlay_by_name_replaces_gcmouse_only_when_set() -> TestResult {
        let mut base = Table {
            rules: vec![Rule {
                name: "defaults".into(),
                exe: ANY_EXE.into(),
                gcmouse: Some(true),
                ..Rule::default()
            }],
        };
        let (over, diagnostics) = Table::parse("name=defaults;dll_overrides=a=");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(base.overlay(over), [] as [String; 0]);
        let rule = base.rules.first().ok_or("no rule")?;
        assert_eq!(rule.gcmouse, Some(true));
        let (over, diagnostics) = Table::parse("name=defaults;gcmouse=false");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(base.overlay(over), [] as [String; 0]);
        let rule = base.rules.first().ok_or("no rule")?;
        assert_eq!(rule.gcmouse, Some(false));
        Ok(())
    }

    #[test]
    fn a_legacy_x87_sidecar_field_is_accepted_and_ignored() {
        // What a launcher written for the x86_64 Wine still sends. The record
        // is kept with everything else it says, whatever the value, and
        // nothing of it is written back.
        let bare = Table::parse(
            "name=old;exe=old.exe;large_address_aware=false\nname=x87-sidecar;enabled=false",
        );
        for value in ["true", "false", "TRUE", "1", ""] {
            let with = Table::parse(&format!(
                "name=old;exe=old.exe;x87_sidecar={value};large_address_aware=false\n\
                 name=x87-sidecar;enabled=false;x87_sidecar={value}"
            ));
            assert_eq!(with, bare, "x87_sidecar={value:?}");
        }
        let (table, diagnostics) = bare;
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(!table.to_env_value().contains("x87_sidecar"));
        // Without its '=' it is a field like any other that cannot be read.
        let (table, diagnostics) = Table::parse("name=old;exe=old.exe;x87_sidecar");
        assert_eq!(table.rules.len(), 1);
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn resolve_matches_case_insensitively_and_folds_in_order() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "first".into(),
                    exe: "game.exe".into(),
                    dxgi: Some(Dxgi::Dxmt),
                    dll_overrides: vec!["a=".into()],
                    ..Rule::default()
                },
                Rule {
                    name: "second".into(),
                    exe: "GAME.EXE".into(),
                    dxgi: Some(Dxgi::Wined3d),
                    dll_overrides: vec!["b=".into()],
                    ..Rule::default()
                },
            ],
        };
        let r = table.resolve("Game.exe", &VersionInfo::default());
        assert_eq!(r.dxgi, Some(Dxgi::Wined3d));
        assert_eq!(r.dll_overrides, vec!["a=".to_string(), "b=".to_string()]);
    }

    #[test]
    fn a_fingerprint_constraint_is_an_optional_extra_filter() {
        let table = Table {
            rules: vec![Rule {
                name: "rockstar-launcher".into(),
                exe: "Launcher.exe".into(),
                company: Some("Rockstar Games".into()),
                product: Some("Rockstar Games Launcher".into()),
                dxgi: Some(Dxgi::Wined3d),
                ..Rule::default()
            }],
        };
        let rockstar = VersionInfo {
            company: "Rockstar Games".into(),
            product: "Rockstar Games Launcher".into(),
            original_filename: "Launcher.exe".into(),
        };
        // Matches by basename + fingerprint regardless of where it is installed.
        assert_eq!(
            table.resolve("Launcher.exe", &rockstar).dxgi,
            Some(Dxgi::Wined3d)
        );
        // A different program that happens to be called Launcher.exe: same
        // basename, no version resource, so the fingerprint fails and it is
        // left alone.
        assert!(
            table
                .resolve("launcher.exe", &VersionInfo::default())
                .is_empty()
        );
    }

    #[test]
    fn a_bare_basename_rule_needs_no_fingerprint() {
        let table = Table {
            rules: vec![Rule {
                name: "steam-web-helper".into(),
                exe: "steamwebhelper.exe".into(),
                arguments: vec!["--in-process-gpu".into()],
                ..Rule::default()
            }],
        };
        let r = table.resolve("steamwebhelper.exe", &VersionInfo::default());
        assert_eq!(r.arguments, vec!["--in-process-gpu".to_string()]);
    }

    /// One `VS_VERSIONINFO` node as a resource compiler lays it out: the three
    /// header words, the key, padding, the value, then, when there are any,
    /// padding and the children. `text` sets `wType`, which decides whether
    /// `wValueLength` counts characters or bytes. `wLength` leaves out the
    /// padding after the node, as real resources do.
    fn version_node(key: &str, value: &[u16], text: bool, children: &[u16]) -> Vec<u16> {
        let mut node = vec![0, 0, u16::from(text)];
        node.extend(key.encode_utf16());
        node.push(0);
        node.resize(node.len().next_multiple_of(2), 0);
        node.extend(value);
        if !children.is_empty() {
            node.resize(node.len().next_multiple_of(2), 0);
            node.extend(children);
        }
        // The fixtures stay far below 64 KB. Were one to grow past it, the
        // saturated length would make the tests that parse it fail.
        let byte_len = |units: usize| u16::try_from(units.saturating_mul(2)).unwrap_or(u16::MAX);
        let length = byte_len(node.len());
        let value_length = if text {
            u16::try_from(value.len()).unwrap_or(u16::MAX)
        } else {
            byte_len(value.len())
        };
        if let [w_length, w_value_length, ..] = node.as_mut_slice() {
            *w_length = length;
            *w_value_length = value_length;
        }
        node
    }

    /// Sibling nodes, each starting on a 32-bit boundary.
    fn siblings(nodes: &[Vec<u16>]) -> Vec<u16> {
        let mut out = Vec::new();
        for node in nodes {
            out.resize(out.len().next_multiple_of(2), 0);
            out.extend(node);
        }
        out
    }

    /// A `String` entry; an empty value is the NUL alone.
    fn string_entry(key: &str, value: &str) -> Vec<u16> {
        let mut text: Vec<u16> = value.encode_utf16().collect();
        text.push(0);
        version_node(key, &text, true, &[])
    }

    /// A whole `VS_VERSIONINFO`: the root with a zeroed `VS_FIXEDFILEINFO`, a
    /// `StringFileInfo` holding one string table per entry of `tables`, and a
    /// `VarFileInfo` after it.
    fn version_resource(tables: &[(&str, &[(&str, &str)])]) -> Vec<u16> {
        let tables: Vec<Vec<u16>> = tables
            .iter()
            .map(|(language, entries)| {
                let strings: Vec<Vec<u16>> = entries
                    .iter()
                    .map(|(key, value)| string_entry(key, value))
                    .collect();
                version_node(language, &[], false, &siblings(&strings))
            })
            .collect();
        let string_info = version_node("StringFileInfo", &[], false, &siblings(&tables));
        let translation = version_node("Translation", &[0x0409, 0x04b0], false, &[]);
        let var_info = version_node("VarFileInfo", &[], false, &translation);
        version_node(
            "VS_VERSION_INFO",
            &[0; 26],
            false,
            &siblings(&[string_info, var_info]),
        )
    }

    #[test]
    fn version_info_reads_the_standard_fields_from_a_resource() {
        let blob = version_resource(&[(
            "040904b0",
            &[
                ("CompanyName", "Rockstar Games"),
                ("FileVersion", "1.0.0.1"),
                ("OriginalFilename", "Launcher.exe"),
                ("ProductName", "Rockstar Games Launcher"),
            ],
        )]);
        let vi = VersionInfo::from_resource(&blob);
        assert_eq!(vi.company, "Rockstar Games");
        assert_eq!(vi.product, "Rockstar Games Launcher");
        assert_eq!(vi.original_filename, "Launcher.exe");
    }

    #[test]
    fn an_empty_value_followed_by_another_entry_is_empty() {
        // The entry after the empty one must not be read as its value.
        let blob = version_resource(&[(
            "040904b0",
            &[
                ("CompanyName", ""),
                ("FileDescription", "Game"),
                ("OriginalFilename", ""),
                ("ProductName", ""),
                ("ProductVersion", "1.0.0.0"),
            ],
        )]);
        assert_eq!(VersionInfo::from_resource(&blob), VersionInfo::default());
    }

    #[test]
    fn an_empty_value_at_the_end_of_a_table_is_empty() {
        // Nothing of the next table or of VarFileInfo leaks into it either.
        let blob = version_resource(&[
            (
                "040904b0",
                &[("CompanyName", "Acme"), ("OriginalFilename", "")],
            ),
            ("040704b0", &[("ProductName", "Rocket Sled")]),
        ]);
        let vi = VersionInfo::from_resource(&blob);
        assert_eq!(vi.company, "Acme");
        assert_eq!(vi.original_filename, "");
        // A field the first table lacks comes from the next one.
        assert_eq!(vi.product, "Rocket Sled");
        let last = version_resource(&[("040904b0", &[("ProductName", "")])]);
        assert_eq!(VersionInfo::from_resource(&last), VersionInfo::default());
    }

    #[test]
    fn the_nostale_client_has_only_a_company() {
        // The string table of NosTale's NostaleClientX.exe (block 041203B5),
        // whose only non-empty names are the company and the versions.
        // Reading the values as NUL-separated runs takes the next entry's
        // header and key for the value of each empty one, so the product read
        // as "4\u{8}\u{1}ProductVersion".
        let blob = version_resource(&[(
            "041203B5",
            &[
                ("CompanyName", "Entwell"),
                ("FileDescription", ""),
                ("FileVersion", "0.9.3.3259"),
                ("InternalName", ""),
                ("LegalCopyright", ""),
                ("LegalTrademarks", ""),
                ("OriginalFilename", ""),
                ("ProductName", ""),
                ("ProductVersion", "1.0.0.0"),
                ("Comments", ""),
                ("PrivateBuild", ""),
                ("SpecialBuild", ""),
            ],
        )]);
        // Two entries as the exe stores them, which pins the fixture's layout
        // to the real one.
        let entry = |header: [u16; 3], text: &str| {
            let mut units = header.to_vec();
            units.extend(text.encode_utf16());
            units
        };
        assert_eq!(
            string_entry("CompanyName", "Entwell"),
            entry([0x30, 8, 1], "CompanyName\0\0Entwell\0")
        );
        assert_eq!(
            string_entry("ProductName", ""),
            entry([0x22, 1, 1], "ProductName\0\0\0")
        );
        let vi = VersionInfo::from_resource(&blob);
        assert_eq!(
            vi,
            VersionInfo {
                company: "Entwell".into(),
                ..VersionInfo::default()
            }
        );
        // A rule that pins a field the exe leaves empty does not match it.
        let pinned = |product: Option<&str>, original_filename: Option<&str>| Rule {
            name: "nostale".into(),
            exe: "NostaleClientX.exe".into(),
            company: Some("Entwell".into()),
            product: product.map(Into::into),
            original_filename: original_filename.map(Into::into),
            ..Rule::default()
        };
        assert!(pinned(None, None).matches("NostaleClientX.exe", &vi));
        assert!(!pinned(Some("ProductVersion"), None).matches("NostaleClientX.exe", &vi));
        assert!(!pinned(None, Some("ProductName")).matches("NostaleClientX.exe", &vi));
    }

    #[test]
    fn keys_match_case_insensitively_and_the_first_entry_wins() {
        let blob = version_resource(&[
            ("040904b0", &[("companyname", "First")]),
            ("040704b0", &[("CompanyName", "Second")]),
        ]);
        assert_eq!(VersionInfo::from_resource(&blob).company, "First");
    }

    #[test]
    fn a_malformed_resource_yields_empty_fields() -> TestResult {
        let blob = version_resource(&[(
            "040904b0",
            &[("CompanyName", "Acme"), ("ProductName", "Rocket Sled")],
        )]);
        // Cut short anywhere: never a panic, and never a value that is not
        // the field's own.
        for len in 0..blob.len() {
            let vi = VersionInfo::from_resource(blob.get(..len).ok_or("cut")?);
            assert!("Acme".starts_with(&vi.company), "{len}: {vi:?}");
            assert!("Rocket Sled".starts_with(&vi.product), "{len}: {vi:?}");
        }
        // A root that is not VS_VERSION_INFO, and a zero length.
        let mut foreign = blob.clone();
        *foreign.get_mut(3).ok_or("no key")? = u16::from(b'X');
        assert_eq!(VersionInfo::from_resource(&foreign), VersionInfo::default());
        let mut zero = blob;
        *zero.first_mut().ok_or("no wLength")? = 0;
        assert_eq!(VersionInfo::from_resource(&zero), VersionInfo::default());
        // A bare key and value with no tree around them.
        let mut runs: Vec<u16> = vec![0x40, 0x0f, 0x01];
        runs.extend("CompanyName\0\0Acme\0".encode_utf16());
        assert_eq!(VersionInfo::from_resource(&runs), VersionInfo::default());
        Ok(())
    }

    #[test]
    fn a_broken_entry_ends_its_table_without_taking_a_neighbours_value() -> TestResult {
        // The wLength of the entry with this key, three words before the key.
        fn w_length<'a>(blob: &'a mut [u16], key: &str) -> Result<&'a mut u16, String> {
            let units = u16s(key);
            blob.windows(units.len())
                .position(|w| w == units)
                .and_then(|at| at.checked_sub(3))
                .and_then(|at| blob.get_mut(at))
                .ok_or_else(|| format!("no entry {key}"))
        }

        let blob = version_resource(&[(
            "040904b0",
            &[
                ("CompanyName", "Acme"),
                ("FileDescription", "Game"),
                ("ProductName", "Rocket Sled"),
            ],
        )]);
        let acme_only = VersionInfo {
            company: "Acme".into(),
            ..VersionInfo::default()
        };
        // A zero or too-small wLength in the middle of the table: the walk
        // stops there, so the entries before it still count.
        for length in [0, 2, 6] {
            let mut broken = blob.clone();
            *w_length(&mut broken, "FileDescription")? = length;
            assert_eq!(
                VersionInfo::from_resource(&broken),
                acme_only,
                "wLength {length}"
            );
        }
        // An entry claiming more than its table holds is cut off at the
        // table's end, and nothing after it is read as a sibling.
        let mut oversized = blob;
        *w_length(&mut oversized, "CompanyName")? = 0xfffe;
        assert_eq!(VersionInfo::from_resource(&oversized), acme_only);
        // A value with wValueLength 0 and no NUL before the next entry ends
        // where its entry does.
        let mut unterminated = version_node("CompanyName", &u16s("Acme"), true, &[]);
        *unterminated.get_mut(1).ok_or("no wValueLength")? = 0;
        assert!(
            unterminated.len().is_multiple_of(2),
            "no padding before the next entry"
        );
        let table = version_node(
            "040904b0",
            &[],
            false,
            &siblings(&[unterminated, string_entry("ProductName", "Rocket Sled")]),
        );
        let info = version_node("StringFileInfo", &[], false, &table);
        let unterminated = version_node("VS_VERSION_INFO", &[0; 26], false, &info);
        assert_eq!(
            VersionInfo::from_resource(&unterminated),
            VersionInfo {
                company: "Acme".into(),
                product: "Rocket Sled".into(),
                ..VersionInfo::default()
            }
        );
        Ok(())
    }

    #[test]
    fn append_cmdline_is_idempotent() -> TestResult {
        let base = u16s("cmd.exe");
        let appends = vec!["--in-process-gpu".to_string()];
        let once = append_cmdline(&base, &appends).ok_or("should append")?;
        assert_eq!(String::from_utf16_lossy(&once), "cmd.exe --in-process-gpu");
        assert!(append_cmdline(&once, &appends).is_none());
        Ok(())
    }

    #[test]
    fn append_cmdline_keeps_the_existing_arguments() -> TestResult {
        // The launcher's `arguments` knob and any switches the parent passed
        // are in the command line already; appending must never drop them.
        let base = u16s("game.exe -console --width 1280");
        let out = append_cmdline(&base, &["--in-process-gpu".to_string()]).ok_or("appended")?;
        assert_eq!(
            String::from_utf16_lossy(&out),
            "game.exe -console --width 1280 --in-process-gpu"
        );
        Ok(())
    }

    #[test]
    fn append_cmdline_applies_each_new_text_once() -> TestResult {
        let base = u16s("app.exe --foo");
        let appends = vec!["--foo".to_string(), "--bar".to_string()];
        let out = append_cmdline(&base, &appends).ok_or("should append --bar")?;
        assert_eq!(String::from_utf16_lossy(&out), "app.exe --foo --bar");
        Ok(())
    }

    #[test]
    fn merge_env_replaces_appends_and_removes() -> TestResult {
        let block = env_block(&["PATH=C:\\windows", "FOO=old", "=C:=C:\\cur"]);
        let out = merge_env(
            &block,
            &[
                ("foo".into(), "new".into()),   // case-insensitive replace
                ("BAR".into(), "added".into()), // append
                ("PATH".into(), String::new()), // remove
            ],
        )
        .ok_or("changed")?;
        let segs = split_env(&out);
        let text: Vec<String> = segs.iter().map(|s| String::from_utf16_lossy(s)).collect();
        assert_eq!(text, vec!["foo=new", "=C:=C:\\cur", "BAR=added"]);
        // Terminated with a double NUL.
        assert_eq!(out.last(), Some(&0));
        Ok(())
    }

    #[test]
    fn merge_env_reports_no_change() {
        let block = env_block(&["FOO=bar"]);
        assert!(merge_env(&block, &[("FOO".into(), "bar".into())]).is_none());
        assert!(merge_env(&block, &[("MISSING".into(), String::new())]).is_none());
    }

    #[test]
    fn basename_handles_both_separators_and_none() {
        assert_eq!(basename_of(&u16s("C:\\a\\b\\Foo.exe")), "Foo.exe");
        assert_eq!(basename_of(&u16s("/a/b/Bar.exe")), "Bar.exe");
        assert_eq!(basename_of(&u16s("Baz.exe")), "Baz.exe");
    }

    #[test]
    fn overlay_merges_by_name_appends_lists_and_adds_new_rules() -> TestResult {
        let mut base = Table {
            rules: vec![Rule {
                name: "rockstar-launcher".into(),
                exe: "Launcher.exe".into(),
                company: Some("Rockstar Games".into()),
                dxgi: Some(Dxgi::Wined3d),
                dll_overrides: vec!["a=".into()],
                ..Rule::default()
            }],
        };
        let unmatched = base.overlay(Table {
            rules: vec![
                Rule {
                    name: "Rockstar-Launcher".into(), // case-insensitive match
                    dxgi: Some(Dxgi::Dxmt),
                    dll_overrides: vec!["b=".into()],
                    ..Rule::default()
                },
                Rule {
                    name: "new".into(),
                    exe: "New.exe".into(),
                    arguments: vec!["-x".into()],
                    ..Rule::default()
                },
            ],
        });
        assert_eq!(unmatched, [] as [String; 0]);
        let launcher = base
            .rules
            .iter()
            .find(|r| r.name == "rockstar-launcher")
            .ok_or("no launcher rule")?;
        assert_eq!(launcher.dxgi, Some(Dxgi::Dxmt)); // override wins
        assert_eq!(launcher.company.as_deref(), Some("Rockstar Games")); // untouched survives
        assert_eq!(launcher.exe, "Launcher.exe"); // an override needs no exe
        assert_eq!(
            launcher.dll_overrides,
            vec!["a=".to_string(), "b=".to_string()]
        ); // appended
        assert!(base.rules.iter().any(|r| r.exe == "New.exe")); // new rule added
        Ok(())
    }

    #[test]
    fn overlay_leaves_a_rule_with_the_same_exe_under_another_name_alone() -> TestResult {
        // The bug named rules exist to fix: keying by executable made a new
        // rule for a common basename merge into an unrelated one and inherit
        // its fingerprint.
        let mut base = Table {
            rules: vec![Rule {
                name: "rockstar-launcher".into(),
                exe: "Launcher.exe".into(),
                company: Some("Rockstar Games".into()),
                dxgi: Some(Dxgi::Wined3d),
                ..Rule::default()
            }],
        };
        base.overlay(Table {
            rules: vec![Rule {
                name: "my-launcher".into(),
                exe: "Launcher.exe".into(),
                dxgi: Some(Dxgi::Dxmt),
                ..Rule::default()
            }],
        });
        assert_eq!(base.rules.len(), 2);
        let mine = base
            .rules
            .iter()
            .find(|r| r.name == "my-launcher")
            .ok_or("no my-launcher rule")?;
        assert!(mine.company.is_none(), "no fingerprint inherited");
        assert_eq!(
            base.rules
                .iter()
                .find(|r| r.name == "rockstar-launcher")
                .map(|r| r.dxgi),
            Some(Some(Dxgi::Wined3d)),
            "the pinned rule is untouched"
        );
        Ok(())
    }

    #[test]
    fn overlay_reports_an_override_that_names_no_rule() {
        let mut base = Table::default();
        let unmatched = base.overlay(Table {
            rules: vec![Rule {
                name: "gone".into(),
                enabled: false,
                ..Rule::default()
            }],
        });
        assert_eq!(unmatched, vec!["gone".to_string()]);
        assert_eq!(base.rules, []);
    }

    #[test]
    fn overlay_disabled_drops_the_rule_of_that_name() {
        let mut base = Table {
            rules: vec![Rule {
                name: "steam-web-helper".into(),
                exe: "steamwebhelper.exe".into(),
                ..Rule::default()
            }],
        };
        let unmatched = base.overlay(Table {
            rules: vec![Rule {
                name: "steam-web-helper".into(),
                enabled: false,
                ..Rule::default()
            }],
        });
        assert_eq!(unmatched, [] as [String; 0]);
        assert_eq!(base.rules, []);
    }

    #[test]
    fn a_disabled_override_round_trips_through_the_wire() {
        let table = Table {
            rules: vec![Rule {
                name: "x".into(),
                enabled: false,
                ..Rule::default()
            }],
        };
        let (parsed, diagnostics) = Table::parse(&table.to_env_value());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(parsed, table);
        assert_eq!(parsed.rules.first().map(|r| r.enabled), Some(false));
    }

    #[test]
    fn a_wildcard_rule_is_the_least_specific_and_a_process_rule_beats_it() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "defaults".into(),
                    exe: ANY_EXE.into(),
                    dxgi: Some(Dxgi::Dxmt),
                    dll_overrides: vec!["global=".into()],
                    ..Rule::default()
                },
                Rule {
                    name: "launcher".into(),
                    exe: "Launcher.exe".into(),
                    dxgi: Some(Dxgi::Wined3d),
                    dll_overrides: vec!["specific=".into()],
                    ..Rule::default()
                },
            ],
        };
        // The wildcard alone reaches an unrelated process.
        let other = table.resolve("Other.exe", &VersionInfo::default());
        assert_eq!(other.dxgi, Some(Dxgi::Dxmt));
        assert_eq!(other.matched, vec!["defaults".to_string()]);
        // Both match Launcher.exe; the more specific rule takes the scalar and
        // the lists accumulate least-specific-first.
        let launcher = table.resolve("launcher.exe", &VersionInfo::default());
        assert_eq!(launcher.dxgi, Some(Dxgi::Wined3d));
        assert_eq!(
            launcher.dll_overrides,
            vec!["global=".to_string(), "specific=".to_string()]
        );
        assert_eq!(
            launcher.matched,
            vec!["defaults".to_string(), "launcher".to_string()]
        );
    }

    #[test]
    fn a_process_rule_wins_however_the_rules_are_ordered() {
        // Table order must not decide a scalar; specificity does. Same two
        // rules as above with the specific one declared first.
        let specific = Rule {
            name: "launcher".into(),
            exe: "Launcher.exe".into(),
            dxgi: Some(Dxgi::Wined3d),
            ..Rule::default()
        };
        let wildcard = Rule {
            name: "defaults".into(),
            exe: ANY_EXE.into(),
            dxgi: Some(Dxgi::Dxmt),
            ..Rule::default()
        };
        let table = Table {
            rules: vec![specific, wildcard],
        };
        assert_eq!(
            table.resolve("Launcher.exe", &VersionInfo::default()).dxgi,
            Some(Dxgi::Wined3d)
        );
    }

    #[test]
    fn two_rules_for_one_basename_resolve_independently() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "rockstar-launcher".into(),
                    exe: "Launcher.exe".into(),
                    company: Some("Rockstar Games".into()),
                    dxgi: Some(Dxgi::Wined3d),
                    ..Rule::default()
                },
                Rule {
                    name: "acme-launcher".into(),
                    exe: "Launcher.exe".into(),
                    company: Some("Acme".into()),
                    dxgi: Some(Dxgi::Dxmt),
                    ..Rule::default()
                },
            ],
        };
        let rockstar = VersionInfo {
            company: "Rockstar Games".into(),
            ..VersionInfo::default()
        };
        let acme = VersionInfo {
            company: "Acme Inc".into(),
            ..VersionInfo::default()
        };
        assert_eq!(
            table.resolve("Launcher.exe", &rockstar).matched,
            vec!["rockstar-launcher".to_string()]
        );
        assert_eq!(table.resolve("Launcher.exe", &acme).dxgi, Some(Dxgi::Dxmt));
        // A third program of that name matches neither.
        assert_eq!(
            table
                .resolve("Launcher.exe", &VersionInfo::default())
                .matched,
            [] as [String; 0]
        );
    }

    #[test]
    fn specificity_orders_wildcard_then_exe_then_fingerprint() {
        let wildcard = Rule {
            exe: ANY_EXE.into(),
            ..Rule::default()
        };
        let wildcard_pinned = Rule {
            exe: ANY_EXE.into(),
            company: Some("Rockstar Games".into()),
            ..Rule::default()
        };
        let named = Rule {
            exe: "a.exe".into(),
            ..Rule::default()
        };
        let named_pinned = Rule {
            exe: "a.exe".into(),
            product: Some("A".into()),
            ..Rule::default()
        };
        assert!(wildcard.specificity() < wildcard_pinned.specificity());
        assert!(wildcard_pinned.specificity() < named.specificity());
        assert!(named.specificity() < named_pinned.specificity());
        assert!(named.is_unfingerprinted());
        assert!(!named_pinned.is_unfingerprinted());
        assert!(!wildcard.is_unfingerprinted());
    }

    #[test]
    fn duplicate_matchers_finds_rules_that_cannot_be_told_apart() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "one".into(),
                    exe: "game.exe".into(),
                    company: Some("Acme".into()),
                    ..Rule::default()
                },
                Rule {
                    name: "two".into(),
                    exe: "GAME.EXE".into(),
                    company: Some("Acme".into()),
                    ..Rule::default()
                },
                Rule {
                    name: "three".into(),
                    exe: "game.exe".into(),
                    ..Rule::default()
                },
            ],
        };
        assert_eq!(
            table.duplicate_matchers(),
            vec![("one".to_string(), "two".to_string())]
        );
    }

    #[test]
    fn wildcard_rules_are_not_duplicates_of_each_other() {
        // Several `*` rules are defaults that stack. A wildcard that pins a
        // fingerprint is an ordinary matcher and is still reported.
        let table = Table {
            rules: vec![
                Rule {
                    name: "no-mono-gecko".into(),
                    exe: ANY_EXE.into(),
                    dll_overrides: vec!["mscoree,mshtml=".into()],
                    ..Rule::default()
                },
                Rule {
                    name: "dpi-aware".into(),
                    exe: ANY_EXE.into(),
                    dpi_aware: Some(true),
                    ..Rule::default()
                },
                Rule {
                    name: "global".into(),
                    exe: ANY_EXE.into(),
                    ..Rule::default()
                },
                Rule {
                    name: "acme-one".into(),
                    exe: ANY_EXE.into(),
                    company: Some("Acme".into()),
                    ..Rule::default()
                },
                Rule {
                    name: "acme-two".into(),
                    exe: ANY_EXE.into(),
                    company: Some("Acme".into()),
                    ..Rule::default()
                },
            ],
        };
        assert_eq!(
            table.duplicate_matchers(),
            vec![("acme-one".to_string(), "acme-two".to_string())]
        );
    }

    /// The arches of an x86 program, which every implementation serves.
    const X86_ARCHES: [Arch; 2] = [Arch::X86_64, Arch::I386];

    #[test]
    fn an_x86_program_defaults_to_dxmt_and_gets_what_it_asks_for() {
        for arch in X86_ARCHES {
            assert_eq!(Dxgi::default_for(arch), Dxgi::Dxmt, "{arch:?}");
            for dxgi in [Dxgi::Dxmt, Dxgi::Wined3d] {
                assert_eq!(Dxgi::effective(Some(dxgi), arch), dxgi);
            }
            assert_eq!(Dxgi::effective(None, arch), Dxgi::Dxmt);
        }
        assert_eq!(Dxgi::Wined3d.as_str(), "wined3d");
        assert_eq!(Dxgi::parse("DXMT"), Some(Dxgi::Dxmt));
        assert_eq!(Dxgi::parse("default"), None);
    }

    #[test]
    fn gptk_is_no_value_of_the_config_but_reads_as_dxmt_on_the_wire() -> TestResult {
        // A launcher's own config must not offer it any more.
        assert_eq!(Dxgi::parse("gptk"), None);
        // A table that still carries it, from a launcher written for both
        // Wines, keeps its record, with the implementation the arm64 Wine
        // always loaded in its place.
        let (table, diagnostics) =
            Table::parse("name=a;exe=a.exe;dxgi=gptk\nname=b;exe=b.exe;dxgi=GPTK");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(table.rules.len(), 2);
        for rule in &table.rules {
            assert_eq!(rule.dxgi, Some(Dxgi::Dxmt), "{}", rule.name);
        }
        // Written back, it is dxmt.
        let first = table.rules.first().ok_or("no rule")?;
        assert_eq!(rule_to_record(first), "name=a;exe=a.exe;dxgi=dxmt");
        // Every arch, a native ARM64 one included, gets DXMT for it.
        let resolved = table.resolve("a.exe", &VersionInfo::default());
        for arch in [Arch::Aarch64, Arch::X86_64, Arch::I386] {
            assert_eq!(Dxgi::effective(resolved.dxgi, arch), Dxgi::Dxmt, "{arch:?}");
        }
        Ok(())
    }

    #[test]
    fn a_native_arm64_program_defaults_to_dxmt_and_gets_what_it_asks_for() {
        let native = Arch::Aarch64;
        assert_eq!(Dxgi::default_for(native), Dxgi::Dxmt);
        assert_eq!(Dxgi::effective(None, native), Dxgi::Dxmt);
        for dxgi in [Dxgi::Dxmt, Dxgi::Wined3d] {
            assert_eq!(Dxgi::effective(Some(dxgi), native), dxgi);
        }
    }

    #[test]
    fn a_native_arm64_program_defaults_to_mtld3d_and_gets_what_it_asks_for() {
        let native = Arch::Aarch64;
        assert_eq!(D3d9::default_for(native), D3d9::Mtld3d);
        assert_eq!(D3d9::effective(None, native), D3d9::Mtld3d);
        for d3d9 in [D3d9::Mtld3d, D3d9::Wined3d] {
            assert_eq!(D3d9::effective(Some(d3d9), native), d3d9);
        }
    }

    #[test]
    fn a_64_bit_process_loads_aarch64_builtins() {
        assert_eq!(Arch::X86_64.pe_dir(), "aarch64-windows");
        assert_eq!(Arch::Aarch64.pe_dir(), "aarch64-windows");
        assert_eq!(Arch::I386.pe_dir(), "i386-windows");
    }

    #[test]
    fn the_process_arch_comes_from_wow64_and_the_main_image() {
        const AMD64: u16 = 0x8664;
        for machine in [None, Some(AMD64), Some(IMAGE_FILE_MACHINE_ARM64)] {
            assert_eq!(Arch::of_process(true, machine), Arch::I386);
        }
        assert_eq!(
            Arch::of_process(false, Some(IMAGE_FILE_MACHINE_ARM64)),
            Arch::Aarch64
        );
        // An x64 program under emulation, an ARM64EC one and an unreadable
        // header all stay x86_64.
        for machine in [None, Some(AMD64), Some(0)] {
            assert_eq!(Arch::of_process(false, machine), Arch::X86_64);
        }
    }

    #[test]
    fn d3d9_defaults_to_mtld3d_for_every_x86_program() {
        for arch in X86_ARCHES {
            assert_eq!(D3d9::default_for(arch), D3d9::Mtld3d);
            for requested in [None, Some(D3d9::Mtld3d)] {
                assert_eq!(D3d9::effective(requested, arch), D3d9::Mtld3d);
            }
            assert_eq!(D3d9::effective(Some(D3d9::Wined3d), arch), D3d9::Wined3d);
        }
        assert_eq!(D3d9::parse("wined3d"), Some(D3d9::Wined3d));
        assert_eq!(D3d9::parse("gptk"), None);
        assert_eq!(D3d9::Mtld3d.as_str(), "mtld3d");
    }

    #[test]
    fn the_two_implementations_resolve_independently() {
        let table = Table {
            rules: vec![
                Rule {
                    name: "defaults".into(),
                    exe: ANY_EXE.into(),
                    dxgi: Some(Dxgi::Wined3d),
                    ..Rule::default()
                },
                Rule {
                    name: "game".into(),
                    exe: "game.exe".into(),
                    d3d9: Some(D3d9::Wined3d),
                    ..Rule::default()
                },
            ],
        };
        let r = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(r.dxgi, Some(Dxgi::Wined3d));
        assert_eq!(r.d3d9, Some(D3d9::Wined3d));
        let other = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(other.dxgi, Some(Dxgi::Wined3d));
        assert_eq!(other.d3d9, None);
    }

    fn pair(name: &str, value: &str) -> (String, String) {
        (name.into(), value.into())
    }

    #[test]
    fn wined3d_parses_round_trips_and_rejects_other_values() {
        assert_eq!(Wined3d::parse("GL"), Some(Wined3d::Gl));
        assert_eq!(Wined3d::parse("Vulkan"), Some(Wined3d::Vulkan));
        assert_eq!(Wined3d::parse("opengl"), None);
        assert_eq!(Wined3d::parse("dxmt"), None);
        assert_eq!(Wined3d::Vulkan.as_str(), "vulkan");
        let table = Table {
            rules: vec![
                Rule {
                    name: "vk".into(),
                    exe: "vk.exe".into(),
                    wined3d: Some(Wined3d::Vulkan),
                    ..Rule::default()
                },
                Rule {
                    name: "gl".into(),
                    exe: "gl.exe".into(),
                    wined3d: Some(Wined3d::Gl),
                    ..Rule::default()
                },
                Rule {
                    name: "unset".into(),
                    exe: "unset.exe".into(),
                    ..Rule::default()
                },
            ],
        };
        let value = table.to_env_value();
        assert!(
            value.contains("name=vk;exe=vk.exe;wined3d=vulkan"),
            "{value}"
        );
        assert!(value.contains("name=gl;exe=gl.exe;wined3d=gl"), "{value}");
        assert!(
            !value.contains("name=unset;exe=unset.exe;wined3d"),
            "{value}"
        );
        let (parsed, diagnostics) = Table::parse(&value);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(parsed, table);
        let (bad, diagnostics) =
            Table::parse("name=x;exe=x.exe;wined3d=opengl\nname=ok;exe=ok.exe");
        assert_eq!(bad.rules.len(), 2);
        assert_eq!(bad.rules.first().map(|r| r.wined3d), Some(None));
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn wined3d_resolves_to_the_most_specific_rule_and_overlays_by_name() {
        let mut table = Table {
            rules: vec![
                Rule {
                    name: "game".into(),
                    exe: "game.exe".into(),
                    wined3d: Some(Wined3d::Gl),
                    ..Rule::default()
                },
                Rule {
                    name: "global".into(),
                    exe: ANY_EXE.into(),
                    wined3d: Some(Wined3d::Vulkan),
                    ..Rule::default()
                },
            ],
        };
        let none = VersionInfo::default();
        assert_eq!(table.resolve("game.exe", &none).wined3d, Some(Wined3d::Gl));
        assert_eq!(
            table.resolve("other.exe", &none).wined3d,
            Some(Wined3d::Vulkan)
        );
        // An override by name changes the setting and keeps the rest.
        let (over, diagnostics) = Table::parse("name=game;wined3d=vulkan");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(table.overlay(over), [] as [String; 0]);
        assert_eq!(
            table.resolve("game.exe", &none).wined3d,
            Some(Wined3d::Vulkan)
        );
        // Dropping the wildcard leaves other processes with no opinion.
        let (over, _) = Table::parse("name=global;enabled=false");
        assert_eq!(table.overlay(over), [] as [String; 0]);
        assert_eq!(table.resolve("other.exe", &none).wined3d, None);
        assert_eq!(
            table.resolve("game.exe", &none).wined3d,
            Some(Wined3d::Vulkan)
        );
    }

    #[test]
    fn a_set_renderer_is_always_named_and_unset_sets_nothing() {
        let resolved = |wined3d| Resolution {
            wined3d,
            env: vec![pair("A", "1")],
            ..Resolution::default()
        };
        // Each renderer a rule sets goes after the rules' env, OpenGL as
        // explicitly as Vulkan, so a child never keeps its parent's Vulkan.
        for (wined3d, value) in [
            (Wined3d::Vulkan, "renderer=vulkan"),
            (Wined3d::Gl, "renderer=gl"),
        ] {
            let mut r = resolved(Some(wined3d));
            assert_eq!(r.apply_wined3d(), Some(wined3d));
            assert_eq!(r.env, [pair("A", "1"), pair("WINE_D3D_CONFIG", value)]);
        }
        // No rule sets it: nothing changes.
        let mut r = resolved(None);
        assert_eq!(r.apply_wined3d(), None);
        assert_eq!(r.env, [pair("A", "1")]);
    }

    #[test]
    fn the_setting_decides_the_renderer_and_keeps_the_other_options_of_an_env_rule() -> TestResult {
        // A rule's own WINE_D3D_CONFIG keeps its other options, loses its
        // renderer, and the setting's entry is the value the block ends with.
        for (wined3d, renderer) in [(Wined3d::Vulkan, "vulkan"), (Wined3d::Gl, "gl")] {
            let mut r = Resolution {
                wined3d: Some(wined3d),
                env: vec![
                    pair("wine_d3d_config", "csmt=0"),
                    pair(
                        "WINE_D3D_CONFIG",
                        "renderer=no3d,csmt=0x1;MaxVersionGL=0x40001",
                    ),
                ],
                ..Resolution::default()
            };
            assert_eq!(r.apply_wined3d(), Some(wined3d));
            let want = format!("renderer={renderer},csmt=0x1,MaxVersionGL=0x40001");
            assert_eq!(r.env.last(), Some(&pair("WINE_D3D_CONFIG", &want)));
            let block: Vec<u16> = "WINE_D3D_CONFIG=shader_backend=glsl\0OTHER=x\0\0"
                .encode_utf16()
                .collect();
            let merged = merge_env(&block, &r.env).ok_or("unchanged")?;
            assert_eq!(
                String::from_utf16_lossy(&merged),
                format!("WINE_D3D_CONFIG={want}\0OTHER=x\0\0")
            );
        }
        // A rule that removes the variable leaves just the renderer.
        let mut r = Resolution {
            wined3d: Some(Wined3d::Vulkan),
            env: vec![pair("WINE_D3D_CONFIG", "")],
            ..Resolution::default()
        };
        r.apply_wined3d();
        assert_eq!(
            r.env.last(),
            Some(&pair("WINE_D3D_CONFIG", "renderer=vulkan"))
        );
        // Without the setting a rule's WINE_D3D_CONFIG applies as it is,
        // renderer and all.
        let mut r = Resolution {
            env: vec![pair("WINE_D3D_CONFIG", "renderer=vulkan,csmt=0")],
            ..Resolution::default()
        };
        assert_eq!(r.apply_wined3d(), None);
        assert_eq!(r.env, [pair("WINE_D3D_CONFIG", "renderer=vulkan,csmt=0")]);
        assert_eq!(r.d3d_config(), Some("renderer=vulkan,csmt=0"));
        Ok(())
    }

    #[test]
    fn the_renderer_of_a_config_and_a_variable_of_a_block_read_as_wined3d_reads_them() {
        assert_eq!(
            Wined3d::renderer_in("csmt=0,renderer=vulkan"),
            Some("vulkan")
        );
        assert_eq!(
            Wined3d::renderer_in("renderer=gl;renderer=vulkan"),
            Some("gl")
        );
        assert_eq!(Wined3d::renderer_in("csmt=0,max_renderer=x"), None);
        assert_eq!(Wined3d::renderer_in(""), None);
        let block: Vec<u16> = "=C:=C:\\\0wine_d3d_config=renderer=vulkan\0WINE_D3D_CONFIG=x\0\0"
            .encode_utf16()
            .collect();
        assert_eq!(
            env_value(&block, "WINE_D3D_CONFIG").as_deref(),
            Some("renderer=vulkan")
        );
        assert_eq!(env_value(&block, "OTHER"), None);
        let empty: Vec<u16> = "A=\0\0".encode_utf16().collect();
        assert_eq!(env_value(&empty, "a").as_deref(), Some(""));
    }
}
