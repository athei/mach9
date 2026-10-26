//! A rule: the processes it matches and the settings it applies to them.

use crate::{
    Argument, D3d9, DebugChannel, DllOverride, Dxgi, EnvEntry, Exe, Fingerprint, OpenGl, Scalar,
    ScalarValue, Specificity, VersionInfo, Wined3d,
};
#[cfg(doc)]
use crate::{Origin, Resolution, Source, Table, metal_hud_env};

/// One rule of the table: what it matches, and the settings it applies to a
/// process that matches. A rule has no name; `exe` is its only required
/// matcher, and everything else is optional.
///
/// [`Rule::new`] makes one for an `exe`, and the `with_` methods add what it
/// matches on and sets. A rule is its content alone: where it comes from is
/// the [`Source`] the [`Table`] keeps beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The executable it matches.
    pub(crate) exe: Exe,
    /// Substring the image's version `CompanyName` must contain.
    pub(crate) company: Option<Fingerprint>,
    /// Substring the image's version `ProductName` must contain.
    pub(crate) product: Option<Fingerprint>,
    /// Substring the image's version `OriginalFilename` must contain.
    pub(crate) original_filename: Option<Fingerprint>,
    /// See [`Rule::dxgi`].
    pub(crate) dxgi: Setting<Dxgi>,
    /// See [`Rule::d3d9`].
    pub(crate) d3d9: Setting<D3d9>,
    /// See [`Rule::wined3d`].
    pub(crate) wined3d: Setting<Wined3d>,
    /// See [`Rule::opengl`].
    pub(crate) opengl: Setting<OpenGl>,
    /// See [`Rule::dpi_aware`].
    pub(crate) dpi_aware: Setting<bool>,
    /// See [`Rule::large_address_aware`].
    pub(crate) large_address_aware: Setting<bool>,
    /// See [`Rule::gcmouse`].
    pub(crate) gcmouse: Setting<bool>,
    /// See [`Rule::force_nx`].
    pub(crate) force_nx: Setting<bool>,
    /// See [`Rule::metal_hud`].
    pub(crate) metal_hud: Setting<bool>,
    /// See [`Rule::dll_overrides`].
    pub(crate) dll_overrides: Vec<DllOverride>,
    /// See [`Rule::arguments`].
    pub(crate) arguments: Vec<Argument>,
    /// See [`Rule::arguments_override`].
    pub(crate) arguments_override: Option<Vec<Argument>>,
    /// See [`Rule::debug_channels`].
    pub(crate) debug_channels: Vec<DebugChannel>,
    /// See [`Rule::env`].
    pub(crate) env: Vec<EnvEntry>,
}

/// The value of a scalar setting that resets it (see [`Setting::Reset`]),
/// and of a `dll_overrides` entry that removes the overrides of its modules.
///
/// Like every word of the format it is matched ASCII-case-insensitively and
/// written in lowercase.
pub const DEFAULT_VALUE: &str = "default";

/// What a rule says about a setting that holds one value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Setting<T> {
    /// Nothing: a rule folded earlier decides, and with none upstream Wine.
    #[default]
    Unset,
    /// This value, which beats that of every rule folded earlier.
    Set(T),
    /// `default`: from this rule on the setting is unset again, so upstream
    /// Wine decides unless a rule folded later sets it.
    Reset,
}

impl<T> Setting<T> {
    /// The value it sets, or `None` when it is unset or reset.
    #[must_use]
    pub fn value(self) -> Option<T> {
        match self {
            Self::Set(value) => Some(value),
            Self::Unset | Self::Reset => None,
        }
    }

    /// Whether it says nothing.
    #[must_use]
    pub const fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }

    /// The same setting with the value, if any, passed through `f`.
    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Setting<U> {
        match self {
            Self::Unset => Setting::Unset,
            Self::Set(value) => Setting::Set(f(value)),
            Self::Reset => Setting::Reset,
        }
    }
}

impl<T> From<T> for Setting<T> {
    /// The setting that sets `value`.
    fn from(value: T) -> Self {
        Self::Set(value)
    }
}

impl Default for Rule {
    /// The rule for every process that sets nothing, [`Rule::new`] of
    /// [`Exe::Any`].
    fn default() -> Self {
        Self::new(Exe::Any)
    }
}

impl Rule {
    /// The rule for the processes `exe` matches, with no fingerprint and no
    /// setting.
    #[must_use]
    pub const fn new(exe: Exe) -> Self {
        Self {
            exe,
            company: None,
            product: None,
            original_filename: None,
            dxgi: Setting::Unset,
            d3d9: Setting::Unset,
            wined3d: Setting::Unset,
            opengl: Setting::Unset,
            dpi_aware: Setting::Unset,
            large_address_aware: Setting::Unset,
            gcmouse: Setting::Unset,
            force_nx: Setting::Unset,
            metal_hud: Setting::Unset,
            dll_overrides: Vec::new(),
            arguments: Vec::new(),
            arguments_override: None,
            debug_channels: Vec::new(),
            env: Vec::new(),
        }
    }

    /// The rule that also pins `company` on the image's `CompanyName`.
    #[must_use]
    pub fn with_company(mut self, company: Fingerprint) -> Self {
        self.company = Some(company);
        self
    }

    /// The rule that also pins `product` on the image's `ProductName`.
    #[must_use]
    pub fn with_product(mut self, product: Fingerprint) -> Self {
        self.product = Some(product);
        self
    }

    /// The rule that also pins `original_filename` on the image's
    /// `OriginalFilename`.
    #[must_use]
    pub fn with_original_filename(mut self, original_filename: Fingerprint) -> Self {
        self.original_filename = Some(original_filename);
        self
    }

    /// The rule with `dxgi` for [`Rule::dxgi`].
    #[must_use]
    pub fn with_dxgi(mut self, dxgi: impl Into<Setting<Dxgi>>) -> Self {
        self.dxgi = dxgi.into();
        self
    }

    /// The rule with `d3d9` for [`Rule::d3d9`].
    #[must_use]
    pub fn with_d3d9(mut self, d3d9: impl Into<Setting<D3d9>>) -> Self {
        self.d3d9 = d3d9.into();
        self
    }

    /// The rule with `wined3d` for [`Rule::wined3d`].
    #[must_use]
    pub fn with_wined3d(mut self, wined3d: impl Into<Setting<Wined3d>>) -> Self {
        self.wined3d = wined3d.into();
        self
    }

    /// The rule with `opengl` for [`Rule::opengl`].
    #[must_use]
    pub fn with_opengl(mut self, opengl: impl Into<Setting<OpenGl>>) -> Self {
        self.opengl = opengl.into();
        self
    }

    /// The rule with `aware` for [`Rule::dpi_aware`].
    #[must_use]
    pub fn with_dpi_aware(mut self, aware: impl Into<Setting<bool>>) -> Self {
        self.dpi_aware = aware.into();
        self
    }

    /// The rule with `laa` for [`Rule::large_address_aware`].
    #[must_use]
    pub fn with_large_address_aware(mut self, laa: impl Into<Setting<bool>>) -> Self {
        self.large_address_aware = laa.into();
        self
    }

    /// The rule with `on` for [`Rule::gcmouse`].
    #[must_use]
    pub fn with_gcmouse(mut self, on: impl Into<Setting<bool>>) -> Self {
        self.gcmouse = on.into();
        self
    }

    /// The rule with `on` for [`Rule::force_nx`].
    #[must_use]
    pub fn with_force_nx(mut self, on: impl Into<Setting<bool>>) -> Self {
        self.force_nx = on.into();
        self
    }

    /// The rule with `on` for [`Rule::metal_hud`].
    #[must_use]
    pub fn with_metal_hud(mut self, on: impl Into<Setting<bool>>) -> Self {
        self.metal_hud = on.into();
        self
    }

    /// The rule with `entry` after its other [`Rule::debug_channels`].
    #[must_use]
    pub fn with_debug_channel(mut self, entry: DebugChannel) -> Self {
        self.debug_channels.push(entry);
        self
    }

    /// The rule with `entry` after its other [`Rule::dll_overrides`].
    #[must_use]
    pub fn with_dll_override(mut self, entry: DllOverride) -> Self {
        self.dll_overrides.push(entry);
        self
    }

    /// The rule with `argument` after its other [`Rule::arguments`].
    #[must_use]
    pub fn with_argument(mut self, argument: Argument) -> Self {
        self.arguments.push(argument);
        self
    }

    /// The rule whose [`Rule::arguments_override`] is `arguments`, which
    /// may be empty to clear the arguments.
    #[must_use]
    pub fn with_arguments_override(
        mut self,
        arguments: impl IntoIterator<Item = Argument>,
    ) -> Self {
        self.arguments_override = Some(arguments.into_iter().collect());
        self
    }

    /// The rule with `entry` after its other [`Rule::env`] entries.
    #[must_use]
    pub fn with_env(mut self, entry: EnvEntry) -> Self {
        self.env.push(entry);
        self
    }

    /// The executable the rule matches.
    #[must_use]
    pub const fn exe(&self) -> &Exe {
        &self.exe
    }

    /// The substring the image's version `CompanyName` must contain, if the
    /// rule pins one.
    #[must_use]
    pub const fn company(&self) -> Option<&Fingerprint> {
        self.company.as_ref()
    }

    /// The substring the image's version `ProductName` must contain, if the
    /// rule pins one.
    #[must_use]
    pub const fn product(&self) -> Option<&Fingerprint> {
        self.product.as_ref()
    }

    /// The substring the image's version `OriginalFilename` must contain, if
    /// the rule pins one.
    #[must_use]
    pub const fn original_filename(&self) -> Option<&Fingerprint> {
        self.original_filename.as_ref()
    }

    /// The DXGI-family implementation to load.
    #[must_use]
    pub const fn dxgi(&self) -> Setting<Dxgi> {
        self.dxgi
    }

    /// The D3D9 implementation to load.
    #[must_use]
    pub const fn d3d9(&self) -> Setting<D3d9> {
        self.d3d9
    }

    /// The renderer of wined3d, for every wined3d user in the process.
    #[must_use]
    pub const fn wined3d(&self) -> Setting<Wined3d> {
        self.wined3d
    }

    /// Which OpenGL the Mac driver gives the process. With no rule setting
    /// it, the Mac driver gives Apple's OpenGL, as upstream Wine does.
    #[must_use]
    pub const fn opengl(&self) -> Setting<OpenGl> {
        self.opengl
    }

    /// Whether the process is DPI-aware (system-aware) or unaware. With no
    /// rule setting it, wine decides (the registry and the manifest).
    #[must_use]
    pub const fn dpi_aware(&self) -> Setting<bool> {
        self.dpi_aware
    }

    /// Whether a 32-bit process gets the 4 GB address space whatever its
    /// executable's `IMAGE_FILE_LARGE_ADDRESS_AWARE` flag says. `false` does
    /// not take the space away from an executable that has the flag; it only
    /// stops forcing it. Has no effect on a 64-bit process.
    #[must_use]
    pub const fn large_address_aware(&self) -> Setting<bool> {
        self.large_address_aware
    }

    /// Whether the Mac driver takes the mouse movement of raw input
    /// (`WM_INPUT`) and `DirectInput` from GameController's `GCMouse`, in the
    /// device's own counts, instead of from the cursor's movement, which macOS
    /// accelerates and scales for retina. With no rule setting it, the Mac
    /// driver keeps upstream Wine's behaviour, the cursor's movement.
    #[must_use]
    pub const fn gcmouse(&self) -> Setting<bool> {
        self.gcmouse
    }

    /// Whether no-execute stays permanently on in a 32-bit process, even when
    /// its executable or a DLL lacks the `NX_COMPAT` flag, for which Wine
    /// turns it off. `false` is the same as unset: Wine's behaviour. Has no
    /// effect on a 64-bit process, which always has no-execute on. Unlike an
    /// `env` entry, the value is the process's own: the processes it starts
    /// get their own rules' value.
    #[must_use]
    pub const fn force_nx(&self) -> Setting<bool> {
        self.force_nx
    }

    /// Whether the Metal performance HUD shows in the process. `compatdb.so`
    /// turns the value into the environment entries of [`metal_hud_env`],
    /// before this rule's own `env` entries (see
    /// [`Resolution::fold_derived_env`]), so an `env` entry for one of the
    /// variables wins: `true` sets the variables that show the HUD, `false`
    /// removes them. Like [`Rule::wined3d`] and unlike the other scalar
    /// settings it reaches the programs the process starts, which inherit
    /// the variables. With no rule setting it, the process keeps the
    /// variables it was started with.
    #[must_use]
    pub const fn metal_hud(&self) -> Setting<bool> {
        self.metal_hud
    }

    /// `WINEDEBUG` entries for the processes the rule names. Wine reads
    /// `WINEDEBUG` before `compatdb.so` loads, so the library only logs
    /// them; a launcher puts them into the variable it starts Wine with
    /// (see [`Table::winedebug`]).
    #[must_use]
    pub fn debug_channels(&self) -> &[DebugChannel] {
        &self.debug_channels
    }

    /// `WINEDLLOVERRIDES` elements. An entry whose order is `default` sets no
    /// load order: it removes the entries that the rules folded before it
    /// gave its modules, so the registry and `WINEDLLOVERRIDES` decide for
    /// them again.
    #[must_use]
    pub fn dll_overrides(&self) -> &[DllOverride] {
        &self.dll_overrides
    }

    /// Text appended to the command line, unless already present.
    #[must_use]
    pub fn arguments(&self) -> &[Argument] {
        &self.arguments
    }

    /// When set, the arguments that replace the list the rules folded before
    /// this one accumulated (see [`Table::resolve`]); `Some` with no entries
    /// clears it. This rule's own [`Rule::arguments`] and those of the rules
    /// folded after it still append.
    #[must_use]
    pub fn arguments_override(&self) -> Option<&[Argument]> {
        self.arguments_override.as_deref()
    }

    /// Environment entries. The library sets them in the process's
    /// environment block and unix environment when it loads, before any PE
    /// code runs. The entries of a built-in rule that matches every process
    /// (`exe` `*`, no fingerprint) are defaults: such an entry sets its
    /// variable only where the process does not have it, so a value the
    /// caller exported wins (see [`Origin::env_is_default`]). Every other
    /// entry replaces the variable.
    #[must_use]
    pub fn env(&self) -> &[EnvEntry] {
        &self.env
    }

    /// Whether the rule sets nothing beyond its matchers: no scalar setting
    /// is set or reset and every list is empty, `arguments_override`
    /// included. Such a rule changes nothing in any process, so a launcher
    /// can leave it out of the table, for example the rule for every process
    /// that its launch-wide settings make when they set nothing.
    #[must_use]
    pub fn sets_nothing(&self) -> bool {
        Scalar::ALL
            .iter()
            .all(|&scalar| self.scalar(scalar).is_unset())
            && self.dll_overrides.is_empty()
            && self.arguments.is_empty()
            && self.arguments_override.is_none()
            && self.debug_channels.is_empty()
            && self.env.is_empty()
    }

    /// What this rule says about the scalar setting `scalar`.
    #[must_use]
    pub fn scalar(&self, scalar: Scalar) -> Setting<ScalarValue> {
        match scalar {
            Scalar::Dxgi => self.dxgi.map(ScalarValue::Dxgi),
            Scalar::D3d9 => self.d3d9.map(ScalarValue::D3d9),
            Scalar::Wined3d => self.wined3d.map(ScalarValue::Wined3d),
            Scalar::OpenGl => self.opengl.map(ScalarValue::OpenGl),
            Scalar::DpiAware => self.dpi_aware.map(ScalarValue::Bool),
            Scalar::LargeAddressAware => self.large_address_aware.map(ScalarValue::Bool),
            Scalar::Gcmouse => self.gcmouse.map(ScalarValue::Bool),
            Scalar::ForceNx => self.force_nx.map(ScalarValue::Bool),
            Scalar::MetalHud => self.metal_hud.map(ScalarValue::Bool),
        }
    }

    /// Make this rule say `setting` about the scalar setting `scalar`. A
    /// value of another setting leaves the rule as it is.
    pub(crate) fn put_scalar(&mut self, scalar: Scalar, setting: Setting<ScalarValue>) {
        /// Put `setting` into `slot`, its value taken out by `pick`, which
        /// gives `None` for a value of another setting.
        fn put<T>(
            slot: &mut Setting<T>,
            setting: Setting<ScalarValue>,
            pick: impl FnOnce(ScalarValue) -> Option<T>,
        ) {
            *slot = match setting {
                Setting::Unset => Setting::Unset,
                Setting::Reset => Setting::Reset,
                Setting::Set(value) => match pick(value) {
                    Some(value) => Setting::Set(value),
                    None => return,
                },
            };
        }
        let on = |value| match value {
            ScalarValue::Bool(on) => Some(on),
            _ => None,
        };
        match scalar {
            Scalar::Dxgi => put(&mut self.dxgi, setting, |value| match value {
                ScalarValue::Dxgi(dxgi) => Some(dxgi),
                _ => None,
            }),
            Scalar::D3d9 => put(&mut self.d3d9, setting, |value| match value {
                ScalarValue::D3d9(d3d9) => Some(d3d9),
                _ => None,
            }),
            Scalar::Wined3d => put(&mut self.wined3d, setting, |value| match value {
                ScalarValue::Wined3d(wined3d) => Some(wined3d),
                _ => None,
            }),
            Scalar::OpenGl => put(&mut self.opengl, setting, |value| match value {
                ScalarValue::OpenGl(opengl) => Some(opengl),
                _ => None,
            }),
            Scalar::DpiAware => put(&mut self.dpi_aware, setting, on),
            Scalar::LargeAddressAware => put(&mut self.large_address_aware, setting, on),
            Scalar::Gcmouse => put(&mut self.gcmouse, setting, on),
            Scalar::ForceNx => put(&mut self.force_nx, setting, on),
            Scalar::MetalHud => put(&mut self.metal_hud, setting, on),
        }
    }

    /// Whether this rule applies to a process with the given image basename
    /// and version resource. `exe` matches the basename case-insensitively, or
    /// every basename when it is [`Exe::Any`]; a fingerprint field, when
    /// present, is an additional case-insensitive substring constraint on the
    /// corresponding version string (so a rule with none matches on the
    /// basename alone).
    #[must_use]
    pub fn matches(&self, exe: &str, version: &VersionInfo) -> bool {
        let pin = |pin: Option<&Fingerprint>, actual: &str| pin.is_none_or(|p| p.matches(actual));
        self.exe.matches(exe)
            && pin(self.company.as_ref(), &version.company)
            && pin(self.product.as_ref(), &version.product)
            && pin(self.original_filename.as_ref(), &version.original_filename)
    }

    /// How specific this rule is, which is the order [`Table::resolve`] folds
    /// the matching rules of a layer in: a wildcard rule first (a launch-wide
    /// default), then one naming an executable, then one that also pins a
    /// version fingerprint. Within a layer a more specific rule therefore
    /// wins a scalar field.
    #[must_use]
    pub const fn specificity(&self) -> Specificity {
        let fingerprinted =
            self.company.is_some() || self.product.is_some() || self.original_filename.is_some();
        match (self.exe.is_any(), fingerprinted) {
            (true, false) => Specificity::Any,
            (true, true) => Specificity::AnyWithFingerprint,
            (false, false) => Specificity::Exe,
            (false, true) => Specificity::ExeWithFingerprint,
        }
    }

    /// Whether this rule matches on its executable alone. Such a rule catches
    /// every program with that file name, which is worth a warning for all but
    /// the few basenames that are unique in practice.
    #[must_use]
    pub const fn is_unfingerprinted(&self) -> bool {
        matches!(self.specificity(), Specificity::Exe)
    }

    /// Whether two rules would match exactly the same processes, which can only
    /// be a mistake in a hand-written table.
    #[must_use]
    pub fn matches_the_same_as(&self, other: &Self) -> bool {
        self.exe.as_str().eq_ignore_ascii_case(other.exe.as_str())
            && self.company == other.company
            && self.product == other.product
            && self.original_filename == other.original_filename
    }

    /// The matchers of this rule as the log prints them: the `exe`, then the
    /// fingerprint values it pins in parentheses, for example
    /// `WoW.exe (Blizzard Entertainment, World of Warcraft, WoW.exe)`.
    #[must_use]
    pub fn matchers(&self) -> String {
        let pins: Vec<&str> = [&self.company, &self.product, &self.original_filename]
            .into_iter()
            .filter_map(|pin| pin.as_ref().map(Fingerprint::as_str))
            .collect();
        if pins.is_empty() {
            self.exe.to_string()
        } else {
            format!("{} ({})", self.exe, pins.join(", "))
        }
    }
}

#[cfg(test)]
mod tests;
