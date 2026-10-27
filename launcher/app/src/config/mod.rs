//! The config: the defaults in `gamelauncher.toml` with the selected game
//! file laid over them and the command line's overrides on top, read into
//! the settings for one launch.

mod executable;
mod files;
mod new_game;
mod selection;
mod template;
#[cfg(test)]
mod test_support;

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
};

use compatdb_core::{FieldError, Key, Severity, check_env_name};
use log::warn;
use toml_edit::{DocumentMut, Item, Value};

use self::{
    executable::{resolve_cli_executable, resolve_game_executable},
    files::{RETIRED_KEYS, merged, read_defaults, read_game},
    selection::choose_game,
};
pub use self::{new_game::add_game, selection::needs_first_game};
use crate::{
    bundle,
    dpi::{self, Dpi},
    retina::{self, RetinaMode},
    rules::{self, Ignored, LaunchRules},
    words,
};

/// The settings a launch can be given, each unset until a file or the command
/// line sets it. A config file's top level reads into one, and so does each
/// `--config` batch; [`Settings::overlay`] lays one over another, a later
/// batch over an earlier one and the command line over the files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// Path of the Windows executable, as written. On the command line it
    /// stands in for the game file altogether.
    pub executable: Option<String>,
    /// Arguments for the executable.
    pub arguments: Option<Vec<String>>,
    /// Name of the game file to use instead of the selected one. Only the
    /// command line sets it.
    pub game: Option<String>,
    pub retina_mode: Option<RetinaMode>,
    pub dpi: Option<Dpi>,
    /// The fields of the launcher's own rule, which matches every process,
    /// by key: the settings of `compatdb.so`'s rules at the top level, each
    /// checked already (see [`launch_wide_keys`]).
    pub rule: BTreeMap<Key, rules::Value>,
    pub env: BTreeMap<String, String>,
}

impl Settings {
    /// Lay `over` on top of these. A value it sets replaces this one, a list
    /// included: an override is for one launch, so it has to be able to take
    /// an entry away. The environment is merged name by name.
    pub fn overlay(&mut self, over: Self) {
        let Self {
            executable,
            arguments,
            game,
            retina_mode,
            dpi,
            rule,
            env,
        } = over;
        self.executable = executable.or_else(|| self.executable.take());
        self.arguments = arguments.or_else(|| self.arguments.take());
        self.game = game.or_else(|| self.game.take());
        self.retina_mode = retina_mode.or(self.retina_mode);
        self.dpi = dpi.or(self.dpi);
        self.rule.extend(rule);
        self.env.extend(env);
    }
}

/// What a config document holds, checked: its top level read into
/// [`Settings`], and its `[process]` rules. Each file is read into one on its
/// own, so an error names the file with the mistake, and the merged files are
/// read once more for the values the launch uses.
#[derive(Debug, Default)]
struct Raw {
    settings: Settings,
    rules: rules::Rules,
    /// The fields the document holds that the launcher ignores, at the top
    /// level or in a rule, each with its warning. Logged when a file is read
    /// on its own, with the file's path, and taken out of the file before it
    /// is merged.
    ignored: Ignored,
}

/// The top-level keys of the launcher's own settings that come before the
/// keys of `compatdb.so`'s rules, in the order the game template declares
/// them.
const LEADING_SETTINGS: &[&str] = &["executable", "arguments", "retina_mode", "dpi"];

/// The same for the tables, which come after them.
const TRAILING_SETTINGS: &[&str] = &["env", "process"];

/// The keys of `compatdb.so`'s rules that the top level and the command line
/// take: those a rule for every process holds, but `arguments` and `env`,
/// which are the launcher's own settings there.
pub fn launch_wide_keys() -> Vec<Key> {
    let own: Vec<&str> = LEADING_SETTINGS
        .iter()
        .chain(TRAILING_SETTINGS)
        .copied()
        .collect();
    rules::launch_wide_keys(&own)
}

/// The keys `--config` takes, in the order the help, the errors and the
/// template's header list them: the launcher's own settings, then the keys
/// of `compatdb.so`'s rules that the launcher's own rule takes, then the
/// environment.
pub fn command_line_keys() -> Vec<&'static str> {
    let rule = launch_wide_keys();
    ["executable", "arguments", "game", "retina_mode", "dpi"]
        .into_iter()
        .chain(rule.iter().map(|key| key.as_str()))
        .chain(["env.NAME"])
        .collect()
}

/// The expanded template of `gamelauncher.toml`, for the tests of the command
/// line.
#[cfg(test)]
pub fn defaults_template() -> &'static str {
    &files::TEMPLATE
}

/// The keys a config file's top level takes. `gamelauncher.toml` takes all
/// but `executable`.
fn settings() -> Vec<&'static str> {
    let keys = launch_wide_keys();
    LEADING_SETTINGS
        .iter()
        .copied()
        .chain(keys.iter().map(|key| key.as_str()))
        .chain(TRAILING_SETTINGS.iter().copied())
        .collect()
}

/// Which kind of file a document is, for the keys it takes. The merged files
/// read as a game file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Defaults,
    Game,
}

impl Source {
    /// The top-level keys this kind of file takes.
    fn keys(self) -> Vec<&'static str> {
        settings()
            .into_iter()
            .filter(|key| self == Self::Game || *key != "executable")
            .collect()
    }
}

impl Raw {
    /// Read every key, refusing one that is not a setting and a value of the
    /// wrong type, so nothing a file says is quietly left out of the launch.
    /// A field the settings layer only warns about is the exception: it is
    /// skipped with a warning (see [`Ignored`]). The settings layer reads a
    /// key of the rules in any case, so two spellings of one key in a file
    /// are refused, as TOML refuses the same spelling twice.
    fn from_document(document: &DocumentMut, source: Source) -> Result<Self, String> {
        let mut raw = Self::default();
        let mut seen = Vec::new();
        for (key, item) in document.iter() {
            let settings = &mut raw.settings;
            match key {
                "executable" if source == Source::Game => {
                    settings.executable = Some(rules::string(item, key)?);
                }
                "arguments" => settings.arguments = Some(rules::strings(item, key)?),
                "retina_mode" => {
                    let mode = parse_word(item, key, RetinaMode::parse, retina::VALUES)?;
                    settings.retina_mode = Some(mode);
                }
                "dpi" => settings.dpi = Some(parse_word(item, key, Dpi::parse, dpi::VALUES)?),
                "env" => settings.env = launch_env(item, key)?,
                "process" => raw.rules = rules::parse(item, &mut raw.ignored)?,
                _ => raw.read_rule_field(key, item, source, &mut seen)?,
            }
        }
        Ok(raw)
    }

    /// Read a top-level `key` that is no setting of the launcher's own: a
    /// field of the launcher's own rule, which the settings layer checks.
    /// `seen` holds the keys of the rules read so far.
    fn read_rule_field(
        &mut self,
        key: &str,
        item: &Item,
        source: Source,
        seen: &mut Vec<Key>,
    ) -> Result<(), String> {
        let field = match Key::from_setting(key) {
            Ok(field) if launch_wide_keys().contains(&field) => field,
            Ok(_) | Err(FieldError::UnknownKey(_)) => return Err(unknown_key(key, source)),
            Err(e) if e.severity() == Severity::Warning => {
                self.ignored.push(None, key, &e);
                return Ok(());
            }
            Err(e) => return Err(rules::describe(key, &e)),
        };
        if seen.contains(&field) {
            return Err(rules::describe(key, &FieldError::Duplicate(field)));
        }
        seen.push(field);
        let value = rules::read(item, field, key)?;
        match rules::check_launch_wide(field, &value) {
            Ok(()) => {
                self.settings.rule.insert(field, value);
            }
            Err(e) if e.severity() == Severity::Warning => self.ignored.push(None, key, &e),
            Err(e) => return Err(rules::describe(key, &e)),
        }
        Ok(())
    }
}

/// The top-level `[env]` table, which goes into the environment Wine starts
/// with. A variable that a setting of `compatdb.so`'s rules replaced is
/// refused, whatever its case: Wine would no longer read it.
fn launch_env(item: &Item, key: &str) -> Result<BTreeMap<String, String>, String> {
    let env = rules::env_pairs(item, key)?;
    for (name, _) in &env {
        check_env_name(name).map_err(|e| rules::describe(&format!("{key}.{name}"), &e))?;
    }
    Ok(env.into_iter().collect())
}

/// A setting TOML may hold as a word or as a plain value, `retina_mode =
/// "auto"` or `false`, `dpi = "off"` or `144`. Either form is read as text by
/// the setting's parser, so both are refused with the same error.
fn parse_word<T>(
    item: &Item,
    key: &str,
    parse: impl FnOnce(&str) -> Option<T>,
    valid: &str,
) -> Result<T, String> {
    let text = match item.as_value() {
        Some(Value::String(text)) => text.value().clone(),
        Some(Value::Boolean(value)) => value.value().to_string(),
        Some(Value::Integer(value)) => value.value().to_string(),
        _ => {
            return Err(format!(
                "`{key}` must be {valid}, got {}",
                rules::shown(item)
            ));
        }
    };
    rules::parse_value(key, &text, parse, valid)
}

/// The error for a top-level key that is not a setting: what replaced it, for
/// a key that used to be one, or else the keys the file takes.
fn unknown_key(key: &str, source: Source) -> String {
    if let Some((_, replacement)) = RETIRED_KEYS.iter().find(|(retired, _)| *retired == key) {
        return format!("`{key}` is no longer read; use {replacement}");
    }
    format!(
        "unknown setting `{key}`; this file takes {}",
        words::listing(&source.keys(), "`")
    )
}

/// The settings for one launch: the launch-wide ones as values, and the
/// per-process ones as the rule table that goes to `compatdb.so`.
#[derive(Debug)]
pub struct Config {
    /// Absolute, UTF-8, with a file name, see `resolve_exe`.
    executable: PathBuf,
    /// Passed verbatim to the Windows executable.
    arguments: Vec<String>,
    /// Name of the game file in use; `None` when `--config executable=` stood
    /// in for one.
    pub game: Option<String>,
    /// Wine Mac Driver retina mode. `auto` by default, which is on when the
    /// main display is high-density; `false` renders at 1x. Applied to the
    /// prefix registry on every launch. Prefix-wide only: `winemac.drv` reads
    /// it from the shared key on purpose, so that every process agrees on the
    /// display modes.
    pub retina_mode: RetinaMode,
    /// The Windows system DPI, `auto` by default, which follows retina mode so
    /// Windows UI comes out the size of macOS UI. Prefix-wide like retina
    /// mode: win32u reads it from the shared `Control Panel\Desktop` key.
    pub dpi: Dpi,
    /// The fields of the launcher's own rule as the config gives them, for
    /// the log; the rule itself is the first of `rules` when it sets anything.
    pub launch_wide: BTreeMap<Key, rules::Value>,
    /// Environment overrides for the whole launch, applied last so they win
    /// over launcher defaults.
    pub env: BTreeMap<String, String>,
    /// The per-process rules this config contributes (the launcher's own rule
    /// for the top-level settings, then every `[process]` block that matches
    /// a process), exported to `compatdb.so`, which folds them with its
    /// built-in rules.
    pub rules: LaunchRules,
}

impl Config {
    /// Read the defaults and the selected game file, creating what is missing
    /// from the embedded templates and asking for the game executable when the
    /// game file names none or a stale one. `overrides` are the command line's,
    /// already checked.
    pub fn load_or_setup(support_dir: &Path, mut overrides: Settings) -> io::Result<Self> {
        let defaults = read_defaults(support_dir)?;
        let home = bundle::home_dir()?;

        // A `--config` executable stands in for the game file altogether.
        let (game, mut raw, executable) = if let Some(configured) = overrides.executable.take() {
            let executable = resolve_cli_executable(&configured, &home)?;
            (None, defaults.raw, executable)
        } else {
            let name = choose_game(support_dir, overrides.game.as_deref())?;
            let file = read_game(support_dir, &name)?;
            let raw = merged(defaults, &file)?;
            let executable = resolve_game_executable(
                raw.settings.executable.as_deref(),
                &file.path,
                &file.text,
                &home,
            )?;
            (Some(name), raw, executable)
        };
        for warning in &raw.rules.warnings {
            warn!("{warning}");
        }

        raw.settings.overlay(overrides);
        let Settings {
            executable: _,
            game: _,
            arguments,
            retina_mode,
            dpi,
            rule,
            env,
        } = raw.settings;
        let own =
            rules::launch_rule(&rule).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let rules = rules::table(own, raw.rules.rules);

        Ok(Self {
            executable,
            arguments: arguments.unwrap_or_default(),
            game,
            retina_mode: retina_mode.unwrap_or_default(),
            dpi: dpi.unwrap_or_default(),
            launch_wide: rule,
            env,
            rules,
        })
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    /// Wine's working directory: the directory holding the executable.
    pub fn game_dir(&self) -> &Path {
        self.executable
            .parent()
            .expect("executable is absolute and has a file name")
    }

    /// File name of the executable, passed to wine as its only argument. Wine
    /// resolves it against the working directory.
    pub fn exe_name(&self) -> &str {
        self.executable
            .file_name()
            .and_then(OsStr::to_str)
            .expect("executable has a UTF-8 file name")
    }

    pub fn display_name(&self) -> &str {
        self.game.as_deref().unwrap_or_else(|| {
            self.executable
                .file_stem()
                .and_then(OsStr::to_str)
                .unwrap_or_else(|| self.exe_name())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use compatdb_core::{
        D3d9, DEFAULT_VALUE, DllOverride, Dxgi, Exe, Fingerprint, OpenGl, Rule, Setting, Table,
        VersionInfo, Wined3d,
    };

    use super::*;
    use crate::{
        config::{template::set_arguments, test_support::*},
        test_keys::{flag, list, retired_key, retired_value, text},
    };

    /// A `[process."<name>"]` block holding `lines`.
    fn block(name: &str, lines: &[String]) -> String {
        format!("\n[process.\"{name}\"]\n{}", lines.concat())
    }

    /// The rule of the config's block called `name`.
    fn named<'a>(config: &'a Config, name: &str) -> &'a Rule {
        let index = config
            .rules
            .names
            .iter()
            .position(|n| n.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no rule named {name}: {:?}", config.rules.names));
        &config.rules.table.rules()[index]
    }

    /// The launcher's own rule, which carries the top-level settings, or one
    /// that sets nothing when the table leaves it out.
    fn own(config: &Config) -> Rule {
        config
            .rules
            .names
            .iter()
            .position(Option::is_none)
            .map_or_else(Rule::default, |index| {
                config.rules.table.rules()[index].clone()
            })
    }

    /// One top-level setting as the command line gives it.
    fn one(key: Key, text: &str) -> Settings {
        Settings {
            rule: BTreeMap::from([(key, rules::Value::One(text.to_owned()))]),
            ..Settings::default()
        }
    }

    /// The error the settings layer gives for `text` in the field `key` of the
    /// launcher's own rule.
    fn refused(key: Key, text: &str) -> String {
        let value = rules::Value::One(text.to_owned());
        rules::describe(
            key.as_str(),
            &rules::check_launch_wide(key, &value).unwrap_err(),
        )
    }

    fn exe(name: &str) -> Exe {
        Exe::new(name).unwrap()
    }

    #[test]
    fn a_game_overrides_the_defaults_value_by_value() {
        let support = TempSupport::new("merge");
        let executable = support.executable();
        support.defaults(&format!(
            "{}\nretina_mode = false\n\n[env]\nKEEP = \"defaults\"\nCHANGE = \"defaults\"\n",
            flag(Key::MetalHud, true)
        ));
        support.game(
            "Game",
            &format!(
                "{}{}\n[env]\nCHANGE = \"game\"\nADDED = \"game\"\n",
                executable_line(&executable),
                flag(Key::MetalHud, false)
            ),
        );

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        assert_eq!(config.executable(), executable);
        assert_eq!(config.game.as_deref(), Some("Game"));
        assert_eq!(config.display_name(), "Game");
        assert_eq!(own(&config).metal_hud(), Setting::Set(false));
        assert_eq!(config.retina_mode, RetinaMode::Off);
        assert_eq!(config.env["KEEP"], "defaults");
        assert_eq!(config.env["CHANGE"], "game");
        assert_eq!(config.env["ADDED"], "game");
    }

    #[test]
    fn every_setting_can_be_set_globally_or_for_one_process() {
        let support = TempSupport::new("scopes");
        let executable = support.executable();
        support.defaults(
            &[
                text(Key::Dxgi, Dxgi::Wined3d),
                text(Key::D3d9, D3d9::Wined3d),
                text(Key::Wined3d, Wined3d::Gl),
                list(Key::DllOverrides, &["a=n"]),
                list(Key::DebugChannels, &["+a"]),
            ]
            .concat(),
        );
        support.game(
            "Game",
            &[
                executable_line(&executable),
                text(Key::Dxgi, Dxgi::Dxmt),
                list(Key::DllOverrides, &["b=n"]),
                block(
                    "tool",
                    &[
                        text(Key::Exe, "Tool.exe"),
                        text(Key::Company, "Acme"),
                        text(Key::Dxgi, Dxgi::Wined3d),
                        text(Key::D3d9, D3d9::Mtld3d),
                        text(Key::Wined3d, Wined3d::Vulkan),
                        list(Key::DebugChannels, &["+b"]),
                        list(Key::Arguments, &["--x"]),
                        flag(Key::MetalHud, true),
                    ],
                ),
            ]
            .concat(),
        );

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        // A scalar is replaced, a list keeps the defaults' entries first.
        let global = own(&config);
        assert_eq!(global.dxgi(), Setting::Set(Dxgi::Dxmt));
        assert_eq!(
            global.d3d9(),
            Setting::Set(D3d9::Wined3d),
            "the defaults' value survives"
        );
        assert_eq!(global.wined3d(), Setting::Set(Wined3d::Gl));
        assert_eq!(global.dll_overrides(), ["a=n", "b=n"]);
        assert_eq!(global.debug_channels(), ["+a"]);

        // The launch-wide settings travel as the rule matching everything,
        // and the process rule is more specific, so it wins for its own executable.
        let acme = VersionInfo {
            company: String::from("Acme Inc"),
            ..VersionInfo::default()
        };
        let tool = config.rules.table.resolve("Tool.exe", &acme);
        assert_eq!(tool.dxgi(), Some(Dxgi::Wined3d));
        assert_eq!(tool.d3d9(), Some(D3d9::Mtld3d));
        assert_eq!(tool.wined3d(), Some(Wined3d::Vulkan));
        let other = config
            .rules
            .table
            .resolve("Other.exe", &VersionInfo::default());
        assert_eq!(other.dxgi(), Some(Dxgi::Dxmt));
        assert_eq!(other.d3d9(), Some(D3d9::Wined3d));
        assert_eq!(other.wined3d(), Some(Wined3d::Gl));
        let rule = named(&config, "tool");
        assert_eq!(rule.arguments(), ["--x"]);
        assert_eq!(rule.debug_channels(), ["+b"]);
        assert_eq!(rule.metal_hud(), Setting::Set(true));
    }

    #[test]
    fn a_game_rule_extends_a_rule_from_the_defaults() {
        let support = TempSupport::new("rule-merge");
        let executable = support.executable();
        support.defaults(&block(
            "shared",
            &[
                text(Key::Exe, "Helper.exe"),
                text(Key::Company, "Acme"),
                list(Key::DllOverrides, &["a="]),
            ],
        ));
        support.game(
            "Game",
            &[
                executable_line(&executable),
                block("shared", &[list(Key::DllOverrides, &["b="])]),
            ]
            .concat(),
        );

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        let rule = named(&config, "shared");
        assert_eq!(config.rules.table.rules().len(), 1);
        assert_eq!(*rule.exe(), "Helper.exe");
        assert_eq!(rule.company().map(Fingerprint::as_str), Some("Acme"));
        assert_eq!(rule.dll_overrides(), ["a=", "b="]);
    }

    #[test]
    fn an_unknown_top_level_key_is_fatal_and_lists_the_settings() {
        let support = TempSupport::new("unknown-key");
        let executable = support.executable();
        let game = support.game(
            "Game",
            &format!("{}bogus = true\n", executable_line(&executable)),
        );

        let error = Config::load_or_setup(support.path(), Settings::default())
            .unwrap_err()
            .to_string();

        assert_eq!(
            error,
            format!(
                "{}: unknown setting `bogus`; this file takes {}",
                game.display(),
                words::listing(&Source::Game.keys(), "`")
            )
        );
        assert!(error.contains("`executable`, `arguments`"), "{error}");
        assert!(error.ends_with("`env` and `process`"), "{error}");

        // The defaults take no `executable`, so their list does not offer it.
        fs::write(&game, executable_line(&executable)).unwrap();
        let defaults = support.defaults("bogus = true\n");
        let error = Config::load_or_setup(support.path(), Settings::default())
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with(&format!(
                "{}: unknown setting `bogus`; this file takes `arguments`,",
                defaults.display()
            )),
            "{error}"
        );
        assert!(!error.contains("`executable`"), "{error}");
    }

    /// A matcher is a field of a rule for some programs, not a setting of
    /// the launch.
    #[test]
    fn a_matcher_at_the_top_level_is_an_unknown_setting() {
        let document: DocumentMut = text(Key::Exe, "Game.exe").parse().unwrap();
        let error = Raw::from_document(&document, Source::Game).unwrap_err();
        assert!(
            error.starts_with(&format!("unknown setting `{}`", Key::Exe)),
            "{error}"
        );
        assert!(launch_wide_keys().iter().all(|key| *key != Key::Exe));
    }

    /// Two spellings of one key of the rules in one file would leave which
    /// one applies to the order of the lines, so they are refused, at the top
    /// level and in a rule.
    #[test]
    fn a_key_of_the_rules_twice_in_one_file_is_fatal() {
        let key = Key::Dxgi;
        let upper = key.as_str().to_ascii_uppercase();
        let duplicate = |written: &str| rules::describe(written, &FieldError::Duplicate(key));
        let twice = [text(key, Dxgi::Dxmt), text(&upper, Dxgi::Wined3d)].concat();
        let document: DocumentMut = twice.parse().unwrap();
        let error = Raw::from_document(&document, Source::Game).unwrap_err();
        assert_eq!(error, duplicate(&upper));
        let document: DocumentMut = block(
            "a",
            &[
                text(Key::Exe, "a.exe"),
                text(&upper, Dxgi::Dxmt),
                text(key, Dxgi::Wined3d),
            ],
        )
        .parse()
        .unwrap();
        let error = Raw::from_document(&document, Source::Game).unwrap_err();
        assert_eq!(
            error,
            format!("process rule `a`: {}", duplicate(key.as_str()))
        );
    }

    #[test]
    fn a_top_level_value_of_the_wrong_type_is_fatal() {
        let switch = |key: Key, got: &str| {
            format!("`{key}` must be `true`, `false` or `\"{DEFAULT_VALUE}\"`, got `{got}`")
        };
        let lists = |key: Key| format!("`{key}` must be a string or a list of strings");
        let string = |key: Key, got: &str| format!("`{key}` must be a string, got `{got}`");
        for (line, expected) in [
            (text(Key::MetalHud, true), switch(Key::MetalHud, "\"true\"")),
            (format!("{} = 1", Key::DpiAware), switch(Key::DpiAware, "1")),
            (
                String::from("arguments = 1"),
                String::from("`arguments` must be a string or a list of strings, got `1`"),
            ),
            (
                format!("{} = [\"a=n\", 2]", Key::DllOverrides),
                lists(Key::DllOverrides),
            ),
            (
                list(Key::Dxgi, &[Dxgi::Dxmt.as_str()]),
                string(Key::Dxgi, &format!("[\"{}\"]", Dxgi::Dxmt)),
            ),
            (flag(Key::Wined3d, true), string(Key::Wined3d, "true")),
            (format!("{} = 1", Key::OpenGl), string(Key::OpenGl, "1")),
            (text(Key::OpenGl, "bogus"), refused(Key::OpenGl, "bogus")),
            (
                String::from("retina_mode = [true]"),
                String::from("`retina_mode` must be `auto`, `true` or `false`, got `[true]`"),
            ),
            (
                String::from("env = \"A=1\""),
                String::from("`env` must be a table"),
            ),
            (
                String::from("env.A = 1"),
                String::from("`env.A` must be a string, got `1`"),
            ),
            (
                String::from("process = 1"),
                String::from("`process` must be a table of rules"),
            ),
        ] {
            let support = TempSupport::new("wrong-type");
            let executable = support.executable();
            support.game("Game", &executable_line(&executable));
            let defaults = support.defaults(&format!("{line}\n"));

            let error = Config::load_or_setup(support.path(), Settings::default())
                .unwrap_err()
                .to_string();

            assert!(
                error.starts_with(&format!("{}: ", defaults.display())),
                "{line} gave {error}"
            );
            assert!(error.contains(&expected), "{line} gave {error}");
        }
    }

    /// A game file may write a setting in another form than the defaults do;
    /// its value replaces the default's like any other single value.
    #[test]
    fn a_game_value_of_another_type_replaces_the_default() {
        let support = TempSupport::new("type-change");
        let executable = support.executable();
        support.defaults("retina_mode = false\n\ndpi = 144\n");
        support.game(
            "Game",
            &format!(
                "{}retina_mode = \"auto\"\ndpi = \"off\"\n",
                executable_line(&executable)
            ),
        );

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        assert_eq!(config.retina_mode, RetinaMode::Auto);
        assert_eq!(config.dpi, Dpi::Fixed(96));
    }

    /// The settings layer checks the value and says what its key takes.
    #[test]
    fn a_value_its_key_does_not_take_is_fatal() {
        for key in [Key::Dxgi, Key::Wined3d, Key::D3d9] {
            let support = TempSupport::new("bad-value");
            let executable = support.executable();
            let game = support.game(
                "Game",
                &[executable_line(&executable), text(key, "bogus")].concat(),
            );

            let error = Config::load_or_setup(support.path(), Settings::default())
                .unwrap_err()
                .to_string();

            assert_eq!(
                error,
                format!("{}: {}", game.display(), refused(key, "bogus"))
            );
        }
    }

    /// A value the settings layer only warns about is ignored with a warning,
    /// in either file and in a rule, as if the line were not there: the other
    /// file's value applies, and without one compatdb.so's built-in rules
    /// decide.
    #[test]
    fn a_value_the_settings_layer_warns_about_is_ignored() {
        let (key, old) = retired_value(Severity::Warning);
        let document: DocumentMut = text(key, old).parse().unwrap();
        let raw = Raw::from_document(&document, Source::Defaults).unwrap();
        assert_eq!(raw.settings, Settings::default());
        let warning =
            rules::check_launch_wide(key, &rules::Value::One(old.to_owned())).unwrap_err();
        assert_eq!(raw.ignored.warnings, [warning.to_string()]);
        assert_eq!(raw.ignored.fields, [(None, key.to_string())]);

        let support = TempSupport::new("retired-value");
        let executable = support.executable();
        support.defaults(&text(key, old));
        support.game(
            "Game",
            &[
                executable_line(&executable),
                block(
                    "old",
                    &[
                        text(Key::Exe, "Old.exe"),
                        text(key, old.to_ascii_uppercase()),
                    ],
                ),
            ]
            .concat(),
        );

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        assert!(config.launch_wide.is_empty());
        let mut expected = Table::new();
        expected.push(Rule::new(exe("Old.exe")));
        assert_eq!(config.rules.table, expected);

        // The other file's value stays, as if the line were not there.
        let word = crate::test_keys::word(key, 0);
        support.defaults(&text(key, word));
        support.game(
            "Game",
            &[executable_line(&executable), text(key, old)].concat(),
        );
        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();
        let mut expected = Rule::new(Exe::Any);
        expected.set(key, word).unwrap();
        assert_eq!(own(&config), expected);
    }

    #[test]
    fn cli_overrides_only_the_named_file_values() {
        let support = TempSupport::new("cli-merge");
        let executable = support.executable();
        support.defaults(&format!(
            "{}\nretina_mode = false\n\n[env]\nKEEP = \"file\"\nCHANGE = \"file\"\n",
            flag(Key::MetalHud, true)
        ));
        let mut text = game_file(&executable_line(&executable));
        text = set_arguments(&text, &[String::from("-console")]);
        let game = support.game("Game", &text);
        let existing = fs::read_to_string(&game).unwrap();

        let config = Config::load_or_setup(
            support.path(),
            Settings {
                env: BTreeMap::from([
                    (String::from("CHANGE"), String::from("cli")),
                    (String::from("ADDED"), String::from("cli")),
                ]),
                ..one(Key::MetalHud, "false")
            },
        )
        .unwrap();

        assert_eq!(config.executable(), executable);
        assert_eq!(config.arguments(), ["-console"]);
        assert_eq!(own(&config).metal_hud(), Setting::Set(false));
        assert_eq!(config.retina_mode, RetinaMode::Off);
        assert_eq!(config.env["KEEP"], "file");
        assert_eq!(config.env["CHANGE"], "cli");
        assert_eq!(config.env["ADDED"], "cli");
        assert_eq!(fs::read_to_string(game).unwrap(), existing);
    }

    #[test]
    fn retina_mode_is_auto_unless_a_file_or_the_cli_says_otherwise() {
        let support = TempSupport::new("retina");
        let executable = support.executable();
        support.game("Game", &game_file(&executable_line(&executable)));
        let load = |mode: Option<RetinaMode>| {
            Config::load_or_setup(
                support.path(),
                Settings {
                    retina_mode: mode,
                    ..Settings::default()
                },
            )
        };
        assert_eq!(load(None).unwrap().retina_mode, RetinaMode::Auto);

        support.defaults("retina_mode = false\n");
        assert_eq!(load(None).unwrap().retina_mode, RetinaMode::Off);
        let cli = load(Some(RetinaMode::Auto)).unwrap();
        assert_eq!(cli.retina_mode, RetinaMode::Auto);

        support.defaults("retina_mode = \"auto\"\n");
        assert_eq!(load(None).unwrap().retina_mode, RetinaMode::Auto);

        support.defaults("retina_mode = \"on\"\n");
        let error = load(None).unwrap_err().to_string();
        assert!(
            error.contains("`retina_mode` must be `auto`, `true` or `false`, got `on`"),
            "{error}"
        );
    }

    #[test]
    fn a_top_level_setting_travels_in_the_launchers_own_rule_only_when_set() {
        let support = TempSupport::new("dpi-aware");
        let executable = support.executable();
        support.game("Game", &game_file(&executable_line(&executable)));
        let load = |overrides: Settings| Config::load_or_setup(support.path(), overrides).unwrap();

        // Unset: no rule at all, so compatdb's built-in rules decide.
        let unset = load(Settings::default());
        assert!(unset.launch_wide.is_empty());
        assert_eq!(unset.rules.table, Table::new());

        support.defaults(&flag(Key::DpiAware, false));
        let file = load(Settings::default());
        let mut expected = Table::new();
        expected.push(Rule::new(Exe::Any).with_dpi_aware(false));
        assert_eq!(file.rules.table, expected);
        let other = file
            .rules
            .table
            .resolve("Other.exe", &VersionInfo::default());
        assert_eq!(other.dpi_aware(), Some(false));

        let cli = load(one(Key::DpiAware, "true"));
        assert_eq!(own(&cli).dpi_aware(), Setting::Set(true));

        // `default` hands the setting back to the registry and the manifest
        // for every process.
        support.defaults(&text(Key::DpiAware, DEFAULT_VALUE));
        let reset = load(Settings::default());
        assert_eq!(own(&reset).dpi_aware(), Setting::Reset);
        let cli = load(one(Key::DpiAware, &DEFAULT_VALUE.to_ascii_uppercase()));
        assert_eq!(own(&cli).dpi_aware(), Setting::Reset);
    }

    #[test]
    fn a_game_files_launch_wide_setting_wins_over_the_defaults() {
        let support = TempSupport::new("dpi-aware-game");
        let executable = support.executable();
        support.defaults(&flag(Key::DpiAware, false));
        support.game(
            "Game",
            &[executable_line(&executable), flag(Key::DpiAware, true)].concat(),
        );

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        assert_eq!(own(&config).dpi_aware(), Setting::Set(true));
        assert_eq!(config.rules.table.rules().len(), 1);
    }

    #[test]
    fn a_rule_wins_over_a_launch_wide_setting_and_a_list_reaches_the_table() {
        let support = TempSupport::new("dpi-aware-rule");
        let executable = support.executable();
        support.game("Game", &game_file(&executable_line(&executable)));
        support.defaults(
            &[
                flag(Key::DpiAware, true),
                list(Key::DllOverrides, &["a=b"]),
                block(
                    "tool",
                    &[text(Key::Exe, "tool.exe"), flag(Key::DpiAware, false)],
                ),
            ]
            .concat(),
        );

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        let mut expected = Table::new();
        expected.push(
            Rule::new(Exe::Any)
                .with_dpi_aware(true)
                .with_dll_override(DllOverride::new("a=b").unwrap()),
        );
        expected.push(Rule::new(exe("tool.exe")).with_dpi_aware(false));
        assert_eq!(config.rules.table, expected);
        let tool = config
            .rules
            .table
            .resolve("Tool.exe", &VersionInfo::default());
        assert_eq!(tool.dpi_aware(), Some(false));
        assert_eq!(tool.dll_overrides, ["a=b"]);
        let other = config
            .rules
            .table
            .resolve("Other.exe", &VersionInfo::default());
        assert_eq!(other.dpi_aware(), Some(true));
        assert_eq!(other.dll_overrides, ["a=b"]);
    }

    #[test]
    fn a_launch_wide_value_merges_and_a_rule_naming_an_executable_wins() {
        let support = TempSupport::new("lae");
        let executable = support.executable();
        support.defaults(
            &[
                flag(Key::LargeAddressAware, false),
                text(Key::OpenGl, OpenGl::Egl),
            ]
            .concat(),
        );
        support.game(
            "Game",
            &[
                executable_line(&executable),
                flag(Key::LargeAddressAware, true),
                text(Key::OpenGl, OpenGl::Cgl.as_str().to_ascii_uppercase()),
                block(
                    "old",
                    &[
                        text(Key::Exe, "Old.exe"),
                        flag(Key::LargeAddressAware, false),
                        text(Key::OpenGl, OpenGl::Egl),
                    ],
                ),
            ]
            .concat(),
        );
        let load = |overrides: Settings| Config::load_or_setup(support.path(), overrides).unwrap();

        // The game file's value replaces the default's, and a rule naming an
        // executable wins over both for that executable.
        let config = load(Settings::default());
        let mut expected = Table::new();
        expected.push(
            Rule::new(Exe::Any)
                .with_opengl(OpenGl::Cgl)
                .with_large_address_aware(true),
        );
        expected.push(
            Rule::new(exe("Old.exe"))
                .with_opengl(OpenGl::Egl)
                .with_large_address_aware(false),
        );
        assert_eq!(config.rules.table, expected);
        let old = config
            .rules
            .table
            .resolve("Old.exe", &VersionInfo::default());
        assert_eq!(old.large_address_aware(), Some(false));
        assert_eq!(old.opengl(), Some(OpenGl::Egl));
        let other = config
            .rules
            .table
            .resolve("Other.exe", &VersionInfo::default());
        assert_eq!(other.large_address_aware(), Some(true));
        assert_eq!(other.opengl(), Some(OpenGl::Cgl));

        // The command line wins over the files.
        let cli = load(one(Key::LargeAddressAware, "false"));
        assert_eq!(own(&cli).large_address_aware(), Setting::Set(false));

        // Unset everywhere: no rule, so the built-in rules of compatdb.so
        // decide.
        support.defaults("");
        support.game("Game", &game_file(&executable_line(&executable)));
        let unset = load(Settings::default());
        assert!(unset.launch_wide.is_empty());
        assert_eq!(unset.rules.table, Table::new());

        // A block written for a former built-in rule reaches nothing, so it
        // is left out (with a warning, see rules.rs) and the game starts as if
        // it were not there.
        let former = compatdb_core::builtin::former_names().next().unwrap();
        support.defaults(&block(former, &[flag("enabled", false)]));
        support.game(
            "Game",
            &[
                executable_line(&executable),
                block(former, &[flag(Key::LargeAddressAware, true)]),
            ]
            .concat(),
        );
        let former = load(Settings::default());
        assert_eq!(former.rules.table, Table::new());
        assert_eq!(former.rules.names, Vec::<Option<String>>::new());
    }

    /// A file written for an older launcher keeps working: the settings layer
    /// only warns about a retired key, so it is skipped with a warning, in
    /// either file and in a rule, whatever its value, and nothing of it
    /// reaches the table.
    #[test]
    fn a_key_the_settings_layer_warns_about_is_ignored() {
        let old = retired_key(Severity::Warning);
        let document: DocumentMut = flag(old, false).parse().unwrap();
        let raw = Raw::from_document(&document, Source::Defaults).unwrap();
        assert_eq!(raw.settings, Settings::default());
        let warning = Key::from_setting(old).unwrap_err();
        assert_eq!(raw.ignored.warnings, [warning.to_string()]);

        let support = TempSupport::new("retired-key");
        let executable = support.executable();
        support.defaults(&flag(old, true));
        let kept = text(old, "no");
        let game = support.game(
            "Game",
            &[
                executable_line(&executable),
                kept.clone(),
                block("old", &[text(Key::Exe, "Old.exe"), flag(old, false)]),
            ]
            .concat(),
        );

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        assert!(config.launch_wide.is_empty());
        let mut expected = Table::new();
        expected.push(Rule::new(exe("Old.exe")));
        assert_eq!(config.rules.table, expected);
        // The line stays for the user to remove; the warning says so.
        assert!(fs::read_to_string(game).unwrap().contains(&kept));
    }

    #[test]
    fn dpi_is_auto_unless_a_file_or_the_cli_says_otherwise() {
        let support = TempSupport::new("dpi");
        let executable = support.executable();
        support.game("Game", &game_file(&executable_line(&executable)));
        let load = |dpi: Option<Dpi>| {
            Config::load_or_setup(
                support.path(),
                Settings {
                    dpi,
                    ..Settings::default()
                },
            )
        };
        assert_eq!(load(None).unwrap().dpi, Dpi::Auto);

        support.defaults("dpi = 144\n");
        assert_eq!(load(None).unwrap().dpi, Dpi::Fixed(144));
        assert_eq!(load(Some(Dpi::Auto)).unwrap().dpi, Dpi::Auto);

        for (text, dpi) in [
            ("\"off\"", 96),
            ("0", 96),
            ("\"OFF\"", 96),
            ("\"120\"", 120),
        ] {
            support.defaults(&format!("dpi = {text}\n"));
            assert_eq!(load(None).unwrap().dpi, Dpi::Fixed(dpi), "{text}");
        }

        for bad in ["1000", "\"large\"", "true", "1.5"] {
            support.defaults(&format!("dpi = {bad}\n"));
            let error = load(None).unwrap_err().to_string();
            assert!(
                error.contains(&format!(
                    "`dpi` must be {}, got `{}`",
                    dpi::VALUES,
                    bad.trim_matches('"')
                )),
                "{error}"
            );
        }
    }

    #[test]
    fn cli_arguments_replace_the_file_arguments() {
        let support = TempSupport::new("cli-arguments");
        let executable = support.executable();
        let text = set_arguments(
            &game_file(&executable_line(&executable)),
            &[String::from("-console")],
        );
        support.game("Game", &text);

        let config = Config::load_or_setup(
            support.path(),
            Settings {
                arguments: Some(vec![String::from("-windowed")]),
                ..Settings::default()
            },
        )
        .unwrap();
        assert_eq!(config.arguments(), ["-windowed"]);

        let config = Config::load_or_setup(
            support.path(),
            Settings {
                arguments: Some(Vec::new()),
                ..Settings::default()
            },
        )
        .unwrap();
        assert_eq!(config.arguments(), Vec::<String>::new());
    }
}
