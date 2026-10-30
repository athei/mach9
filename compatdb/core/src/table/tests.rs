use super::*;
use crate::{DebugChannel, DllOverride, Exe, Fingerprint, Setting};

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn preview_is_what_compatdb_so_resolves_with_the_table_in_the_variable() -> TestResult {
    let mut table = Table::new();
    table.push(Rule::new(Exe::Any).with_metal_hud(true));
    table.push(
        Rule::new(Exe::new("Game.exe")?)
            .with_dxgi(crate::Dxgi::Wined3d)
            .with_wined3d(crate::Wined3d::Vulkan),
    );
    let none = VersionInfo::default();
    let game = table.preview("game.exe", &none);
    // The built-in rules first, then the table's, as the library folds
    // them; the derived env is folded in.
    let expected = {
        let (sent, diagnostics) = Table::parse_lenient(&table.to_string());
        assert_eq!(diagnostics, []);
        let mut resolution = sent.with_builtin().resolve("game.exe", &none, None);
        resolution.fold_derived_env();
        resolution
    };
    assert_eq!(game, expected);
    assert_eq!(game.dxgi(), Some(crate::Dxgi::Wined3d));
    assert_eq!(game.opengl(), Some(crate::OpenGl::Egl));
    assert_eq!(game.metal_hud(), Some(true));
    let names: Vec<&str> = game.env.iter().map(|e| e.name().as_str()).collect();
    assert_eq!(
        names,
        [
            "FEX_X87REDUCEDPRECISION",
            "MTL_HUD_ENABLED",
            "MTL_HUD_ALIGNMENT",
            "MTL_HUD_ELEMENTS",
            "WINE_D3D_CONFIG"
        ]
    );
    // A table that holds the built-in rules already does not get them
    // twice.
    assert_eq!(
        table.clone().with_builtin().preview("game.exe", &none),
        game
    );
    // Without rules of its own, a process gets the built-in rules alone.
    let mut plain = Table::builtin().resolve("other.exe", &none, None);
    plain.fold_derived_env();
    assert_eq!(Table::new().preview("other.exe", &none), plain);
    Ok(())
}

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
fn preview_knows_no_steam_app_id() -> TestResult {
    let mut table = Table::new();
    table.push(Rule::new(Exe::new("steam:440")?).with_dpi_aware(false));
    let none = VersionInfo::default();
    assert_eq!(
        table.preview("game.exe", &none),
        Table::new().preview("game.exe", &none)
    );
    Ok(())
}

#[test]
fn winedebug_leaves_out_what_no_exe_scope_can_hold() -> TestResult {
    // A rule read from text or built with a `with_` method can hold them;
    // an unscoped entry would reach every process.
    let mut table = Table::new();
    table.push(Rule::new(Exe::new("steam:440")?).with_debug_channel(DebugChannel::new("+relay")?));
    table.push(Rule::new(Exe::new("Game.exe")?).with_debug_channel(DebugChannel::new("+seh")?));
    assert_eq!(table.winedebug(), ["Game.exe:+seh"]);
    Ok(())
}

#[test]
fn duplicate_matchers_finds_steam_rules_for_the_same_app() -> TestResult {
    let steam = |text: &str| Exe::new(text).map(Rule::new);
    let table = Table::parse_strict(
        "exe=steam:440;dpi_aware=true\n\
         exe=STEAM:0440;gcmouse=false\n\
         exe=steam:220;dpi_aware=true\n\
         exe=steam:440;product=Foo\n\
         exe=steam:440;product=Foo\n\
         exe=Game.exe;dpi_aware=true",
    )?;
    assert_eq!(table.duplicate_matchers(), vec![(0, 1), (3, 4)]);
    assert!(steam("steam:440")?.matches_the_same_as(&steam("STEAM:440")?));
    assert!(!steam("steam:440")?.matches_the_same_as(&steam("steam:220")?));
    assert!(!steam("steam:440")?.matches_the_same_as(&steam("440")?));
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
