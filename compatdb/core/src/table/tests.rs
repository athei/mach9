use super::*;
use crate::{ANY_EXE, Setting};

#[test]
fn duplicate_matchers_finds_rules_that_cannot_be_told_apart() {
    let table = Table {
        rules: vec![
            Rule {
                exe: "game.exe".into(),
                company: Some("Acme".into()),
                ..Rule::default()
            },
            Rule {
                exe: "GAME.EXE".into(),
                company: Some("Acme".into()),
                ..Rule::default()
            },
            Rule {
                exe: "game.exe".into(),
                ..Rule::default()
            },
        ],
    };
    assert_eq!(table.duplicate_matchers(), vec![(0, 1)]);
}

#[test]
fn wildcard_rules_are_not_duplicates_of_each_other() {
    // Several `*` rules are defaults that stack. A wildcard that pins a
    // fingerprint is an ordinary matcher and is still reported.
    let table = Table {
        rules: vec![
            Rule {
                exe: ANY_EXE.into(),
                dll_overrides: vec!["mscoree,mshtml=".into()],
                ..Rule::default()
            },
            Rule {
                exe: ANY_EXE.into(),
                dpi_aware: Setting::Set(true),
                ..Rule::default()
            },
            Rule {
                exe: ANY_EXE.into(),
                ..Rule::default()
            },
            Rule {
                exe: ANY_EXE.into(),
                company: Some("Acme".into()),
                ..Rule::default()
            },
            Rule {
                exe: ANY_EXE.into(),
                company: Some("Acme".into()),
                ..Rule::default()
            },
        ],
    };
    assert_eq!(table.duplicate_matchers(), vec![(3, 4)]);
}
