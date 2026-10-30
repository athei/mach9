//! The table of rules, each with where it comes from.

#[cfg(doc)]
use crate::Origin;
use crate::{ENV_VAR, Exe, FILE_ENV_VAR, Resolution, Rule, Specificity, VersionInfo, builtin};

/// Where a rule of a [`Table`] comes from. It decides how the log names the
/// rule's values (see [`Origin`]) and whether its `env` entries are defaults
/// (see [`Origin::env_is_default`]). Never serialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Compiled into `compatdb.so`: a rule of [`Table::builtin`].
    BuiltIn,
    /// Line `n` of [`ENV_VAR`], counted from 1 as [`Table::parse_lenient`]
    /// reads the value, blank lines and a legacy header included.
    Line(usize),
    /// Line `n` of the file [`FILE_ENV_VAR`] names, counted from 1 as
    /// [`Table::parse_file_lenient`] reads the file. The file counts its own
    /// lines, so a rule here can share its number with a [`Source::Line`].
    FileLine(usize),
}

/// Which of the two texts of the caller's rules a line is in. Each counts its
/// own lines, so a line number alone does not say where a problem is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulesText {
    /// The value of [`ENV_VAR`].
    Variable,
    /// The file [`FILE_ENV_VAR`] names.
    File,
}

impl RulesText {
    /// How the log names the text: by its variable.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Variable => ENV_VAR,
            Self::File => FILE_ENV_VAR,
        }
    }

    /// The source of a rule on line `line` of the text.
    pub(crate) const fn source(self, line: usize) -> Source {
        match self {
            Self::Variable => Source::Line(line),
            Self::File => Source::FileLine(line),
        }
    }
}

/// The whole database: the built-in rules followed by the rules of
/// [`ENV_VAR`] and then those of the file [`FILE_ENV_VAR`] names, in the
/// order they were declared, each with its [`Source`].
///
/// A launcher builds the rules it sends with [`Table::new`] and
/// [`Table::push`] and writes them with `to_string()`, which is the value of
/// [`ENV_VAR`]; [`Table::with_builtin`] puts the built-in rules in front, as
/// `compatdb.so` does, to see what a process would get.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Table {
    /// The rules, in the order they were declared, which is the order
    /// [`Table::resolve`] folds rules of equal place in its order in.
    pub(crate) rules: Vec<Rule>,
    /// Where each rule comes from, at the same index.
    pub(crate) sources: Vec<Source>,
}

impl Table {
    /// The table without rules.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rules: Vec::new(),
            sources: Vec::new(),
        }
    }

    /// The table of the built-in rules ([`builtin::rules`]), which
    /// `compatdb.so` puts in front of the rules of [`ENV_VAR`].
    #[must_use]
    pub fn builtin() -> Self {
        Self::of_builtin(builtin::rules())
    }

    /// The table of `rules`, each a built-in rule.
    pub(crate) fn of_builtin(rules: Vec<Rule>) -> Self {
        Self {
            sources: vec![Source::BuiltIn; rules.len()],
            rules,
        }
    }

    /// This table with the built-in rules in front of its own, as
    /// `compatdb.so` builds its table.
    #[must_use]
    pub fn with_builtin(self) -> Self {
        let mut table = Self::builtin();
        table.append(self);
        table
    }

    /// Add the rules of `other` after these, with their sources, as
    /// `compatdb.so` adds the rules of the file [`FILE_ENV_VAR`] names after
    /// those of [`ENV_VAR`].
    ///
    /// A table that holds a file's rules is for [`Table::resolve`] and
    /// [`Table::preview`]. `Display` writes those rules too, so its text is
    /// no value for [`ENV_VAR`] beside the file: they would fold twice.
    pub fn append(&mut self, other: Self) {
        self.rules.extend(other.rules);
        self.sources.extend(other.sources);
    }

    /// Add `rule` after the others, with the source `source`.
    pub(crate) fn push_from(&mut self, rule: Rule, source: Source) {
        self.rules.push(rule);
        self.sources.push(source);
    }

    /// Add `rule` after the others, as a rule of [`ENV_VAR`] on the line
    /// after the last line of that variable in the table, which for a table
    /// built with this method alone is its line in the value the table
    /// writes.
    pub fn push(&mut self, rule: Rule) {
        let last = self
            .sources
            .iter()
            .filter_map(|source| match source {
                Source::Line(n) => Some(*n),
                Source::BuiltIn | Source::FileLine(_) => None,
            })
            .max()
            .unwrap_or(0);
        self.push_from(rule, Source::Line(last.saturating_add(1)));
    }

    /// The rules, in the order they were declared.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Where each rule of [`Table::rules`] comes from, at the same index.
    #[must_use]
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// Each rule with its source, in the order they were declared.
    pub(crate) fn entries(&self) -> impl Iterator<Item = (Source, &Rule)> {
        self.sources.iter().copied().zip(&self.rules)
    }

    /// What a process with the image basename `exe` and the version
    /// resource `version` would get from `compatdb.so` with this table in
    /// [`ENV_VAR`]: the built-in rules followed by this table's own (a
    /// built-in rule in this table is left out, since the library has it
    /// already), resolved, with [`Resolution::fold_derived_env`] done.
    ///
    /// Which env entries apply also depends on the environment the process
    /// starts with, which [`Resolution::env_steps`] takes.
    ///
    /// The process has no Steam app id here, so a rule for one
    /// ([`Exe::Steam`]) does not apply.
    #[must_use]
    pub fn preview(&self, exe: &str, version: &VersionInfo) -> Resolution {
        let mut table = Self::builtin();
        for (source, rule) in self.entries() {
            if source != Source::BuiltIn {
                table.rules.push(rule.clone());
                table.sources.push(source);
            }
        }
        let mut resolution = table.resolve(exe, version, None);
        resolution.fold_derived_env();
        resolution
    }

    /// The `WINEDEBUG` entries the `debug_channels` of this table's rules
    /// stand for, in table order. A launcher joins them with `,` after its
    /// own entries and starts Wine with the result, since Wine reads the
    /// variable before `compatdb.so` loads.
    ///
    /// An entry of a rule for an executable is scoped to it as
    /// `<exe>:<entry>`, which is Wine's own syntax: Wine compares the prefix
    /// with the image file name ASCII-case-insensitively and knows nothing
    /// of fingerprints, so the entry reaches every program of that name. An
    /// entry of a rule with `exe` `*` is unscoped. Wine lets a later entry
    /// win for the same channel.
    ///
    /// An entry of a rule for a Steam app id is left out: Wine has no scope
    /// for an app id, and without one the entry would reach every process.
    /// [`Rule::set`] refuses such an entry.
    #[must_use]
    pub fn winedebug(&self) -> Vec<String> {
        self.rules
            .iter()
            .flat_map(|rule| {
                rule.debug_channels
                    .iter()
                    .filter_map(move |channel| match &rule.exe {
                        Exe::Any => Some(channel.to_string()),
                        Exe::Name(name) => Some(format!("{name}:{channel}")),
                        Exe::Steam(_) => None,
                    })
            })
            .collect()
    }

    /// Every pair of rules that matches exactly the same processes, by their
    /// index in the table. A duplicate can only be a mistake, so the
    /// launcher warns about it.
    ///
    /// Plain wildcard rules (`exe = *` with no fingerprint) are left out: they
    /// are defaults that are meant to stack, one per setting, so several of
    /// them matching every process is not a mistake.
    #[must_use]
    pub fn duplicate_matchers(&self) -> Vec<(usize, usize)> {
        let mut pairs = Vec::new();
        for (i, rule) in self.rules.iter().enumerate() {
            if rule.specificity() == Specificity::Any {
                continue;
            }
            for (j, other) in self.rules.iter().enumerate().skip(i.saturating_add(1)) {
                if rule.matches_the_same_as(other) {
                    pairs.push((i, j));
                }
            }
        }
        pairs
    }
}

#[cfg(test)]
mod tests;
