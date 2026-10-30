use super::*;
use crate::{D3d9, Dxgi, OpenGl, Table, Wined3d};

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The rule for `Game.exe` with nothing set.
fn game() -> Result<Rule, FieldError> {
    Exe::new("Game.exe").map(Rule::new)
}

#[test]
fn a_setting_names_every_key_in_any_case() {
    for &key in Key::ALL {
        assert_eq!(Key::from_setting(key.as_str()), Ok(key));
        assert_eq!(
            Key::from_setting(&key.as_str().to_ascii_uppercase()),
            Ok(key)
        );
    }
    // A launcher's own keys are no keys of a rule.
    for other in ["retina_mode", "dpi", "enabled", "compat", "executable"] {
        assert_eq!(
            Key::from_setting(other),
            Err(FieldError::UnknownKey(other.into()))
        );
        assert_eq!(
            Key::from_setting(other).map_err(|e| e.severity()),
            Err(Severity::Error)
        );
    }
}

#[test]
fn a_retired_key_is_a_warning_or_an_error_with_its_reason() {
    let x87 = Key::from_setting("x87_sidecar");
    assert_eq!(
        x87.as_ref().map_err(FieldError::severity),
        Err(Severity::Warning)
    );
    assert_eq!(
        x87.map_err(|e| e.to_string()),
        Err(
            "x87_sidecar is no longer read: this Wine runs x87 code in FEX and has no x87 \
             sidecar; remove it"
                .into()
        )
    );
    let append = Key::from_setting("cmdline_append");
    assert_eq!(
        append.as_ref().map_err(FieldError::severity),
        Err(Severity::Error)
    );
    assert_eq!(
        append.map_err(|e| e.to_string()),
        Err("cmdline_append is no longer read; use arguments".into())
    );
    // The key as written is in the message.
    assert_eq!(
        Key::from_setting("X87_Sidecar").map_err(|e| e.to_string().starts_with("X87_Sidecar ")),
        Err(true)
    );
    // On the wire they stay unknown keys, which cost only their field.
    assert_eq!(
        "x87_sidecar".parse::<Key>(),
        Err(FieldError::UnknownKey("x87_sidecar".into()))
    );
}

#[test]
fn set_reads_each_scalar_in_any_case_and_default_resets_it() -> TestResult {
    let mut rule = game()?;
    for (key, text) in [
        (Key::Dxgi, "WineD3D"),
        (Key::D3d9, "mtld3d"),
        (Key::Wined3d, "Vulkan"),
        (Key::OpenGl, "CGL"),
        (Key::DpiAware, "false"),
        (Key::LargeAddressAware, "TRUE"),
        (Key::Gcmouse, "False"),
        (Key::ForceNx, "true"),
        (Key::MetalHud, "true"),
    ] {
        rule.set(key, text)?;
    }
    assert_eq!(rule.dxgi(), Setting::Set(Dxgi::Wined3d));
    assert_eq!(rule.d3d9(), Setting::Set(D3d9::Mtld3d));
    assert_eq!(rule.wined3d(), Setting::Set(Wined3d::Vulkan));
    assert_eq!(rule.opengl(), Setting::Set(OpenGl::Cgl));
    assert_eq!(rule.dpi_aware(), Setting::Set(false));
    assert_eq!(rule.large_address_aware(), Setting::Set(true));
    assert_eq!(rule.gcmouse(), Setting::Set(false));
    assert_eq!(rule.force_nx(), Setting::Set(true));
    assert_eq!(rule.metal_hud(), Setting::Set(true));
    for &scalar in Scalar::ALL {
        rule.set(scalar.key(), "Default")?;
        assert_eq!(rule.scalar(scalar), Setting::Reset, "{scalar}");
    }
    // A word the setting does not take is an error, and changes nothing.
    let before = rule.clone();
    let error = rule.set(Key::DpiAware, "off");
    assert_eq!(
        error,
        Err(FieldError::BadValue {
            key: Key::DpiAware,
            value: "off".into(),
            expected: &["true", "false", "default"],
        })
    );
    assert_eq!(error.map_err(|e| e.severity()), Err(Severity::Error));
    assert_eq!(rule, before);
    Ok(())
}

#[test]
fn dxgi_gptk_is_ignored_with_a_warning_and_d3d9_gptk_is_an_error() -> TestResult {
    let mut rule = game()?.with_dxgi(Dxgi::Dxmt);
    let before = rule.clone();
    for text in ["gptk", "GPTK"] {
        let error = rule.set(Key::Dxgi, text);
        assert_eq!(
            error.as_ref().map_err(FieldError::severity),
            Err(Severity::Warning)
        );
        assert_eq!(rule, before, "the value is ignored");
        assert_eq!(
            error.map_err(|e| e.to_string()),
            Err(format!(
                "dxgi value \"{text}\" asks for Apple's D3DMetal, which this Wine does not have, \
                 so it is ignored as if it were not there: without another dxgi value the \
                 built-in rules decide, which give DXMT unless they pin the program to another \
                 tree; remove it"
            ))
        );
    }
    // Only dxgi had it: for d3d9 it is an unknown value.
    let error = rule.set(Key::D3d9, "gptk");
    assert!(
        matches!(error, Err(FieldError::BadValue { key: Key::D3d9, .. })),
        "{error:?}"
    );
    assert_eq!(error.map_err(|e| e.severity()), Err(Severity::Error));
    Ok(())
}

#[test]
fn set_reads_matchers_and_adds_one_entry_to_a_list() -> TestResult {
    let mut rule = Rule::new(Exe::Any);
    rule.set(Key::Exe, "Game.exe")?;
    rule.set(Key::Company, "Acme")?;
    rule.set(Key::Product, "Game")?;
    rule.set(Key::OriginalFilename, "game.exe")?;
    rule.set(Key::DllOverrides, "d3d12=")?;
    rule.set(Key::DllOverrides, "mscoree=default")?;
    rule.set(Key::Arguments, "-a")?;
    rule.set(Key::Arguments, "-b")?;
    rule.set(Key::ArgumentsOverride, "-c")?;
    rule.set(Key::DebugChannels, "+relay")?;
    rule.set(Key::Env, "A=b=c")?;
    rule.set(Key::Env, "GONE")?;
    assert_eq!(rule.matchers(), "Game.exe (Acme, Game, game.exe)");
    assert_eq!(rule.dll_overrides(), ["d3d12=", "mscoree=default"]);
    assert_eq!(rule.arguments(), ["-a", "-b"]);
    assert_eq!(rule.arguments_override(), Some(&[Argument::new("-c")?][..]));
    assert_eq!(rule.debug_channels(), ["+relay"]);
    let env: Vec<String> = rule.env().iter().map(ToString::to_string).collect();
    assert_eq!(env, ["A=b=c", "GONE="]);
    // A part that is empty, or holds a NUL, is the part's error.
    for (key, text, error) in [
        (Key::Exe, "", FieldError::MissingExe),
        (Key::Arguments, "", FieldError::EmptyArgument),
        (Key::DebugChannels, "", FieldError::EmptyDebugChannel),
        (
            Key::DllOverrides,
            "mscoree",
            FieldError::InvalidDllOverride("mscoree".into()),
        ),
        (Key::Company, "a\0", FieldError::Nul),
        (Key::Env, "=1", FieldError::InvalidEnvName(String::new())),
    ] {
        let before = rule.clone();
        assert_eq!(rule.set(key, text), Err(error), "{key} {text:?}");
        assert_eq!(rule, before, "{key}");
    }
    Ok(())
}

#[test]
fn a_debug_channels_value_is_split_at_each_comma_and_each_part_is_scoped() -> TestResult {
    let mut rule = game()?;
    rule.set(Key::DebugChannels, "+relay,warn+heap")?;
    rule.set_list(Key::DebugChannels, ["+seh,+tid", "-x"])?;
    assert_eq!(rule.debug_channels(), ["+seh", "+tid", "-x"]);
    let mut table = Table::new();
    table.push(rule.clone());
    assert_eq!(
        table.winedebug(),
        ["Game.exe:+seh", "Game.exe:+tid", "Game.exe:-x"]
    );
    // An empty part is refused, and the rule is left as it was.
    for bad in ["", "+relay,", ",+relay", "a,,b"] {
        let before = rule.clone();
        assert_eq!(
            rule.set(Key::DebugChannels, bad),
            Err(FieldError::EmptyDebugChannel),
            "{bad:?}"
        );
        assert_eq!(rule, before, "{bad:?}");
    }
    Ok(())
}

#[test]
fn the_inherited_conflicts_are_the_variables_a_launcher_removes() {
    assert_eq!(
        inherited_conflicts(),
        [
            "WINEDLLOVERRIDES",
            "WINE_LARGE_ADDRESS_AWARE",
            "WINE_COMPATDB_FILE"
        ]
    );
    // Only WINEDLLOVERRIDES is retired: a launcher may still set the others
    // for a launch on purpose.
    assert_eq!(retired_env("WINEDLLOVERRIDES"), Some(Key::DllOverrides));
    assert_eq!(retired_env("WINE_LARGE_ADDRESS_AWARE"), None);
    assert_eq!(retired_env(FILE_ENV_VAR), None);
}

#[test]
fn a_key_spelt_in_two_cases_is_one_key_and_the_last_value_wins() -> TestResult {
    let mut rule = game()?;
    for (name, value) in [("dxgi", "dxmt"), ("DXGI", "wined3d")] {
        rule.set(Key::from_setting(name)?, value)?;
    }
    assert_eq!(rule.dxgi(), Setting::Set(Dxgi::Wined3d));
    Ok(())
}

#[test]
fn set_list_replaces_a_list_and_an_empty_arguments_override_clears() -> TestResult {
    let mut rule = game()?;
    rule.set(Key::Arguments, "-old")?;
    rule.set_list(Key::Arguments, ["-a", "-b"])?;
    assert_eq!(rule.arguments(), ["-a", "-b"]);
    rule.set_list(Key::DllOverrides, vec![String::from("a=n")])?;
    assert_eq!(rule.dll_overrides(), ["a=n"]);
    rule.set_list(Key::DebugChannels, ["+seh", "-heap"])?;
    assert_eq!(rule.debug_channels(), ["+seh", "-heap"]);
    rule.set_list(Key::Env, ["A=1"])?;
    assert_eq!(rule.env().len(), 1);
    rule.set_list(Key::ArgumentsOverride, ["-x"])?;
    assert_eq!(rule.arguments_override(), Some(&[Argument::new("-x")?][..]));
    rule.set_list(Key::ArgumentsOverride, [] as [&str; 0])?;
    assert_eq!(rule.arguments_override(), Some(&[][..]));
    // A bad entry leaves the whole list as it was.
    let before = rule.clone();
    assert_eq!(
        rule.set_list(Key::Arguments, ["-c", ""]),
        Err(FieldError::EmptyArgument)
    );
    assert_eq!(rule, before);
    // A key that holds one value is no list.
    for key in [Key::Exe, Key::Dxgi, Key::MetalHud] {
        assert_eq!(
            rule.set_list(key, ["x"]),
            Err(FieldError::NotAList(key)),
            "{key}"
        );
    }
    assert_eq!(
        FieldError::NotAList(Key::Dxgi).to_string(),
        "dxgi holds one value, not a list"
    );
    Ok(())
}

#[test]
fn arguments_override_is_refused_in_a_rule_for_every_process() -> TestResult {
    let mut any = Rule::new(Exe::Any);
    let errors = [
        any.set(Key::ArgumentsOverride, "-x"),
        any.set_list(Key::ArgumentsOverride, [] as [&str; 0]),
    ];
    for error in errors {
        assert_eq!(error, Err(FieldError::ArgumentsOverrideOnAny));
        assert_eq!(error.map_err(|e| e.severity()), Err(Severity::Error));
    }
    assert_eq!(any.arguments_override(), None);
    // Whichever comes first: a rule that has one cannot become a `*` rule.
    let mut rule = game()?;
    rule.set(Key::ArgumentsOverride, "-x")?;
    assert_eq!(
        rule.set(Key::Exe, "*"),
        Err(FieldError::ArgumentsOverrideOnAny)
    );
    assert_eq!(rule.exe().as_str(), "Game.exe");
    rule.set(Key::Exe, "Other.exe")?;
    // A `*` with a fingerprint matches every program too.
    let mut pinned = Rule::new(Exe::Any);
    pinned.set(Key::Company, "Acme")?;
    assert_eq!(
        pinned.set(Key::ArgumentsOverride, "-x"),
        Err(FieldError::ArgumentsOverrideOnAny)
    );
    assert_eq!(
        FieldError::ArgumentsOverrideOnAny.to_string(),
        "arguments_override needs an exe other than *: for every process it would also take \
         away the arguments the built-in rules give single programs, which some of them need to \
         work"
    );
    Ok(())
}

#[test]
fn a_retired_environment_variable_names_the_setting_that_replaced_it() -> TestResult {
    for (name, key) in [
        ("WINEDEBUG", Key::DebugChannels),
        ("WINEDLLOVERRIDES", Key::DllOverrides),
        ("WINE_CMDLINE_APPEND", Key::Arguments),
        ("WINE_MAC_OPENGL", Key::OpenGl),
        ("WINE_FORCE_NX", Key::ForceNx),
        ("WineDllOverrides", Key::DllOverrides),
    ] {
        assert_eq!(retired_env(name), Some(key), "{name}");
        let mut rule = game()?;
        let error = rule.set_env(name, "x");
        assert_eq!(
            error,
            Err(FieldError::RetiredEnv {
                name: name.into(),
                key
            })
        );
        assert_eq!(error.map_err(|e| e.severity()), Err(Severity::Error));
        assert!(rule.set(Key::Env, &format!("{name}=x")).is_err(), "{name}");
        assert_eq!(rule.env(), []);
    }
    assert_eq!(
        FieldError::RetiredEnv {
            name: "WINEDEBUG".into(),
            key: Key::DebugChannels
        }
        .to_string(),
        "env WINEDEBUG is no longer read; use the debug_channels setting"
    );
    // Wine still reads these, so a rule may set them.
    for name in [
        "WINE_LARGE_ADDRESS_AWARE",
        "MTL_HUD_ENABLED",
        "WINE_D3D_CONFIG",
    ] {
        assert_eq!(retired_env(name), None, "{name}");
    }
    let mut rule = game()?;
    rule.set_env("MTL_HUD_ENABLED", "0")?;
    rule.set_env("FEX_X87REDUCEDPRECISION", "")?;
    let env: Vec<String> = rule.env().iter().map(ToString::to_string).collect();
    assert_eq!(env, ["MTL_HUD_ENABLED=0", "FEX_X87REDUCEDPRECISION="]);
    Ok(())
}

/// A value every field of `key` takes.
const fn sample(key: Key) -> &'static str {
    match key {
        Key::Exe => "Game.exe",
        Key::Company | Key::Product | Key::OriginalFilename => "Acme; 100%",
        Key::Dxgi => "dxmt",
        Key::D3d9 => "default",
        Key::Wined3d => "gl",
        Key::OpenGl => "egl",
        Key::DllOverrides => "d3d12=",
        Key::ArgumentsOverride | Key::Arguments => "-x \"two words\"",
        Key::DebugChannels => "+relay",
        Key::Env => "A=1;2",
        _ => "true",
    }
}

#[test]
fn allowed_on_any_is_what_a_rule_for_every_process_takes() -> TestResult {
    let allowed: Vec<Key> = Key::ALL
        .iter()
        .copied()
        .filter(|key| key.allowed_on_any())
        .collect();
    assert_eq!(
        allowed,
        [
            Key::Dxgi,
            Key::D3d9,
            Key::Wined3d,
            Key::OpenGl,
            Key::DpiAware,
            Key::LargeAddressAware,
            Key::Gcmouse,
            Key::ForceNx,
            Key::MetalHud,
            Key::DllOverrides,
            Key::Arguments,
            Key::DebugChannels,
            Key::Env,
        ]
    );
    for &key in Key::ALL {
        if key.kind() == KeyKind::Matcher {
            continue;
        }
        // `set`, `set_list` and `set_env` agree with it, with or without a
        // fingerprint on the `*` rule.
        for any in [
            Rule::new(Exe::Any),
            Rule::new(Exe::Any).with_company(Fingerprint::new("Acme")?),
        ] {
            let refused = Err(FieldError::ArgumentsOverrideOnAny);
            let set = any.clone().set(key, sample(key));
            assert_eq!(set.is_ok(), key.allowed_on_any(), "{key}: {set:?}");
            if !key.allowed_on_any() {
                assert_eq!(set, refused, "{key}");
            }
            if key.kind() == KeyKind::List || key.kind() == KeyKind::Env {
                let list = any.clone().set_list(key, [] as [&str; 0]);
                assert_eq!(list.is_ok(), key.allowed_on_any(), "{key}: {list:?}");
            }
            // A rule that holds the setting cannot become a `*` rule
            // unless the setting is allowed there.
            let mut rule = game()?;
            rule.set(key, sample(key))?;
            assert_eq!(
                rule.set(Key::Exe, Exe::ANY).is_ok(),
                key.allowed_on_any(),
                "{key}"
            );
        }
    }
    // A `*` rule still takes its matchers: they decide what it matches.
    let mut any = Rule::new(Exe::Any);
    for key in [Key::Company, Key::Product, Key::OriginalFilename, Key::Exe] {
        assert!(!key.allowed_on_any(), "{key}");
        any.set(key, sample(key))?;
    }
    Ok(())
}

#[test]
fn only_arguments_override_among_the_lists_replaces_on_merge() {
    for &key in Key::ALL {
        let expected = match key.kind() {
            KeyKind::Matcher | KeyKind::Scalar(_) => true,
            KeyKind::List => key == Key::ArgumentsOverride,
            KeyKind::Env => false,
        };
        assert_eq!(key.replaces_on_merge(), expected, "{key}");
    }
    let replaced: Vec<Key> = Key::ALL
        .iter()
        .copied()
        .filter(|key| key.kind() == KeyKind::List && key.replaces_on_merge())
        .collect();
    assert_eq!(replaced, [Key::ArgumentsOverride]);
}

#[test]
fn check_env_name_gives_the_errors_of_set_env() -> TestResult {
    for name in [
        "WINE_MAC_OPENGL",
        "WineDebug",
        "WINEDLLOVERRIDES",
        "",
        "A=B",
        "A\0",
        "VKD3D_CONFIG",
        "MTL_HUD_ENABLED",
    ] {
        let mut rule = game()?;
        assert_eq!(check_env_name(name), rule.set_env(name, "1"), "{name:?}");
    }
    assert_eq!(
        check_env_name("WINE_MAC_OPENGL"),
        Err(FieldError::RetiredEnv {
            name: "WINE_MAC_OPENGL".into(),
            key: Key::OpenGl
        })
    );
    assert_eq!(
        check_env_name("A=B"),
        Err(FieldError::InvalidEnvName("A=B".into()))
    );
    assert_eq!(
        check_env_name(""),
        Err(FieldError::InvalidEnvName(String::new()))
    );
    assert_eq!(check_env_name("A\0"), Err(FieldError::Nul));
    assert_eq!(check_env_name("VKD3D_CONFIG"), Ok(()));
    Ok(())
}

#[test]
fn every_example_is_a_value_set_takes() -> TestResult {
    let mut rule = game()?;
    for &key in Key::ALL {
        rule.set(key, key.example())?;
    }
    let (name, value) = Key::Env.example().split_once('=').ok_or("no =")?;
    check_env_name(name)?;
    assert_ne!(value, "");
    Ok(())
}

#[test]
fn the_retired_lists_are_what_the_settings_layer_refuses() -> TestResult {
    let keys: Vec<_> = retired_keys().collect();
    assert_eq!(
        keys,
        [
            ("x87_sidecar", Severity::Warning),
            ("cmdline_append", Severity::Error)
        ]
    );
    for (name, severity) in keys {
        let error = Key::from_setting(name);
        assert!(
            matches!(error, Err(FieldError::RetiredKey { .. })),
            "{name}"
        );
        assert_eq!(error.map_err(|e| e.severity()), Err(severity), "{name}");
    }
    let values: Vec<_> = retired_values().collect();
    assert_eq!(values, [(Key::Dxgi, "gptk", Severity::Warning)]);
    for (key, value, severity) in values {
        let error = game()?.set(key, value);
        assert!(
            matches!(error, Err(FieldError::RetiredValue { .. })),
            "{key} {value}"
        );
        assert_eq!(error.map_err(|e| e.severity()), Err(severity), "{value}");
    }
    let envs: Vec<_> = retired_envs().collect();
    assert_eq!(
        envs,
        [
            ("WINEDEBUG", Key::DebugChannels),
            ("WINEDLLOVERRIDES", Key::DllOverrides),
            ("WINE_CMDLINE_APPEND", Key::Arguments),
            ("WINE_MAC_OPENGL", Key::OpenGl),
            ("WINE_FORCE_NX", Key::ForceNx),
        ]
    );
    for (name, key) in envs {
        assert_eq!(retired_env(name), Some(key), "{name}");
        assert_eq!(
            check_env_name(name),
            Err(FieldError::RetiredEnv {
                name: name.into(),
                key
            })
        );
    }
    Ok(())
}

#[test]
fn a_rule_the_settings_layer_builds_travels_as_it_is() -> TestResult {
    // Every key, the way a launcher sets it, then through the wire.
    let mut rule = game()?;
    for &key in Key::ALL {
        rule.set(key, sample(key))?;
    }
    let mut table = Table::new();
    table.push(rule);
    assert_eq!(Table::parse_strict(&table.to_string())?, table);
    Ok(())
}
