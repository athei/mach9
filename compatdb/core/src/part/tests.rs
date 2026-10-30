use super::*;

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn an_exe_is_any_or_a_name_and_never_empty() -> TestResult {
    assert_eq!(Exe::new("*")?, Exe::Any);
    let name = Exe::new("Game.EXE")?;
    assert_eq!(name.as_str(), "Game.EXE");
    assert!(name.matches("game.exe", None));
    assert!(!name.matches("other.exe", None));
    assert!(Exe::Any.matches("anything.exe", None));
    assert_eq!(Exe::new(""), Err(FieldError::MissingExe));
    assert_eq!(Exe::new("a\0b"), Err(FieldError::Nul));
    assert_eq!("x.exe".parse::<Exe>()?.to_string(), "x.exe");
    assert_eq!(Exe::Any.to_string(), "*");
    // `**` and ` *` are names like any other.
    assert!(!Exe::new("**")?.is_any());
    assert!(!Exe::new(" *")?.is_any());
    Ok(())
}

#[test]
fn a_steam_exe_reads_in_any_case_and_writes_its_canonical_text() -> TestResult {
    for text in ["steam:440", "STEAM:440", "Steam:440", "sTeAm:0440"] {
        let exe = Exe::new(text)?;
        assert_eq!(exe.as_str(), "steam:440", "{text}");
        assert_eq!(exe.to_string(), "steam:440", "{text}");
        assert_eq!(exe, "steam:440", "{text}");
        assert!(exe.is_steam_app(), "{text}");
        assert!(!exe.is_any(), "{text}");
        assert!(matches!(&exe, Exe::Steam(app) if app.id() == 440), "{text}");
    }
    let last = "steam:4294967295";
    assert!(matches!(Exe::new(last)?, Exe::Steam(app) if app.id() == u32::MAX));
    assert_eq!("STEAM:1".parse::<Exe>()?.to_string(), "steam:1");
    Ok(())
}

#[test]
fn a_steam_exe_needs_an_app_id_and_keeps_the_text_of_the_error() {
    for bad in [
        "steam:",
        "steam:0",
        "STEAM:000",
        "steam:abc",
        "steam:12a",
        "steam:a12",
        "steam:+1",
        "steam:-1",
        "steam: 1",
        "steam:1 ",
        "steam:1.0",
        "steam:٣",
        "steam:4294967296",
        "steam:99999999999999999999",
    ] {
        assert_eq!(
            Exe::new(bad),
            Err(FieldError::BadSteamApp(bad.into())),
            "{bad}"
        );
    }
    assert_eq!(Exe::new("steam:\0"), Err(FieldError::Nul));
    assert_eq!(
        FieldError::BadSteamApp("steam:0".into()).to_string(),
        "invalid Steam app id \"steam:0\""
    );
}

#[test]
fn only_a_steam_prefix_makes_a_steam_exe() -> TestResult {
    // Names like any other, in the case they were written in: no `:`, or
    // another word in front of it.
    for name in ["steam", "Steam.exe", "steam.exe", "steamer:1", "c:steam:1"] {
        let exe = Exe::new(name)?;
        assert!(!exe.is_steam_app(), "{name}");
        assert_eq!(exe.as_str(), name);
    }
    Ok(())
}

#[test]
fn a_steam_exe_matches_its_app_id_and_not_a_basename() -> TestResult {
    let exe = Exe::new("steam:440")?;
    assert!(exe.matches("hl2.exe", Some(440)));
    assert!(exe.matches("steam:440", Some(440)));
    assert!(!exe.matches("hl2.exe", Some(441)));
    assert!(!exe.matches("hl2.exe", None));
    assert!(!exe.matches("steam:440", None));
    // The other kinds of `exe` do not look at the id.
    assert!(Exe::Any.matches("hl2.exe", Some(440)));
    assert!(Exe::new("hl2.exe")?.matches("hl2.exe", Some(440)));
    assert!(!Exe::new("other.exe")?.matches("hl2.exe", Some(440)));
    Ok(())
}

#[test]
fn a_fingerprint_matches_a_substring_in_any_case() -> TestResult {
    let pin = Fingerprint::new("rockstar")?;
    assert!(pin.matches("Rockstar Games"));
    assert!(!pin.matches("Valve"));
    assert!(Fingerprint::new("")?.matches(""));
    assert_eq!(Fingerprint::new("a\0"), Err(FieldError::Nul));
    Ok(())
}

#[test]
fn an_argument_is_not_empty() -> TestResult {
    assert_eq!(Argument::new("-x")?.as_str(), "-x");
    assert_eq!(Argument::new(" ")?.as_str(), " ");
    assert_eq!(Argument::new(""), Err(FieldError::EmptyArgument));
    assert_eq!(Argument::new("\0"), Err(FieldError::Nul));
    Ok(())
}

#[test]
fn a_debug_channel_is_not_empty_and_keeps_its_text() -> TestResult {
    assert_eq!(DebugChannel::new("+relay")?.as_str(), "+relay");
    assert_eq!(
        "warn-heap".parse::<DebugChannel>()?.to_string(),
        "warn-heap"
    );
    assert_eq!(DebugChannel::new(""), Err(FieldError::EmptyDebugChannel));
    assert_eq!(DebugChannel::new("+seh\0"), Err(FieldError::Nul));
    // A `,` would start an entry that an `<exe>:` scope does not reach.
    assert_eq!(
        DebugChannel::new("+relay,+seh"),
        Err(FieldError::CommaInDebugChannel("+relay,+seh".into()))
    );
    assert_eq!(
        FieldError::CommaInDebugChannel("a,b".into()).to_string(),
        "debug channel \"a,b\" with a ','"
    );
    assert_eq!(
        FieldError::EmptyDebugChannel.to_string(),
        "an empty debug channel"
    );
    Ok(())
}

#[test]
fn a_dll_override_needs_an_equals_and_keeps_its_text() -> TestResult {
    let entry = DllOverride::new(" d3d12 , d3d12core = n,b")?;
    assert_eq!(entry.to_string(), " d3d12 , d3d12core = n,b");
    assert_eq!(entry.modules().collect::<Vec<_>>(), ["d3d12", "d3d12core"]);
    assert_eq!(entry.order(), " n,b");
    assert!(!entry.is_default());
    assert!(DllOverride::new("a, b =Default ")?.is_default());
    assert_eq!(DllOverride::new("mscoree,mshtml=")?.order(), "");
    assert_eq!(DllOverride::new("a=b=c")?.order(), "b=c");
    assert_eq!(
        DllOverride::new("mscoree"),
        Err(FieldError::InvalidDllOverride("mscoree".into()))
    );
    assert_eq!(DllOverride::new("a=\0"), Err(FieldError::Nul));
    Ok(())
}

#[test]
fn an_env_name_is_not_empty_and_has_no_equals() -> TestResult {
    assert_eq!(EnvName::new("Path")?.as_str(), "Path");
    for bad in ["", "A=B", "=C:"] {
        assert_eq!(
            EnvName::new(bad),
            Err(FieldError::InvalidEnvName(bad.into())),
            "{bad:?}"
        );
    }
    assert_eq!(EnvName::new("A\0"), Err(FieldError::Nul));
    Ok(())
}

#[test]
fn an_env_entry_with_an_empty_value_removes_the_variable() -> TestResult {
    let name = EnvName::new("A")?;
    let set = EnvEntry::new(name.clone(), "1")?;
    assert_eq!(set.value(), Some("1"));
    assert_eq!(set.to_string(), "A=1");
    let removal = EnvEntry::new(name.clone(), "")?;
    assert_eq!(removal, EnvEntry::remove(name.clone()));
    assert_eq!(removal.value(), None);
    assert_eq!(removal.to_string(), "A=");
    assert_eq!(EnvEntry::new(name, "\0"), Err(FieldError::Nul));
    Ok(())
}

#[test]
fn specificity_orders_any_then_exe_and_a_fingerprint_after_each() {
    assert!(Specificity::Any < Specificity::AnyWithFingerprint);
    assert!(Specificity::AnyWithFingerprint < Specificity::Exe);
    assert!(Specificity::Exe < Specificity::ExeWithFingerprint);
}

#[test]
fn the_new_errors_say_what_is_wrong() {
    assert_eq!(FieldError::Nul.to_string(), "a NUL in its value");
    assert_eq!(FieldError::EmptyArgument.to_string(), "an empty argument");
    assert_eq!(
        FieldError::InvalidEnvName("A=B".into()).to_string(),
        "invalid variable name \"A=B\""
    );
    assert_eq!(
        FieldError::InvalidDllOverride("x".into()).to_string(),
        "DLL override \"x\" without '=' before its load order"
    );
}
