//! Folding the rules that match a process into the settings it gets, with
//! where each value came from.

use std::collections::BTreeMap;

#[cfg(doc)]
use crate::runtime::{basename_of, steam_app};
use crate::{
    Argument, D3d9, DebugChannel, DllOverride, Dxgi, ENV_VAR, EnvEntry, FILE_ENV_VAR, OpenGl, Rule,
    Scalar, ScalarValue, Setting, Source, Specificity, Table, VersionInfo, Wined3d, metal_hud_env,
    runtime,
};

/// The settings that apply to one process.
///
/// Every matching rule folded in the order [`Table::resolve`] describes, so a
/// scalar holds the value of the last rule that set it and the lists
/// accumulate. Also says where each value came from, for the log.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The state of each scalar setting a rule set or reset; one that is
    /// missing is unset.
    scalars: BTreeMap<Scalar, ScalarState>,
    /// The `dll_overrides` entries of every matching rule in fold order,
    /// without the `default` entries, which only take modules out of the
    /// entries before them.
    pub dll_overrides: Vec<DllOverride>,
    /// Where each entry of [`Resolution::dll_overrides`] came from, at the
    /// same index.
    pub dll_overrides_from: Vec<Origin>,
    /// Each module a `<module>=default` entry named, in fold order, with
    /// what that did to the entries before it.
    pub dll_overrides_reset: Vec<DllOverrideReset>,
    /// The texts to append to the command line, in fold order, after any
    /// `arguments_override` replaced what came before it.
    pub arguments: Vec<Argument>,
    /// Where each entry of [`Resolution::arguments`] came from, at the same
    /// index.
    pub arguments_from: Vec<Origin>,
    /// Each `arguments_override` that dropped entries from the argument
    /// list, in fold order.
    pub arguments_dropped: Vec<ArgumentsDropped>,
    /// The rule of each `arguments_override` that was ignored, in fold order:
    /// that of a rule for a Steam app id (see [`Rule::arguments_override`]).
    pub arguments_override_ignored: Vec<Origin>,
    /// The `debug_channels` entries of every matching rule in fold order,
    /// which `compatdb.so` only logs (see [`Rule::debug_channels`]).
    pub debug_channels: Vec<DebugChannel>,
    /// Where each entry of [`Resolution::debug_channels`] came from, at the
    /// same index.
    pub debug_channels_from: Vec<Origin>,
    /// The `env` entries of every matching rule in fold order, followed by
    /// those [`Resolution::fold_derived_env`] adds.
    /// [`Resolution::env_steps`] says which of them apply in a process.
    pub env: Vec<EnvEntry>,
    /// Where each entry of [`Resolution::env`] came from, at the same index.
    pub env_from: Vec<EnvFrom>,
    /// Whether [`Resolution::fold_derived_env`] has run.
    derived_env: bool,
    /// Where in [`Resolution::env`] the rule that set `metal_hud` started
    /// its entries, which is where [`Resolution::fold_derived_env`] puts
    /// the entries of the setting.
    metal_hud_at: Option<usize>,
}

/// Where a scalar setting of a [`Resolution`] ended up.
#[non_exhaustive]
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct ScalarState {
    /// The value of the last rule folded that set it, or `None` when no rule
    /// set it or a later rule reset it.
    pub value: Option<ScalarValue>,
    /// The rule of that value; `Some` exactly when `value` is.
    pub set_by: Option<Origin>,
    /// The rule that reset the setting to `default` when no rule folded
    /// after it set it again; never `Some` together with `value`.
    pub reset_by: Option<Origin>,
}

/// The state of a setting no rule set or reset.
static UNSET: ScalarState = ScalarState {
    value: None,
    set_by: None,
    reset_by: None,
};

/// What a `<module>=default` entry of `dll_overrides` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DllOverrideReset {
    /// The module it names.
    pub module: String,
    /// The rule of the entry.
    pub by: Origin,
    /// Where each entry it took the module out of came from; empty when no
    /// rule folded before it had given the module a load order.
    pub removed: Vec<Origin>,
}

/// Where a value of a [`Resolution`] came from: the rule that set it, by its
/// [`Source`] and, for a built-in rule that does not match every process,
/// its matchers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A built-in rule that matches every process.
    BuiltIn,
    /// A built-in rule for the processes its matchers (see
    /// [`Rule::matchers`]) describe.
    BuiltInFor(String),
    /// Line `n` of [`ENV_VAR`].
    Line(usize),
    /// Line `n` of the file [`FILE_ENV_VAR`] names.
    FileLine(usize),
}

impl Origin {
    /// Where the values of `rule` come from, a rule of a table with the
    /// source `source`.
    #[must_use]
    pub fn of(source: Source, rule: &Rule) -> Self {
        match source {
            Source::Line(n) => Self::Line(n),
            Source::FileLine(n) => Self::FileLine(n),
            Source::BuiltIn if rule.specificity() == Specificity::Any => Self::BuiltIn,
            Source::BuiltIn => Self::BuiltInFor(rule.matchers()),
        }
    }

    /// Whether the `env` entries of a rule from here are defaults, which
    /// leave a variable the process already has alone: those of a built-in
    /// rule that matches every process (see [`Rule::env`]).
    #[must_use]
    pub const fn env_is_default(&self) -> bool {
        matches!(self, Self::BuiltIn)
    }

    /// How the log names it: `built-in`, `built-in for <matchers>`,
    /// `WINE_COMPATDB line <n>` or `WINE_COMPATDB_FILE line <n>`.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::BuiltIn => String::from("built-in"),
            Self::BuiltInFor(matchers) => format!("built-in for {matchers}"),
            Self::Line(n) => format!("{ENV_VAR} line {n}"),
            Self::FileLine(n) => format!("{FILE_ENV_VAR} line {n}"),
        }
    }
}

/// An `arguments_override` that dropped the arguments the rules folded
/// before it had accumulated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgumentsDropped {
    /// The rule whose `arguments_override` it is.
    pub by: Origin,
    /// The arguments it dropped, each with where it came from.
    pub entries: Vec<(Argument, Origin)>,
}

/// Where an env entry of a [`Resolution`] came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvFrom {
    /// The rule that set it.
    pub origin: Origin,
    /// Whether it is a default (see [`Origin::env_is_default`]), which leaves
    /// a variable the process already has alone.
    pub default: bool,
}

/// What one env entry of a [`Resolution`] does in a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvStep {
    /// The entry: the variable and the value it sets, or none to remove it.
    pub entry: EnvEntry,
    /// The rule that set it.
    pub origin: Origin,
    /// False for a default whose variable the process already had, which it
    /// leaves alone.
    pub applied: bool,
    /// The origin of a later entry for the same variable that applies, which
    /// replaces what this one did: the variable ends up with that entry's
    /// value.
    pub replaced_by: Option<Origin>,
    /// For an entry that removes the variable, the origin of the earlier
    /// entry for it that applied and set a value, which the removal undoes.
    pub removes: Option<Origin>,
}

impl Resolution {
    /// Whether there is nothing to apply or log: no value of a scalar
    /// setting and no entry of a list.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scalars.values().all(|state| state.value.is_none())
            && self.dll_overrides.is_empty()
            && self.arguments.is_empty()
            && self.debug_channels.is_empty()
            && self.env.is_empty()
    }

    /// Where the scalar setting `scalar` ended up: its value and the rule
    /// that set it, or the rule that reset it.
    #[must_use]
    pub fn scalar(&self, scalar: Scalar) -> &ScalarState {
        self.scalars.get(&scalar).unwrap_or(&UNSET)
    }

    /// The DXGI-family implementation (see [`Rule::dxgi`]), `None` when no
    /// rule sets it.
    #[must_use]
    pub fn dxgi(&self) -> Option<Dxgi> {
        match self.scalar(Scalar::Dxgi).value {
            Some(ScalarValue::Dxgi(dxgi)) => Some(dxgi),
            _ => None,
        }
    }

    /// The D3D9 implementation (see [`Rule::d3d9`]), `None` when no rule
    /// sets it.
    #[must_use]
    pub fn d3d9(&self) -> Option<D3d9> {
        match self.scalar(Scalar::D3d9).value {
            Some(ScalarValue::D3d9(d3d9)) => Some(d3d9),
            _ => None,
        }
    }

    /// The renderer of wined3d (see [`Rule::wined3d`]), `None` when no rule
    /// sets it.
    #[must_use]
    pub fn wined3d(&self) -> Option<Wined3d> {
        match self.scalar(Scalar::Wined3d).value {
            Some(ScalarValue::Wined3d(wined3d)) => Some(wined3d),
            _ => None,
        }
    }

    /// Which OpenGL the Mac driver gives (see [`Rule::opengl`]), `None` when
    /// no rule sets it.
    #[must_use]
    pub fn opengl(&self) -> Option<OpenGl> {
        match self.scalar(Scalar::OpenGl).value {
            Some(ScalarValue::OpenGl(opengl)) => Some(opengl),
            _ => None,
        }
    }

    /// The DPI awareness (see [`Rule::dpi_aware`]), `None` when no rule sets
    /// it.
    #[must_use]
    pub fn dpi_aware(&self) -> Option<bool> {
        self.switch(Scalar::DpiAware)
    }

    /// Whether large address awareness is forced (see
    /// [`Rule::large_address_aware`]), `None` when no rule sets it.
    #[must_use]
    pub fn large_address_aware(&self) -> Option<bool> {
        self.switch(Scalar::LargeAddressAware)
    }

    /// Whether mouse movement comes from `GCMouse` (see [`Rule::gcmouse`]),
    /// `None` when no rule sets it.
    #[must_use]
    pub fn gcmouse(&self) -> Option<bool> {
        self.switch(Scalar::Gcmouse)
    }

    /// Whether no-execute stays on (see [`Rule::force_nx`]), `None` when no
    /// rule sets it.
    #[must_use]
    pub fn force_nx(&self) -> Option<bool> {
        self.switch(Scalar::ForceNx)
    }

    /// Whether the Metal performance HUD shows (see [`Rule::metal_hud`]),
    /// `None` when no rule sets it.
    #[must_use]
    pub fn metal_hud(&self) -> Option<bool> {
        self.switch(Scalar::MetalHud)
    }

    /// The value of the true-or-false setting `scalar`.
    fn switch(&self, scalar: Scalar) -> Option<bool> {
        match self.scalar(scalar).value {
            Some(ScalarValue::Bool(on)) => Some(on),
            _ => None,
        }
    }

    /// The value the rules' own `env` entries leave `WINE_D3D_CONFIG` at:
    /// the last entry for it, `Some("")` when that one removes it, and `None`
    /// when no rule sets it.
    #[must_use]
    pub fn d3d_config(&self) -> Option<&str> {
        self.env
            .iter()
            .rev()
            .find(|entry| is_d3d_config(entry))
            .map(|entry| entry.value().unwrap_or_default())
    }

    /// Fold the settings that `compatdb.so` applies as environment entries
    /// into [`Resolution::env`], each with the rule that set the setting as
    /// its origin and none of them a default:
    ///
    /// - [`Resolution::wined3d`], when a rule sets it: the `WINE_D3D_CONFIG`
    ///   that names the renderer, after the rules' own entries. It is the
    ///   value the variable ends up with, and it keeps the other options a
    ///   rule's `WINE_D3D_CONFIG` set (see [`runtime::wined3d_config`]).
    ///   Unset, the process keeps whatever `WINE_D3D_CONFIG` it has, from a
    ///   rule's `env` or inherited, and without one wined3d uses OpenGL.
    /// - [`Resolution::metal_hud`], when a rule sets it: the entries of
    ///   [`metal_hud_env`], which set or remove the `MTL_HUD_` variables.
    ///   They go where the rule that set it folded, before that rule's own
    ///   `env` entries, so an `env` entry for one of the variables from that
    ///   rule or from any rule folded after it wins. Unset, the process keeps
    ///   the ones it was started with.
    ///
    /// Call it after [`Table::resolve`], which records where each setting
    /// came from; a setting without that record adds nothing. A second call
    /// changes nothing.
    pub fn fold_derived_env(&mut self) {
        if std::mem::replace(&mut self.derived_env, true) {
            return;
        }
        if let (Some(wined3d), Some(origin)) =
            (self.wined3d(), self.scalar(Scalar::Wined3d).set_by.clone())
        {
            let config = self.d3d_config().filter(|value| !value.is_empty());
            let value = runtime::wined3d_config(wined3d, config);
            // The value is the renderer and the options of a checked one.
            self.push_derived(EnvEntry::literal(Wined3d::CONFIG_VAR, &value), origin);
        }
        if let (Some(on), Some(origin)) = (
            self.metal_hud(),
            self.scalar(Scalar::MetalHud).set_by.clone(),
        ) {
            let at = self
                .metal_hud_at
                .map_or(self.env.len(), |at| at.min(self.env.len()));
            let entries = metal_hud_env(on);
            let from: Vec<EnvFrom> = entries
                .iter()
                .map(|_| EnvFrom {
                    origin: origin.clone(),
                    default: false,
                })
                .collect();
            self.env.splice(at..at, entries);
            self.env_from.splice(at..at, from);
        }
    }

    /// Add `entry`, which a setting from `origin` stands for, after the
    /// other env entries.
    fn push_derived(&mut self, entry: EnvEntry, origin: Origin) {
        self.env.push(entry);
        self.env_from.push(EnvFrom {
            origin,
            default: false,
        });
    }

    /// Where the env entry that leaves `WINE_D3D_CONFIG` at
    /// [`Resolution::d3d_config`] came from, or `None` when no rule sets it.
    #[must_use]
    pub fn d3d_config_origin(&self) -> Option<&Origin> {
        let i = self.env.iter().rposition(is_d3d_config)?;
        self.env_from.get(i).map(|from| &from.origin)
    }

    /// Fold what a rule from `origin` says about the scalar setting
    /// `scalar`: a value replaces the value or reset of a rule folded before,
    /// and a reset unsets the setting from here on.
    pub(crate) fn fold_scalar(
        &mut self,
        scalar: Scalar,
        setting: Setting<ScalarValue>,
        origin: &Origin,
    ) {
        let state = match setting {
            Setting::Unset => return,
            Setting::Set(value) => ScalarState {
                value: Some(value),
                set_by: Some(origin.clone()),
                reset_by: None,
            },
            Setting::Reset => ScalarState {
                value: None,
                set_by: None,
                reset_by: Some(origin.clone()),
            },
        };
        self.scalars.insert(scalar, state);
    }

    /// Fold the `dll_overrides` entries of `rule` into the list: an entry
    /// whose order is `default` takes its modules out of the entries before
    /// it, dropping an entry left without one, and every other entry is
    /// appended.
    fn fold_dll_overrides(&mut self, rule: &Rule, origin: &Origin) {
        for entry in &rule.dll_overrides {
            if !entry.is_default() {
                self.dll_overrides.push(entry.clone());
                self.dll_overrides_from.push(origin.clone());
                continue;
            }
            for module in entry.modules() {
                let mut removed = Vec::new();
                let mut kept = Vec::new();
                let entries = std::mem::take(&mut self.dll_overrides)
                    .into_iter()
                    .zip(std::mem::take(&mut self.dll_overrides_from));
                for (earlier, from) in entries {
                    match without_module(&earlier, module) {
                        Without::Absent => kept.push((earlier, from)),
                        Without::Emptied => removed.push(from),
                        Without::Rest(rest) => {
                            removed.push(from.clone());
                            kept.push((rest, from));
                        }
                    }
                }
                let (entries, from): (Vec<_>, Vec<_>) = kept.into_iter().unzip();
                self.dll_overrides = entries;
                self.dll_overrides_from = from;
                self.dll_overrides_reset.push(DllOverrideReset {
                    module: module.to_string(),
                    by: origin.clone(),
                    removed,
                });
            }
        }
    }

    /// Fold the arguments of `rule` into the list: its `arguments_override`,
    /// when set, replaces what the rules folded before it accumulated, unless
    /// `rule` is for a Steam app id, and its `arguments` append.
    fn fold_arguments(&mut self, rule: &Rule, origin: &Origin) {
        match &rule.arguments_override {
            Some(_) if rule.exe.is_steam_app() => {
                self.arguments_override_ignored.push(origin.clone());
            }
            Some(over) => {
                let entries: Vec<(Argument, Origin)> = std::mem::take(&mut self.arguments)
                    .into_iter()
                    .zip(std::mem::take(&mut self.arguments_from))
                    .collect();
                if !entries.is_empty() {
                    self.arguments_dropped.push(ArgumentsDropped {
                        by: origin.clone(),
                        entries,
                    });
                }
                self.arguments.extend(over.iter().cloned());
                self.arguments_from
                    .extend(over.iter().map(|_| origin.clone()));
            }
            None => {}
        }
        self.arguments.extend(rule.arguments.iter().cloned());
        self.arguments_from
            .extend(rule.arguments.iter().map(|_| origin.clone()));
    }

    /// The env entries in order, each with the rule that set it and whether
    /// it applies in a process where `has` says which variables the process
    /// already has. A default leaves such a variable alone, so a value the
    /// caller exported wins over a built-in `*` rule; every other entry
    /// applies, replacing the value or, when empty, removing the variable.
    /// An entry that a later applied entry for the same variable replaces
    /// says where that entry came from.
    ///
    /// [`Resolution::env_from`] has one entry per env entry, as
    /// [`Table::resolve`] and [`Resolution::fold_derived_env`] keep it.
    #[must_use]
    pub fn env_steps(&self, has: impl Fn(&str) -> bool) -> Vec<EnvStep> {
        let mut steps: Vec<EnvStep> = self
            .env
            .iter()
            .zip(&self.env_from)
            .map(|(entry, from)| EnvStep {
                entry: entry.clone(),
                origin: from.origin.clone(),
                applied: !(from.default && has(entry.name().as_str())),
                replaced_by: None,
                removes: None,
            })
            .collect();
        let same = |a: &EnvStep, b: &EnvStep| {
            a.applied
                && a.entry
                    .name()
                    .as_str()
                    .eq_ignore_ascii_case(b.entry.name().as_str())
        };
        let links: Vec<(Option<Origin>, Option<Origin>)> = steps
            .iter()
            .enumerate()
            .map(|(i, step)| {
                let replaced_by = steps
                    .iter()
                    .skip(i.saturating_add(1))
                    .rev()
                    .find(|later| same(later, step))
                    .map(|later| later.origin.clone());
                let removes = steps
                    .iter()
                    .take(i)
                    .rev()
                    .find(|earlier| same(earlier, step))
                    .filter(|earlier| {
                        step.entry.value().is_none() && earlier.entry.value().is_some()
                    })
                    .map(|earlier| earlier.origin.clone());
                (replaced_by, removes)
            })
            .collect();
        for (step, (replaced_by, removes)) in steps.iter_mut().zip(links) {
            step.replaced_by = replaced_by;
            step.removes = removes;
        }
        steps
    }
}

impl Table {
    /// Fold every matching rule into one [`Resolution`]. `exe` is the image
    /// basename (see [`basename_of`]); `version` is the image's version
    /// resource, for the optional fingerprint constraints; `steam_app` is the
    /// process's Steam app id, if it has one (see [`steam_app`]), for the
    /// rules that name one.
    ///
    /// The rules fold in this order, each group in table order, which is
    /// line order for the rules of [`ENV_VAR`] and then line order for those
    /// of the file [`FILE_ENV_VAR`] names:
    ///
    /// 1. built-in `*` rules, those without a fingerprint first;
    /// 2. the caller's `*` rules, those without a fingerprint first;
    /// 3. built-in rules naming an executable, those without a fingerprint
    ///    first;
    /// 4. the caller's rules naming an executable, those without a
    ///    fingerprint first.
    ///
    /// So a scalar field ends up holding the value of the last rule that
    /// sets it. Whether a rule names an executable decides first, then where
    /// it comes from, then whether it pins a fingerprint (see
    /// [`Rule::specificity`]). The variable and the file are one place in
    /// that order, so only table order tells them apart: a rule of the file
    /// beats a rule of the variable of equal specificity, because it is
    /// declared later, and a `*` rule of the file does not beat a rule of
    /// the variable that names an executable. A launch-wide `*` rule of the
    /// caller replaces a built-in default but not what a built-in rule for
    /// one program sets, so that fix survives launch-wide settings, and a
    /// rule of the caller for the program beats it, with or without a
    /// fingerprint. The lists accumulate across every rule that matched, in
    /// the same order. The exceptions are a
    /// rule's `arguments_override`, which replaces the arguments the rules
    /// folded before it accumulated (the `arguments` of the rule itself and
    /// of the rules folded after it append to it), a scalar set to
    /// `default` (see [`Setting::Reset`]), and a `dll_overrides` entry with
    /// the order `default`, which takes its modules out of the entries
    /// before it.
    ///
    /// The sort has to stay stable: rules of equal place in that order fold
    /// in table order.
    ///
    /// A rule whose executable matches but whose fingerprint does not simply
    /// does not apply; nothing falls back to it, and the rules that do match
    /// still apply.
    ///
    /// A rule for a Steam app id counts as naming an executable: it folds in
    /// the same groups, in table order with the rules for a basename. Its
    /// `arguments_override` is ignored (see [`Rule::arguments_override`]).
    #[must_use]
    pub fn resolve(&self, exe: &str, version: &VersionInfo, steam_app: Option<u32>) -> Resolution {
        let mut matching: Vec<(Source, &Rule)> = self
            .entries()
            .filter(|(_, rule)| rule.matches(exe, version, steam_app))
            .collect();
        matching.sort_by_key(|(source, rule)| {
            let specificity = rule.specificity();
            let names_exe = matches!(
                specificity,
                Specificity::Exe | Specificity::ExeWithFingerprint
            );
            // The two sources of the caller's rules compare equal here.
            (names_exe, *source != Source::BuiltIn, specificity)
        });
        let mut r = Resolution::default();
        for (source, rule) in matching {
            let origin = Origin::of(source, rule);
            for &scalar in Scalar::ALL {
                r.fold_scalar(scalar, rule.scalar(scalar), &origin);
            }
            r.fold_dll_overrides(rule, &origin);
            r.fold_arguments(rule, &origin);
            r.debug_channels.extend(rule.debug_channels.iter().cloned());
            r.debug_channels_from
                .extend(rule.debug_channels.iter().map(|_| origin.clone()));
            match rule.metal_hud {
                Setting::Set(_) => r.metal_hud_at = Some(r.env.len()),
                Setting::Reset => r.metal_hud_at = None,
                Setting::Unset => {}
            }
            let default = origin.env_is_default();
            r.env.extend(rule.env.iter().cloned());
            r.env_from.extend(rule.env.iter().map(|_| EnvFrom {
                origin: origin.clone(),
                default,
            }));
        }
        r
    }
}

/// Whether `entry` is for the variable wined3d reads its settings from,
/// matched ASCII-case-insensitively as in a process environment.
fn is_d3d_config(entry: &EnvEntry) -> bool {
    entry
        .name()
        .as_str()
        .eq_ignore_ascii_case(Wined3d::CONFIG_VAR)
}

/// Whether two module names of `dll_overrides` entries name the same module:
/// compared case-insensitively, with or without `.dll`.
fn same_module(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    a.strip_suffix(".dll").unwrap_or(&a) == b.strip_suffix(".dll").unwrap_or(&b)
}

/// A `dll_overrides` entry with one module taken out (see
/// [`without_module`]).
#[derive(Debug, PartialEq, Eq)]
enum Without {
    /// The entry does not name the module.
    Absent,
    /// The module was the entry's only one, so nothing is left.
    Emptied,
    /// The entry with its other modules and the same order.
    Rest(DllOverride),
}

/// The `dll_overrides` entry `entry` without `module`.
fn without_module(entry: &DllOverride, module: &str) -> Without {
    let all: Vec<&str> = entry.modules().collect();
    let rest: Vec<&str> = all
        .iter()
        .copied()
        .filter(|name| !same_module(name, module))
        .collect();
    if rest.len() == all.len() {
        Without::Absent
    } else if rest.is_empty() {
        Without::Emptied
    } else {
        // Made of the parts of a checked entry: its `=` is there.
        Without::Rest(DllOverride::literal(&format!(
            "{}={}",
            rest.join(","),
            entry.order()
        )))
    }
}

#[cfg(test)]
mod tests;
