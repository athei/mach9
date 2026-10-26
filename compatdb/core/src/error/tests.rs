use super::*;

/// A diagnostic for line 3 about `field`.
fn at(field: &str, error: FieldError) -> Diagnostic {
    Diagnostic {
        line: 3,
        field: Some(field.into()),
        error,
    }
}

#[test]
fn a_diagnostic_says_what_lenient_parsing_did() {
    let cases = [
        (
            at("bogus=1", FieldError::UnknownKey("bogus".into())),
            "WINE_COMPATDB line 3: ignoring unknown key \"bogus\"",
        ),
        (
            at(
                "dxgi=x",
                FieldError::BadValue {
                    key: Key::Dxgi,
                    value: "x".into(),
                    expected: Key::Dxgi.values(),
                },
            ),
            "WINE_COMPATDB line 3: ignoring unknown dxgi value \"x\"",
        ),
        (
            at("bogus", FieldError::MissingEquals),
            "WINE_COMPATDB line 3: ignoring field \"bogus\" without '='",
        ),
        (
            Diagnostic {
                line: 3,
                field: None,
                error: FieldError::MissingExe,
            },
            "WINE_COMPATDB line 3: ignoring the line, it has no exe",
        ),
        (
            at("gcmouse=true", FieldError::Duplicate(Key::Gcmouse)),
            "WINE_COMPATDB line 3: gcmouse given more than once, the last one applies",
        ),
        (
            at("arguments=50%", FieldError::BadEscape),
            "WINE_COMPATDB line 3: field \"arguments=50%\" has a '%' without two hex digits \
             after it, which is kept as written",
        ),
        (
            at("arguments=%FF", FieldError::InvalidUtf8),
            "WINE_COMPATDB line 3: ignoring field \"arguments=%FF\": escapes that are not UTF-8",
        ),
        (
            at("company=%00", FieldError::Nul),
            "WINE_COMPATDB line 3: ignoring field \"company=%00\": a NUL in its value",
        ),
        (
            at("arguments=", FieldError::EmptyArgument),
            "WINE_COMPATDB line 3: ignoring field \"arguments=\": an empty argument",
        ),
        (
            at("debug_channels=", FieldError::EmptyDebugChannel),
            "WINE_COMPATDB line 3: ignoring field \"debug_channels=\": an empty debug channel",
        ),
        (
            at("env==1", FieldError::InvalidEnvName(String::new())),
            "WINE_COMPATDB line 3: ignoring invalid variable name \"\"",
        ),
        (
            at(
                "dll_overrides=a",
                FieldError::InvalidDllOverride("a".into()),
            ),
            "WINE_COMPATDB line 3: ignoring DLL override \"a\" without '=' before its load order",
        ),
    ];
    for (diagnostic, said) in cases {
        assert_eq!(diagnostic.to_string(), said);
    }
}

#[test]
fn a_parse_error_lists_every_diagnostic() {
    let error = ParseError(vec![
        at("bogus", FieldError::MissingEquals),
        Diagnostic {
            line: 4,
            field: None,
            error: FieldError::MissingExe,
        },
    ]);
    assert_eq!(
        error.to_string(),
        "WINE_COMPATDB line 3: ignoring field \"bogus\" without '='\n\
         WINE_COMPATDB line 4: ignoring the line, it has no exe"
    );
    let lines: Vec<usize> = error.diagnostics().iter().map(|d| d.line).collect();
    assert_eq!(lines, [3, 4]);
    // A diagnostic is an error whose source is its field's.
    let diagnostic = at("bogus", FieldError::MissingEquals);
    let source = std::error::Error::source(&diagnostic).map(ToString::to_string);
    assert_eq!(source.as_deref(), Some("a field without '='"));
    assert_eq!(FieldError::MissingEquals.to_string(), "a field without '='");
    assert_eq!(
        FieldError::Duplicate(Key::Exe).to_string(),
        "exe given more than once"
    );
}
