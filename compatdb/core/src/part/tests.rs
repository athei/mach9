use super::*;

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn an_exe_is_any_or_a_name_and_never_empty() -> TestResult {
    assert_eq!(Exe::new("*")?, Exe::Any);
    let name = Exe::new("Game.EXE")?;
    assert_eq!(name.as_str(), "Game.EXE");
    assert!(name.matches("game.exe"));
    assert!(!name.matches("other.exe"));
    assert!(Exe::Any.matches("anything.exe"));
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
