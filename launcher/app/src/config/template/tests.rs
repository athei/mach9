use std::fs;

use toml_edit::DocumentMut;

use super::*;
use crate::{
    config::{
        Config, Raw, Settings, Source,
        files::{GAME_TEMPLATE, TEMPLATE},
        launch_wide_keys, settings,
        test_support::*,
    },
    test_keys::text,
};

/// What the picker writes has to come back as the same path.
#[test]
fn a_written_path_reads_back_whole() {
    for path in [
        "/Games/Game.exe",
        "/Games/My Game/Game.exe",
        r#"/Games/od"d/Game.exe"#,
        r"/Games/back\slash/Game.exe",
    ] {
        let updated = set_executable("executable = \"/old/Game.exe\"\n", Path::new(path));
        let raw = raw_of(&updated);
        assert_eq!(raw.settings.executable.as_deref(), Some(path));
        assert!(raw.settings.arguments.is_none());
    }
}

/// The directives are gone from an expanded template, and what they stand
/// for comes from compatdb-core: a block per key of the rules with the
/// key's summary, link and example, the keys of the command line and the
/// links to the crate's sections.
#[test]
fn a_template_is_expanded_from_the_crate() {
    for template in [&*TEMPLATE, &*GAME_TEMPLATE] {
        assert!(!template.contains("#@"), "{template}");
        assert!(!template.contains(EXE), "{template}");
        assert!(template.contains(&format!("`{}`", Key::Exe)), "{template}");
        for key in launch_wide_keys() {
            let block = setting_block(key);
            assert!(template.contains(&block), "{key}: {template}");
            assert!(block.contains(key.docs_url()), "{block}");
            assert!(
                block.ends_with(&format!("# {}\n", assignment(key))),
                "{block}"
            );
        }
        for key in rule_example() {
            assert!(
                template.contains(&format!("# {}\n", assignment(key))),
                "{key}"
            );
        }
    }
    let prose: Vec<&str> = TEMPLATE
        .lines()
        .map(|line| line.trim_start_matches('#').trim())
        .collect();
    let keys = words::listing(&super::super::command_line_keys(), "");
    assert!(
        prose
            .join(" ")
            .contains(&format!("Supported keys are {keys}."))
    );
    for section in [
        DocSection::Matching,
        DocSection::Folding,
        DocSection::LauncherResponsibilities,
    ] {
        assert!(
            TEMPLATE.contains(&format!("# {}\n", section.url())),
            "{section:?}"
        );
    }
}

/// A comment is wrapped at the width of a line, a word longer than one
/// line on a line of its own.
#[test]
fn a_comment_is_wrapped() {
    let text = "a ".repeat(100);
    let wrapped = comment(&text);
    assert!(wrapped.lines().count() > 1, "{wrapped}");
    for line in wrapped.lines() {
        assert!(line.len() <= WIDTH, "{line}");
        assert!(line.starts_with("# "), "{line}");
    }
    let long = "x".repeat(WIDTH * 2);
    assert_eq!(comment(&format!("a {long}")), format!("# a\n# {long}\n"));
}

/// Every example the templates show, uncommented, is a setting the
/// launcher takes, so a user who uncomments one gets a working file.
#[test]
fn every_template_example_is_accepted_uncommented() {
    // An option line, or a dotted assignment such as a rule's `env.NAME =
    // ...`, which `option_name` does not take.
    let example = |line: &str| {
        let line = line.trim_start_matches('#').trim();
        let assigns_dotted = line.split_once('=').is_some_and(|(name, _)| {
            let name = name.trim();
            name.contains('.') && name.split('.').all(is_key)
        });
        (option_name(line).is_some() || assigns_dotted).then(|| format!("{line}\n"))
    };
    for (template, source) in [
        (&*TEMPLATE, Source::Defaults),
        (&*GAME_TEMPLATE, Source::Game),
    ] {
        let text: String = template.lines().filter_map(example).collect();
        assert!(text.contains(&format!("{}.", Key::Env)), "{text}");
        let document: DocumentMut = text.parse().unwrap_or_else(|e| panic!("{e}\n{text}"));
        if let Err(e) = Raw::from_document(&document, source) {
            panic!("{e}\n{text}");
        }
    }
}

#[test]
fn templates_need_no_sync_against_themselves() {
    assert!(sync_template(&TEMPLATE, &TEMPLATE).is_none());
    assert!(sync_template(&GAME_TEMPLATE, &GAME_TEMPLATE).is_none());
}

/// The templates' layout is what `sync_template` reads: one option per
/// block, and no option hidden in the header prose. Each template
/// documents every key its file takes once, in the order it chooses for its
/// reader, so neither declares a key the parser refuses or leaves one out.
#[test]
fn templates_declare_every_option_once() {
    fn sorted(mut keys: Vec<&str>) -> Vec<&str> {
        keys.sort_unstable();
        keys
    }
    let declared = sorted(declared_options(&TEMPLATE).collect());
    assert_eq!(
        declared,
        sorted(Source::Defaults.keys()),
        "gamelauncher.toml layout drifted: one option per block, blocks \
         separated by blank lines, every key the file takes declared"
    );
    let declared = sorted(declared_options(&GAME_TEMPLATE).collect());
    assert_eq!(
        declared,
        sorted(settings()),
        "game.toml layout drifted: one option per block, blocks separated by \
         blank lines, every setting declared"
    );
}

/// The example rule at the end of a template holds keys that are also
/// top-level options, and those lines belong to the rule.
#[test]
fn a_key_inside_a_table_is_no_top_level_mention() {
    let exe = Key::Exe.as_str();
    let example = format!(
        "# retina_mode = true\n\n# [process.\"my-rule\"]\n# {}# arguments = [\"--x\"]\n",
        text(exe, "Game.exe")
    );
    assert!(mentions(&example, "retina_mode"));
    assert!(mentions(&example, "process"));
    assert!(!mentions(&example, "arguments"));
    assert!(!mentions(&example, exe));
    // A commented-out table ends with its block, a real one does not.
    assert!(mentions(
        &format!("{example}\n# arguments = []\n"),
        "arguments"
    ));
    let real = "[env]\nA = \"b\"\n\ndpi = 144\n";
    assert!(mentions(real, "env"));
    assert!(!mentions(real, "dpi"));
    // A commented-out header owns no real line, even in its own block.
    let after_example = "# [env]\n# A = \"b\"\ndpi = 144\n";
    assert!(mentions(after_example, "dpi"));
    assert!(sync_template(after_example, "# dpi = \"auto\"\n").is_none());
    // A real header with a trailing comment is a table all the same.
    let commented_header = format!("[process.\"x\"] # mine\n{}", text(exe, "x.exe"));
    assert!(mentions(&commented_header, "process"));
    assert!(!mentions(&commented_header, exe));
    // A quoted `#` is part of the header, not a comment.
    assert!(mentions("# [process.\"a#b\"]\n", "process"));
}

/// A key of the rules is mentioned in any case, as the settings layer reads
/// it, so a file that writes it in another case gets no second block.
#[test]
fn a_key_of_the_rules_is_mentioned_in_any_case() {
    let key = launch_wide_keys()[0];
    let upper = format!("{} = 1\n", key.as_str().to_ascii_uppercase());
    assert!(mentions(&upper, key.as_str()));
    assert!(sync_template(&upper, &format!("# {}\n", assignment(key))).is_none());
    assert!(!mentions("RETINA_MODE = 1\n", "retina_mode"));
}

/// A gamelauncher.toml written before an option existed gets its block,
/// commented out, at the next launch, and still reads as the defaults.
#[test]
fn an_older_defaults_file_is_topped_up_with_the_blocks_it_lacks() {
    let keys: Vec<Key> = launch_wide_keys().into_iter().take(3).collect();
    let new: Vec<&str> = std::iter::once("arguments")
        .chain(keys.iter().map(|key| key.as_str()))
        .collect();
    let older = blocks(&TEMPLATE)
        .filter(|block| block_option(block).is_none_or(|option| !new.contains(&option)))
        .collect::<Vec<_>>()
        .join("\n\n");
    for option in &new {
        assert!(!mentions(&older, option), "{option} still in {older}");
    }
    let support = TempSupport::new("top-up-defaults");
    let executable = support.executable();
    let path = support.defaults(&older);
    support.game("Game", &game_file(&executable_line(&executable)));

    let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

    assert!(config.launch_wide.is_empty());
    assert_eq!(config.arguments(), Vec::<String>::new());
    let text = fs::read_to_string(&path).unwrap();
    let document: DocumentMut = text.parse().unwrap();
    for option in &new {
        assert!(mentions(&text, option), "{option} was not added: {text}");
        assert!(!document.contains_key(option), "{option} is set: {text}");
    }
    for key in keys {
        assert!(text.contains(&setting_block(key)), "{key}: {text}");
    }
    assert!(text.contains("# arguments = []\n"), "{text}");
    assert!(sync_template(&text, &TEMPLATE).is_none(), "{text}");
}

#[test]
fn templates_parse_and_the_game_template_takes_an_executable() {
    let raw = raw_of(&TEMPLATE);
    assert_eq!(raw.settings, Settings::default());
    assert_eq!(raw.rules.rules, Vec::<crate::rules::NamedRule>::new());
    let defaults: DocumentMut = TEMPLATE.parse().unwrap();
    assert!(!defaults.contains_key("executable"));

    let updated = set_executable(&GAME_TEMPLATE, Path::new("/Games/Game.exe"));
    let raw = raw_of(&updated);
    assert_eq!(raw.settings.executable.as_deref(), Some("/Games/Game.exe"));
    assert!(updated.contains("# Wine Mac Driver retina mode"));
    assert!(sync_template(&updated, &GAME_TEMPLATE).is_none());
}

/// The value belongs under the comment that explains it, not at the top of
/// the file, and the example it replaces must not survive as a duplicate.
#[test]
fn writes_the_executable_where_the_example_was() {
    let updated = set_executable(&GAME_TEMPLATE, Path::new("/Games/Game.exe"));
    assert!(updated.starts_with("# One game."));
    assert_eq!(
        updated
            .lines()
            .filter(|line| is_option(line, "executable"))
            .collect::<Vec<_>>(),
        [r#"executable = "/Games/Game.exe""#]
    );
    let example = GAME_TEMPLATE
        .lines()
        .position(|line| is_option(line, "executable"))
        .unwrap();
    assert_eq!(
        updated.lines().nth(example),
        Some(r#"executable = "/Games/Game.exe""#)
    );
}

#[test]
fn replaces_an_existing_executable_in_place() {
    let existing = "# comment\nexecutable = \"/old/Game.exe\"\n\n[env]\nA = \"b\"\n";
    assert_eq!(
        set_executable(existing, Path::new("/new/Game.exe")),
        "# comment\nexecutable = \"/new/Game.exe\"\n\n[env]\nA = \"b\"\n"
    );
}

#[test]
fn adds_the_executable_back_when_the_block_is_gone() {
    let existing = "retina_mode = true\n\n[env]\nA = \"b\"\n";
    assert_eq!(
        set_executable(existing, Path::new("/new/Game.exe")),
        "retina_mode = true\n\nexecutable = \"/new/Game.exe\"\n\n[env]\nA = \"b\"\n"
    );
}

#[test]
fn appends_missing_options_with_their_comments() {
    let existing = "executable = \"/Games/Game.exe\"\n";
    let template = "# what it does\n# retina_mode = true\n";
    let synced = sync_template(existing, template).unwrap();
    assert_eq!(synced.added, ["retina_mode"]);
    assert_eq!(
        synced.text,
        "executable = \"/Games/Game.exe\"\n\n# what it does\n# retina_mode = true\n"
    );
}

#[test]
fn keeps_new_options_out_of_an_existing_table() {
    let existing = "executable = \"/Games/Game.exe\"\n\n[env]\nA = \"b\"\n";
    let template = "# retina_mode = true\n";
    let synced = sync_template(existing, template).unwrap();
    assert_eq!(
        synced.text,
        "executable = \"/Games/Game.exe\"\n\n# retina_mode = true\n\n[env]\nA = \"b\"\n"
    );
}

#[test]
fn leaves_options_the_user_commented_out_alone() {
    let existing = "executable = \"/Games/Game.exe\"\n\n# retina_mode = true\n";
    let template = "# retina_mode = true\n";
    assert!(sync_template(existing, template).is_none());
}
