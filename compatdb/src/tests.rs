use compatdb_core::{Argument, D3d9, DllOverride, Dxgi, ENV_VAR, EnvName, FILE_ENV_VAR};

use super::*;

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn a_native_arm64_program_gets_dxmt_and_mtld3d_and_either_tree_a_rule_asks_for() {
    let native = Arch::Aarch64;
    // The built-in rules give it dxmt and mtld3d, as an x86 program.
    let game = Table::builtin().resolve("game.exe", &VersionInfo::default());
    assert_eq!(effective_dxgi(game.dxgi(), native), Dxgi::Dxmt);
    assert_eq!(effective_d3d9(game.d3d9(), native), D3d9::Mtld3d);
    // Without a rule, wined3d, as in upstream Wine.
    assert_eq!(effective_dxgi(None, native), Dxgi::Wined3d);
    assert_eq!(effective_d3d9(None, native), D3d9::Wined3d);
    // Either tree of each family is what it gets when a rule asks for it.
    for requested in [Dxgi::Dxmt, Dxgi::Wined3d] {
        assert_eq!(effective_dxgi(Some(requested), native), requested);
    }
    for requested in [D3d9::Mtld3d, D3d9::Wined3d] {
        assert_eq!(effective_d3d9(Some(requested), native), requested);
    }
}

/// The source of the built-in rule for World of Warcraft, as the log
/// names it.
fn wow() -> Origin {
    Origin::BuiltInFor("WoW.exe (Blizzard Entertainment, World of Warcraft, WoW.exe)".into())
}

#[test]
fn the_wined3d_line_names_the_renderer() {
    let unset = source(Key::Wined3d, None, None);
    let line = Origin::Line(2);
    // A renderer a rule set wins the line over any config, and reads like
    // any other setting.
    for config in [None, Some(("renderer=vulkan", None))] {
        assert_eq!(
            wined3d_line(Some(Wined3d::Gl), &line.label(), None, config),
            "  wined3d = gl, WINE_COMPATDB line 2"
        );
    }
    assert_eq!(
        wined3d_line(Some(Wined3d::Vulkan), &wow().label(), None, None),
        "  wined3d = vulkan, built-in for WoW.exe (Blizzard Entertainment, World of \
         Warcraft, WoW.exe)"
    );
    // Not set: what a config asks for, inherited or from a rule's env,
    // else what upstream Wine does, which may still come from the
    // registry.
    assert_eq!(
        wined3d_line(None, &unset, None, Some(("csmt=0,renderer=vulkan", None))),
        "  wined3d = vulkan, from the inherited WINE_D3D_CONFIG"
    );
    assert_eq!(
        wined3d_line(None, &unset, None, Some(("renderer=vulkan", Some(&line)))),
        "  wined3d = vulkan, from the WINE_D3D_CONFIG of WINE_COMPATDB line 2"
    );
    assert_eq!(
        wined3d_line(
            None,
            &unset,
            None,
            Some(("renderer=vulkan", Some(&Origin::BuiltIn)))
        ),
        "  wined3d = vulkan, from the WINE_D3D_CONFIG of built-in"
    );
    for config in [None, Some(("", None)), Some(("csmt=0", Some(&line)))] {
        assert_eq!(
            wined3d_line(None, &unset, None, config),
            format!("  wined3d = {unset}")
        );
    }
    // A reset says so, with or without a config that asks for a
    // renderer.
    let reset = source(Key::Wined3d, None, Some(&line));
    assert_eq!(
        wined3d_line(None, &reset, Some(&line), None),
        format!(
            "  wined3d = unset (upstream: {}), reset by WINE_COMPATDB line 2",
            upstream(Key::Wined3d)
        )
    );
    assert_eq!(
        wined3d_line(None, &reset, Some(&line), Some(("renderer=vulkan", None))),
        "  wined3d = vulkan, from the inherited WINE_D3D_CONFIG, reset by WINE_COMPATDB line 2"
    );
    assert_eq!(
        wined3d_line(
            None,
            &reset,
            Some(&line),
            Some(("renderer=gl", Some(&Origin::Line(3))))
        ),
        "  wined3d = gl, from the WINE_D3D_CONFIG of WINE_COMPATDB line 3, reset by \
         WINE_COMPATDB line 2"
    );
}

#[test]
fn a_reset_scalar_and_a_default_dll_override_say_which_line_reset_them() {
    assert_eq!(
        source(Key::DpiAware, None, Some(&Origin::Line(3))),
        format!(
            "unset (upstream: {}), reset by WINE_COMPATDB line 3",
            upstream(Key::DpiAware)
        )
    );
    // A value set after the reset is what the line shows.
    assert_eq!(
        source(Key::DpiAware, Some(&Origin::Line(4)), None),
        "WINE_COMPATDB line 4"
    );
    let reset = |removed: Vec<Origin>| DllOverrideReset {
        module: "mscoree".into(),
        by: Origin::Line(1),
        removed,
    };
    assert_eq!(
        dll_reset_line(&reset(vec![Origin::BuiltIn])),
        "  dll_overrides mscoree=default, WINE_COMPATDB line 1: removes the override from \
         built-in"
    );
    assert_eq!(
        dll_reset_line(&reset(vec![Origin::BuiltIn, wow()])),
        "  dll_overrides mscoree=default, WINE_COMPATDB line 1: removes the override from \
         built-in; built-in for WoW.exe (Blizzard Entertainment, World of Warcraft, WoW.exe)"
    );
    assert_eq!(
        dll_reset_line(&reset(Vec::new())),
        "  dll_overrides mscoree=default, WINE_COMPATDB line 1: no override to remove"
    );
}

#[test]
fn the_list_and_env_lines_name_the_source_of_each_entry_and_what_happened() -> TestResult {
    assert_eq!(
        list_line(Key::Arguments, &[] as &[String], &[], Some(&[])),
        "  arguments = unset (upstream: the command line the process was started with)"
    );
    let args = ["--in-process-gpu".to_string(), "-x".to_string()];
    let from = [wow(), Origin::Line(1)];
    assert_eq!(
        list_line(Key::Arguments, &args, &from, Some(&[false, true])),
        "  arguments = --in-process-gpu (built-in for WoW.exe (Blizzard Entertainment, \
         World of Warcraft, WoW.exe), already present); -x (WINE_COMPATDB line 1, \
         appended)"
    );
    assert_eq!(
        list_line(
            Key::DllOverrides,
            &[DllOverride::new("mscoree,mshtml=")?],
            &[Origin::BuiltIn],
            None
        ),
        "  dll_overrides = mscoree,mshtml= (built-in)"
    );
    let name = EnvName::new("FEX_X87REDUCEDPRECISION")?;
    let one = EnvEntry::new(name.clone(), "1")?;
    let removal = EnvEntry::remove(name);
    let step = |value: &str, applied, replaced_by: Option<Origin>| EnvStep {
        entry: if value.is_empty() {
            removal.clone()
        } else {
            one.clone()
        },
        origin: Origin::BuiltIn,
        applied,
        replaced_by,
        removes: None,
    };
    assert_eq!(
        env_line(&step("1", true, None), None),
        "  env FEX_X87REDUCEDPRECISION=1, built-in: set"
    );
    assert_eq!(
        env_line(&step("", true, None), Some("0")),
        "  env FEX_X87REDUCEDPRECISION=, built-in: removed"
    );
    assert_eq!(
        env_line(&step("", true, None), None),
        "  env FEX_X87REDUCEDPRECISION=, built-in: not set, nothing to remove"
    );
    // A removal after an earlier entry set the variable undoes that one,
    // whether or not the process had it.
    let undo = EnvStep {
        origin: Origin::Line(3),
        removes: Some(Origin::BuiltIn),
        ..step("", true, None)
    };
    assert_eq!(
        env_line(&undo, None),
        "  env FEX_X87REDUCEDPRECISION=, WINE_COMPATDB line 3: removes the value from \
         built-in"
    );
    assert_eq!(
        env_line(&step("1", false, None), Some("0")),
        "  env FEX_X87REDUCEDPRECISION=1, built-in: left alone, the process has \
         FEX_X87REDUCEDPRECISION=0"
    );
    assert_eq!(
        env_line(&step("1", true, Some(Origin::Line(2))), None),
        "  env FEX_X87REDUCEDPRECISION=1, built-in: replaced by a later entry from \
         WINE_COMPATDB line 2"
    );
    assert_eq!(
        env_line(&step("1", true, Some(wow())), None),
        "  env FEX_X87REDUCEDPRECISION=1, built-in: replaced by a later entry from \
         built-in for WoW.exe (Blizzard Entertainment, World of Warcraft, WoW.exe)"
    );
    Ok(())
}

#[test]
fn an_arguments_override_line_names_what_it_dropped() -> TestResult {
    let dropped = ArgumentsDropped {
        by: Origin::Line(1),
        entries: vec![
            (Argument::new("--in-process-gpu")?, wow()),
            (Argument::new("-x")?, Origin::BuiltIn),
        ],
    };
    assert_eq!(
        dropped_line(&dropped),
        "  arguments_override, WINE_COMPATDB line 1: dropped --in-process-gpu (built-in \
         for WoW.exe (Blizzard Entertainment, World of Warcraft, WoW.exe)); -x (built-in)"
    );
    Ok(())
}

#[test]
fn an_unset_line_says_what_upstream_wine_does() {
    assert_eq!(
        unset_line(Key::Gcmouse),
        "  gcmouse = unset (upstream: off, the cursor's movement)"
    );
    assert_eq!(
        unset_line(Key::ForceNx),
        "  force_nx = unset (upstream: no-execute off when the executable or a DLL lacks the \
         NX_COMPAT flag)"
    );
    assert_eq!(
        unset_line(Key::OpenGl),
        "  opengl = unset (upstream: Apple's OpenGL)"
    );
    assert_eq!(
        unset_line(Key::Env),
        "  env = unset (upstream: the environment the process was started with)"
    );
    assert_eq!(source(Key::Dxgi, None, None), "unset (upstream: wined3d)");
    assert_eq!(source(Key::Dxgi, Some(&Origin::BuiltIn), None), "built-in");
    assert_eq!(
        source(Key::Dxgi, Some(&Origin::Line(4)), None),
        "WINE_COMPATDB line 4"
    );
    // Every key the log prints a line for says what upstream Wine does.
    for scalar in Scalar::ALL {
        assert!(!upstream(scalar.key()).is_empty(), "{scalar}");
    }
    for key in [Key::DllOverrides, Key::Arguments, Key::Env] {
        assert!(!upstream(key).is_empty(), "{key}");
    }
}

#[test]
fn the_metal_hud_and_debug_channels_lines_name_value_and_source() -> TestResult {
    let table = Table::parse_strict(
        "exe=game.exe;metal_hud=true;debug_channels=+relay;debug_channels=-heap\n\
         exe=tool.exe;metal_hud=default",
    )?
    .with_builtin();
    let none = VersionInfo::default();
    let game = table.resolve("game.exe", &none);
    assert_eq!(
        metal_hud_line(&game),
        "  metal_hud = true, WINE_COMPATDB line 1"
    );
    assert_eq!(
        debug_channels_line(&game),
        "  debug_channels = +relay (WINE_COMPATDB line 1); -heap (WINE_COMPATDB line 1); \
         logged only: Wine reads WINEDEBUG before compatdb.so loads"
    );
    let other = table.resolve("other.exe", &none);
    assert_eq!(
        metal_hud_line(&other),
        "  metal_hud = unset (upstream: the MTL_HUD_ variables of the environment the process \
         was started with)"
    );
    assert_eq!(
        debug_channels_line(&other),
        "  debug_channels = unset (upstream: the WINEDEBUG the process was started with)"
    );
    let tool = table.resolve("tool.exe", &none);
    assert_eq!(
        metal_hud_line(&tool),
        "  metal_hud = unset (upstream: the MTL_HUD_ variables of the environment the process \
         was started with), reset by WINE_COMPATDB line 2"
    );
    Ok(())
}

#[test]
fn the_log_labels_each_arch() {
    assert_eq!(arch_name(Arch::X86_64), "x86_64 (arm64ec)");
    assert_eq!(arch_name(Arch::I386), "i386");
    assert_eq!(arch_name(Arch::Aarch64), "aarch64");
}

#[test]
fn without_wine_compatdb_the_table_is_the_builtin_one() {
    // load_table reads the variables a developer's shell may set; the answer
    // is only predictable without them.
    if std::env::var_os(ENV_VAR).is_some() || std::env::var_os(FILE_ENV_VAR).is_some() {
        return;
    }
    let (table, lines) = load_table();
    assert_eq!(table, Table::builtin());
    assert_eq!(lines, [] as [String; 0]);
}
