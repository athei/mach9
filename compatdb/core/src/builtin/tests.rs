use super::*;
use crate::{
    ArgumentsDropped, DllOverrideReset, EnvName, Origin, Resolution, Scalar, ScalarValue, Source,
    VersionInfo, Wined3d,
};

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Record in `resolution` that the rule `origin` names set `scalar` to
/// `value`, as resolving a more specific rule does.
fn set_by(resolution: &mut Resolution, scalar: Scalar, value: ScalarValue, origin: &Origin) {
    resolution.fold_scalar(scalar, Setting::Set(value), origin);
}

/// The env entries a process gets applied when it already has the
/// variables in `has`, as `NAME=value`, and the names of those left
/// alone.
fn applied(resolution: &Resolution, has: &[&str]) -> (Vec<String>, Vec<String>) {
    let steps = resolution.env_steps(|name| has.contains(&name));
    let (set, kept): (Vec<_>, Vec<_>) = steps.into_iter().partition(|s| s.applied);
    (
        set.into_iter().map(|s| s.entry.to_string()).collect(),
        kept.into_iter()
            .map(|s| s.entry.name().to_string())
            .collect(),
    )
}

/// The env entry that sets `name` to `value`, or removes it when `value`
/// is empty.
fn env(name: &str, value: &str) -> EnvEntry {
    EnvEntry::literal(name, value)
}

/// The env of the FEX rule. The variable is spelled out here to guard
/// against a typo: FEX matches the name exactly and parses the value as
/// an integer.
fn fex_env() -> Vec<EnvEntry> {
    vec![env("FEX_X87REDUCEDPRECISION", "1")]
}

/// The env every process gets from the built-in rules.
fn wildcard_env() -> Vec<EnvEntry> {
    fex_env()
}

/// The DLL overrides every process gets from the built-in rules: Mono
/// and Gecko are disabled.
fn wildcard_overrides() -> Vec<DllOverride> {
    vec![DllOverride::literal("mscoree,mshtml=")]
}

/// `wildcard_env()` followed by `more`.
fn wildcard_env_and(more: &[EnvEntry]) -> Vec<EnvEntry> {
    let mut env = wildcard_env();
    env.extend_from_slice(more);
    env
}

/// The built-in table with the lines of `text` appended, as compatdb.so
/// builds it from `WINE_COMPATDB`; `text` must parse cleanly.
fn with_lines(text: &str) -> Table {
    let mut table = Table::builtin();
    let (rules, diagnostics) = Table::parse(text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    table.rules.extend(rules.rules);
    table
}

/// What a process without a rule of its own gets: the `*` rules alone.
fn plain() -> Resolution {
    Table::builtin().resolve("game.exe", &VersionInfo::default())
}

/// The built-in rule for the executable `exe`.
fn rule<'a>(table: &'a Table, exe: &str) -> Result<&'a Rule, String> {
    table
        .rules
        .iter()
        .find(|r| r.exe == exe)
        .ok_or_else(|| format!("no rule for {exe}"))
}

/// The version resource that the built-in `rule` pins, filled in with
/// its own fingerprint values.
fn version_of(rule: &Rule) -> VersionInfo {
    let text = |pin: Option<&Fingerprint>| pin.map(|p| p.as_str().to_owned()).unwrap_or_default();
    VersionInfo {
        company: text(rule.company()),
        product: text(rule.product()),
        original_filename: text(rule.original_filename()),
    }
}

#[test]
fn a_variable_the_caller_set_wins_over_a_builtin_default() {
    let game = plain();
    // Unset, the defaults apply.
    let (set, kept) = applied(&game, &[]);
    assert_eq!(set, ["FEX_X87REDUCEDPRECISION=1"]);
    assert_eq!(kept, [] as [&str; 0]);
    // Exported by the caller, the default leaves it alone.
    let (set, kept) = applied(&game, &["FEX_X87REDUCEDPRECISION"]);
    assert_eq!(set, [] as [&str; 0]);
    assert_eq!(kept, ["FEX_X87REDUCEDPRECISION"]);
    // The steps say where each entry came from.
    let steps = game.env_steps(|_| false);
    assert_eq!(steps.first().map(|s| &s.origin), Some(&Origin::BuiltIn));
}

#[test]
fn a_rule_for_one_game_or_from_the_launcher_replaces_what_the_caller_set() {
    // A rule for one game and a launcher's own `*` rule both replace the
    // caller's value, since whoever wrote them chose it for that
    // process.
    for text in [
        "exe=game.exe;env=FEX_X87REDUCEDPRECISION=0",
        "exe=*;env=FEX_X87REDUCEDPRECISION=0",
    ] {
        let table = with_lines(text);
        let game = table.resolve("game.exe", &VersionInfo::default());
        let (set, kept) = applied(&game, &["FEX_X87REDUCEDPRECISION"]);
        assert_eq!(kept, ["FEX_X87REDUCEDPRECISION"], "{text}");
        assert_eq!(set, ["FEX_X87REDUCEDPRECISION=0"], "{text}");
    }
    // Only the built-in `*` rules' own entries are defaults.
    let table = with_lines("exe=*;env=A=1");
    for r in &table.rules {
        let builtin_star = r.source == Source::BuiltIn && r.exe.is_any();
        assert_eq!(r.env_is_default(), builtin_star, "{}", r.matchers());
    }
}

#[test]
fn the_table_asks_fex_for_reduced_x87_precision() {
    let table = Table::builtin();
    let fex: Vec<&Rule> = table.rules.iter().filter(|r| !r.env.is_empty()).collect();
    // One `*` rule sets it; the other env rules are the CEF helpers'.
    let star: Vec<&&Rule> = fex.iter().filter(|r| r.exe.is_any()).collect();
    assert_eq!(star.len(), 1);
    assert_eq!(star.first().map(|r| r.env.clone()), Some(fex_env()));
    // Every process gets it: x86_64, i386 and native ARM64 alike, since
    // rules do not match on the arch.
    assert_eq!(plain().env, wildcard_env());
}

#[test]
fn a_rule_turns_reduced_x87_precision_off_for_one_game_or_every_process() -> TestResult {
    // For one game: an empty value, which removes the variable, folds
    // after the default.
    let table = with_lines("exe=old.exe;env=FEX_X87REDUCEDPRECISION=");
    let old = table.resolve("Old.exe", &VersionInfo::default());
    let removed = env("FEX_X87REDUCEDPRECISION", "");
    assert_eq!(old.env, wildcard_env_and(std::slice::from_ref(&removed)));
    let block: Vec<u16> = "FEX_X87REDUCEDPRECISION=1\0OTHER=x\0\0"
        .encode_utf16()
        .collect();
    let merged = crate::merge_env(&block, &old.env).ok_or("unchanged")?;
    assert_eq!(String::from_utf16_lossy(&merged), "OTHER=x\0\0");
    // Another process keeps it.
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.env, wildcard_env());
    // For every process: a `*` rule removes it, even where the caller
    // exported it, since it is not a default.
    let table = with_lines("exe=*;env=FEX_X87REDUCEDPRECISION=");
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.env, wildcard_env_and(&[removed]));
    let (set, _) = applied(&game, &["FEX_X87REDUCEDPRECISION"]);
    assert_eq!(set, ["FEX_X87REDUCEDPRECISION="]);
    Ok(())
}

#[test]
fn the_table_asks_the_mac_driver_for_mesas_egl_unless_a_rule_opts_out() {
    // It is a setting, not an env entry, so nothing a process starts
    // inherits it.
    let game = plain();
    assert_eq!(game.opengl(), Some(OpenGl::Egl));
    assert_eq!(
        game.scalar(Scalar::OpenGl).set_by.as_ref(),
        Some(&Origin::BuiltIn)
    );
    assert_eq!(game.env, wildcard_env());
    // A rule for one game gives it Apple's OpenGL and changes nothing else
    // for it; another process keeps the default.
    let table = with_lines("exe=game.exe;opengl=cgl");
    let game = table.resolve("Game.exe", &VersionInfo::default());
    let mut expected = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(expected.opengl(), Some(OpenGl::Egl));
    set_by(
        &mut expected,
        Scalar::OpenGl,
        ScalarValue::OpenGl(OpenGl::Cgl),
        &Origin::Line(1),
    );
    assert_eq!(game, expected);
    // A `*` rule gives every process Apple's OpenGL.
    let table = with_lines("exe=*;opengl=cgl");
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.opengl(), Some(OpenGl::Cgl));
    assert_eq!(game.env, wildcard_env());
}

#[test]
fn the_table_keeps_no_execute_on_unless_a_rule_opts_out() {
    // Any process gets it, whatever its name or version resource. It is
    // a setting, not an env entry, so nothing a process starts inherits
    // it.
    let game = Table::builtin().resolve("Some Game.exe", &VersionInfo::default());
    assert_eq!(game.force_nx(), Some(true));
    assert_eq!(
        game.scalar(Scalar::ForceNx).set_by.as_ref(),
        Some(&Origin::BuiltIn)
    );
    assert_eq!(game.env, wildcard_env());
    // A rule for one game opts that game out and nothing else changes for
    // it; another process keeps the default.
    let table = with_lines("exe=game.exe;force_nx=false");
    let game = table.resolve("Game.exe", &VersionInfo::default());
    let mut expected = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(expected.force_nx(), Some(true));
    set_by(
        &mut expected,
        Scalar::ForceNx,
        ScalarValue::Bool(false),
        &Origin::Line(1),
    );
    assert_eq!(game, expected);
    // A `*` rule gives every process Wine's behaviour.
    let table = with_lines("exe=*;force_nx=false");
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.force_nx(), Some(false));
}

#[test]
fn wined3d_on_vulkan_is_a_setting_for_one_game_and_never_a_default() {
    // wined3d on KosmicKrisp still crashes in too many places to be every
    // process's renderer, so no built-in rule sets the wined3d setting or
    // WINE_D3D_CONFIG. A game rule opts in, and only that game gets it.
    assert!(
        !Table::builtin()
            .rules
            .iter()
            .any(|r| !r.wined3d.is_unset() || r.env.iter().any(|e| *e.name() == "WINE_D3D_CONFIG"))
    );
    let mut game = plain();
    assert_eq!(game.apply_wined3d(), None);
    assert_eq!(game.env, wildcard_env());
    let table = with_lines("exe=game.exe;wined3d=vulkan");
    let mut game = table.resolve("Game.exe", &VersionInfo::default());
    assert_eq!(game.apply_wined3d(), Some(Wined3d::Vulkan));
    let vulkan = env("WINE_D3D_CONFIG", "renderer=vulkan");
    assert_eq!(game.env, wildcard_env_and(&[vulkan]));
    assert_eq!(
        game.env_from.last().map(|f| &f.origin),
        Some(&Origin::Line(1))
    );
    let mut other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.apply_wined3d(), None);
    assert_eq!(other.env, wildcard_env());
}

#[test]
fn the_table_leaves_vulkan_1_alone() {
    // The bundle ships Wine's Vulkan, so no built-in rule disables the
    // loader.
    assert!(!Table::builtin().rules.iter().any(|r| {
        r.dll_overrides
            .iter()
            .any(|o| o.as_str().contains("vulkan"))
    }));
    assert_eq!(plain().dll_overrides, wildcard_overrides());
}

#[test]
fn the_builtin_table_pins_the_launcher_and_round_trips() -> TestResult {
    let table = Table::builtin();
    let launcher = rule(&table, "Launcher.exe")?;
    assert_eq!(launcher.dxgi, Setting::Set(Dxgi::Wined3d));
    assert_eq!(
        launcher.company().map(Fingerprint::as_str),
        Some("Rockstar Games")
    );
    let gta = rule(&table, "GTAIV.exe")?;
    assert!(
        gta.arguments
            .first()
            .is_some_and(|a| a.as_str().starts_with("-availablevidmem"))
    );
    // The D3D9 quirks are mtld3d's, not ours.
    assert_eq!(gta.env, []);

    // Everything but the source travels, and the parser gives each rule
    // its line.
    let (parsed, diagnostics) = Table::parse(&table.to_env_value());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let mut sent = table;
    for (i, rule) in sent.rules.iter_mut().enumerate() {
        rule.source = Source::Line(i.saturating_add(1));
    }
    assert_eq!(parsed, sent);
    Ok(())
}

#[test]
fn the_builtin_table_pins_the_cef_helpers_to_dxmt() -> TestResult {
    let table = Table::builtin();
    for exe in ["steamwebhelper.exe", "SocialClubHelper.exe"] {
        let helper = rule(&table, exe)?;
        assert_eq!(helper.dxgi, Setting::Set(Dxgi::Dxmt), "{exe}");
        assert_eq!(helper.arguments, ["--in-process-gpu"]);
    }
    Ok(())
}

#[test]
fn only_the_cef_helpers_set_env_and_it_turns_the_metal_hud_off() -> TestResult {
    // Every rule that matches a launcher or a game must leave the
    // environment alone, or the games started from it would inherit the
    // entries. The `*` env rule is the exception: it is meant for every
    // process, so a child that inherits it gets what it would get
    // anyway.
    let hud_off = vec![env("MTL_HUD_ENABLED", "0")];
    let helpers = ["steamwebhelper.exe", "SocialClubHelper.exe"];
    let table = Table::builtin();
    for rule in &table.rules {
        if helpers.contains(&rule.exe.as_str()) {
            assert_eq!(rule.env, hud_off, "{}", rule.matchers());
        } else if rule.exe.is_any() && !rule.env.is_empty() {
            assert_eq!(rule.env, fex_env(), "{}", rule.matchers());
        } else {
            assert_eq!(rule.env, [], "{}", rule.matchers());
        }
    }
    let hud_off = wildcard_env_and(&hud_off);
    assert_eq!(rule(&table, "Launcher.exe")?.env, []);
    // The resolved environment of a helper is that entry after the
    // wildcard ones.
    let steam = VersionInfo {
        company: "Valve Corporation".into(),
        product: "Steam Client WebHelper".into(),
        original_filename: "steamwebhelper.exe".into(),
    };
    assert_eq!(table.resolve("steamwebhelper.exe", &steam).env, hud_off);
    let social = VersionInfo {
        company: "Take-Two Interactive Software, Inc.".into(),
        product: "Social Club UI".into(),
        original_filename: "SocialClubHelper.exe".into(),
    };
    assert_eq!(table.resolve("SocialClubHelper.exe", &social).env, hud_off);
    let launcher_version = VersionInfo {
        company: "Rockstar Games".into(),
        product: "Rockstar Games Launcher".into(),
        original_filename: "Launcher.exe".into(),
    };
    let resolution = table.resolve("Launcher.exe", &launcher_version);
    assert_eq!(
        resolution.scalar(Scalar::Dxgi).set_by.as_ref(),
        Some(&Origin::BuiltInFor(
            "Launcher.exe (Rockstar Games, Rockstar Games Launcher)".into()
        ))
    );
    assert_eq!(resolution.env, wildcard_env());
    Ok(())
}

#[test]
fn every_builtin_rule_has_an_exe_and_none_matches_a_bare_basename() -> TestResult {
    // A rule naming an executable must also pin its version resource, or
    // it would catch every program of that name. The `*` rules are a
    // different case: they are meant for every process, and several of
    // them are not duplicates of each other.
    let table = Table::builtin();
    for rule in &table.rules {
        // Its parts are literals, which must be what the checked
        // constructors make of their text.
        assert_eq!(Exe::new(rule.exe.as_str()).as_ref(), Ok(&rule.exe));
        for pin in [&rule.company, &rule.product, &rule.original_filename]
            .into_iter()
            .flatten()
        {
            assert_eq!(Fingerprint::new(pin.as_str()).as_ref(), Ok(pin));
        }
        for entry in &rule.dll_overrides {
            assert_eq!(DllOverride::new(entry.as_str()).as_ref(), Ok(entry));
        }
        for argument in &rule.arguments {
            assert_eq!(Argument::new(argument.as_str()).as_ref(), Ok(argument));
        }
        for entry in &rule.env {
            let name = EnvName::new(entry.name().as_str())?;
            let checked = EnvEntry::new(name, entry.value().unwrap_or_default())?;
            assert_eq!(&checked, entry);
        }
        assert!(
            !rule.is_unfingerprinted(),
            "{} matches on its executable alone",
            rule.matchers()
        );
        assert_eq!(rule.source, Source::BuiltIn, "{}", rule.matchers());
    }
    assert_eq!(table.duplicate_matchers(), []);
    Ok(())
}

#[test]
fn every_process_gets_the_wildcard_defaults() {
    let resolution = plain();
    assert_eq!(resolution.dll_overrides, wildcard_overrides());
    assert_eq!(resolution.dpi_aware(), Some(true));
    // No `*` rule forces large address awareness: the executable's flag
    // decides, as in upstream Wine.
    assert_eq!(resolution.large_address_aware(), None);
    assert_eq!(
        resolution.scalar(Scalar::LargeAddressAware).set_by.as_ref(),
        None
    );
    assert_eq!(resolution.gcmouse(), Some(true));
    assert_eq!(resolution.force_nx(), Some(true));
    assert_eq!(resolution.opengl(), Some(OpenGl::Egl));
    assert_eq!(resolution.env, wildcard_env());
    assert_eq!(resolution.dxgi(), Some(Dxgi::Dxmt));
    assert_eq!(resolution.d3d9(), Some(D3d9::Mtld3d));
    assert_eq!(resolution.arguments, [] as [&str; 0]);
    // Every value comes from a built-in `*` rule.
    let set: Vec<Scalar> = Scalar::ALL
        .iter()
        .copied()
        .filter(|&scalar| resolution.scalar(scalar).value.is_some())
        .collect();
    assert_eq!(set.len(), 6);
    for scalar in set {
        assert_eq!(
            resolution.scalar(scalar).set_by,
            Some(Origin::BuiltIn),
            "{scalar}"
        );
    }
    assert!(
        resolution
            .dll_overrides_from
            .iter()
            .chain(resolution.env_from.iter().map(|f| &f.origin))
            .all(|origin| *origin == Origin::BuiltIn)
    );
}

#[test]
fn the_pins_for_one_program_beat_the_builtin_direct3d_rules() -> TestResult {
    // Every built-in rule for one program that names a tree keeps it
    // over the built-in `*` rules.
    let table = Table::builtin();
    let pins: Vec<&Rule> = table
        .rules
        .iter()
        .filter(|r| !r.exe.is_any() && (!r.dxgi.is_unset() || !r.d3d9.is_unset()))
        .collect();
    assert_ne!(pins.len(), 0);
    for pin in &pins {
        let resolution = table.resolve(pin.exe.as_str(), &version_of(pin));
        let from = Origin::BuiltInFor(pin.matchers());
        if let Setting::Set(dxgi) = pin.dxgi {
            assert_eq!(resolution.dxgi(), Some(dxgi), "{}", pin.matchers());
            assert_eq!(resolution.scalar(Scalar::Dxgi).set_by.as_ref(), Some(&from));
        }
        if let Setting::Set(d3d9) = pin.d3d9 {
            assert_eq!(resolution.d3d9(), Some(d3d9), "{}", pin.matchers());
            assert_eq!(resolution.scalar(Scalar::D3d9).set_by.as_ref(), Some(&from));
        }
    }
    // A `*` rule from WINE_COMPATDB folds after every built-in rule, so
    // it moves the pinned programs too.
    let star = with_lines("exe=*;dxgi=dxmt;d3d9=wined3d");
    for pin in pins {
        let resolution = star.resolve(pin.exe.as_str(), &version_of(pin));
        assert_eq!(resolution.dxgi(), Some(Dxgi::Dxmt), "{}", pin.matchers());
        assert_eq!(resolution.d3d9(), Some(D3d9::Wined3d), "{}", pin.matchers());
        assert_eq!(
            resolution.scalar(Scalar::Dxgi).set_by.as_ref(),
            Some(&Origin::Line(1))
        );
    }
    assert_eq!(
        rule(&table, "Launcher.exe")?.dxgi,
        Setting::Set(Dxgi::Wined3d)
    );
    Ok(())
}

#[test]
fn a_star_rule_gives_every_process_wined3d() {
    // For each family on its own.
    let table = with_lines("exe=*;dxgi=wined3d");
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.dxgi(), Some(Dxgi::Wined3d));
    assert_eq!(
        game.scalar(Scalar::Dxgi).set_by.as_ref(),
        Some(&Origin::Line(1))
    );
    assert_eq!(game.d3d9(), Some(D3d9::Mtld3d));
    let table = with_lines("exe=*;d3d9=wined3d");
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.dxgi(), Some(Dxgi::Dxmt));
    assert_eq!(game.d3d9(), Some(D3d9::Wined3d));
    // A rule for one game gives it wined3d and leaves the others alone.
    let table = with_lines("exe=game.exe;dxgi=wined3d;d3d9=wined3d");
    let game = table.resolve("Game.exe", &VersionInfo::default());
    assert_eq!(game.dxgi(), Some(Dxgi::Wined3d));
    assert_eq!(game.d3d9(), Some(D3d9::Wined3d));
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.dxgi(), Some(Dxgi::Dxmt));
}

#[test]
fn a_game_rule_turns_gcmouse_off() {
    let table = with_lines("exe=Game.exe;gcmouse=false");
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.gcmouse(), Some(false));
    // Nothing else changes for that game.
    let mut expected = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(expected.gcmouse(), Some(true));
    set_by(
        &mut expected,
        Scalar::Gcmouse,
        ScalarValue::Bool(false),
        &Origin::Line(1),
    );
    assert_eq!(game, expected);
}

#[test]
fn a_launch_wide_rule_turns_gcmouse_off_and_a_game_rule_back_on() {
    // Off for every process is what upstream Wine does without a value.
    let table = with_lines("exe=*;gcmouse=false\nexe=Game.exe;gcmouse=true");
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.gcmouse(), Some(false));
    assert_eq!(
        other.scalar(Scalar::Gcmouse).set_by.as_ref(),
        Some(&Origin::Line(1))
    );
    assert_eq!(other.force_nx(), Some(true));
    assert_eq!(other.dpi_aware(), Some(true));
    let game = table.resolve("Game.exe", &VersionInfo::default());
    assert_eq!(game.gcmouse(), Some(true));
    assert_eq!(
        game.scalar(Scalar::Gcmouse).set_by.as_ref(),
        Some(&Origin::Line(2))
    );
}

#[test]
fn a_game_rule_gives_one_game_4_gb() {
    let table = with_lines("exe=old.exe;large_address_aware=true");
    let old = table.resolve("Old.exe", &VersionInfo::default());
    assert_eq!(old.large_address_aware(), Some(true));
    assert_eq!(
        old.scalar(Scalar::LargeAddressAware).set_by.as_ref(),
        Some(&Origin::Line(1))
    );
    // Nothing else changes for that game.
    let mut expected = table.resolve("other.exe", &VersionInfo::default());
    set_by(
        &mut expected,
        Scalar::LargeAddressAware,
        ScalarValue::Bool(true),
        &Origin::Line(1),
    );
    assert_eq!(old, expected);
    // Another process keeps the executable's flag.
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.large_address_aware(), None);
    // A launch-wide rule gives every process 4 GB, and a rule for one
    // game takes it back to the executable's flag.
    let table = with_lines(
        "exe=*;large_address_aware=true\n\
         exe=old.exe;large_address_aware=false",
    );
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.large_address_aware(), Some(true));
    let old = table.resolve("Old.exe", &VersionInfo::default());
    assert_eq!(old.large_address_aware(), Some(false));
}

#[test]
fn a_line_of_the_old_format_changes_nothing() {
    // A launcher that still drops or changes a built-in rule by name
    // sends lines without an exe. Each is ignored with a warning for its
    // old keys and one for the line, and the table stays the built-in
    // one: dropping no longer turns a default off.
    for text in [
        "name=dpi-aware;enabled=false",
        "name=large-address-aware;large_address_aware=false",
        "name=world-of-warcraft;large_address_aware=false",
    ] {
        let mut table = Table::builtin();
        let (rules, diagnostics) = Table::parse(text);
        assert!(
            diagnostics
                .last()
                .is_some_and(|d| d == "WINE_COMPATDB line 1: ignoring the line, it has no exe"),
            "{text}: {diagnostics:?}"
        );
        assert!(
            diagnostics
                .first()
                .is_some_and(|d| d == "WINE_COMPATDB line 1: ignoring unknown key \"name\""),
            "{text}: {diagnostics:?}"
        );
        table.rules.extend(rules.rules);
        assert_eq!(table, Table::builtin(), "{text}");
    }
    // With an exe, the rest of the line applies as a rule of its own.
    let (rules, diagnostics) = Table::parse("name=my-game;exe=game.exe;dpi_aware=false");
    assert_eq!(
        diagnostics,
        ["WINE_COMPATDB line 1: ignoring unknown key \"name\""]
    );
    let mut table = Table::builtin();
    table.rules.extend(rules.rules);
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.dpi_aware(), Some(false));
}

/// The version resource of World of Warcraft's `WoW.exe`, as compatdb.so
/// reads it from the shipped 1.12.1 and 3.3.5a clients, the patched
/// 1.12.1 client of the Turtle server included: the three strings are the
/// same in all of them.
fn wow_version() -> VersionInfo {
    VersionInfo {
        company: "Blizzard Entertainment".into(),
        product: "World of Warcraft".into(),
        original_filename: "WoW.exe".into(),
    }
}

/// Where the built-in rule for World of Warcraft says its values come
/// from.
fn wow() -> Origin {
    Origin::BuiltInFor("WoW.exe (Blizzard Entertainment, World of Warcraft, WoW.exe)".into())
}

#[test]
fn world_of_warcraft_gets_4_gb_and_nothing_else_changes() {
    let table = Table::builtin();
    let mut expected = plain();
    set_by(
        &mut expected,
        Scalar::LargeAddressAware,
        ScalarValue::Bool(true),
        &wow(),
    );
    assert_eq!(table.resolve("WoW.exe", &wow_version()), expected);
    // The 3.3.5a client spells its file Wow.exe; the basename matches
    // case-insensitively.
    assert_eq!(table.resolve("Wow.exe", &wow_version()), expected);
    // A WoW.exe without that resource or from another company, and an
    // executable of another name with it, keep the executable's flag.
    let other = VersionInfo {
        company: "Some Vendor".into(),
        ..wow_version()
    };
    for (exe, version) in [
        ("WoW.exe", VersionInfo::default()),
        ("WoW.exe", other),
        ("WowError.exe", wow_version()),
    ] {
        assert_eq!(table.resolve(exe, &version), plain(), "{exe}");
    }
}

#[test]
fn a_rule_from_wine_compatdb_leaves_world_of_warcraft_to_its_flag() {
    // The rules of WINE_COMPATDB fold after every built-in rule, so a
    // rule on the basename alone beats the fingerprinted built-in one.
    for text in [
        "exe=WoW.exe;large_address_aware=false",
        "exe=WoW.exe;company=Blizzard;large_address_aware=false",
        "exe=*;large_address_aware=false",
    ] {
        let table = with_lines(text);
        let client = table.resolve("WoW.exe", &wow_version());
        assert_eq!(client.large_address_aware(), Some(false), "{text}");
        assert_eq!(
            client.scalar(Scalar::LargeAddressAware).set_by.as_ref(),
            Some(&Origin::Line(1)),
            "{text}"
        );
    }
    // `default` hands it back to the executable's flag, as unset.
    let table = with_lines("exe=WoW.exe;large_address_aware=default");
    let client = table.resolve("WoW.exe", &wow_version());
    assert_eq!(client.large_address_aware(), None);
    assert_eq!(
        client.scalar(Scalar::LargeAddressAware).set_by.as_ref(),
        None
    );
    assert_eq!(
        client.scalar(Scalar::LargeAddressAware).reset_by.as_ref(),
        Some(&Origin::Line(1))
    );
}

#[test]
fn within_wine_compatdb_a_more_specific_rule_still_wins() {
    // Whatever the line order, the rule for the game beats the `*` rule
    // of the same layer.
    for text in [
        "exe=*;dpi_aware=false\nexe=game.exe;dpi_aware=true",
        "exe=game.exe;dpi_aware=true\nexe=*;dpi_aware=false",
    ] {
        let table = with_lines(text);
        let game = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.dpi_aware(), Some(true), "{text}");
        let other = table.resolve("other.exe", &VersionInfo::default());
        assert_eq!(other.dpi_aware(), Some(false), "{text}");
    }
}

#[test]
fn default_hands_a_setting_back_to_upstream_wine() {
    // For every process: each built-in `*` value becomes unset.
    let table = with_lines(
        "exe=*;dxgi=default;d3d9=default;opengl=default;dpi_aware=default;gcmouse=default;\
         force_nx=default",
    );
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.dxgi(), None);
    assert_eq!(game.d3d9(), None);
    assert_eq!(game.opengl(), None);
    assert_eq!(game.dpi_aware(), None);
    assert_eq!(game.gcmouse(), None);
    assert_eq!(game.force_nx(), None);
    // Each says which rule reset it; wined3d and large_address_aware, which
    // no built-in rule sets, the line leaves alone.
    for &scalar in Scalar::ALL {
        let state = game.scalar(scalar);
        assert_eq!(state.set_by, None, "{scalar}");
        let reset = !matches!(scalar, Scalar::Wined3d | Scalar::LargeAddressAware);
        assert_eq!(state.reset_by, reset.then_some(Origin::Line(1)), "{scalar}");
    }
    // Then the trees are upstream Wine's.
    assert_eq!(
        Dxgi::effective(game.dxgi(), crate::Arch::X86_64),
        Dxgi::Wined3d
    );
    // A rule folded later sets it again, and the reset is gone.
    let table = with_lines("exe=*;dpi_aware=default\nexe=game.exe;dpi_aware=false");
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.dpi_aware(), Some(false));
    assert_eq!(game.scalar(Scalar::DpiAware).reset_by.as_ref(), None);
    assert_eq!(
        game.scalar(Scalar::DpiAware).set_by.as_ref(),
        Some(&Origin::Line(2))
    );
    let other = table.resolve("other.exe", &VersionInfo::default());
    assert_eq!(other.dpi_aware(), None);
    assert_eq!(
        other.scalar(Scalar::DpiAware).reset_by.as_ref(),
        Some(&Origin::Line(1))
    );
}

#[test]
fn a_default_dll_override_hands_mono_and_gecko_back_to_the_registry() {
    // Each module comes out of the built-in entry on its own.
    let table = with_lines("exe=*;dll_overrides=mscoree=default");
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.dll_overrides, ["mshtml="]);
    assert_eq!(game.dll_overrides_from, [Origin::BuiltIn]);
    assert_eq!(
        game.dll_overrides_reset,
        [DllOverrideReset {
            module: "mscoree".into(),
            by: Origin::Line(1),
            removed: vec![Origin::BuiltIn],
        }]
    );
    // Both, in one entry or two, leave no entry at all.
    for text in [
        "exe=*;dll_overrides=mscoree,mshtml=default",
        "exe=*;dll_overrides=mscoree=default;dll_overrides=MSHTML.dll=default",
    ] {
        let table = with_lines(text);
        let game = table.resolve("game.exe", &VersionInfo::default());
        assert_eq!(game.dll_overrides, [] as [&str; 0], "{text}");
        assert_eq!(game.dll_overrides_reset.len(), 2, "{text}");
    }
    // A module no entry names is reported with nothing removed, and a
    // later entry gives a reset module a load order again.
    let table = with_lines("exe=*;dll_overrides=d3d12=default;dll_overrides=mscoree=b");
    let game = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(game.dll_overrides, ["mscoree,mshtml=", "mscoree=b"]);
    assert_eq!(
        game.dll_overrides_reset,
        [DllOverrideReset {
            module: "d3d12".into(),
            by: Origin::Line(1),
            removed: Vec::new(),
        }]
    );
}

#[test]
fn a_launch_wide_rule_beats_the_builtin_dpi_default() {
    // Both are plain `*` rules, so specificity does not separate them.
    // The launcher's rule comes after the built-ins in the table, and
    // the stable sort in `resolve` keeps that order.
    let table = with_lines("exe=*;dpi_aware=false");
    let resolution = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(resolution.dpi_aware(), Some(false));
    assert_eq!(
        resolution.scalar(Scalar::DpiAware).set_by.as_ref(),
        Some(&Origin::Line(1))
    );
}

#[test]
fn a_rule_for_one_program_makes_it_unaware() {
    let table = with_lines("exe=old.exe;dpi_aware=false");
    assert_eq!(
        table
            .resolve("Old.exe", &VersionInfo::default())
            .dpi_aware(),
        Some(false)
    );
    assert_eq!(
        table
            .resolve("other.exe", &VersionInfo::default())
            .dpi_aware(),
        Some(true)
    );
}

#[test]
fn a_launch_wide_rule_re_enables_mono_after_the_builtins() {
    // What a launcher sends for a top-level `dll_overrides`: its own `*`
    // rule. Within the wildcard tier the rules fold in table order, which
    // puts the built-ins first, and ntdll keeps the last entry for a
    // module, so `mscoree=b` wins while mshtml stays disabled.
    let table = with_lines("exe=*;dll_overrides=mscoree=b");
    let resolution = table.resolve("game.exe", &VersionInfo::default());
    assert_eq!(resolution.dll_overrides, ["mscoree,mshtml=", "mscoree=b"]);
    assert_eq!(
        resolution.dll_overrides_from,
        [Origin::BuiltIn, Origin::Line(1)]
    );
    assert_eq!(table.duplicate_matchers(), []);
}

#[test]
fn an_arguments_override_drops_a_builtin_helpers_switch() -> TestResult {
    let table = Table::builtin();
    let steam = rule(&table, "steamwebhelper.exe")?;
    let version = version_of(steam);
    let from = Origin::BuiltInFor(steam.matchers());
    // A rule from WINE_COMPATDB folds after the helper's, with or without
    // a fingerprint, and replaces its switch.
    for (text, want) in [
        (
            "exe=steamwebhelper.exe;company=Valve;arguments_override=",
            vec![],
        ),
        ("exe=steamwebhelper.exe;arguments_override=-x", vec!["-x"]),
    ] {
        let table = with_lines(text);
        let helper = table.resolve("steamwebhelper.exe", &version);
        assert_eq!(helper.arguments, want, "{text}");
        assert_eq!(
            helper.arguments_dropped,
            [ArgumentsDropped {
                by: Origin::Line(1),
                entries: vec![(Argument::new("--in-process-gpu")?, from.clone())],
            }],
            "{text}"
        );
    }
    Ok(())
}

#[test]
fn the_rules_for_one_program_match_the_real_binaries_resources() {
    // Guards the fingerprints against a typo: these are the strings read
    // out of the shipped exes. Each program gets its rule's value from
    // that rule.
    let steam = VersionInfo {
        company: "Valve Corporation".into(),
        product: "Steam Client WebHelper".into(),
        original_filename: "steamwebhelper.exe".into(),
    };
    let social = VersionInfo {
        company: "Take-Two Interactive Software, Inc.".into(),
        product: "Social Club UI".into(),
        original_filename: "SocialClubHelper.exe".into(),
    };
    let gta = VersionInfo {
        company: "Rockstar Games".into(),
        product: "Grand Theft Auto IV".into(),
        original_filename: "GTAIV.exe".into(),
    };
    // NosTale's client names its company and nothing else.
    let nostale = VersionInfo {
        company: "Entwell".into(),
        ..VersionInfo::default()
    };
    let table = Table::builtin();
    let builtin = |m: &str| Origin::BuiltInFor(m.into());
    let r = table.resolve("steamwebhelper.exe", &steam);
    assert_eq!(
        r.arguments_from,
        [builtin(
            "steamwebhelper.exe (Valve Corporation, Steam Client WebHelper)"
        )]
    );
    let r = table.resolve("SocialClubHelper.exe", &social);
    assert_eq!(
        r.arguments_from,
        [builtin(
            "SocialClubHelper.exe (Take-Two Interactive Software, Social Club UI)"
        )]
    );
    let r = table.resolve("GTAIV.exe", &gta);
    assert_eq!(
        r.arguments_from,
        [builtin("GTAIV.exe (Rockstar Games, Grand Theft Auto IV)")]
    );
    let r = table.resolve("NostaleClientX.exe", &nostale);
    assert_eq!(
        r.scalar(Scalar::DpiAware).set_by.as_ref(),
        Some(&builtin("NostaleClientX.exe (Entwell)"))
    );
    let r = table.resolve("AoK HD.exe", &aoe2_hd_version());
    assert_eq!(
        r.scalar(Scalar::DpiAware).set_by.as_ref(),
        Some(&builtin("AoK HD.exe (Microsoft Corporation)"))
    );
    let r = table.resolve("WoW.exe", &wow_version());
    assert_eq!(
        r.scalar(Scalar::LargeAddressAware).set_by.as_ref(),
        Some(&wow())
    );
    // An unrelated program of the same name gets only the `*` rules.
    for exe in [
        "steamwebhelper.exe",
        "SocialClubHelper.exe",
        "Launcher.exe",
        "GTAIV.exe",
        "NostaleClientX.exe",
        "AoK HD.exe",
        "WoW.exe",
    ] {
        assert_eq!(
            table.resolve(exe, &VersionInfo::default()),
            plain(),
            "{exe}"
        );
    }
}

#[test]
fn the_nostale_client_is_dpi_unaware_and_nothing_else_changes() {
    // The fingerprinted rule beats the DPI default for the client alone;
    // every other setting stays the wildcards'.
    let nostale = VersionInfo {
        company: "Entwell".into(),
        ..VersionInfo::default()
    };
    let table = Table::builtin();
    let mut expected = plain();
    set_by(
        &mut expected,
        Scalar::DpiAware,
        ScalarValue::Bool(false),
        &Origin::BuiltInFor("NostaleClientX.exe (Entwell)".into()),
    );
    assert_eq!(table.resolve("NostaleClientX.exe", &nostale), expected);
    assert_eq!(
        table
            .resolve("NostaleClientX.exe", &VersionInfo::default())
            .dpi_aware(),
        Some(true)
    );
}

/// The version resource of Age of Empires II HD's `AoK HD.exe`, as
/// compatdb.so reads it from the shipped exe.
fn aoe2_hd_version() -> VersionInfo {
    VersionInfo {
        company: "Microsoft Corporation".into(),
        product: "age2_x1".into(),
        original_filename: "age2_x1.exe".into(),
    }
}

#[test]
fn age_of_empires_ii_hd_is_dpi_unaware_and_nothing_else_changes() {
    // The fingerprinted rule beats the DPI default for the game alone;
    // every other setting stays the wildcards'.
    let table = Table::builtin();
    let mut expected = plain();
    set_by(
        &mut expected,
        Scalar::DpiAware,
        ScalarValue::Bool(false),
        &Origin::BuiltInFor("AoK HD.exe (Microsoft Corporation)".into()),
    );
    assert_eq!(table.resolve("AoK HD.exe", &aoe2_hd_version()), expected);
    // Only the company is pinned, so a build with another product name
    // still matches.
    let newer = VersionInfo {
        product: "Age of Empires II HD".into(),
        ..aoe2_hd_version()
    };
    assert_eq!(table.resolve("AoK HD.exe", &newer).dpi_aware(), Some(false));
    // An executable of another name with the same version resource keeps
    // the default, and so does one of the same name from another company.
    assert_eq!(
        table.resolve("age2_x1.exe", &aoe2_hd_version()).dpi_aware(),
        Some(true)
    );
    let other = VersionInfo {
        company: "Some Vendor".into(),
        ..aoe2_hd_version()
    };
    assert_eq!(table.resolve("AoK HD.exe", &other), plain());
}
