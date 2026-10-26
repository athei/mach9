use super::*;
use crate::{D3d9, DebugChannel, Dxgi, KeyKind, OpenGl, Origin, VersionInfo, Wined3d};

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The table of `rules` as a launcher builds it: each pushed, so it has
/// its line of the value the table writes as its source.
fn sent(rules: Vec<Rule>) -> Table {
    let mut table = Table::new();
    for rule in rules {
        table.push(rule);
    }
    table
}

/// [`Table::parse_lenient`], with the diagnostics as `compatdb.so` logs
/// them.
fn parse(text: &str) -> (Table, Vec<String>) {
    let (table, diagnostics) = Table::parse_lenient(text);
    (table, diagnostics.iter().map(ToString::to_string).collect())
}

/// `raw` escaped as a value.
fn encode(raw: &str) -> String {
    let mut out = String::new();
    encode_into(raw, &mut out);
    out
}

/// `encoded` decoded, and whether it had a `%` that is no escape.
fn decode(encoded: &str) -> (Result<String, FieldError>, bool) {
    let mut decoder = Decoder::default();
    let text = decoder.decode(encoded);
    (text, decoder.bad_escape)
}

#[test]
fn encode_decode_round_trips_specials() {
    for raw in [
        "plain",
        "a;b",
        "a%b",
        "a=b,c d",
        "line\nfeed\r",
        "tab\there\u{7f}",
        "café ünïcode 😀",
        "",
    ] {
        assert_eq!(
            decode(&encode(raw)),
            (Ok(raw.to_owned()), false),
            "round trip of {raw:?}"
        );
    }
}

#[test]
fn encode_escapes_only_the_dangerous_characters() {
    assert_eq!(encode("d3d12,vulkan-1="), "d3d12,vulkan-1=");
    assert_eq!(encode("a;b"), "a%3Bb");
    assert_eq!(encode("100%"), "100%25");
    assert_eq!(encode("a\r\n\tb\u{1}"), "a%0D%0A%09b%01");
    assert_eq!(encode("é😀"), "é😀");
    assert_eq!(encode("A=B"), "A=B");
}

#[test]
fn decode_reads_escapes_as_utf_8_bytes() {
    // Each escape is one byte, so a character takes as many as its UTF-8
    // form has, and lowercase hex reads like uppercase.
    assert_eq!(decode("caf%C3%A9"), (Ok("café".into()), false));
    assert_eq!(decode("%f0%9f%98%80"), (Ok("😀".into()), false));
    assert_eq!(decode("%3b%3B"), (Ok(";;".into()), false));
    // Bytes that are no UTF-8, and a NUL, cannot be read.
    assert_eq!(decode("%FF").0, Err(FieldError::InvalidUtf8));
    assert_eq!(decode("%C3").0, Err(FieldError::InvalidUtf8));
    assert_eq!(decode("a%00b").0, Err(FieldError::Nul));
}

#[test]
fn decode_keeps_a_percent_that_starts_no_escape() {
    for text in ["50%", "%zz", "%2", "%+1", "%-1", "% 1"] {
        assert_eq!(decode(text), (Ok(text.into()), true), "{text}");
    }
}

#[test]
fn table_round_trips_through_the_env_value() {
    let table = sent(vec![
        Rule {
            exe: Exe::literal("Launcher.exe"),
            company: Some(Fingerprint::literal("Rockstar Games")),
            product: Some(Fingerprint::literal("Rockstar Games Launcher")),
            dxgi: Setting::Set(Dxgi::Wined3d),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("SocialClubHelper.exe"),
            arguments: vec![Argument::literal("--in-process-gpu --disable-gpu")],
            original_filename: Some(Fingerprint::literal("SocialClubHelper.exe")),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("WoW.exe"),
            d3d9: Setting::Set(D3d9::Wined3d),
            dll_overrides: vec![DllOverride::literal("d3d12,vulkan-1=")],
            env: vec![EnvEntry::literal("DXMT_CONFIG", "d3d11.maxFrameRate=60")],
            ..Rule::default()
        },
    ]);
    let (parsed, diagnostics) = parse(&table.to_string());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, table);
}

#[test]
fn env_field_survives_an_equals_in_the_value() {
    let table = sent(vec![Rule {
        exe: Exe::literal("x.exe"),
        env: vec![EnvEntry::literal("A", "b=c;d")],
        ..Rule::default()
    }]);
    let (parsed, _) = parse(&table.to_string());
    assert_eq!(parsed, table);
}

#[test]
fn a_legacy_header_on_the_first_line_is_skipped() {
    // What a launcher written for the versioned format sends: the same
    // records mean the same with any version header in front, or none,
    // and the header counts as a line.
    let records = "exe=Launcher.exe;dxgi=dxmt\n\
                   exe=WoW.exe;company=Blizzard;d3d9=wined3d;wined3d=vulkan;env=A=1";
    let (bare, diagnostics) = parse(records);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(bare.rules.len(), 2);
    for header in ["v=5", "v=6", "v=7", "v=10"] {
        let (table, diagnostics) = parse(&format!("{header}\n{records}"));
        assert!(diagnostics.is_empty(), "{header}: {diagnostics:?}");
        assert_eq!(
            table.sources(),
            [Source::Line(2), Source::Line(3)],
            "{header}"
        );
        assert_eq!(table.rules(), bare.rules(), "{header}");
    }
}

#[test]
fn only_a_first_line_of_v_and_digits_is_a_legacy_header() {
    // The `v` is a key, so its case does not matter.
    for header in ["v=6", "v=0", "v=123", "V=6"] {
        assert!(is_legacy_header(header), "{header}");
    }
    for other in ["v=", "v=6a", "v=-1", " v=6", "v=6;exe=x", "exe=x;v=6"] {
        assert!(!is_legacy_header(other), "{other}");
    }
    // Anything else on the first line is a record: one with an exe is a
    // rule, and one without, a legacy header that looks almost right
    // included, is ignored with a diagnostic.
    let (table, diagnostics) = parse("exe=x.exe\nexe=y.exe");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(table.rules.len(), 2);
    let (table, diagnostics) = parse("v=6a\nexe=x.exe");
    assert_eq!(table.rules.len(), 1);
    assert_eq!(
        diagnostics,
        [
            "WINE_COMPATDB line 1: ignoring unknown key \"v\"",
            "WINE_COMPATDB line 1: ignoring the line, it has no exe"
        ]
    );
    // Only the first line can be one; a later one is a line without an
    // exe.
    let (table, diagnostics) = parse("exe=x.exe\nv=6");
    assert_eq!(table.rules.len(), 1);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
}

#[test]
fn the_env_value_is_the_records_alone() {
    let table = sent(vec![
        Rule {
            exe: Exe::literal("game.exe"),
            dxgi: Setting::Set(Dxgi::Wined3d),
            wined3d: Setting::Set(Wined3d::Vulkan),
            env: vec![EnvEntry::literal("A", "1")],
            ..Rule::default()
        },
        // The source does not travel: pushing makes it the line.
        Rule {
            exe: Exe::Any,
            gcmouse: Setting::Set(false),
            ..Rule::default()
        },
    ]);
    assert_eq!(table.sources(), [Source::Line(1), Source::Line(2)]);
    let value = table.to_string();
    assert_eq!(
        value,
        "exe=game.exe;dxgi=wined3d;wined3d=vulkan;env=A=1\n\
         exe=*;gcmouse=false"
    );
    let (parsed, diagnostics) = parse(&value);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, table);
    assert_eq!(parsed.to_string(), value);
    assert_eq!(Table::default().to_string(), "");
    assert_eq!(parse(""), (Table::default(), Vec::new()));
}

#[test]
fn parse_numbers_the_lines_and_counts_blank_ones() {
    let (table, diagnostics) = parse("exe=a.exe\n\nexe=b.exe\n  \nexe=c.exe");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
        table.sources(),
        [Source::Line(1), Source::Line(3), Source::Line(5)]
    );
}

#[test]
fn parse_ignores_a_line_without_an_exe_and_keeps_the_rest() {
    // What a launcher wrote to change a built-in rule by name in the old
    // format: it has no exe, so it matches no process.
    for (line, said) in [
        ("dxgi=dxmt", vec![]),
        ("exe=", vec![]),
        (
            "name=rockstar-launcher;dxgi=dxmt",
            vec!["WINE_COMPATDB line 1: ignoring unknown key \"name\""],
        ),
        (
            "name=dpi-aware;enabled=false",
            vec![
                "WINE_COMPATDB line 1: ignoring unknown key \"name\"",
                "WINE_COMPATDB line 1: ignoring unknown key \"enabled\"",
            ],
        ),
    ] {
        let (table, diagnostics) = parse(&format!("{line}\nexe=ok.exe"));
        assert_eq!(table.rules.len(), 1, "{line}");
        assert_eq!(table.rules.first().map(|r| r.exe.as_str()), Some("ok.exe"));
        let mut want: Vec<String> = said.into_iter().map(String::from).collect();
        want.push("WINE_COMPATDB line 1: ignoring the line, it has no exe".into());
        assert_eq!(diagnostics, want, "{line}");
    }
}

#[test]
fn the_name_and_enabled_of_the_old_format_are_unknown_keys() {
    // A launcher written for the old format names its rules and may
    // send `enabled`. Each such field is ignored with a warning, and the
    // rest of the line applies: `enabled=false` no longer drops a rule,
    // and a rule named like a built-in one is a rule of its own.
    let (table, diagnostics) = parse(
        "name=my-game;exe=Game.exe;dpi_aware=false\nname=x;enabled=false;exe=*;gcmouse=false",
    );
    assert_eq!(
        table.rules,
        [
            Rule {
                exe: Exe::literal("Game.exe"),
                dpi_aware: Setting::Set(false),
                ..Rule::default()
            },
            Rule {
                exe: Exe::Any,
                gcmouse: Setting::Set(false),
                ..Rule::default()
            },
        ]
    );
    assert_eq!(
        diagnostics,
        [
            "WINE_COMPATDB line 1: ignoring unknown key \"name\"",
            "WINE_COMPATDB line 2: ignoring unknown key \"name\"",
            "WINE_COMPATDB line 2: ignoring unknown key \"enabled\"",
        ]
    );
}

#[test]
fn a_field_that_cannot_be_read_is_ignored_and_the_rest_applies() -> TestResult {
    // An unknown key, a value its key does not take and a field without
    // '=' each cost only that field, with one diagnostic naming the line
    // and the field. The record before and after it still applies.
    let want = Rule {
        exe: Exe::literal("x.exe"),
        d3d9: Setting::Set(D3d9::Wined3d),
        env: vec![EnvEntry::literal("A", "1")],
        ..Rule::default()
    };
    for (field, said) in [
        ("bogus=1", "unknown key \"bogus\""),
        ("dxgi=vulkan", "unknown dxgi value \"vulkan\""),
        // The x86_64 Wine's D3DMetal is a value like any other unknown one.
        ("dxgi=gptk", "unknown dxgi value \"gptk\""),
        ("d3d9=gptk", "unknown d3d9 value \"gptk\""),
        ("wined3d=opengl", "unknown wined3d value \"opengl\""),
        ("dpi_aware=yes", "unknown dpi_aware value \"yes\""),
        ("bogus", "field \"bogus\" without '='"),
        // A list entry or a fingerprint value its type does not take.
        ("arguments=", "field \"arguments=\": an empty argument"),
        (
            "arguments_override=%00",
            "field \"arguments_override=%00\": a NUL in its value",
        ),
        ("company=a%00", "field \"company=a%00\": a NUL in its value"),
        (
            "product=%C3%28",
            "field \"product=%C3%28\": escapes that are not UTF-8",
        ),
        (
            "dll_overrides=mscoree",
            "DLL override \"mscoree\" without '=' before its load order",
        ),
        ("env==1", "invalid variable name \"\""),
        ("env=A%3DB=1", "invalid variable name \"A=B\""),
    ] {
        let text = format!("{field};exe=x.exe;d3d9=wined3d;env=A=1\nexe=ok.exe");
        let (table, diagnostics) = parse(&text);
        assert_eq!(table.rules.first(), Some(&want), "{field}");
        assert_eq!(
            table.rules.get(1).map(|r| r.exe.as_str()),
            Some("ok.exe"),
            "{field}"
        );
        let diagnostic = diagnostics.first().ok_or("no diagnostic")?;
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(
            diagnostic,
            &format!("WINE_COMPATDB line 1: ignoring {said}")
        );
    }
    Ok(())
}

#[test]
fn dpi_aware_round_trips_in_both_values_and_is_absent_when_unset() {
    let table = sent(vec![
        Rule {
            exe: Exe::Any,
            dpi_aware: Setting::Set(true),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("old.exe"),
            d3d9: Setting::Set(D3d9::Wined3d),
            dpi_aware: Setting::Set(false),
            dll_overrides: vec![DllOverride::literal("a=")],
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("other.exe"),
            ..Rule::default()
        },
    ]);
    let text = table.to_string();
    assert_eq!(
        text,
        "exe=*;dpi_aware=true\n\
         exe=old.exe;d3d9=wined3d;dpi_aware=false;dll_overrides=a=\n\
         exe=other.exe"
    );
    let (parsed, diagnostics) = parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, table);
}

#[test]
fn a_bad_dpi_aware_value_is_ignored() {
    for bad in ["yes", "1", "truee", ""] {
        let (table, diagnostics) = parse(&format!("exe=x.exe;dpi_aware={bad}\nexe=ok.exe"));
        assert_eq!(table.rules.len(), 2, "dpi_aware={bad:?}");
        assert_eq!(
            table.rules.first().map(|r| r.dpi_aware),
            Some(Setting::Unset)
        );
        assert_eq!(diagnostics.len(), 1, "dpi_aware={bad:?}");
    }
}

#[test]
fn large_address_aware_round_trips_and_is_absent_when_unset() {
    let table = sent(vec![
        Rule {
            exe: Exe::Any,
            large_address_aware: Setting::Set(true),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("old.exe"),
            dpi_aware: Setting::Set(false),
            large_address_aware: Setting::Set(false),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("other.exe"),
            ..Rule::default()
        },
    ]);
    let text = table.to_string();
    assert_eq!(
        text,
        "exe=*;large_address_aware=true\n\
         exe=old.exe;dpi_aware=false;large_address_aware=false\n\
         exe=other.exe"
    );
    let (parsed, diagnostics) = parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, table);
}

#[test]
fn a_bad_large_address_aware_value_is_ignored() {
    let key = "large_address_aware";
    for bad in ["yes", "1", "fals", ""] {
        let (table, diagnostics) = parse(&format!(
            "exe=x.exe;dpi_aware=false;{key}={bad}\nexe=ok.exe"
        ));
        assert_eq!(table.rules.len(), 2, "{key}={bad:?}");
        let x = table.rules.first();
        assert_eq!(x.map(|r| r.large_address_aware), Some(Setting::Unset));
        assert_eq!(x.map(|r| r.dpi_aware), Some(Setting::Set(false)));
        assert_eq!(diagnostics.len(), 1, "{key}={bad:?}");
        assert!(
            diagnostics.first().is_some_and(|d| d.contains(key)),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn gcmouse_round_trips_and_is_absent_when_unset() {
    let table = sent(vec![
        Rule {
            exe: Exe::Any,
            gcmouse: Setting::Set(true),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("old.exe"),
            large_address_aware: Setting::Set(false),
            gcmouse: Setting::Set(false),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("other.exe"),
            ..Rule::default()
        },
    ]);
    let text = table.to_string();
    assert_eq!(
        text,
        "exe=*;gcmouse=true\n\
         exe=old.exe;large_address_aware=false;gcmouse=false\n\
         exe=other.exe"
    );
    let (parsed, diagnostics) = parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, table);
}

#[test]
fn a_bad_gcmouse_value_is_ignored() {
    let key = "gcmouse";
    for bad in ["yes", "1", "fals", ""] {
        let (table, diagnostics) = parse(&format!(
            "exe=x.exe;dpi_aware=false;{key}={bad}\nexe=ok.exe"
        ));
        assert_eq!(table.rules.len(), 2, "{key}={bad:?}");
        let x = table.rules.first();
        assert_eq!(x.map(|r| r.gcmouse), Some(Setting::Unset));
        assert_eq!(x.map(|r| r.dpi_aware), Some(Setting::Set(false)));
        assert_eq!(diagnostics.len(), 1, "{key}={bad:?}");
        assert!(
            diagnostics.first().is_some_and(|d| d.contains(key)),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn force_nx_round_trips_and_is_absent_when_unset() {
    let table = sent(vec![
        Rule {
            exe: Exe::Any,
            force_nx: Setting::Set(true),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("old.exe"),
            gcmouse: Setting::Set(false),
            force_nx: Setting::Set(false),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("other.exe"),
            ..Rule::default()
        },
    ]);
    let text = table.to_string();
    assert_eq!(
        text,
        "exe=*;force_nx=true\n\
         exe=old.exe;gcmouse=false;force_nx=false\n\
         exe=other.exe"
    );
    let (parsed, diagnostics) = parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, table);
}

#[test]
fn a_bad_force_nx_value_is_ignored() {
    let key = "force_nx";
    for bad in ["yes", "1", "fals", ""] {
        let (table, diagnostics) = parse(&format!(
            "exe=x.exe;dpi_aware=false;{key}={bad}\nexe=ok.exe"
        ));
        assert_eq!(table.rules.len(), 2, "{key}={bad:?}");
        let x = table.rules.first();
        assert_eq!(x.map(|r| r.force_nx), Some(Setting::Unset));
        assert_eq!(x.map(|r| r.dpi_aware), Some(Setting::Set(false)));
        assert_eq!(diagnostics.len(), 1, "{key}={bad:?}");
        assert!(
            diagnostics.first().is_some_and(|d| d.contains(key)),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn a_reset_round_trips_through_the_env_value() {
    let table = sent(vec![Rule {
        exe: Exe::literal("a.exe"),
        dxgi: Setting::Set(Dxgi::Wined3d),
        opengl: Setting::Reset,
        force_nx: Setting::Reset,
        dll_overrides: vec![DllOverride::literal("mscoree=default")],
        ..Rule::default()
    }]);
    let text = table.to_string();
    assert_eq!(
        text,
        "exe=a.exe;dxgi=wined3d;opengl=default;force_nx=default;dll_overrides=mscoree=default"
    );
    let (parsed, diagnostics) = parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, table);
    let r = table.resolve("a.exe", &VersionInfo::default());
    assert_eq!(r.dxgi(), Some(Dxgi::Wined3d));
    assert_eq!(r.scalar(Scalar::Dxgi).reset_by, None);
    assert_eq!(r.scalar(Scalar::OpenGl).reset_by, Some(Origin::Line(1)));
}

#[test]
fn the_x86_64_wines_x87_sidecar_key_is_an_unknown_key() {
    // A launcher written for that Wine may still send it. Like any key
    // the library does not know, it costs only its own field, with a
    // warning, and the rest of the record still applies.
    let (table, diagnostics) = parse("exe=old.exe;x87_sidecar=false;large_address_aware=false");
    assert_eq!(
        table.rules.first().map(|r| r.large_address_aware),
        Some(Setting::Set(false))
    );
    assert_eq!(
        diagnostics,
        ["WINE_COMPATDB line 1: ignoring unknown key \"x87_sidecar\"".to_string()]
    );
}

#[test]
fn every_field_a_record_can_carry_is_a_key_in_the_order_of_key_all() -> TestResult {
    // The literal names every field, so a new one does not compile until
    // it is added here, and with it its key.
    let rule = Rule {
        exe: Exe::literal("all.exe"),
        company: Some(Fingerprint::literal("c")),
        product: Some(Fingerprint::literal("p")),
        original_filename: Some(Fingerprint::literal("o")),
        dxgi: Setting::Set(Dxgi::Dxmt),
        d3d9: Setting::Set(D3d9::Mtld3d),
        wined3d: Setting::Set(Wined3d::Gl),
        opengl: Setting::Set(OpenGl::Egl),
        dpi_aware: Setting::Set(true),
        large_address_aware: Setting::Set(true),
        gcmouse: Setting::Set(true),
        force_nx: Setting::Set(true),
        metal_hud: Setting::Set(true),
        dll_overrides: vec![DllOverride::literal("a=")],
        arguments: vec![Argument::literal("-a")],
        arguments_override: Some(vec![Argument::literal("-b")]),
        debug_channels: vec![DebugChannel::new("+relay")?],
        env: vec![EnvEntry::literal("A", "1")],
    };
    let record = rule_to_record(&rule);
    let mut keys = Vec::new();
    for field in record.split(';') {
        let (key, _) = field.split_once('=').ok_or("a field without '='")?;
        keys.push(key.parse::<Key>()?);
    }
    assert_eq!(keys, Key::ALL);
    // Every setting says what upstream Wine does without it; a matcher
    // is no setting.
    for key in &keys {
        assert_eq!(
            key.upstream().is_none(),
            key.kind() == KeyKind::Matcher,
            "{key}"
        );
        assert_eq!(key.to_string(), key.as_str());
    }
    // The scalar keys are the settings that are not lists, in order.
    let scalars: Vec<Scalar> = keys.iter().filter_map(|key| key.scalar()).collect();
    assert_eq!(scalars, Scalar::ALL);
    Ok(())
}

#[test]
fn metal_hud_and_debug_channels_round_trip() -> TestResult {
    let table = sent(vec![
        Rule::new(Exe::new("Game.exe")?)
            .with_metal_hud(true)
            .with_argument(Argument::new("-x")?)
            .with_debug_channel(DebugChannel::new("+relay")?)
            .with_debug_channel(DebugChannel::new("warn+heap;x%")?)
            .with_env(EnvEntry::new(EnvName::new("A")?, "1")?),
        Rule::new(Exe::Any).with_metal_hud(false),
        Rule::new(Exe::new("Tool.exe")?).with_metal_hud(Setting::Reset),
    ]);
    let text = table.to_string();
    assert_eq!(
        text,
        "exe=Game.exe;metal_hud=true;arguments=-x;debug_channels=+relay;\
         debug_channels=warn+heap%3Bx%25;env=A=1\n\
         exe=*;metal_hud=false\n\
         exe=Tool.exe;metal_hud=default"
    );
    assert_eq!(Table::parse_strict(&text)?, table);
    // Read in any case, like every word of the format.
    let (parsed, diagnostics) = parse("exe=a.exe;METAL_HUD=True;Debug_Channels=+seh");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let rule = parsed.rules.first().ok_or("no rule")?;
    assert_eq!(rule.metal_hud(), Setting::Set(true));
    assert_eq!(rule.debug_channels(), ["+seh"]);
    // A bad value or an empty channel costs only its field.
    let (parsed, diagnostics) =
        parse("exe=a.exe;metal_hud=1;debug_channels=;debug_channels=+relay,+seh;dpi_aware=false");
    assert_eq!(
        diagnostics,
        [
            "WINE_COMPATDB line 1: ignoring unknown metal_hud value \"1\"",
            "WINE_COMPATDB line 1: ignoring field \"debug_channels=\": an empty debug channel",
            "WINE_COMPATDB line 1: ignoring debug channel \"+relay,+seh\" with a ','",
        ]
    );
    let rule = parsed.rules.first().ok_or("no rule")?;
    assert_eq!(rule.metal_hud(), Setting::Unset);
    assert_eq!(rule.debug_channels(), [] as [&str; 0]);
    assert_eq!(rule.dpi_aware(), Setting::Set(false));
    Ok(())
}

#[test]
fn every_word_of_the_format_is_read_in_any_case_and_written_in_lowercase() -> TestResult {
    // Keys, values, `true`/`false` and `default` alike.
    let (table, diagnostics) = parse(
        "EXE=Game.EXE;Company=ACME;DXGI=WineD3D;D3D9=MtlD3D;Wined3d=VULKAN;OpenGL=CGL;\
         DPI_Aware=TRUE;Large_Address_Aware=False;GCMOUSE=Default;Force_NX=DEFAULT;\
         DLL_Overrides=D3D12=;Arguments=-X;Env=Name=Value\n\
         exe=x.exe;opengl=EGL",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let rule = table.rules.first().ok_or("no rule")?;
    assert_eq!(rule.dxgi, Setting::Set(Dxgi::Wined3d));
    assert_eq!(rule.d3d9, Setting::Set(D3d9::Mtld3d));
    assert_eq!(rule.wined3d, Setting::Set(Wined3d::Vulkan));
    assert_eq!(rule.opengl, Setting::Set(OpenGl::Cgl));
    assert_eq!(rule.dpi_aware, Setting::Set(true));
    assert_eq!(rule.large_address_aware, Setting::Set(false));
    assert_eq!(rule.gcmouse, Setting::Reset);
    assert_eq!(rule.force_nx, Setting::Reset);
    // The user data keeps its case.
    assert_eq!(
        table.to_string(),
        "exe=Game.EXE;company=ACME;dxgi=wined3d;d3d9=mtld3d;wined3d=vulkan;opengl=cgl;\
         dpi_aware=true;large_address_aware=false;gcmouse=default;force_nx=default;\
         dll_overrides=D3D12=;arguments=-X;env=Name=Value\n\
         exe=x.exe;opengl=egl"
    );
    // An unknown key or value is reported as it was written.
    let (_, diagnostics) = parse("exe=x.exe;Bogus=1;DXGI=Vulkan");
    assert_eq!(
        diagnostics,
        [
            "WINE_COMPATDB line 1: ignoring unknown key \"Bogus\"",
            "WINE_COMPATDB line 1: ignoring unknown dxgi value \"Vulkan\"",
        ]
    );
    Ok(())
}

#[test]
fn arguments_override_round_trips_and_an_empty_one_clears() {
    let table = sent(vec![
        Rule {
            exe: Exe::literal("a.exe"),
            arguments_override: Some(vec![Argument::literal("-x"), Argument::literal("-y z")]),
            arguments: vec![Argument::literal("-w")],
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("b.exe"),
            arguments_override: Some(Vec::new()),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("c.exe"),
            ..Rule::default()
        },
    ]);
    let text = table.to_string();
    assert_eq!(
        text,
        "exe=a.exe;arguments_override=-x;arguments_override=-y z;arguments=-w\n\
         exe=b.exe;arguments_override=\n\
         exe=c.exe"
    );
    let (parsed, diagnostics) = parse(&text);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, table);
    // An empty value among others adds no entry.
    let (parsed, _) = parse("exe=a.exe;arguments_override=;arguments_override=-x");
    assert_eq!(
        parsed.rules.first().and_then(Rule::arguments_override),
        Some(&[Argument::literal("-x")][..])
    );
}

#[test]
fn wined3d_parses_round_trips_and_rejects_other_values() {
    assert_eq!("GL".parse(), Ok(Wined3d::Gl));
    assert_eq!("Vulkan".parse(), Ok(Wined3d::Vulkan));
    assert!("opengl".parse::<Wined3d>().is_err());
    assert!("dxmt".parse::<Wined3d>().is_err());
    assert_eq!(Wined3d::Vulkan.as_str(), "vulkan");
    let table = sent(vec![
        Rule {
            exe: Exe::literal("vk.exe"),
            wined3d: Setting::Set(Wined3d::Vulkan),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("gl.exe"),
            wined3d: Setting::Set(Wined3d::Gl),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("unset.exe"),
            ..Rule::default()
        },
    ]);
    let value = table.to_string();
    assert!(value.contains("exe=vk.exe;wined3d=vulkan"), "{value}");
    assert!(value.contains("exe=gl.exe;wined3d=gl"), "{value}");
    assert!(!value.contains("exe=unset.exe;wined3d"), "{value}");
    let (parsed, diagnostics) = parse(&value);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, table);
    let (bad, diagnostics) = parse("exe=x.exe;wined3d=opengl\nexe=ok.exe");
    assert_eq!(bad.rules.len(), 2);
    assert_eq!(bad.rules.first().map(|r| r.wined3d), Some(Setting::Unset));
    assert_eq!(diagnostics.len(), 1);
}

#[test]
fn opengl_parses_round_trips_and_rejects_other_values() {
    assert_eq!("EGL".parse(), Ok(OpenGl::Egl));
    assert_eq!("Cgl".parse(), Ok(OpenGl::Cgl));
    assert!("gl".parse::<OpenGl>().is_err());
    assert!("".parse::<OpenGl>().is_err());
    assert_eq!(OpenGl::Egl.as_str(), "egl");
    assert_eq!(OpenGl::Egl.hook_value(), 1);
    assert_eq!(OpenGl::Cgl.hook_value(), 0);
    let table = sent(vec![
        Rule {
            exe: Exe::Any,
            opengl: Setting::Set(OpenGl::Egl),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("apple.exe"),
            opengl: Setting::Set(OpenGl::Cgl),
            ..Rule::default()
        },
        Rule {
            exe: Exe::literal("unset.exe"),
            ..Rule::default()
        },
    ]);
    let value = table.to_string();
    assert_eq!(
        value,
        "exe=*;opengl=egl\n\
         exe=apple.exe;opengl=cgl\n\
         exe=unset.exe"
    );
    let (parsed, diagnostics) = parse(&value);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(parsed, table);
    let (bad, diagnostics) = parse("exe=x.exe;opengl=gl\nexe=ok.exe");
    assert_eq!(bad.rules.len(), 2);
    assert_eq!(bad.rules.first().map(|r| r.opengl), Some(Setting::Unset));
    assert_eq!(diagnostics.len(), 1);
    assert!(
        diagnostics.first().is_some_and(|d| d.contains("opengl")),
        "{diagnostics:?}"
    );
}

/// A table with every key and the characters the format escapes or must
/// keep as they are in every field of user data.
fn awkward() -> Result<Table, FieldError> {
    let text = "a;b%c=d\r\n\te é 😀 ";
    let mut table = Table::new();
    table.push(
        Rule::new(Exe::new(format!("x{text}.exe"))?)
            .with_company(Fingerprint::new(text)?)
            .with_product(Fingerprint::new("")?)
            .with_original_filename(Fingerprint::new("%25")?)
            .with_dxgi(Dxgi::Dxmt)
            .with_d3d9(Setting::Reset)
            .with_wined3d(Wined3d::Gl)
            .with_opengl(Setting::Reset)
            .with_dpi_aware(false)
            .with_large_address_aware(Setting::Reset)
            .with_gcmouse(true)
            .with_force_nx(Setting::Reset)
            .with_dll_override(DllOverride::new(format!("{text}=n,b"))?)
            .with_dll_override(DllOverride::new("a=")?)
            .with_argument(Argument::new(text)?)
            .with_argument(Argument::new("%")?)
            .with_arguments_override([Argument::new(text)?, Argument::new(";")?])
            .with_env(EnvEntry::new(EnvName::new(" é;%\t")?, text)?)
            .with_env(EnvEntry::remove(EnvName::new("GONE")?)),
    );
    table.push(Rule::new(Exe::Any).with_arguments_override([]));
    table.push(Rule::new(Exe::new("*.exe")?));
    Ok(table)
}

#[test]
fn a_table_reads_back_as_itself_and_its_text_writes_back_as_itself() -> TestResult {
    let table = awkward()?;
    let text = table.to_string();
    assert_eq!(Table::parse_strict(&text), Ok(table.clone()));
    assert_eq!(text.parse::<Table>(), Ok(table));
    // Canonical text reserializes byte for byte.
    assert_eq!(Table::parse_strict(&text)?.to_string(), text);
    // Each rule is one line: the escapes keep CR and LF out of it.
    assert_eq!(text.lines().count(), 3);
    // Text in another case and with a legacy header reads as the same
    // rules and writes the canonical text, and that reads back the same.
    let loose = "V=7\nEXE=Game.exe;DPI_AWARE=False;Env=A=1\nexe=*;GCMOUSE=DEFAULT";
    let canonical = Table::parse_strict(loose)?.to_string();
    assert_eq!(
        canonical,
        "exe=Game.exe;dpi_aware=false;env=A=1\nexe=*;gcmouse=default"
    );
    assert_eq!(Table::parse_strict(&canonical)?.to_string(), canonical);
    Ok(())
}

#[test]
fn strict_parsing_refuses_what_lenient_parsing_reports() -> TestResult {
    for text in [
        "exe=x.exe;bogus=1",
        "exe=x.exe;dxgi=vulkan",
        "exe=x.exe;bogus",
        "dxgi=dxmt",
        "exe=",
        "exe=x.exe;arguments=",
        "exe=x.exe;arguments=50%",
        "exe=x.exe;company=%FF",
        "exe=x.exe;env=A=%00",
        "exe=x.exe;dll_overrides=mscoree",
        "exe=x.exe;exe=y.exe",
        "exe=x.exe;dpi_aware=true;dpi_aware=false",
    ] {
        let (_, diagnostics) = Table::parse_lenient(text);
        assert_eq!(diagnostics.len(), 1, "{text}: {diagnostics:?}");
        assert_eq!(
            Table::parse_strict(text),
            Err(ParseError(diagnostics)),
            "{text}"
        );
    }
    // Clean text, the empty one included, is no error.
    for text in ["", "\n\n", "v=6", "exe=x.exe", "exe=x.exe;;dpi_aware=true;"] {
        assert!(Table::parse_strict(text).is_ok(), "{text:?}");
    }
    // Every problem is reported, in line order.
    let error = Table::parse_strict("exe=a.exe;bogus=1\nnothing=1").err();
    let error = error.ok_or("no error")?;
    let lines: Vec<usize> = error.diagnostics().iter().map(|d| d.line).collect();
    assert_eq!(lines, [1, 2, 2]);
    Ok(())
}

#[test]
fn one_bad_field_gives_one_diagnostic() {
    // A field left out says only why, not also that it had a bare `%`.
    for (text, error) in [
        (
            "exe=x.exe;dxgi=50%",
            FieldError::BadValue {
                key: Key::Dxgi,
                value: "50%".into(),
                expected: &["dxmt", "wined3d", "default"],
            },
        ),
        ("exe=x.exe;arguments=%%00", FieldError::Nul),
        (
            "exe=x.exe;dll_overrides=50%",
            FieldError::InvalidDllOverride("50%".into()),
        ),
        // A second field says that, and not also its bare `%`.
        (
            "exe=x.exe;company=a;company=50%",
            FieldError::Duplicate(Key::Company),
        ),
    ] {
        let (_, diagnostics) = Table::parse_lenient(text);
        let errors: Vec<&FieldError> = diagnostics.iter().map(|d| &d.error).collect();
        assert_eq!(errors, [&error], "{text}");
    }
}

#[test]
fn a_bad_value_of_a_field_lists_default_among_the_values() {
    let (_, diagnostics) = Table::parse_lenient("exe=x.exe;dpi_aware=yes;opengl=zink");
    let expected: Vec<&[&str]> = diagnostics
        .iter()
        .filter_map(|d| match &d.error {
            FieldError::BadValue { expected, .. } => Some(*expected),
            _ => None,
        })
        .collect();
    assert_eq!(
        expected,
        [
            &["true", "false", "default"][..],
            &["egl", "cgl", "default"][..]
        ]
    );
}

#[test]
fn a_table_read_from_canonical_text_reads_back_as_itself() -> TestResult {
    let text = awkward()?.to_string();
    let table = Table::parse_strict(&text)?;
    assert_eq!(Table::parse_strict(&table.to_string()), Ok(table.clone()));
    assert_eq!(table.to_string(), text);
    Ok(())
}

#[test]
fn a_second_field_for_a_matcher_or_a_scalar_replaces_the_first() -> TestResult {
    let (table, diagnostics) = Table::parse_lenient(
        "exe=a.exe;company=A;exe=b.exe;company=B;dxgi=dxmt;dxgi=default;\
         arguments=-x;arguments=-x;env=A=1;env=A=2;dll_overrides=a=;dll_overrides=a=",
    );
    let rule = table.rules().first().ok_or("no rule")?;
    assert_eq!(rule.exe().as_str(), "b.exe");
    assert_eq!(rule.company().map(Fingerprint::as_str), Some("B"));
    assert_eq!(rule.dxgi(), Setting::Reset);
    // The lists take every entry.
    assert_eq!(rule.arguments(), ["-x", "-x"]);
    assert_eq!(rule.env().len(), 2);
    assert_eq!(rule.dll_overrides(), ["a=", "a="]);
    let errors: Vec<&FieldError> = diagnostics.iter().map(|d| &d.error).collect();
    assert_eq!(
        errors,
        [
            &FieldError::Duplicate(Key::Exe),
            &FieldError::Duplicate(Key::Company),
            &FieldError::Duplicate(Key::Dxgi),
        ]
    );
    let fields: Vec<Option<&str>> = diagnostics.iter().map(|d| d.field.as_deref()).collect();
    assert_eq!(
        fields,
        [Some("exe=b.exe"), Some("company=B"), Some("dxgi=default")]
    );
    // A field that cannot be read sets nothing, so it is no second one.
    let (table, diagnostics) = Table::parse_lenient("exe=a.exe;gcmouse=true;gcmouse=yes");
    assert_eq!(
        table.rules().first().map(Rule::gcmouse),
        Some(Setting::Set(true))
    );
    assert_eq!(diagnostics.len(), 1);
    // A second, empty exe leaves the line without one.
    let (table, diagnostics) = parse("exe=a.exe;exe=");
    assert_eq!(table, Table::new());
    assert_eq!(
        diagnostics,
        [
            "WINE_COMPATDB line 1: exe given more than once, the last one applies",
            "WINE_COMPATDB line 1: ignoring the line, it has no exe",
        ]
    );
    Ok(())
}

#[test]
fn a_bare_percent_is_kept_as_written_with_a_warning() -> TestResult {
    let (table, diagnostics) = parse("exe=x.exe;arguments=100%;env=A%=50%;company=%zz");
    let rule = table.rules().first().ok_or("no rule")?;
    assert_eq!(rule.arguments(), ["100%"]);
    assert_eq!(
        rule.env().first().map(EnvEntry::to_string),
        Some("A%=50%".into())
    );
    assert_eq!(rule.company().map(Fingerprint::as_str), Some("%zz"));
    // One warning per field, however many such `%` it has.
    assert_eq!(
        diagnostics,
        [
            "WINE_COMPATDB line 1: field \"arguments=100%\" has a '%' without two hex digits \
             after it, which is kept as written",
            "WINE_COMPATDB line 1: field \"env=A%=50%\" has a '%' without two hex digits after \
             it, which is kept as written",
            "WINE_COMPATDB line 1: field \"company=%zz\" has a '%' without two hex digits after \
             it, which is kept as written",
        ]
    );
    // Written again, the `%` is escaped, and that text is clean.
    let text = table.to_string();
    assert_eq!(
        text,
        "exe=x.exe;company=%25zz;arguments=100%25;env=A%25=50%25"
    );
    assert_eq!(Table::parse_strict(&text), Ok(table));
    Ok(())
}

#[test]
fn escapes_decode_as_utf_8_and_a_nul_is_refused() -> TestResult {
    let (table, diagnostics) = parse("exe=caf%C3%A9.exe;env=%F0%9F%98%80=%c3%a9");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let rule = table.rules().first().ok_or("no rule")?;
    assert_eq!(rule.exe().as_str(), "café.exe");
    assert_eq!(
        rule.env().first().map(EnvEntry::to_string),
        Some("😀=é".into())
    );
    // Written again, non-ASCII is plain UTF-8.
    assert_eq!(table.to_string(), "exe=café.exe;env=😀=é");
    // Escapes of bytes that are no UTF-8, or of a NUL, cost the field.
    let (table, diagnostics) =
        Table::parse_lenient("exe=x.exe;arguments=%E9;arguments=a%00;dpi_aware=false");
    let rule = table.rules().first().ok_or("no rule")?;
    assert_eq!(rule.arguments(), [] as [&str; 0]);
    assert_eq!(rule.dpi_aware(), Setting::Set(false));
    let errors: Vec<&FieldError> = diagnostics.iter().map(|d| &d.error).collect();
    assert_eq!(errors, [&FieldError::InvalidUtf8, &FieldError::Nul]);
    Ok(())
}

#[test]
fn an_equals_in_an_env_name_cannot_travel() {
    // The name ends at the first `=`; one escaped as `%3D` decodes to a
    // name no variable can have, so the field is refused.
    let (_, diagnostics) = parse("exe=x.exe;env=A%3DB=1");
    assert_eq!(
        diagnostics,
        ["WINE_COMPATDB line 1: ignoring invalid variable name \"A=B\""]
    );
    // The value may hold any `=`, written as it is.
    let (table, diagnostics) = parse("exe=x.exe;env=A=b=c");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(table.to_string(), "exe=x.exe;env=A=b=c");
}

#[test]
fn the_text_leaves_the_builtin_rules_out_and_push_numbers_the_lines() -> TestResult {
    let mut table = Table::new();
    table.push(Rule::new(Exe::new("a.exe")?));
    table.push(Rule::new(Exe::Any).with_gcmouse(false));
    assert_eq!(table.sources(), [Source::Line(1), Source::Line(2)]);
    let text = table.to_string();
    let full = table.with_builtin();
    assert_eq!(full.rules().len(), Table::builtin().rules().len() + 2);
    assert_eq!(full.to_string(), text);
    // Pushed after parsed rules, a rule takes the line after the last.
    let (mut parsed, _) = Table::parse_lenient("\n\nexe=a.exe");
    parsed.push(Rule::new(Exe::Any));
    assert_eq!(parsed.sources(), [Source::Line(3), Source::Line(4)]);
    Ok(())
}
