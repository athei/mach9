//! Reading the config files: the embedded templates, each file read and
//! checked on its own, the retired keys, and how a game file is laid over the
//! defaults.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use compatdb_core::{Key, KeyKind};
use gamelauncher_core::{fsutil::write_atomic, games};
use log::{info, warn};
use toml_edit::{DocumentMut, Item, TableLike};

use super::{
    Raw, Source,
    template::{expand, top_up},
};
use crate::bundle;

/// Default config, written on first run and used to top up existing files
/// with options added since they were created: `gamelauncher.toml` with its
/// directives expanded from compatdb-core (see [`expand`]). Its layout is
/// load-bearing, see `sync_template`. It declares every key the file takes,
/// all of [`super::settings`] but `executable`, which a test checks.
pub(super) static TEMPLATE: LazyLock<String> =
    LazyLock::new(|| expand(include_str!("../../gamelauncher.toml")));

/// Same for a game file: what a new game is written into, and what an
/// existing game file is topped up from. It declares every key of
/// [`super::settings`], which a test checks.
pub(super) static GAME_TEMPLATE: LazyLock<String> =
    LazyLock::new(|| expand(include_str!("../../game.toml")));

/// Keys of the launcher's own that were once settings, with the sentence that
/// names what replaced each.
///
/// Using one is an error rather than a warning: the old spelling does nothing
/// now, so honouring the file quietly would start the game without a setting
/// the file plainly says it has. The settings layer of compatdb-core knows the
/// retired keys of the rules.
pub(super) const RETIRED_KEYS: &[(&str, &str)] = &[(
    "compat",
    "per-process settings are `[process.\"<rule name>\"]` blocks now, each naming the \
     executable it matches; see https://github.com/athei/mach9/blob/main/docs/app.md#settings",
)];

/// Whether the field `key` holds a list, which a file may also write as a
/// single string.
fn is_list(key: &str) -> bool {
    Key::from_setting(key).is_ok_and(|key| key.kind() == KeyKind::List)
}

/// One config file, checked on its own and then topped up from its template.
pub(super) struct File {
    pub(super) path: PathBuf,
    /// The text after the top-up, which the picker writes the executable into.
    pub(super) text: String,
    /// The same text parsed, with every list written as an array, ready to be
    /// merged.
    document: DocumentMut,
    pub(super) raw: Raw,
}

/// Read the file at `path`, creating it from `template` when it is missing,
/// and check it on its own: `check` first, then every setting it holds. Only
/// then is it topped up with the options it does not mention yet, so a file
/// that has to be edited by hand is left exactly as it was rather than
/// rewritten first.
fn load_file(
    path: PathBuf,
    template: &str,
    source: Source,
    check: impl FnOnce(&Path, &DocumentMut) -> io::Result<()>,
) -> io::Result<File> {
    let text = read_or_create(&path, template)?;
    let document = parse_document(&path, &text)?;
    check(&path, &document)?;
    let raw = Raw::from_document(&document, source)
        .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
    for warning in &raw.ignored.warnings {
        warn!("{}: {warning}", path.display());
    }
    let text = top_up(&path, text, template)?;
    let mut document = parse_document(&path, &text)?;
    // The ignored fields by their keys as written, before those change.
    without(&mut document, &raw.ignored.fields);
    normalized(&mut document);
    Ok(File {
        path,
        text,
        document,
        raw,
    })
}

/// Read `gamelauncher.toml`, which must not hold an `executable`.
pub(super) fn read_defaults(support_dir: &Path) -> io::Result<File> {
    load_file(
        bundle::config_path(support_dir),
        &TEMPLATE,
        Source::Defaults,
        |path, document| {
            if !document.contains_key("executable") {
                return Ok(());
            }
            Err(invalid(format!(
                "`executable` belongs in a game file: move the line from {} into {}/<Name>.toml \
             and write <Name> into {}",
                path.display(),
                games::games_dir(support_dir).display(),
                games::selection_path(support_dir).display()
            )))
        },
    )
}

/// Read the game file of that name.
pub(super) fn read_game(support_dir: &Path, name: &str) -> io::Result<File> {
    load_file(
        games::game_path(support_dir, name),
        &GAME_TEMPLATE,
        Source::Game,
        |_, _| Ok(()),
    )
}

/// Lay the game file over the defaults and read the result. Both were checked
/// on their own, so an error here comes from the two together, and says so.
pub(super) fn merged(defaults: File, game: &File) -> io::Result<Raw> {
    let mut document = defaults.document;
    take_replaced_lists(&mut document, &game.document);
    merge(document.as_item_mut(), game.document.as_item().clone());
    // Defensive: no mistake found so far passes both files on their own and
    // fails merged, but the message says where to look if one ever does.
    Raw::from_document(&document, Source::Game).map_err(|e| {
        invalid(format!(
            "{} merged over {}: {e}",
            game.path.display(),
            defaults.path.display()
        ))
    })
}

/// Whether the field `key` is a list that a later file's value replaces
/// instead of extending ([`Key::replaces_on_merge`]). The merge replaces a
/// value that is no list anyway.
fn replaced_list(key: &str) -> bool {
    Key::from_setting(key).is_ok_and(|key| key.kind() == KeyKind::List && key.replaces_on_merge())
}

/// Take each list that the game file sets in a rule and that replaces the
/// earlier value out of the same rule of the defaults, so the merge puts the
/// game's value in its place.
fn take_replaced_lists(defaults: &mut DocumentMut, game: &DocumentMut) {
    let (Some(base), Some(over)) = (
        defaults
            .get_mut("process")
            .and_then(Item::as_table_like_mut),
        game.get("process").and_then(Item::as_table_like),
    ) else {
        return;
    };
    for (name, rule) in over.iter() {
        let (Some(base_rule), Some(rule)) = (
            base.get_mut(name).and_then(Item::as_table_like_mut),
            rule.as_table_like(),
        ) else {
            continue;
        };
        for (key, _) in rule.iter().filter(|(key, _)| replaced_list(key)) {
            base_rule.remove(key);
        }
    }
}

/// Make two files merge key by key the way the settings layer reads them, at
/// the top level and in each rule: write each key of the rules the way the
/// crate does ([`Key::as_str`]), since it reads them in any case, and write
/// every list setting a file wrote as a single string as a list of one, so
/// that merging appends a game's entries to the defaults' however either file
/// wrote them. At the top level only the keys of the launcher's own rule are
/// rewritten, so that the launcher's own settings keep their case.
fn normalized(document: &mut DocumentMut) {
    fn normalize(table: &mut dyn TableLike, rewrites: impl Fn(Key) -> bool) {
        let renamed: Vec<(String, &'static str)> = table
            .iter()
            .filter_map(|(key, _)| {
                let canonical = Key::from_setting(key).ok().filter(|key| rewrites(*key))?;
                (key != canonical.as_str()).then(|| (key.to_owned(), canonical.as_str()))
            })
            .collect();
        for (key, canonical) in renamed {
            if let Some(item) = table.remove(&key) {
                table.insert(canonical, item);
            }
        }
        for (key, item) in table.iter_mut() {
            if is_list(&key)
                && let Some(text) = item.as_str()
            {
                let list: toml_edit::Array = std::iter::once(text.to_owned()).collect();
                *item = toml_edit::value(list);
            }
        }
    }
    let launch_wide = super::launch_wide_keys();
    normalize(document.as_table_mut(), |key| launch_wide.contains(&key));
    if let Some(process) = document
        .get_mut("process")
        .and_then(Item::as_table_like_mut)
    {
        for (_, rule) in process.iter_mut() {
            if let Some(rule) = rule.as_table_like_mut() {
                normalize(rule, |_| true);
            }
        }
    }
}

/// Take out every field the launcher ignores, at the top level and in the
/// rules, which the file was already warned about (see
/// [`crate::rules::Ignored`]), so the merged files read as if the line were
/// not there: a value the other file sets for the same key stays.
fn without(document: &mut DocumentMut, fields: &[(Option<String>, String)]) {
    for (block, key) in fields {
        let table = match block {
            None => Some(document.as_table_mut() as &mut dyn TableLike),
            Some(name) => document
                .get_mut("process")
                .and_then(Item::as_table_like_mut)
                .and_then(|process| process.get_mut(name))
                .and_then(Item::as_table_like_mut),
        };
        if let Some(table) = table {
            table.remove(key);
        }
    }
}

/// A config that cannot be used as written.
fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Create `path` from `template` when it is missing. Returns the file's text.
fn read_or_create(path: &Path, template: &str) -> io::Result<String> {
    if !path.exists() {
        write_atomic(path, template)?;
        info!("wrote default config to {}", path.display());
    }
    fs::read_to_string(path)
        .map_err(|e| io::Error::new(e.kind(), format!("failed to read {}: {e}", path.display())))
}

/// A syntax error is fatal on purpose: the file is hand-edited, so it is never
/// rewritten from under the user.
fn parse_document(path: &Path, text: &str) -> io::Result<DocumentMut> {
    text.parse()
        .map_err(|e| invalid(format!("failed to parse {}: {e}", path.display())))
}

/// Lay a game's settings over the defaults: tables merge key by key, arrays
/// take the game's entries after the defaults', and anything else, a value of
/// another type included, is replaced by the game's value.
fn merge(base: &mut Item, over: Item) {
    if let (Some(base_table), Some(over_table)) = (base.as_table_like_mut(), over.as_table_like()) {
        for (key, value) in over_table.iter() {
            match base_table.get_mut(key) {
                Some(existing) => merge(existing, value.clone()),
                None => {
                    base_table.insert(key, value.clone());
                }
            }
        }
        return;
    }
    if let (Some(base_array), Some(over_array)) = (base.as_array_mut(), over.as_array()) {
        base_array.extend(over_array.iter().cloned());
        return;
    }
    if let (Some(base_tables), Some(over_tables)) =
        (base.as_array_of_tables_mut(), over.as_array_of_tables())
    {
        base_tables.extend(over_tables.iter().cloned());
        return;
    }
    *base = over;
}

#[cfg(test)]
mod tests {
    use std::fs;

    use compatdb_core::{D3d9, Dxgi, Exe, FieldError, Rule, Setting, Severity, check_env_name};

    use super::*;
    use crate::{
        config::{
            Config, Settings, launch_wide_keys,
            template::{is_option, mentions, sync_template},
            test_support::*,
        },
        retina::RetinaMode,
        rules,
        test_keys::{flag, list, retired_env, retired_key, retired_value, text},
    };

    /// A `[process."<name>"]` block holding `lines`.
    fn block(name: &str, lines: &[String]) -> String {
        format!("\n[process.\"{name}\"]\n{}", lines.concat())
    }

    /// The shape of the files already on disk: an executable, a few commented
    /// options, a real `[env]` table at the end, and stale comment blocks left
    /// by earlier top-ups. Adding the new settings must not disturb any of it.
    #[test]
    fn a_file_from_the_old_layout_tops_up_and_still_loads() {
        let support = TempSupport::new("top-up-old");
        let executable = support.executable();
        let path = support.game(
            "Game",
            &format!(
                "# One game.\n\n{}\n# A setting.\n# {}\n# Wine Mac Driver retina mode.\n\
                 retina_mode = true\n\n\
                 # An example the launcher no longer writes.\n\
                 # [compat.\"Game.exe\"]\n# {}\n\
                 [env]\n#{} = \"x\"\n",
                executable_line(&executable),
                flag(Key::MetalHud, true),
                text(retired_key(Severity::Error), "x"),
                retired_env()
            ),
        );

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        assert_eq!(config.executable(), executable);
        assert_eq!(config.retina_mode, RetinaMode::On);
        let text = fs::read_to_string(&path).unwrap();
        for key in launch_wide_keys() {
            assert!(mentions(&text, key.as_str()), "{key} was not added: {text}");
        }
        assert!(mentions(&text, "process"), "process was not added: {text}");
        // The commented-out `[compat]` example is text, not config, so it is
        // left alone; a real one would have been refused above.
        assert!(text.contains("# [compat.\"Game.exe\"]"));
        assert!(text.contains("[env]"));
        // Nothing new landed inside the table the user wrote.
        let env_at = text.find("\n[env]").expect("the real table");
        assert!(
            !text[env_at..].lines().any(|line| launch_wide_keys()
                .iter()
                .any(|key| is_option(line, key.as_str()))),
            "a top-level option ended up inside [env]: {text}"
        );
        // And the file is stable: a second launch adds nothing.
        assert!(sync_template(&text, &GAME_TEMPLATE).is_none());
    }

    /// A retired key of the launcher's own, one the settings layer refuses,
    /// and a variable of `[env]` that a setting of the rules replaced, in
    /// either file, are errors that name the file and say what to write
    /// instead, and the file is left as it was.
    #[test]
    fn a_retired_key_is_fatal_and_names_the_file() {
        let refused = |written: &str, error: FieldError| rules::describe(written, &error);
        let old = retired_key(Severity::Error);
        let old_line = text(old, "-console");
        let old_error = refused(old, Key::from_setting(old).unwrap_err());
        let env_error =
            |name: &str| refused(&format!("env.{name}"), check_env_name(name).unwrap_err());
        let env_line = |name: &str| format!("[env]\n{name} = \"x\"\n");
        let env = retired_env();
        let lower = env.to_ascii_lowercase();
        let compat = String::from("[compat.\"Game.exe\"]\na = \"b\"\n");
        for (text, expected) in [
            (old_line.clone(), old_error.clone()),
            (
                compat.clone(),
                String::from("`compat` is no longer read; use per-process settings"),
            ),
            (env_line(env), env_error(env)),
            // A variable matches in any case, as in a process environment.
            (env_line(&lower), env_error(&lower)),
        ] {
            let text = text.as_str();
            let support = TempSupport::new("retired-defaults");
            let executable = support.executable();
            support.game("Game", &executable_line(&executable));
            let defaults = support.defaults(text);

            let error = Config::load_or_setup(support.path(), Settings::default())
                .unwrap_err()
                .to_string();

            assert!(
                error.starts_with(&format!("{}: {expected}", defaults.display())),
                "{text:?} gave {error}"
            );
        }
        for (text, expected) in [
            (old_line, old_error),
            (compat, String::from("`compat` is no longer read")),
            (env_line(env), env_error(env)),
        ] {
            let support = TempSupport::new("retired-game");
            let executable = support.executable();
            let game = support.game("Game", &format!("{}{text}", executable_line(&executable)));
            let existing = fs::read_to_string(&game).unwrap();

            let error = Config::load_or_setup(support.path(), Settings::default())
                .unwrap_err()
                .to_string();

            assert!(
                error.starts_with(&format!("{}: ", game.display())),
                "{text:?} gave {error}"
            );
            assert!(error.contains(&expected), "{text:?} gave {error}");
            assert_eq!(fs::read_to_string(&game).unwrap(), existing);
        }
    }

    /// The defaults are checked before the game file is laid over them, so a
    /// mistake in `gamelauncher.toml` is not blamed on the game.
    #[test]
    fn an_error_names_the_file_that_has_the_mistake() {
        let support = TempSupport::new("names-the-file");
        let executable = support.executable();
        let game = support.game(
            "Game",
            &[executable_line(&executable), text(Key::Dxgi, Dxgi::Dxmt)].concat(),
        );
        let defaults = support.defaults(&text(Key::D3d9, "bogus"));
        let error = Config::load_or_setup(support.path(), Settings::default())
            .unwrap_err()
            .to_string();
        let refused =
            rules::check_launch_wide(Key::D3d9, &rules::Value::One("bogus".into())).unwrap_err();
        assert_eq!(
            error,
            format!(
                "{}: {}",
                defaults.display(),
                rules::describe(Key::D3d9.as_str(), &refused)
            )
        );

        support.defaults(&text(Key::D3d9, D3d9::Wined3d));
        fs::write(
            &game,
            format!(
                "{}[process.\"a\"]\n{}enabled = \"false\"\n",
                executable_line(&executable),
                text(Key::Exe, "a.exe")
            ),
        )
        .unwrap();
        let error = Config::load_or_setup(support.path(), Settings::default())
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with(&format!(
                "{}: process rule `a`: `enabled` must be `true` or `false`, got `\"false\"`",
                game.display()
            )),
            "{error}"
        );
    }

    /// A list may be a single string at the top level as in a rule, and a game
    /// file's entries still come after the defaults' whichever way each file
    /// wrote them.
    #[test]
    fn a_list_written_as_one_string_still_merges_as_a_list() {
        let support = TempSupport::new("single-string-lists");
        let executable = support.executable();
        let rule_head = [text(Key::Exe, "p.exe"), text(Key::Company, "Acme")];
        support.defaults(
            &[
                text(Key::DllOverrides, "a=n"),
                list(Key::DebugChannels, &["+a"]),
                text("arguments", "-one"),
                block("p", &[rule_head.concat(), text(Key::Arguments, "--x")]),
            ]
            .concat(),
        );
        support.game(
            "Game",
            &[
                executable_line(&executable),
                list(Key::DllOverrides, &["b=n"]),
                text(Key::DebugChannels, "+b"),
                text("arguments", "-two"),
                block("p", &[text(Key::Arguments, "--y")]),
            ]
            .concat(),
        );

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        assert_eq!(config.arguments(), ["-one", "-two"]);
        // The launcher's own rule carries the top-level lists of the rules.
        assert_eq!(config.rules.names, [None, Some(String::from("p"))]);
        let own = &config.rules.table.rules()[0];
        assert_eq!(own.dll_overrides(), ["a=n", "b=n"]);
        assert_eq!(own.debug_channels(), ["+a", "+b"]);
        assert_eq!(config.rules.table.rules()[1].arguments(), ["--x", "--y"]);
    }

    /// The settings layer reads a key of the rules in any case, so the two
    /// files merge it whichever case each wrote, at the top level and in a
    /// rule. The launcher's own settings keep their case.
    #[test]
    fn a_key_of_the_rules_merges_whatever_its_case() {
        let support = TempSupport::new("key-case-merge");
        let executable = support.executable();
        let upper = |key: Key| key.as_str().to_ascii_uppercase();
        let capital = |key: Key| {
            let name = key.as_str();
            format!("{}{}", name[..1].to_ascii_uppercase(), &name[1..])
        };
        support.defaults(
            &[
                text(upper(Key::DllOverrides), "a=n"),
                text(upper(Key::Dxgi), Dxgi::Wined3d),
                block(
                    "p",
                    &[
                        text(Key::Exe, "p.exe"),
                        text(Key::Company, "Acme"),
                        text(upper(Key::Arguments), "--x"),
                    ],
                ),
            ]
            .concat(),
        );
        support.game(
            "Game",
            &[
                executable_line(&executable),
                list(Key::DllOverrides, &["b=n"]),
                text(Key::Dxgi, Dxgi::Dxmt),
                block("p", &[text(capital(Key::Arguments), "--y")]),
            ]
            .concat(),
        );

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        let own = &config.rules.table.rules()[0];
        assert_eq!(own.dll_overrides(), ["a=n", "b=n"]);
        assert_eq!(own.dxgi(), Setting::Set(Dxgi::Dxmt));
        assert_eq!(config.rules.table.rules()[1].arguments(), ["--x", "--y"]);

        // `ARGUMENTS` at the top level is no setting: the launcher's own
        // `arguments` is spelled in lowercase.
        support.defaults("ARGUMENTS = [\"-one\"]\n");
        let error = Config::load_or_setup(support.path(), Settings::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown setting `ARGUMENTS`"), "{error}");
    }

    /// A game file's list that replaces the earlier value
    /// ([`Key::replaces_on_merge`]) replaces the one gamelauncher.toml gives
    /// the same rule, so `[]` clears it, where every other list appends.
    #[test]
    fn a_game_files_replacing_list_replaces_the_defaults() {
        let key = *Key::ALL
            .iter()
            .find(|key| key.kind() == KeyKind::List && key.replaces_on_merge())
            .expect("a list that replaces");
        let support = TempSupport::new("replacing-list-merge");
        let executable = support.executable();
        support.defaults(&block(
            "p",
            &[
                text(Key::Exe, "p.exe"),
                text(Key::Company, "Acme"),
                list(key, &["-one"]),
                list(Key::Arguments, &["--x"]),
            ],
        ));
        let override_of = |game_block: &[String]| {
            support.game(
                "Game",
                &[executable_line(&executable), block("p", game_block)].concat(),
            );
            let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();
            assert_eq!(config.rules.names, [Some(String::from("p"))]);
            config.rules.table.rules()[0].clone()
        };
        let expected = |entries: &[&str]| {
            let mut rule = Rule::new(Exe::new("p.exe").unwrap());
            rule.set(Key::Company, "Acme").unwrap();
            rule.set_list(key, entries).unwrap();
            rule
        };

        assert_eq!(
            override_of(&[list(key, &[]), list(Key::Arguments, &["--y"])]),
            {
                let mut rule = expected(&[]);
                rule.set_list(Key::Arguments, ["--x", "--y"]).unwrap();
                rule
            }
        );
        let mut two = expected(&["-two"]);
        two.set(Key::Arguments, "--x").unwrap();
        assert_eq!(override_of(&[text(key, "-two")]), two);
        // Without one of its own the game keeps the defaults' value.
        let mut kept = expected(&["-one"]);
        kept.set_list(Key::Arguments, ["--x", "--y"]).unwrap();
        assert_eq!(override_of(&[list(Key::Arguments, &["--y"])]), kept);
    }

    /// A field the launcher ignores, a value the settings layer only warns
    /// about, is taken out before the merge, at the top level and in a rule
    /// alike, so its warning is the file's, logged with its path.
    #[test]
    fn an_ignored_field_in_a_rule_warns_with_the_file() {
        let (key, old) = retired_value(Severity::Warning);
        let support = TempSupport::new("ignored-rule");
        let executable = support.executable();
        support.game(
            "Game",
            &[
                executable_line(&executable),
                block("x", &[text(Key::Exe, "X.exe"), text(key, old)]),
            ]
            .concat(),
        );

        let file = read_game(support.path(), "Game").unwrap();

        let warning =
            rules::check_launch_wide(key, &rules::Value::One(old.to_owned())).unwrap_err();
        assert_eq!(
            file.raw.ignored.warnings,
            [format!("process rule `x`: {warning}")]
        );
        assert_eq!(
            file.raw.ignored.fields,
            [(Some(String::from("x")), key.to_string())]
        );
        assert!(
            !file
                .raw
                .rules
                .warnings
                .contains(&file.raw.ignored.warnings[0]),
            "{:?}",
            file.raw.rules.warnings
        );
        assert!(
            file.document["process"]["x"].get(key.as_str()).is_none(),
            "the merge must not see it"
        );
    }

    #[test]
    fn a_retired_key_leaves_the_file_untouched_for_editing() {
        let support = TempSupport::new("retired-untouched");
        let executable = support.executable();
        support.game("Game", &executable_line(&executable));
        let text = "[compat.\"Game.exe\"]\na = \"b\"\n";
        let path = support.defaults(text);

        assert!(Config::load_or_setup(support.path(), Settings::default()).is_err());

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            text,
            "the check runs before the top-up rewrites the file"
        );
    }

    #[test]
    fn merge_appends_lists_and_recurses_into_tables() {
        let mut base: DocumentMut =
            "list = [1, 2]\nscalar = \"base\"\n[table]\na = 1\nnested = { x = 1 }\n[[rows]]\nid = 1\n"
                .parse()
                .unwrap();
        let over: DocumentMut =
            "list = [3]\nscalar = \"over\"\nnew = true\n[table]\nb = 2\nnested = { y = 2 }\n[[rows]]\nid = 2\n"
                .parse()
                .unwrap();

        merge(base.as_item_mut(), over.into_item());

        let ints = |item: &Item| -> Vec<i64> {
            item.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_integer().unwrap())
                .collect()
        };
        assert_eq!(ints(&base["list"]), [1, 2, 3]);
        assert_eq!(base["scalar"].as_str(), Some("over"));
        assert_eq!(base["new"].as_bool(), Some(true));
        assert_eq!(base["table"]["a"].as_integer(), Some(1));
        assert_eq!(base["table"]["b"].as_integer(), Some(2));
        assert_eq!(base["table"]["nested"]["x"].as_integer(), Some(1));
        assert_eq!(base["table"]["nested"]["y"].as_integer(), Some(2));
        let ids: Vec<i64> = base["rows"]
            .as_array_of_tables()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_integer().unwrap())
            .collect();
        assert_eq!(ids, [1, 2]);
    }

    #[test]
    fn merge_replaces_a_value_of_another_type() {
        let mut base: DocumentMut = "dpi = 144\nlist = [1]\n[table]\na = 1\n".parse().unwrap();
        let over: DocumentMut = "dpi = \"auto\"\nlist = \"one\"\ntable = 2\n"
            .parse()
            .unwrap();

        merge(base.as_item_mut(), over.into_item());

        assert_eq!(base["dpi"].as_str(), Some("auto"));
        assert_eq!(base["list"].as_str(), Some("one"));
        assert_eq!(base["table"].as_integer(), Some(2));
    }

    #[test]
    fn an_executable_in_the_defaults_is_fatal() {
        let support = TempSupport::new("executable-in-defaults");
        let executable = support.executable();
        let defaults = support.defaults(&format!("{}\n", executable_line(&executable)));
        let existing = fs::read_to_string(&defaults).unwrap();
        support.game("Game", &executable_line(&executable));

        let error = Config::load_or_setup(support.path(), Settings::default()).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(
            error
                .to_string()
                .contains("`executable` belongs in a game file"),
            "unexpected error: {error}"
        );
        assert_eq!(fs::read_to_string(defaults).unwrap(), existing);
    }

    #[test]
    fn a_game_file_is_topped_up_from_its_template() {
        let support = TempSupport::new("game-top-up");
        let executable = support.executable();
        let game = support.game("Game", &executable_line(&executable));

        Config::load_or_setup(support.path(), Settings::default()).unwrap();

        let text = fs::read_to_string(game).unwrap();
        assert!(text.contains("# Wine Mac Driver retina mode"));
        assert!(text.contains("# arguments = "));
        for key in launch_wide_keys() {
            assert!(text.contains(key.docs_url()), "{key}: {text}");
        }
        assert!(sync_template(&text, &GAME_TEMPLATE).is_none());
        assert_eq!(
            text.lines()
                .filter(|line| is_option(line, "executable"))
                .count(),
            1
        );
    }
}
