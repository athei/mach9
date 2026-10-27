use std::str::FromStr;

use clap::{CommandFactory, Parser, error::ErrorKind};
use compatdb_core::{FieldError, Key, KeyKind, check_env_name};
use gamelauncher_core::games;

use crate::{
    config::{self, Settings},
    dpi::{self, Dpi},
    retina::{self, RetinaMode},
    rules::{self, parse_value},
    words,
};

/// The keys `--config` takes, see [`config::command_line_keys`].
fn keys() -> Vec<&'static str> {
    config::command_line_keys()
}

/// One `--config` value: a batch of assignments, each checked as it is read,
/// so a mistake is reported against the flag rather than found later.
#[derive(Clone, Debug)]
struct OverrideBatch(Settings);

impl FromStr for OverrideBatch {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        if raw.is_empty() {
            return Err(String::from("the override list must not be empty"));
        }

        let mut overrides = Settings::default();
        for (index, assignment) in split_assignments(raw).into_iter().enumerate() {
            let assignment = assignment.trim();
            if assignment.is_empty() {
                return Err(format!(
                    "config assignment {} is empty",
                    index.saturating_add(1)
                ));
            }
            let (key, value) = assignment.split_once('=').ok_or_else(|| {
                format!("config assignment `{assignment}` must have the form `key=value`")
            })?;
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() {
                return Err(String::from("config key must not be empty"));
            }
            assign(&mut overrides, key, value)?;
        }

        Ok(Self(overrides))
    }
}

/// Check one `key=value` assignment of a batch, and record its value in
/// `overrides`.
fn assign(overrides: &mut Settings, key: &str, value: &str) -> Result<(), String> {
    match key {
        "executable" => {
            let word = one_word(key, value)?;
            if word.is_empty() {
                return Err(String::from("`executable` must not be empty"));
            }
            overrides.executable = Some(word);
        }
        "arguments" => overrides.arguments = Some(word_list(key, value)?),
        "game" => {
            let word = one_word(key, value)?;
            if word.is_empty() {
                return Err(String::from("`game` must not be empty"));
            }
            if !games::valid_name(&word) {
                return Err(format!(
                    "`game` must be the name of a file under games/, got `{word}`"
                ));
            }
            overrides.game = Some(word);
        }
        "retina_mode" => {
            let word = one_word(key, value)?;
            overrides.retina_mode =
                Some(parse_value(key, &word, RetinaMode::parse, retina::VALUES)?);
        }
        "dpi" => {
            let word = one_word(key, value)?;
            overrides.dpi = Some(parse_value(key, &word, Dpi::parse, dpi::VALUES)?);
        }
        _ => {
            let Some(name) = key.strip_prefix("env.") else {
                return assign_rule_field(overrides, key, value);
            };
            if name.is_empty() {
                return Err(String::from("environment override name must not be empty"));
            }
            check_env_name(name).map_err(|e| rules::describe(key, &e))?;
            overrides.env.insert(name.to_owned(), one_word(key, value)?);
        }
    }
    Ok(())
}

/// Check one assignment of a field of the launcher's own rule, which the
/// settings layer reads, and record it. A command line is written for one
/// launch, so every error of the settings layer refuses it, also one that a
/// file only gets a warning for.
fn assign_rule_field(overrides: &mut Settings, key: &str, value: &str) -> Result<(), String> {
    let unknown = || {
        format!(
            "unknown config key `{key}`; supported keys are {}",
            words::listing(&keys(), "`")
        )
    };
    let field = match Key::from_setting(key) {
        Ok(field) if config::launch_wide_keys().contains(&field) => field,
        Ok(_) | Err(FieldError::UnknownKey(_)) => return Err(unknown()),
        Err(e) => return Err(rules::describe(key, &e)),
    };
    let value = if field.kind() == KeyKind::List {
        rules::Value::List(word_list(key, value)?)
    } else {
        rules::Value::One(one_word(key, value)?)
    };
    rules::check_launch_wide(field, &value).map_err(|e| rules::describe(key, &e))?;
    overrides.rule.insert(field, value);
    Ok(())
}

/// A path or an environment value is one word under the command-line quoting
/// rules, so one holding a space or a semicolon is written in quotes:
/// `env.SOME_CONFIG="a=1;b=2"`. Nothing at all is the empty
/// string.
fn one_word(key: &str, value: &str) -> Result<String, String> {
    let mut words = word_list(key, value)?;
    match words.len() {
        0 => Ok(String::new()),
        1 => Ok(words.remove(0)),
        _ => Err(format!(
            "`{key}` holds more than one word; quote a value that contains spaces"
        )),
    }
}

/// Split a batch at semicolons, except inside quotes. The quoting read here
/// is the one `words::split` applies to the values afterwards, so an argument
/// or an environment value may hold a semicolon.
fn split_assignments(raw: &str) -> Vec<&str> {
    let mut assignments = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;

    for (index, ch) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match quote {
            Some('"') => match ch {
                '"' => quote = None,
                '\\' => escaped = true,
                _ => {}
            },
            Some(_) => {
                if ch == '\'' {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                '\\' => escaped = true,
                ';' => {
                    assignments.push(&raw[start..index]);
                    start = index + ch.len_utf8();
                }
                _ => {}
            },
        }
    }
    assignments.push(&raw[start..]);
    assignments
}

/// Any number of words, for a list setting.
fn word_list(key: &str, value: &str) -> Result<Vec<String>, String> {
    words::split(value).map_err(|e| format!("`{key}` {e}"))
}

/// The long help for `--config`, which names the keys from [`keys`].
fn config_help() -> String {
    format!(
        "Override config values for this launch without writing them to the TOML file.\n\n\
         Supported keys: {}. Assignments are separated by semicolons. Values are split like a \
         shell command line: quotes group, a backslash escapes, so a value holding a space or a \
         semicolon is quoted. executable is the path of the Windows executable to run; it \
         bypasses the game files entirely, so it cannot be combined with game. arguments are \
         the words passed to it. game names a file under games/ to use instead of the selected \
         one. env.NAME is one word: env.SOME_CONFIG=\"a=1;b=2\". Repeat --config to provide \
         more assignments; later values win.",
        words::listing(&keys(), "")
    )
}

#[derive(Debug, Parser)]
#[command(
    name = "gamelauncher",
    about = "Launch the configured Windows game through the bundled Wine"
)]
struct Cli {
    /// Override config values for this launch without writing them to the TOML file.
    #[arg(
        long,
        value_name = "KEY=VALUE[;KEY=VALUE...]",
        long_help = config_help()
    )]
    config: Vec<OverrideBatch>,

    /// Open the New Executable window: ask for a Windows executable with its
    /// arguments and environment, save it as a new game under games/ and
    /// select it (or run it once), then start it in a fresh instance.
    #[arg(long, conflicts_with = "config")]
    add_game: bool,

    /// Check for an update now, ignoring the interval a normal launch
    /// respects, and start nothing. The Dock menu's Check for Updates opens
    /// the app this way.
    #[arg(long, conflicts_with_all = ["config", "add_game"])]
    check_updates: bool,

    /// Resume a launch after its updater process has exited. This applies
    /// only to this instance and never changes the saved update schedule.
    #[arg(long, hide = true, conflicts_with_all = ["add_game", "check_updates", "relaunch_after"])]
    skip_update_check: bool,

    /// Wait for the process to exit, then open this app again with the
    /// arguments after `--`. The launcher runs itself this way after a
    /// dialog, see `bundle::relaunch`.
    #[arg(long, hide = true, value_name = "PID", conflicts_with_all = ["config", "add_game", "check_updates"])]
    relaunch_after: Option<u32>,

    #[arg(last = true, hide = true, requires = "relaunch_after")]
    relaunch_args: Vec<String>,

    /// Copy wine's output from stdin, a pty, to stdout, the log, while the
    /// process with this pid runs, and kill it with SIGKILL on SIGTERM. The
    /// launcher starts itself this way before it execs into wine, see
    /// `launch::exec_via_pty`.
    #[arg(long, hide = true, value_name = "PID", conflicts_with_all = ["config", "add_game", "check_updates", "skip_update_check", "relaunch_after"])]
    log_pump: Option<i32>,
}

/// What the command line asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub overrides: Settings,
    pub add_game: bool,
    pub check_updates: bool,
    pub skip_update_check: bool,
    pub relaunch: Option<Relaunch>,
    /// The `--log-pump` mode: the pid of wine, the process to copy the
    /// output of.
    pub log_pump: Option<i32>,
    /// Original validated arguments, without argv[0], for a fresh instance.
    pub original: Vec<String>,
}

/// The `--relaunch-after` mode: the process to outlive and the arguments to
/// reopen the app with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relaunch {
    pub pid: u32,
    pub args: Vec<String>,
}

impl Cli {
    fn into_args(self) -> Result<Args, clap::Error> {
        let mut overrides = Settings::default();
        for batch in self.config {
            overrides.overlay(batch.0);
        }
        if overrides.executable.is_some() && overrides.game.is_some() {
            return Err(Self::command().error(
                ErrorKind::ArgumentConflict,
                "`--config` takes `executable` or `game`, not both: an executable stands in \
                 for the game file",
            ));
        }
        Ok(Args {
            overrides,
            add_game: self.add_game,
            check_updates: self.check_updates,
            skip_update_check: self.skip_update_check,
            relaunch: self.relaunch_after.map(|pid| Relaunch {
                pid,
                args: self.relaunch_args,
            }),
            log_pump: self.log_pump,
            original: Vec::new(),
        })
    }

    #[cfg(test)]
    fn into_overrides(self) -> Settings {
        self.into_args().unwrap().overrides
    }
}

pub fn parse() -> Args {
    parse_from(std::env::args_os()).unwrap_or_else(|error| error.exit())
}

/// Validate and retain the same argument list so a relaunch does not have to
/// reconstruct quoting or collapse repeated overrides.
pub fn parse_from<I, S>(args: I) -> Result<Args, clap::Error>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let command_line: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
    let mut parsed = Cli::try_parse_from(&command_line)?.into_args()?;
    parsed.original = command_line
        .into_iter()
        .skip(1)
        .map(|arg| {
            arg.into_string().map_err(|_| {
                clap::Error::raw(clap::error::ErrorKind::InvalidUtf8, "argument is not UTF-8")
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use compatdb_core::{DEFAULT_VALUE, Exe, Rule, Setting, Severity};

    use super::*;
    use crate::test_keys::{retired_env, retired_key};

    /// The launcher's own rule the overrides make.
    fn rule_of(overrides: &Settings) -> Rule {
        rules::launch_rule(&overrides.rule).unwrap()
    }

    /// `key=value` for every key of the rules the command line takes, with
    /// the crate's example of each, and the rule the settings layer makes of
    /// them.
    fn every_rule_field() -> (String, Rule) {
        let mut rule = Rule::new(Exe::Any);
        let assignments: Vec<String> = config::launch_wide_keys()
            .into_iter()
            .map(|key| {
                rule.set(key, key.example()).unwrap();
                format!("{key}={}", words::quote(key.example()))
            })
            .collect();
        (assignments.join(";"), rule)
    }

    #[test]
    fn parses_all_override_types_and_literal_values() {
        let (fields, rule) = every_rule_field();
        let cli = Cli::try_parse_from([
            "gamelauncher",
            "--config",
            &format!(
                " executable = /Games/Game.exe ; arguments = -console 'two words' ; \
                 retina_mode=true;dpi=off;{fields};env.EMPTY= "
            ),
        ])
        .unwrap();

        let overrides = cli.into_overrides();
        assert_eq!(
            Settings {
                rule: BTreeMap::new(),
                ..overrides.clone()
            },
            Settings {
                executable: Some(String::from("/Games/Game.exe")),
                arguments: Some(vec![String::from("-console"), String::from("two words")]),
                game: None,
                retina_mode: Some(RetinaMode::On),
                dpi: Some(Dpi::Fixed(96)),
                rule: BTreeMap::new(),
                env: BTreeMap::from([(String::from("EMPTY"), String::new())]),
            }
        );
        assert_eq!(rule_of(&overrides), rule);
    }

    /// A list takes any number of words.
    #[test]
    fn a_list_of_the_rules_takes_any_number_of_words() {
        let batch = OverrideBatch::from_str(&format!("{}='a=n' 'b=n'", Key::DllOverrides))
            .unwrap()
            .0;
        let mut expected = Rule::new(Exe::Any);
        expected
            .set_list(Key::DllOverrides, ["a=n", "b=n"])
            .unwrap();
        assert_eq!(rule_of(&batch), expected);
    }

    #[test]
    fn a_retired_environment_variable_names_the_setting_that_replaced_it() {
        let name = retired_env();
        let error = Cli::try_parse_from(["gamelauncher", "--config", &format!("env.{name}=x")])
            .expect_err("should be rejected")
            .to_string();
        let refused = check_env_name(name).unwrap_err();
        assert!(
            error.contains(&rules::describe(&format!("env.{name}"), &refused)),
            "{error}"
        );
        // Matched without regard to case, as the variables are.
        let lower = format!("env.{}=x", name.to_ascii_lowercase());
        assert!(Cli::try_parse_from(["gamelauncher", "--config", &lower]).is_err());
    }

    #[test]
    fn repeated_flags_and_assignments_use_the_last_value() {
        let cli = Cli::try_parse_from([
            "gamelauncher",
            "--config=retina_mode=true;env.MODE=first",
            "--config",
            "retina_mode=false;env.MODE=second",
        ])
        .unwrap();
        let overrides = cli.into_overrides();

        assert_eq!(overrides.retina_mode, Some(RetinaMode::Off));
        assert_eq!(overrides.env["MODE"], "second");
    }

    #[test]
    fn takes_a_quoted_executable_and_uses_the_last_one() {
        let cli = Cli::try_parse_from([
            "gamelauncher",
            "--config",
            r#"executable="/Games/My Game/Game.exe";retina_mode=true"#,
            "--config",
            "executable=/Games/Other.exe",
        ])
        .unwrap();
        let overrides = cli.into_overrides();

        assert_eq!(overrides.executable.as_deref(), Some("/Games/Other.exe"));
        assert_eq!(overrides.retina_mode, Some(RetinaMode::On));

        let batch = OverrideBatch::from_str(r#"executable="/Games/My Game/Game.exe""#)
            .unwrap()
            .0;
        assert_eq!(batch.executable.as_deref(), Some("/Games/My Game/Game.exe"));
    }

    /// Values are quoted the way a shell quotes them, so a semicolon inside
    /// quotes belongs to the value, not to the assignment list.
    #[test]
    fn keeps_semicolons_inside_quotes() {
        for raw in [
            r"arguments='semi;colon' name=value;env.MODE=after",
            r#"arguments="semi;colon" name=value;env.MODE=after"#,
            r"arguments=semi\;colon name=value;env.MODE=after",
        ] {
            let batch = OverrideBatch::from_str(raw).unwrap().0;
            assert_eq!(
                batch.arguments.as_deref(),
                Some(&[String::from("semi;colon"), String::from("name=value")][..]),
                "split too early: {batch:?}"
            );
            assert_eq!(batch.env["MODE"], "after");
        }

        let batch = OverrideBatch::from_str(r#"env.SOME_CONFIG="a=1;b=2";env.SPACED='two words'"#)
            .unwrap()
            .0;
        assert_eq!(batch.env["SOME_CONFIG"], "a=1;b=2");
        assert_eq!(batch.env["SPACED"], "two words");
    }

    /// The true-or-false settings of the rules the command line takes.
    fn switches() -> Vec<Key> {
        config::launch_wide_keys()
            .into_iter()
            .filter(|key| key.kind() == KeyKind::Scalar(compatdb_core::ValueKind::Bool))
            .collect()
    }

    #[test]
    fn a_true_or_false_setting_takes_one_word_in_any_case() {
        for (spelled, on) in [
            ("FALSE", false),
            ("False", false),
            ("'true'", true),
            ("\"true\"", true),
        ] {
            for key in switches() {
                let batch = OverrideBatch::from_str(&format!("{key}={spelled}"))
                    .unwrap()
                    .0;
                let scalar = key.scalar().unwrap();
                let expected = compatdb_core::ScalarValue::Bool(on);
                assert_eq!(rule_of(&batch).scalar(scalar), Setting::Set(expected));
            }
        }
        for key in switches() {
            for value in ["0", "yes", "on off", ""] {
                let assignment = format!("{key}={value}");
                assert!(
                    OverrideBatch::from_str(&assignment).is_err(),
                    "{assignment}"
                );
            }
        }
    }

    /// A command line is written for one launch, not kept from an older
    /// version, so a key the settings layer only warns about in a file is an
    /// error here.
    #[test]
    fn a_key_a_file_only_gets_a_warning_for_is_refused() {
        let old = retired_key(Severity::Warning);
        let error = OverrideBatch::from_str(&format!("{old}=false")).unwrap_err();
        let retired = Key::from_setting(old).unwrap_err();
        assert_eq!(error, rules::describe(old, &retired));
    }

    #[test]
    fn retina_mode_takes_auto_as_well_as_the_booleans() {
        let batch = OverrideBatch::from_str("retina_mode=auto").unwrap().0;
        assert_eq!(batch.retina_mode, Some(RetinaMode::Auto));
        let batch = OverrideBatch::from_str("retina_mode=false").unwrap().0;
        assert_eq!(batch.retina_mode, Some(RetinaMode::Off));
    }

    /// Every scalar is one word under the same quoting rules, so a quoted
    /// value means what it says and two words are an error.
    #[test]
    fn scalar_values_are_one_word() {
        let switch = switches()[0];
        let batch = OverrideBatch::from_str(&format!(
            "game='My Game';retina_mode=\"auto\";{switch}='true'"
        ))
        .unwrap()
        .0;
        assert_eq!(batch.game.as_deref(), Some("My Game"));
        assert_eq!(batch.retina_mode, Some(RetinaMode::Auto));
        let mut expected = Rule::new(Exe::Any);
        expected.set(switch, "true").unwrap();
        assert_eq!(rule_of(&batch), expected);
        for value in [
            String::from("game=My Game"),
            format!("{switch}=true false"),
            String::from("retina_mode='auto"),
        ] {
            assert!(OverrideBatch::from_str(&value).is_err(), "{value}");
        }
    }

    #[test]
    fn an_empty_game_says_so() {
        for value in ["game=", "game=''"] {
            let error = OverrideBatch::from_str(value).unwrap_err();
            assert_eq!(error, "`game` must not be empty", "{value}");
        }
    }

    #[test]
    fn an_empty_argument_list_is_no_arguments() {
        let batch = OverrideBatch::from_str("arguments=").unwrap().0;
        assert_eq!(batch.arguments, Some(Vec::new()));
    }

    #[test]
    fn rejects_invalid_override_values() {
        let (old_key, old_value) = crate::test_keys::retired_value(Severity::Warning);
        let invalid = config::launch_wide_keys()
            .into_iter()
            .filter(|key| key.kind() != KeyKind::List)
            .flat_map(|key| [format!("{key}=bogus"), format!("{key}=a b")]);
        for value in [
            String::new(),
            String::from(";"),
            String::from("retina_mode"),
            String::from("=true"),
            String::from("executable="),
            String::from("executable=/Games/Game.exe -console"),
            String::from("executable='/Games/Game.exe"),
            String::from("arguments=\"unterminated"),
            String::from("game="),
            String::from("game=.hidden"),
            String::from("game=a/b"),
            format!("{old_key}={old_value}"),
            String::from("dpi=huge"),
            String::from("retina_mode=0"),
            String::from("env.=value"),
            String::from("env.X=two words"),
            String::from("unknown=value"),
            String::from("retina_mode=true;"),
        ]
        .into_iter()
        .chain(invalid)
        {
            assert!(
                Cli::try_parse_from(["gamelauncher", "--config", &value]).is_err(),
                "accepted invalid override: {value:?}"
            );
        }
    }

    /// Keys and value words are matched without regard to case.
    #[test]
    fn keys_and_value_words_ignore_case() {
        let mut expected = Rule::new(Exe::Any);
        let assignments: Vec<String> = config::launch_wide_keys()
            .into_iter()
            .filter(|key| key.kind() != KeyKind::List)
            .map(|key| {
                let value = key.example();
                expected.set(key, value).unwrap();
                format!(
                    "{}={}",
                    key.as_str().to_ascii_uppercase(),
                    value.to_ascii_uppercase()
                )
            })
            .collect();
        let batch = OverrideBatch::from_str(&format!(
            "retina_mode=AUTO;dpi=Off;{}",
            assignments.join(";")
        ))
        .unwrap()
        .0;
        assert_eq!(rule_of(&batch), expected);
        assert_eq!(batch.retina_mode, Some(RetinaMode::Auto));
        assert_eq!(batch.dpi, Some(Dpi::Fixed(96)));
    }

    /// Every setting of compatdb.so's rules that holds one value takes
    /// `default`, which hands it back to upstream Wine; the launcher's own
    /// settings do not.
    #[test]
    fn the_settings_of_the_rules_take_default() {
        for key in config::launch_wide_keys() {
            let Some(scalar) = key.scalar() else {
                continue;
            };
            let upper = DEFAULT_VALUE.to_ascii_uppercase();
            let batch = OverrideBatch::from_str(&format!("{key}='{upper}'"))
                .unwrap()
                .0;
            assert_eq!(rule_of(&batch).scalar(scalar), Setting::Reset, "{key}");
        }
        for key in ["retina_mode", "dpi"] {
            let value = format!("{key}={DEFAULT_VALUE}");
            assert!(OverrideBatch::from_str(&value).is_err(), "{value}");
        }
    }

    /// A bad value fails the whole command line, with the flag, the key and
    /// what the key takes in the message.
    #[test]
    fn a_bad_value_is_refused_on_the_command_line() {
        let refused = |key: Key, text: &str| {
            let value = rules::Value::One(text.to_owned());
            let error = rules::check_launch_wide(key, &value).unwrap_err();
            rules::describe(key.as_str(), &error)
        };
        let env = retired_env();
        let word_key = *config::launch_wide_keys()
            .iter()
            .find(|key| key.kind() == KeyKind::Scalar(compatdb_core::ValueKind::Word))
            .expect("a setting of words");
        for (assignment, expected) in [
            (format!("{word_key}=bogus"), refused(word_key, "bogus")),
            (
                format!("{}=yes", switches()[0]),
                refused(switches()[0], "yes"),
            ),
            (
                format!("env.{env}=x"),
                rules::describe(&format!("env.{env}"), &check_env_name(env).unwrap_err()),
            ),
            (
                String::from("dpi=huge"),
                String::from("`dpi` must be `auto`, `off` or `0`"),
            ),
        ] {
            let assignment = assignment.as_str();
            let error = parse_from(["gamelauncher", "--config", assignment]).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ValueValidation, "{assignment}");
            let message = error.to_string();
            assert!(message.contains("--config"), "{message}");
            assert!(message.contains(&expected), "{message}");
        }
    }

    #[test]
    fn an_executable_and_a_game_together_are_refused() {
        for argv in [
            &[
                "gamelauncher",
                "--config",
                "executable=/Games/x.exe;game=Game",
            ][..],
            &[
                "gamelauncher",
                "--config",
                "game=Game",
                "--config",
                "executable=/Games/x.exe",
            ],
        ] {
            let error = parse_from(argv.iter().copied()).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
            assert!(error.to_string().contains("not both"), "{error}");
        }
    }

    /// The template's header lists the keys for someone reading the file; it
    /// has to be the same list the parser takes.
    #[test]
    fn the_template_lists_the_supported_keys() {
        let prose: Vec<&str> = config::defaults_template()
            .lines()
            .map(|line| line.trim_start_matches('#').trim())
            .collect();
        let prose = prose.join(" ");
        let expected = format!("Supported keys are {}.", words::listing(&keys(), ""));
        assert!(prose.contains(&expected), "expected: {expected}");
    }

    #[test]
    fn rejects_unknown_arguments_and_missing_values() {
        assert!(Cli::try_parse_from(["gamelauncher", "unexpected"]).is_err());
        assert!(Cli::try_parse_from(["gamelauncher", "--unknown"]).is_err());
        assert!(Cli::try_parse_from(["gamelauncher", "--config"]).is_err());
    }

    #[test]
    fn add_game_stands_alone() {
        let args = Cli::try_parse_from(["gamelauncher", "--add-game"])
            .unwrap()
            .into_args()
            .unwrap();
        assert!(args.add_game);
        assert_eq!(args.overrides, Settings::default());
        assert!(
            Cli::try_parse_from(["gamelauncher", "--add-game", "--config", "retina_mode=true"])
                .is_err()
        );
    }

    #[test]
    fn relaunch_after_takes_the_arguments_after_a_double_dash() {
        let args = Cli::try_parse_from([
            "gamelauncher",
            "--relaunch-after",
            "4242",
            "--",
            "--config",
            "executable=/Games/x.exe",
        ])
        .unwrap()
        .into_args()
        .unwrap();
        assert_eq!(
            args.relaunch,
            Some(Relaunch {
                pid: 4242,
                args: vec![
                    String::from("--config"),
                    String::from("executable=/Games/x.exe")
                ],
            })
        );
        assert_eq!(args.overrides, Settings::default());

        let bare = Cli::try_parse_from(["gamelauncher", "--relaunch-after", "1"])
            .unwrap()
            .into_args()
            .unwrap();
        assert_eq!(
            bare.relaunch,
            Some(Relaunch {
                pid: 1,
                args: Vec::new()
            })
        );
        assert!(Cli::try_parse_from(["gamelauncher", "--", "x"]).is_err());
        assert!(
            Cli::try_parse_from(["gamelauncher", "--relaunch-after", "1", "--add-game"]).is_err()
        );
    }

    #[test]
    fn help_documents_ephemeral_overrides() {
        let error = Cli::try_parse_from(["gamelauncher", "--help"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayHelp);
        let help = error.to_string();
        assert!(help.contains("without writing them to the TOML file"));
        assert!(help.contains("env.NAME"));
        assert!(help.contains("--add-game"));
        assert!(help.contains("arguments are the words passed to it"));
        assert!(help.contains("later values win"));
        assert!(help.contains(&words::listing(&keys(), "")));
        assert!(!help.contains("relaunch"));
        assert!(!help.contains("skip-update-check"));
        assert!(!help.contains("log-pump"));
    }

    #[test]
    fn log_pump_takes_a_pid_and_stands_alone() {
        let args = parse_from(["gamelauncher", "--log-pump", "4242"]).unwrap();
        assert_eq!(args.log_pump, Some(4242));
        assert_eq!(args.overrides, Settings::default());
        for mode in [
            "--add-game",
            "--check-updates",
            "--skip-update-check",
            "--relaunch-after=42",
            "--config=game=Game",
        ] {
            let error = parse_from(["gamelauncher", "--log-pump", "4242", mode]).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ArgumentConflict, "{mode}");
        }
        assert!(parse_from(["gamelauncher", "--log-pump"]).is_err());
        assert_eq!(parse_from(["gamelauncher"]).unwrap().log_pump, None);
    }

    #[test]
    fn update_continuation_preserves_literal_overrides() {
        let argv = [
            "gamelauncher",
            "--config=executable='/Games/Setup Kit/setup.exe';retina_mode=true",
            "--config",
            "arguments='two words' '' 'name=value;still-one';env.MODE=first",
            "--config",
            "retina_mode=false;env.MODE='second; with spaces'",
            "--",
        ];
        let original = parse_from(argv).unwrap();
        assert_eq!(original.original, argv[1..]);

        let mut continuation = original.original.clone();
        continuation.insert(0, String::from("--skip-update-check"));
        let resumed =
            parse_from(std::iter::once(String::from("gamelauncher")).chain(continuation.clone()))
                .unwrap();
        assert!(resumed.skip_update_check);
        assert_eq!(resumed.overrides, original.overrides);
        assert_eq!(resumed.original, continuation);
        assert_eq!(resumed.overrides.retina_mode, Some(RetinaMode::Off));
        assert_eq!(
            resumed.overrides.arguments.unwrap(),
            ["two words", "", "name=value;still-one"]
        );
        assert_eq!(resumed.overrides.env["MODE"], "second; with spaces");
    }

    #[test]
    fn setup_continuation_keeps_the_update_bypass() {
        let original = parse_from([
            "gamelauncher",
            "--skip-update-check",
            "--config",
            "game=Game;arguments=",
        ])
        .unwrap();
        let resumed = parse_from(
            std::iter::once(String::from("gamelauncher")).chain(original.original.clone()),
        )
        .unwrap();
        assert_eq!(resumed, original);
        assert_eq!(resumed.overrides.arguments, Some(Vec::new()));
    }

    #[test]
    fn update_bypass_is_only_a_normal_launch_option() {
        for mode in ["--add-game", "--check-updates", "--relaunch-after=42"] {
            let error = parse_from(["gamelauncher", "--skip-update-check", mode]).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
        }
        let helper = parse_from([
            "gamelauncher",
            "--relaunch-after",
            "42",
            "--",
            "--skip-update-check",
            "--config=game=Game",
        ])
        .unwrap();
        assert!(!helper.skip_update_check);
        assert_eq!(
            helper.relaunch.unwrap().args,
            ["--skip-update-check", "--config=game=Game"]
        );
    }
}
