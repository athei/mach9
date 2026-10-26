//! Folding the rules that match a process into the settings it gets, with
//! where each value came from.

use std::collections::BTreeMap;

#[cfg(doc)]
use crate::basename_of;
use crate::{
    Argument, D3d9, DllOverride, Dxgi, ENV_VAR, EnvEntry, OpenGl, Rule, Scalar, ScalarValue,
    Setting, Source, Specificity, Table, VersionInfo, Wined3d,
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
    /// The `env` entries of every matching rule in fold order.
    /// [`Resolution::env_steps`] says which of them apply in a process.
    pub env: Vec<EnvEntry>,
    /// Where each entry of [`Resolution::env`] came from, at the same index.
    pub env_from: Vec<EnvFrom>,
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
}

impl Origin {
    /// Where the values of `rule` come from.
    #[must_use]
    pub fn of(rule: &Rule) -> Self {
        match rule.source {
            Source::Line(n) => Self::Line(n),
            Source::BuiltIn if rule.specificity() == Specificity::Any => Self::BuiltIn,
            Source::BuiltIn => Self::BuiltInFor(rule.matchers()),
        }
    }

    /// How the log names it: `built-in`, `built-in for <matchers>` or
    /// `WINE_COMPATDB line <n>`.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::BuiltIn => String::from("built-in"),
            Self::BuiltInFor(matchers) => format!("built-in for {matchers}"),
            Self::Line(n) => format!("{ENV_VAR} line {n}"),
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
    /// Whether it is a default (see [`Rule::env_is_default`]), which leaves
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
    /// Whether there is nothing to apply.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scalars.values().all(|state| state.value.is_none())
            && self.dll_overrides.is_empty()
            && self.arguments.is_empty()
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

    /// When a rule sets [`Resolution::wined3d`], that renderer, with the
    /// `WINE_D3D_CONFIG` that names it folded into [`Resolution::env`]. That
    /// entry goes after the rules' own entries, so it is the value the
    /// variable ends up with when the env is applied, and it keeps the other
    /// options a rule's `WINE_D3D_CONFIG` set (see [`Wined3d::config`]).
    /// `None`, with nothing changed, when no rule sets it: then the process
    /// keeps whatever `WINE_D3D_CONFIG` it has, from a rule's `env` or
    /// inherited, and without one wined3d uses OpenGL. Call it once, after
    /// [`Table::resolve`], which records where the setting came from; a
    /// resolution without that record is left as it is.
    pub fn apply_wined3d(&mut self) -> Option<Wined3d> {
        let wined3d = self.wined3d()?;
        let origin = self.scalar(Scalar::Wined3d).set_by.clone()?;
        let value = wined3d.config(self.d3d_config().filter(|value| !value.is_empty()));
        // The value is the renderer and the options of a checked one.
        self.env
            .push(EnvEntry::literal(Wined3d::CONFIG_VAR, &value));
        self.env_from.push(EnvFrom {
            origin,
            default: false,
        });
        Some(wined3d)
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
    fn fold_dll_overrides(&mut self, rule: &Rule) {
        let origin = Origin::of(rule);
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
    /// when set, replaces what the rules folded before it accumulated, and
    /// its `arguments` append.
    fn fold_arguments(&mut self, rule: &Rule) {
        let origin = Origin::of(rule);
        if let Some(over) = &rule.arguments_override {
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
    /// [`Table::resolve`] and [`Resolution::apply_wined3d`] keep it.
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
    /// resource, for the optional fingerprint constraints.
    ///
    /// The built-in rules fold first and the rules of [`ENV_VAR`] after
    /// them, so any value a rule from the variable sets beats every
    /// built-in value. Within each of the two layers the rules fold least
    /// specific first (see [`Rule::specificity`]) and within a tier in table
    /// order, which is line order for the variable's rules. So a scalar
    /// field ends up holding the value of the last rule that sets it: in a
    /// layer, a rule naming a process beats a launch-wide default. The lists
    /// accumulate across every rule that matched. The exceptions are a
    /// rule's `arguments_override`, which replaces the arguments the rules
    /// folded before it accumulated (the `arguments` of the rule itself and
    /// of the rules folded after it append to it), a scalar set to
    /// `default` (see [`Setting::Reset`]), and a `dll_overrides` entry with
    /// the order `default`, which takes its modules out of the entries
    /// before it.
    ///
    /// The sort has to stay stable: rules of equal layer and specificity
    /// fold in table order, which is what lets a launcher's `*` rule,
    /// appended after the built-in `*` rules, add its list entries after
    /// theirs.
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
        matching.sort_by_key(|rule| (rule.source != Source::BuiltIn, rule.specificity()));
        let mut r = Resolution::default();
        for rule in matching {
            let origin = Origin::of(rule);
            for &scalar in Scalar::ALL {
                r.fold_scalar(scalar, rule.scalar(scalar), &origin);
            }
            r.fold_dll_overrides(rule);
            r.fold_arguments(rule);
            let default = rule.env_is_default();
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
