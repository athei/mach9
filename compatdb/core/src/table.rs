//! The table of rules.

#[cfg(doc)]
use crate::ENV_VAR;
use crate::Rule;

/// The whole database: the built-in rules followed by the rules of
/// [`ENV_VAR`], in the order they were declared.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Table {
    /// The rules, in the order they were declared, which is the order
    /// [`Table::resolve`] folds rules of equal layer and specificity in.
    pub rules: Vec<Rule>,
}

impl Table {
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
            if rule.specificity() == 0 {
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
