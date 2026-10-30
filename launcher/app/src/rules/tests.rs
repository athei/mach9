use std::collections::BTreeMap;

use compatdb_core::{
    D3d9, DebugChannel, DllOverride, Dxgi, Fingerprint, OpenGl, Scalar, Setting, Source,
    VersionInfo, check_env_name,
};
use toml_edit::DocumentMut;

use super::*;
use crate::test_keys::{example, flag, list, retired_env, retired_key, retired_value, text};

/// The rules and the fields the launcher ignores (see [`parse`]).
fn parsed_with_ignored(toml: &str) -> Result<(Rules, Ignored), String> {
    let doc: DocumentMut = toml.parse().unwrap();
    let mut ignored = Ignored::default();
    let rules = parse(doc.get("process").expect("a process item"), &mut ignored)?;
    Ok((rules, ignored))
}

fn parsed(toml: &str) -> Result<Rules, String> {
    parsed_with_ignored(toml).map(|(rules, _)| rules)
}

fn parsed_rules(toml: &str) -> Vec<NamedRule> {
    parsed(toml).expect("valid").rules
}

fn rules_of(toml: &str) -> LaunchRules {
    table(every(), parsed_rules(toml))
}

fn error_of(toml: &str) -> String {
    parsed(toml).expect_err("should be rejected")
}

/// A `[process."<name>"]` block holding `lines`.
fn block(name: &str, lines: &[String]) -> String {
    format!("[process.\"{name}\"]\n{}\n", lines.concat())
}

/// The [`Key::Exe`] line for `name`.
fn exe_line(name: &str) -> String {
    text(Key::Exe, name)
}

/// What the warning about block `name` without a [`Key::Exe`] starts with,
/// for a block with `enabled` as given.
fn without_exe(name: &str, enabled: bool) -> String {
    let exe = Key::Exe;
    if enabled {
        format!("process rule `{name}` sets no `{exe}`")
    } else {
        format!("process rule `{name}` has `enabled = false` and no `{exe}`")
    }
}

/// The `company` line for `name`, a fingerprint.
fn company_line(name: &str) -> String {
    text(Key::Company, name)
}

/// The rule of the block called `name`.
fn rule<'a>(launch: &'a LaunchRules, name: &str) -> &'a Rule {
    let index = launch
        .names
        .iter()
        .position(|n| n.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no rule named {name}"));
    &launch.table.rules()[index]
}

fn every() -> Rule {
    Rule::new(Exe::Any)
}

fn exe(name: &str) -> Exe {
    Exe::new(name).unwrap()
}

fn fingerprint(text: &str) -> Fingerprint {
    Fingerprint::new(text).unwrap()
}

fn none() -> VersionInfo {
    VersionInfo::default()
}

/// `rule` with the field `key` set from `value` through the settings layer.
fn with(mut rule: Rule, key: Key, value: &str) -> Rule {
    rule.set(key, value).unwrap();
    rule
}

/// The error the settings layer gives for `text` in the field `key` of a
/// rule for one program.
fn refused(key: Key, text: &str) -> String {
    let mut rule = Rule::new(exe("a.exe"));
    describe(key.as_str(), &rule.set(key, text).unwrap_err())
}

#[test]
fn no_process_table_is_empty() {
    let built = table(every(), Vec::new());
    assert_eq!(built.table, Table::new());
    assert_eq!(built.names, Vec::<Option<String>>::new());
}

/// The launcher's own rule comes first, as line 1, and is left out when it
/// sets nothing.
#[test]
fn the_launchers_own_rule_comes_first_when_it_sets_anything() {
    let game = block("game", &[exe_line("Game.exe")]);
    let built = table(every().with_dxgi(Dxgi::Wined3d), parsed_rules(&game));
    assert_eq!(built.names, [None, Some(String::from("game"))]);
    assert_eq!(built.table.sources(), [Source::Line(1), Source::Line(2)]);
    assert_eq!(built.table.rules()[0], every().with_dxgi(Dxgi::Wined3d));

    let built = table(every(), parsed_rules(&game));
    assert_eq!(built.names, [Some(String::from("game"))]);
}

/// Every field goes through the settings layer: a block with the crate's
/// example of every key makes the rule the settings layer makes of them.
#[test]
fn a_block_reads_into_the_rule_its_fields_name() {
    let lines: Vec<String> = Key::ALL.iter().map(|key| example(*key)).collect();
    let built = rules_of(&block("all", &lines));
    let expected = Key::ALL
        .iter()
        .filter(|key| **key != Key::Exe)
        .fold(Rule::new(exe(Key::Exe.example())), |rule, key| {
            with(rule, *key, key.example())
        });
    assert_eq!(*rule(&built, "all"), expected);
    // No name travels: the table is the rule alone.
    let mut table = Table::new();
    table.push(expected);
    assert_eq!(built.table, table);
}

/// The settings layer reads every key and word of the format in any case.
#[test]
fn keys_and_words_are_read_without_regard_to_case() {
    let upper = |key: Key| key.as_str().to_ascii_uppercase();
    let wined3d = Dxgi::Wined3d.as_str().to_ascii_uppercase();
    let cgl = OpenGl::Cgl.as_str().to_ascii_uppercase();
    let built = rules_of(&block(
        "a",
        &[
            text(upper(Key::Exe), "Game.exe"),
            text(upper(Key::Dxgi), wined3d),
            text(upper(Key::OpenGl), cgl),
            text(Key::Gcmouse, DEFAULT_VALUE.to_ascii_uppercase()),
        ],
    ));
    assert_eq!(
        *rule(&built, "a"),
        Rule::new(exe("Game.exe"))
            .with_dxgi(Dxgi::Wined3d)
            .with_opengl(OpenGl::Cgl)
            .with_gcmouse(Setting::Reset)
    );
}

/// [`DEFAULT_VALUE`] hands a setting back to upstream Wine: a true-or-false
/// setting takes it as a string beside the booleans.
#[test]
fn default_resets_a_setting() {
    let built = rules_of(&block(
        "tool",
        &[
            exe_line("Tool.exe"),
            company_line("Acme"),
            text(Key::OpenGl, DEFAULT_VALUE),
            text(Key::ForceNx, DEFAULT_VALUE.to_ascii_uppercase()),
        ],
    ));
    let tool = rule(&built, "tool");
    assert_eq!(tool.opengl(), Setting::Reset);
    assert_eq!(tool.force_nx(), Setting::Reset);
    // A rule naming the program sets it again over the launch-wide reset.
    let built = table(
        every().with_dpi_aware(Setting::Reset),
        parsed_rules(&block(
            "tool",
            &[exe_line("tool.exe"), flag(Key::DpiAware, false)],
        )),
    );
    let tool = built.table.resolve("Tool.exe", &none(), None);
    assert_eq!(tool.dpi_aware(), Some(false));
    assert_eq!(tool.scalar(Scalar::DpiAware).reset_by, None);
    let other = built.table.resolve("Other.exe", &none(), None);
    assert_eq!(other.dpi_aware(), None);
    assert_eq!(
        other.scalar(Scalar::DpiAware).reset_by,
        Some(Origin::Line(1))
    );
}

/// A value its key does not take is the settings layer's error, with the
/// values the key takes.
#[test]
fn a_value_its_key_does_not_take_is_an_error() {
    for key in [Key::Dxgi, Key::Wined3d, Key::D3d9] {
        let error = error_of(&block("a", &[exe_line("a.exe"), text(key, "bogus")]));
        assert_eq!(
            error,
            format!("process rule `a`: {}", refused(key, "bogus"))
        );
    }
    let error = refused(Key::Dxgi, "bogus");
    for value in Key::Dxgi.values() {
        assert!(error.contains(&format!("`{value}`")), "{error}");
    }
}

/// The list that replaces the arguments of the rules folded before: present
/// as an empty list, it clears them.
#[test]
fn an_empty_list_is_a_value_of_its_own() {
    let argument = Key::Arguments.example();
    let built = rules_of(
        &[
            block(
                "clear",
                &[
                    exe_line("Clear.exe"),
                    company_line("Acme"),
                    list(Key::ArgumentsOverride, &[]),
                ],
            ),
            block(
                "one",
                &[
                    exe_line("One.exe"),
                    company_line("Acme"),
                    text(Key::ArgumentsOverride, argument),
                    list(Key::Arguments, &["-console"]),
                ],
            ),
            block("plain", &[exe_line("Plain.exe"), company_line("Acme")]),
        ]
        .concat(),
    );
    let acme = |name| Rule::new(exe(name)).with_company(fingerprint("Acme"));
    assert_eq!(
        *rule(&built, "clear"),
        acme("Clear.exe").with_arguments_override([])
    );
    assert_eq!(
        *rule(&built, "one"),
        with(
            with(acme("One.exe"), Key::ArgumentsOverride, argument),
            Key::Arguments,
            "-console"
        )
    );
    assert_eq!(*rule(&built, "plain"), acme("Plain.exe"));
}

/// The settings layer refuses some fields in a rule for every process; a
/// block without a [`Key::Exe`] is checked as a rule for some program, since
/// it is left out anyway.
#[test]
fn a_field_a_rule_for_every_process_refuses_is_an_error() {
    let key = *Key::ALL
        .iter()
        .find(|key| key.kind() == KeyKind::List && !key.allowed_on_any())
        .expect("a list a rule for every process refuses");
    let refused = describe(
        key.as_str(),
        &every()
            .set_list(key, std::iter::empty::<&str>())
            .unwrap_err(),
    );
    for shape in [
        vec![exe_line(Exe::ANY)],
        vec![exe_line(Exe::ANY), company_line("Acme")],
    ] {
        let lines: Vec<String> = shape.into_iter().chain([list(key, &[])]).collect();
        let error = error_of(&block("a", &lines));
        assert_eq!(error, format!("process rule `a`: {refused}"));
    }
    let rules = parsed(&block("a", &[list(key, &[])])).expect("no error");
    assert_eq!(rules.rules, Vec::<NamedRule>::new());
}

/// A Steam app takes no field of its own: a block names it through its
/// [`Key::Exe`], with no warning that it matches a file name, and the table
/// carries the canonical text.
#[test]
fn a_block_can_name_a_steam_app_through_its_exe() {
    let rules = parsed(&block(
        "tf2",
        &[
            exe_line("STEAM:440"),
            text(Key::Dxgi, Dxgi::Wined3d.as_str()),
        ],
    ))
    .expect("valid");
    assert_eq!(rules.warnings, Vec::<String>::new());
    let built = table(every(), rules.rules);
    assert_eq!(built.table.to_string(), "exe=steam:440;dxgi=wined3d");
    assert_eq!(
        built.table.resolve("hl2.exe", &none(), Some(440)).dxgi(),
        Some(Dxgi::Wined3d)
    );
    assert_eq!(
        built.table.resolve("hl2.exe", &none(), Some(220)).dxgi(),
        None
    );
    assert_eq!(built.table.resolve("hl2.exe", &none(), None).dxgi(), None);
    assert_eq!(built.table.winedebug(), Vec::<String>::new());
}

/// A block that names a program warns, so the test above shows what the
/// Steam app leaves out, and a fingerprint is taken as for a program.
#[test]
fn a_steam_block_takes_a_fingerprint_and_a_program_block_still_warns() {
    let rules = parsed(&block(
        "tf2",
        &[exe_line("steam:440"), company_line("Valve")],
    ))
    .expect("valid");
    assert_eq!(rules.warnings, Vec::<String>::new());
    assert_eq!(
        rules.rules[0].rule,
        Rule::new(exe("steam:440")).with_company(fingerprint("Valve"))
    );
    let rules = parsed(&block("game", &[exe_line("Game.exe")])).expect("valid");
    assert_eq!(rules.warnings.len(), 1, "{:?}", rules.warnings);
    assert!(rules.warnings[0].contains("every program called Game.exe"));
}

/// The block is checked with its [`Key::Exe`] first, wherever the file writes
/// it, so a Steam app refuses the fields the settings layer refuses for one.
#[test]
fn a_field_a_steam_rule_refuses_is_an_error() {
    for key in [Key::DebugChannels, Key::ArgumentsOverride] {
        let field = list(key, &["+relay"]);
        let refused = describe(key.as_str(), &FieldError::RefusedOnSteamApp(key));
        for lines in [
            vec![exe_line("steam:440"), field.clone()],
            vec![field.clone(), exe_line("steam:440")],
        ] {
            assert_eq!(
                error_of(&block("a", &lines)),
                format!("process rule `a`: {refused}")
            );
        }
    }
}

/// An `exe` that says `steam:` without an app id refuses the file, with the
/// text as written.
#[test]
fn a_bad_steam_app_id_is_an_error() {
    let error = error_of(&block("a", &[exe_line("steam:0")]));
    let refused = describe(
        Key::Exe.as_str(),
        &FieldError::BadSteamApp(String::from("steam:0")),
    );
    assert_eq!(error, format!("process rule `a`: {refused}"));
}

/// A field the settings layer only warns about goes to `ignored`, its value
/// unread, and the rest of the block still applies.
#[test]
fn a_field_the_settings_layer_warns_about_is_ignored() {
    let old_key = retired_key(Severity::Warning);
    let (value_key, old_value) = retired_value(Severity::Warning);
    let (rules, ignored) = parsed_with_ignored(
        &[
            block(
                "old",
                &[
                    exe_line("Old.exe"),
                    company_line("Acme"),
                    flag(old_key, false),
                    text(value_key, old_value.to_ascii_uppercase()),
                ],
            ),
            block("unnamed", &[text(old_key, "not even a bool")]),
        ]
        .concat(),
    )
    .expect("a warning is no error");
    let retired = Key::from_setting(old_key).unwrap_err();
    let value = Rule::new(exe("a.exe"))
        .set(value_key, &old_value.to_ascii_uppercase())
        .unwrap_err();
    assert_eq!(
        ignored.warnings,
        [
            format!("process rule `old`: {retired}"),
            format!("process rule `old`: {value}"),
            format!("process rule `unnamed`: {retired}"),
        ]
    );
    assert_eq!(
        ignored.fields,
        [
            (Some(String::from("old")), String::from(old_key)),
            (Some(String::from("old")), value_key.to_string()),
            (Some(String::from("unnamed")), String::from(old_key)),
        ]
    );
    assert_eq!(rules.warnings.len(), 1, "{:?}", rules.warnings);
    assert!(rules.warnings[0].starts_with(&without_exe("unnamed", true)));
    let built = table(every(), rules.rules);
    assert_eq!(
        *rule(&built, "old"),
        Rule::new(exe("Old.exe")).with_company(fingerprint("Acme"))
    );
}

/// A key the settings layer refuses names what replaced it.
#[test]
fn a_key_the_settings_layer_refuses_is_an_error() {
    let old = retired_key(Severity::Error);
    let error = error_of(&block("a", &[exe_line("a.exe"), text(old, "-x")]));
    let refused = Key::from_setting(old).unwrap_err();
    assert_eq!(
        error,
        format!("process rule `a`: {}", describe(old, &refused))
    );
}

/// The variable matches in any case, as in a process environment.
#[test]
fn a_variable_a_setting_replaced_is_an_error_in_a_rules_env() {
    let name = retired_env();
    for name in [name.to_owned(), name.to_ascii_lowercase()] {
        let error = error_of(&block(
            "a",
            &[exe_line("a.exe"), text(format!("{}.{name}", Key::Env), "x")],
        ));
        let refused = check_env_name(&name).unwrap_err();
        assert_eq!(
            error,
            format!(
                "process rule `a`: {}",
                describe(Key::Env.as_str(), &refused)
            )
        );
    }
}

/// An error names the key as the file wrote it, also one whose text does not.
#[test]
fn an_error_names_the_key_as_written() {
    let written = Key::Company.as_str().to_ascii_uppercase();
    let error = error_of(&block("a", &[exe_line("a.exe"), text(&written, "A\0")]));
    assert_eq!(
        error,
        format!("process rule `a`: {}", describe(&written, &FieldError::Nul))
    );
    assert!(
        error.starts_with(&format!("process rule `a`: `{written}`: ")),
        "{error}"
    );
}

/// Two spellings of one key in a block are refused.
#[test]
fn a_key_twice_in_a_block_is_an_error() {
    let upper = Key::Exe.as_str().to_ascii_uppercase();
    let error = error_of(&block("a", &[text(upper, "a.exe"), exe_line("b.exe")]));
    assert_eq!(
        error,
        format!(
            "process rule `a`: {}",
            describe(Key::Exe.as_str(), &FieldError::Duplicate(Key::Exe))
        )
    );
}

#[test]
fn a_rejected_field_is_an_error_that_says_why() {
    let error = error_of(&block(
        "a",
        &[exe_line("a.exe"), flag("retina_mode", false)],
    ));
    assert!(
        error.contains("only be set for the whole launch"),
        "{error}"
    );
    let error = error_of(&block(
        "a",
        &[exe_line("a.exe"), String::from("dpi = 144\n")],
    ));
    assert!(error.contains("dpi"), "{error}");
    assert!(
        error.contains("only be set for the whole launch"),
        "{error}"
    );
}

#[test]
fn an_unknown_field_is_an_error_that_lists_the_known_ones() {
    let error = error_of(&block("a", &[exe_line("a.exe"), text("bogus", "x")]));
    assert!(
        error.starts_with("process rule `a`: unknown field `bogus`; a rule takes "),
        "{error}"
    );
    for key in Key::ALL {
        assert!(error.contains(&format!("`{key}`")), "{key}: {error}");
    }
    assert!(error.ends_with(" and `enabled`"), "{error}");
}

/// A value of the wrong type is an error rather than something that quietly
/// reads as unset: a string where a key of the rules or `enabled` takes a
/// boolean, a number where it takes a string, a list or a table.
#[test]
fn a_field_of_the_wrong_type_is_an_error() {
    let wrong = |key: Key| -> &'static str {
        match key.kind() {
            KeyKind::Scalar(compatdb_core::ValueKind::Bool) => "\"false\"",
            KeyKind::Env => "\"A=1\"",
            KeyKind::List => "{ a = 1 }",
            _ => "1",
        }
    };
    let fields: Vec<(String, &str)> = Key::ALL
        .iter()
        .map(|key| (key.to_string(), wrong(*key)))
        .chain([
            (String::from("enabled"), "\"false\""),
            (format!("{}.A", Key::Env), "1"),
            (Key::DllOverrides.to_string(), "[\"a=n\", 1]"),
        ])
        .collect();
    for (field, value) in &fields {
        for base in [exe_line("a.exe"), String::new()] {
            if *field == Key::Exe.as_str() && !base.is_empty() {
                continue;
            }
            let error = error_of(&format!("[process.\"a\"]\n{base}{field} = {value}\n"));
            assert!(error.starts_with("process rule `a`: "), "{field}: {error}");
            assert!(
                error.contains(&format!("`{field}` must be")),
                "{field}: {error}"
            );
            assert!(
                error.contains(&format!("got `{value}`")),
                "{field}: {error}"
            );
        }
    }
}

#[test]
fn a_rule_that_is_not_a_table_is_an_error() {
    let error = error_of("process = 1\n");
    assert!(
        error.contains("`process` must be a table of rules"),
        "{error}"
    );
    let error = error_of("[process]\na = \"a.exe\"\n");
    assert!(
        error.contains("process rule `a` must be a table"),
        "{error}"
    );
}

/// `enabled = false` leaves the block out, which is how a game file turns
/// off a rule of gamelauncher.toml; `true` changes nothing.
#[test]
fn enabled_false_leaves_the_block_out() {
    let rules = parsed(
        &[
            block(
                "off",
                &[
                    exe_line("a.exe"),
                    flag(Key::DpiAware, false),
                    flag("enabled", false),
                ],
            ),
            block(
                "on",
                &[
                    exe_line("b.exe"),
                    company_line("Acme"),
                    flag("enabled", true),
                ],
            ),
        ]
        .concat(),
    )
    .expect("valid");
    assert!(rules.warnings.is_empty(), "{:?}", rules.warnings);
    let names: Vec<&str> = rules.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["on"]);
}

/// Two former names of built-in rules, the second in another case.
fn former_names() -> (&'static str, String) {
    let mut names = builtin::former_names();
    let first = names.next().expect("a former name");
    let second = names.next().expect("two former names");
    (first, second.to_ascii_uppercase())
}

/// A block without a [`Key::Exe`] used to extend or turn off a built-in rule of
/// that name. Built-in rules have no names any more, so it matches no process
/// and is left out, with a warning that says what to write instead: for a
/// former built-in rule, what compatdb-core says does that now.
#[test]
fn a_block_without_an_executable_is_left_out_with_a_warning() {
    let (extended, off) = former_names();
    let rules = parsed(
        &[
            block(
                extended,
                &[flag(Key::DpiAware, false), example(Key::DebugChannels)],
            ),
            block(&off, &[flag("enabled", false)]),
            block("mine", &[text(Key::Dxgi, Dxgi::Wined3d)]),
            block("old", &[flag("enabled", false)]),
            block("kept", &[exe_line("Kept.exe"), company_line("Acme")]),
        ]
        .concat(),
    )
    .expect("no error");
    let names: Vec<&str> = rules.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["kept"]);
    let warnings = &rules.warnings;
    assert_eq!(warnings.len(), 4, "{warnings:?}");
    for warning in warnings {
        assert!(
            warning.contains("so it matches no process and is left out; "),
            "{warning}"
        );
    }
    let former = builtin::former(extended).unwrap();
    assert!(
        warnings[0].starts_with(&without_exe(extended, true))
            && warnings[0].ends_with(&format!("{former}; {}", former.instead_of_extending())),
        "{}",
        warnings[0]
    );
    let former = builtin::former(&off).unwrap();
    assert!(
        warnings[1].starts_with(&without_exe(&off, false))
            && warnings[1].ends_with(&format!("{former}; {}", former.instead_of_turning_off())),
        "{}",
        warnings[1]
    );
    assert!(
        warnings[2].starts_with(&without_exe("mine", true))
            && warnings[2].ends_with(&format!(
                "give the block an `{}` to make it a rule of its own, or remove it",
                Key::Exe
            )),
        "{}",
        warnings[2]
    );
    assert!(
        warnings[3].starts_with(&without_exe("old", false))
            && warnings[3].ends_with("; remove the block"),
        "{}",
        warnings[3]
    );
}

/// A block named after a former built-in rule turned that rule off with
/// `enabled = false` whatever executable it named. It is still left out, but it no
/// longer does that, so it gets a warning; a disabled block of any other name
/// is how a game turns off a rule of gamelauncher.toml.
#[test]
fn a_disabled_block_with_a_former_built_in_name_warns() {
    let (_, name) = former_names();
    let rules = parsed(
        &[
            block(&name, &[exe_line(Exe::ANY), flag("enabled", false)]),
            block(
                "mine",
                &[
                    exe_line("Mine.exe"),
                    company_line("Acme"),
                    flag("enabled", false),
                ],
            ),
        ]
        .concat(),
    )
    .expect("no error");
    assert_eq!(rules.rules, Vec::<NamedRule>::new());
    let former = builtin::former(&name).unwrap();
    assert_eq!(
        rules.warnings,
        [format!(
            "process rule `{name}` has `enabled = false`, so it is left out; {former}; {}",
            former.instead_of_turning_off()
        )]
    );
}

/// A file is read on its own before it is merged, so the warnings go back to
/// the caller, which logs the merged rules' only.
#[test]
fn warnings_are_kept_for_the_caller() {
    let rules = parsed(
        &[
            block("off", &[flag("enabled", false), exe_line("a.exe")]),
            block("extends", &[text(Key::Dxgi, Dxgi::Dxmt)]),
            block("bare", &[exe_line("Tool.exe")]),
            block("pinned", &[exe_line("Tool.exe"), company_line("Acme")]),
        ]
        .concat(),
    )
    .unwrap();
    assert_eq!(rules.warnings.len(), 2, "{:?}", rules.warnings);
    assert!(rules.warnings[0].starts_with(&without_exe("extends", true)));
    assert!(rules.warnings[1].contains("`bare` matches every program called Tool.exe"));
    // The warning names the fingerprint keys the crate has.
    for key in Key::ALL {
        if key.kind() == KeyKind::Matcher && *key != Key::Exe {
            assert!(rules.warnings[1].contains(&format!("`{key}`")), "{key}");
        }
    }
}

#[test]
fn two_rules_may_name_the_same_executable() {
    let built = rules_of(
        &[
            block("first", &[exe_line("Tool.exe"), company_line("First")]),
            block("acme", &[exe_line("Tool.exe"), company_line("Acme")]),
        ]
        .concat(),
    );
    assert_eq!(built.table.rules().len(), 2);
    assert_eq!(
        rule(&built, "acme").company().map(Fingerprint::as_str),
        Some("Acme")
    );
    assert_eq!(built.duplicate_warnings(), Vec::<String>::new());
}

/// The table's duplicate check counts by index, which the warning turns back
/// into the blocks' names.
#[test]
fn rules_that_match_the_same_processes_are_named_in_the_warning() {
    let built = table(
        every().with_dpi_aware(true),
        parsed_rules(
            &[
                block("one", &[exe_line("Game.exe"), company_line("Acme")]),
                block("other", &[exe_line("Other.exe"), company_line("Acme")]),
                block("two", &[exe_line("game.exe"), company_line("Acme")]),
                block("all", &[exe_line(Exe::ANY), flag(Key::DpiAware, false)]),
            ]
            .concat(),
        ),
    );
    assert_eq!(
        built.duplicate_warnings(),
        [
            "rule `one` and rule `two` match exactly the same processes; both apply, which is \
              unlikely to be what was meant"
        ]
    );
}

/// Each rule's source is its line in the table's text, so a table resolved
/// here names a value's rule the way compatdb.so's log does, and the launcher
/// maps that back to the block.
#[test]
fn a_value_resolved_here_names_its_block() {
    let built = table(
        every().with_gcmouse(false),
        parsed_rules(
            &[
                block("game", &[exe_line("Game.exe"), flag(Key::DpiAware, false)]),
                block("all", &[exe_line(Exe::ANY), flag(Key::ForceNx, false)]),
            ]
            .concat(),
        ),
    );
    assert_eq!(
        built.names,
        [None, Some(String::from("game")), Some(String::from("all"))]
    );
    let game = built.table.resolve("game.exe", &none(), None);
    let describe = |scalar: Scalar| {
        let origin = game.scalar(scalar).set_by.as_ref();
        built.describe(origin.expect("set"))
    };
    assert_eq!(describe(Scalar::Gcmouse), "top level");
    assert_eq!(describe(Scalar::DpiAware), "rule `game`");
    assert_eq!(describe(Scalar::ForceNx), "rule `all`");
    assert_eq!(built.describe(&Origin::Line(9)), Origin::Line(9).label());
    assert_eq!(built.describe(&Origin::BuiltIn), Origin::BuiltIn.label());
    // A line of a rules file is no block of the config.
    assert_eq!(
        built.describe(&Origin::FileLine(1)),
        Origin::FileLine(1).label()
    );
}

/// A `[process]` block for [`Exe::Any`] folds after the top level.
#[test]
fn a_wildcard_rule_is_folded_after_the_launchers_own_rule() {
    let built = table(
        every().with_dpi_aware(true),
        parsed_rules(&block(
            "all",
            &[exe_line(Exe::ANY), flag(Key::DpiAware, false)],
        )),
    );
    let resolved = built.table.resolve("a.exe", &none(), None);
    assert_eq!(resolved.dpi_aware(), Some(false));
}

/// `global` was the name of the launcher's own rule while names travelled.
/// They no longer do, so it is a name like any other.
#[test]
fn a_block_may_be_called_global() {
    let built = rules_of(&block("global", &[exe_line("a.exe"), company_line("Acme")]));
    assert_eq!(*rule(&built, "global").exe(), "a.exe");
}

/// The table the launcher writes reads back as itself.
#[test]
fn rules_round_trip_through_the_text() {
    let channels = format!("{},-a", Key::DebugChannels.example());
    let built = table(
        every()
            .with_dpi_aware(false)
            .with_dll_override(DllOverride::new("a=n").unwrap()),
        parsed_rules(
            &[
                block(
                    "game",
                    &[
                        exe_line("Game.exe"),
                        text(Key::Dxgi, Dxgi::Wined3d),
                        flag(Key::MetalHud, false),
                        text(Key::DebugChannels, &channels),
                        text(format!("{}.SOME_CONFIG", Key::Env), "a;b"),
                    ],
                ),
                block(
                    "reset",
                    &[exe_line("Reset.exe"), text(Key::Gcmouse, DEFAULT_VALUE)],
                ),
                block("unset", &[exe_line("Other.exe")]),
            ]
            .concat(),
        ),
    );
    assert_eq!(
        Table::parse_strict(&built.table.to_string()).as_ref(),
        Ok(&built.table)
    );
    assert_eq!(
        rule(&built, "game").debug_channels(),
        [
            DebugChannel::new(Key::DebugChannels.example()).unwrap(),
            DebugChannel::new("-a").unwrap()
        ]
    );
    assert!(rule(&built, "unset").sets_nothing());
}

/// The launcher's own rule takes every key a rule for every process can
/// hold, but those whose names the launcher's own settings take.
#[test]
fn the_launchers_own_rule_takes_the_keys_a_rule_for_every_process_holds() {
    let keys = launch_wide_keys(&[Key::Arguments.as_str()]);
    for key in Key::ALL {
        let taken = key.allowed_on_any() && *key != Key::Arguments;
        assert_eq!(keys.contains(key), taken, "{key}");
    }
    let wined3d = D3d9::Wined3d.to_string();
    let fields = BTreeMap::from([
        (Key::D3d9, Value::One(wined3d)),
        (Key::MetalHud, Value::One(true.to_string())),
        (Key::DllOverrides, Value::List(vec![String::from("a=n")])),
    ]);
    assert_eq!(
        launch_rule(&fields),
        Ok(every()
            .with_d3d9(D3d9::Wined3d)
            .with_metal_hud(true)
            .with_dll_override(DllOverride::new("a=n").unwrap()))
    );
}
