use super::*;
use crate::{DllOverride, Exe, Fingerprint, Setting};

#[test]
fn duplicate_matchers_finds_rules_that_cannot_be_told_apart() {
    let table = Table::of_builtin(vec![
        Rule {
            exe: Exe::literal("game.exe"),
            company: Some(Fingerprint::literal("Acme")),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("GAME.EXE"),
            company: Some(Fingerprint::literal("Acme")),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("game.exe"),
            ..Rule::default()
        },
    ]);
    assert_eq!(table.duplicate_matchers(), vec![(0, 1)]);
}

#[test]
fn wildcard_rules_are_not_duplicates_of_each_other() {
    // Several `*` rules are defaults that stack. A wildcard that pins a
    // fingerprint is an ordinary matcher and is still reported.
    let table = Table::of_builtin(vec![
        Rule {
            exe: Exe::Any,
            dll_overrides: vec![DllOverride::literal("mscoree,mshtml=")],
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            dpi_aware: Setting::Set(true),
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            company: Some(Fingerprint::literal("Acme")),
            ..Rule::default()
        },
        Rule {
            exe: Exe::Any,
            company: Some(Fingerprint::literal("Acme")),
            ..Rule::default()
        },
    ]);
    assert_eq!(table.duplicate_matchers(), vec![(3, 4)]);
}
