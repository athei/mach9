use super::*;

#[test]
fn every_key_reads_back_from_its_spelling_in_any_case() {
    for &key in Key::ALL {
        let text = key.to_string();
        assert_eq!(text, key.as_str());
        assert_eq!(text, text.to_ascii_lowercase(), "{key}");
        assert_eq!(text.parse(), Ok(key));
        assert_eq!(text.to_ascii_uppercase().parse(), Ok(key));
    }
    assert_eq!("Original_FileName".parse(), Ok(Key::OriginalFilename));
    // The keys of the old format, and those of other Wines, are unknown.
    for unknown in [
        "name",
        "enabled",
        "x87_sidecar",
        "cmdline_append",
        "",
        "exe ",
    ] {
        assert_eq!(
            unknown.parse::<Key>(),
            Err(FieldError::UnknownKey(unknown.into()))
        );
    }
}

#[test]
fn every_key_is_listed_once_and_every_scalar_has_its_key() {
    for (i, key) in Key::ALL.iter().enumerate() {
        assert!(!Key::ALL.iter().skip(i + 1).any(|k| k == key), "{key}");
    }
    let scalars: Vec<Scalar> = Key::ALL.iter().filter_map(|key| key.scalar()).collect();
    assert_eq!(scalars, Scalar::ALL);
    for &scalar in Scalar::ALL {
        assert_eq!(scalar.key().scalar(), Some(scalar));
        assert_eq!(scalar.to_string(), scalar.key().as_str());
        assert_eq!(
            scalar.key().kind(),
            KeyKind::Scalar(scalar.value_kind()),
            "{scalar}"
        );
        assert_eq!(scalar.key().upstream(), Some(scalar.upstream()));
    }
}

#[test]
fn each_key_has_its_kind() {
    let kinds: Vec<(Key, KeyKind)> = Key::ALL.iter().map(|&key| (key, key.kind())).collect();
    let word = KeyKind::Scalar(ValueKind::Word);
    let bool = KeyKind::Scalar(ValueKind::Bool);
    assert_eq!(
        kinds,
        [
            (Key::Exe, KeyKind::Matcher),
            (Key::Company, KeyKind::Matcher),
            (Key::Product, KeyKind::Matcher),
            (Key::OriginalFilename, KeyKind::Matcher),
            (Key::Dxgi, word),
            (Key::D3d9, word),
            (Key::Wined3d, word),
            (Key::OpenGl, word),
            (Key::DpiAware, bool),
            (Key::LargeAddressAware, bool),
            (Key::Gcmouse, bool),
            (Key::ForceNx, bool),
            (Key::DllOverrides, KeyKind::List),
            (Key::ArgumentsOverride, KeyKind::List),
            (Key::Arguments, KeyKind::List),
            (Key::Env, KeyKind::Env),
        ]
    );
}

#[test]
fn the_values_of_a_scalar_are_its_spellings_then_default() {
    fn names<T: fmt::Display>(all: &[T]) -> Vec<String> {
        let mut names: Vec<String> = all.iter().map(ToString::to_string).collect();
        names.push(DEFAULT_VALUE.into());
        names
    }
    assert_eq!(Key::Dxgi.values(), names(Dxgi::ALL));
    assert_eq!(Key::D3d9.values(), names(D3d9::ALL));
    assert_eq!(Key::Wined3d.values(), names(Wined3d::ALL));
    assert_eq!(Key::OpenGl.values(), names(OpenGl::ALL));
    for &key in Key::ALL {
        match key.kind() {
            KeyKind::Scalar(ValueKind::Bool) => {
                assert_eq!(key.values(), ["true", "false", "default"], "{key}");
            }
            KeyKind::Scalar(ValueKind::Word) => {
                assert_eq!(key.values().last(), Some(&DEFAULT_VALUE), "{key}");
            }
            KeyKind::Matcher | KeyKind::List | KeyKind::Env => {
                assert_eq!(key.values(), [] as [&str; 0], "{key}");
            }
        }
    }
}

#[test]
fn a_scalar_reads_its_values_in_any_case_and_names_the_ones_it_takes() {
    assert_eq!(
        Scalar::Dxgi.parse_value("DXMT"),
        Ok(ScalarValue::Dxgi(Dxgi::Dxmt))
    );
    assert_eq!(
        Scalar::Wined3d.parse_value("Vulkan"),
        Ok(ScalarValue::Wined3d(Wined3d::Vulkan))
    );
    for (text, on) in [("true", true), ("TRUE", true), ("False", false)] {
        assert_eq!(
            Scalar::DpiAware.parse_value(text),
            Ok(ScalarValue::Bool(on)),
            "{text}"
        );
    }
    // `default` is no value, and neither is anything but the words.
    for bad in ["default", "yes", "1", ""] {
        assert_eq!(
            Scalar::Gcmouse.parse_value(bad),
            Err(FieldError::BadValue {
                key: Key::Gcmouse,
                value: bad.into(),
                expected: &["true", "false", "default"],
            }),
            "{bad}"
        );
    }
    assert_eq!(
        Scalar::OpenGl
            .parse_value("Zink")
            .map_err(|error| error.to_string()),
        Err("unknown opengl value \"Zink\"".to_string())
    );
    assert_eq!(
        FieldError::UnknownKey("Bogus".into()).to_string(),
        "unknown key \"Bogus\""
    );
}

#[test]
fn a_scalar_value_prints_in_lowercase() {
    for (value, text) in [
        (ScalarValue::Dxgi(Dxgi::Dxmt), "dxmt"),
        (ScalarValue::D3d9(D3d9::Mtld3d), "mtld3d"),
        (ScalarValue::Wined3d(Wined3d::Gl), "gl"),
        (ScalarValue::OpenGl(OpenGl::Egl), "egl"),
        (ScalarValue::Bool(true), "true"),
        (ScalarValue::Bool(false), "false"),
    ] {
        assert_eq!(value.to_string(), text);
    }
}

#[test]
fn the_labels_are_the_ones_a_launcher_shows() {
    let labels: Vec<&str> = Scalar::ALL.iter().map(|s| s.key().label()).collect();
    assert_eq!(
        labels,
        [
            "DXGI",
            "D3D9",
            "wined3d renderer",
            "OpenGL",
            "DPI-aware",
            "Large-address-aware",
            "GameController mouse",
            "Force no-execute",
        ]
    );
    for &key in Key::ALL {
        assert!(!key.label().is_empty(), "{key}");
    }
}
