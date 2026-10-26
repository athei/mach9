use super::*;
use crate::{DebugChannel, DllOverride, Exe, Fingerprint, Setting};

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn winedebug_scopes_the_channels_of_a_rule_for_an_exe_and_not_those_of_a_star_rule() -> TestResult {
    let channel = |text: &str| DebugChannel::new(text);
    let mut table = Table::new();
    table.push(
        Rule::new(Exe::new("Launcher.exe")?)
            .with_company(Fingerprint::new("Acme")?)
            .with_debug_channel(channel("+relay")?)
            .with_debug_channel(channel("+seh")?),
    );
    table.push(Rule::new(Exe::new("Other.exe")?));
    table.push(Rule::new(Exe::Any).with_debug_channel(channel("+loaddll")?));
    // A `*` rule with a fingerprint is unscoped too: Wine's prefix is an
    // image name.
    table.push(
        Rule::new(Exe::Any)
            .with_company(Fingerprint::new("Acme")?)
            .with_debug_channel(channel("warn+heap")?),
    );
    assert_eq!(
        table.winedebug(),
        [
            "Launcher.exe:+relay",
            "Launcher.exe:+seh",
            "+loaddll",
            "warn+heap"
        ]
    );
    assert_eq!(Table::new().winedebug(), [] as [&str; 0]);
    assert_eq!(Table::builtin().winedebug(), [] as [&str; 0]);
    Ok(())
}

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
