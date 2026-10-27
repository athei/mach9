//! The New Executable window's answer, saved as a new game or turned into
//! one-shot `--config` overrides.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
};

use compatdb_core::check_env_name;
use gamelauncher_core::games;
use log::info;

use super::{
    executable::resolve_exe,
    files::GAME_TEMPLATE,
    template::{append_env, set_arguments, set_executable},
};
use crate::{
    bundle, rules,
    ui::{self, GameForm},
    words,
};

/// What a fresh instance of the app is started with once the New Executable
/// window has been answered: nothing for a saved game, which the `game` file
/// now names, or the `--config` overrides that run a one-shot command.
#[derive(Debug)]
pub struct Launch {
    pub args: Vec<String>,
}

/// Show the New Executable window, then save its answer as a new selected
/// game or turn it into one-shot overrides. `None` means the user cancelled.
/// This is what `--add-game`, the Dock menu and the first launch run.
pub fn add_game(support_dir: &Path) -> io::Result<Option<Launch>> {
    let app = bundle::app_name();
    let home = bundle::home_dir()?;
    let explanation = format!(
        "Run a Windows executable through {app}; its folder becomes the working \
         directory. Arguments are split like a shell command line. Environment \
         variables override those in {}.",
        bundle::config_path(support_dir).display()
    );
    let validate = |form: &GameForm| GameSpec::parse(form, support_dir, &home).map(|_| ());
    let Some(form) = ui::game_form(&app, &explanation, &home, &validate) else {
        info!("no executable selected");
        return Ok(None);
    };
    let spec = GameSpec::parse(&form, support_dir, &home).map_err(io::Error::other)?;

    let Some(name) = &spec.name else {
        info!("running {} once", spec.executable.display());
        return Ok(Some(Launch {
            args: spec.override_args(),
        }));
    };
    let path = games::create(support_dir, name, &spec.game_text())?;
    games::select(support_dir, name)?;
    info!(
        "saved game {name} ({}) to {}",
        spec.executable.display(),
        path.display()
    );
    Ok(Some(Launch { args: Vec::new() }))
}

/// The window's answer, checked. `name` is set when the game is to be saved.
#[derive(Debug, PartialEq, Eq)]
struct GameSpec {
    name: Option<String>,
    executable: PathBuf,
    arguments: Vec<String>,
    env: BTreeMap<String, String>,
}

impl GameSpec {
    /// Check every field, with messages meant for the window rather than the
    /// log: they name what to change.
    fn parse(form: &GameForm, support_dir: &Path, home: &Path) -> Result<Self, String> {
        let raw_exe = form.executable.trim();
        if raw_exe.is_empty() {
            return Err(String::from("Pick the executable to run."));
        }
        let executable = resolve_exe(Path::new(raw_exe), home).map_err(|e| e.to_string())?;
        if !executable.is_file() {
            return Err(format!("There is no file at {}.", executable.display()));
        }

        let arguments =
            words::split(&form.arguments).map_err(|e| format!("The argument list {e}."))?;
        let env = parse_env_lines(&form.env_text)?;
        // Otherwise the window would happily write a game file that refuses to
        // launch, with the explanation only in the log.
        if let Some((name, error)) = env
            .keys()
            .find_map(|name| check_env_name(name).err().map(|error| (name, error)))
        {
            return Err(format!(
                "{}. The setting goes in the game file; see \
                 https://github.com/athei/mach9/blob/main/docs/app.md#settings",
                rules::describe(name, &error)
            ));
        }

        let name = if form.save {
            let name = form.name.trim();
            if name.is_empty() {
                let stem = executable
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .expect("resolve_exe requires a UTF-8 file name");
                Some(games::unique_name(support_dir, stem))
            } else {
                if !games::valid_name(name) {
                    return Err(format!(
                        "{name} is not a usable game name: it becomes a file name under {}.",
                        games::games_dir(support_dir).display()
                    ));
                }
                if games::game_path(support_dir, name).exists() {
                    return Err(format!(
                        "A game named {name} exists already; pick another name or edit {}.",
                        games::game_path(support_dir, name).display()
                    ));
                }
                Some(name.to_owned())
            }
        } else {
            None
        };

        Ok(Self {
            name,
            executable,
            arguments,
            env,
        })
    }

    /// The game file: the template with the answers filled in, so the file
    /// documents the options like a hand-written one.
    fn game_text(&self) -> String {
        let mut text = set_executable(&GAME_TEMPLATE, &self.executable);
        if !self.arguments.is_empty() {
            text = set_arguments(&text, &self.arguments);
        }
        if !self.env.is_empty() {
            text = append_env(&text, &self.env);
        }
        text
    }

    /// The same answers as `--config` overrides, one assignment per flag, so
    /// a fresh instance runs them without a file.
    fn override_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        let mut push = |assignment: String| {
            args.push(String::from("--config"));
            args.push(assignment);
        };
        let exe = self
            .executable
            .to_str()
            .expect("resolve_exe rejects non-UTF-8");
        push(format!("executable={}", words::quote(exe)));
        if !self.arguments.is_empty() {
            let quoted: Vec<String> = self.arguments.iter().map(|a| words::quote(a)).collect();
            push(format!("arguments={}", quoted.join(" ")));
        }
        for (name, value) in &self.env {
            push(format!("env.{name}={}", words::quote(value)));
        }
        args
    }
}

/// One `NAME=value` per line; blank lines are skipped.
fn parse_env_lines(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut env = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            return Err(format!(
                "The environment line `{line}` must have the form NAME=value."
            ));
        };
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!(
                "`{name}` is not an environment variable name (letters, digits and _ only)."
            ));
        }
        env.insert(name.to_owned(), value.trim().to_owned());
    }
    Ok(env)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        cli,
        config::{
            Settings,
            template::{is_option, sync_template},
            test_support::*,
        },
    };

    #[test]
    fn the_window_refuses_a_retired_environment_variable() {
        let support = TempSupport::new("window-retired");
        let executable = support.executable();
        let mut form = form(&executable);
        let name = crate::test_keys::retired_env();
        form.env_text = format!("{name}=x\n");

        let error = GameSpec::parse(&form, support.path(), Path::new(HOME))
            .expect_err("would write a game file that cannot launch");

        let refused = check_env_name(name).unwrap_err();
        assert!(
            error.starts_with(&rules::describe(name, &refused)),
            "{error}"
        );
    }

    /// A saved game file carries every answer, parses back to the same values,
    /// and is complete as far as the template is concerned.
    #[test]
    fn a_saved_game_round_trips_through_the_template() {
        let support = TempSupport::new("game-text");
        let executable = support.path().join("My Game").join("Game.exe");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, b"x").unwrap();
        let mut form = form(&executable);
        form.name = String::from("Mine");
        form.arguments = String::from(r#"-console "two words" semi\;colon"#);
        form.env_text =
            String::from("SOME_CONFIG = a=1;b=2\n\n  OTHER=fps  \nQUOTED=\"keep quotes\"\n");

        let spec = GameSpec::parse(&form, support.path(), Path::new(HOME)).unwrap();
        let text = spec.game_text();

        assert!(text.starts_with("# One game."));
        assert!(sync_template(&text, &GAME_TEMPLATE).is_none());
        let raw = raw_of(&text).settings;
        assert_eq!(
            raw.executable.as_deref(),
            Some(executable.to_str().unwrap())
        );
        assert_eq!(
            raw.arguments.unwrap(),
            ["-console", "two words", "semi;colon"]
        );
        assert_eq!(raw.env["SOME_CONFIG"], "a=1;b=2");
        assert_eq!(raw.env["OTHER"], "fps");
        assert_eq!(raw.env["QUOTED"], "\"keep quotes\"");
        assert_eq!(spec.name.as_deref(), Some("Mine"));
        assert!(
            text.lines().filter(|line| is_option(line, "env")).count() == 2,
            "the commented example and the real table: {text}"
        );
    }

    /// The one-shot overrides go through the real command-line parser and come
    /// back as the same executable, arguments and environment.
    #[test]
    fn one_shot_overrides_survive_the_command_line() {
        let support = TempSupport::new("one-shot");
        let executable = support.path().join("Setup Kit").join("setup.exe");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, b"x").unwrap();
        let mut form = form(&executable);
        form.save = false;
        form.arguments = String::from("/S 'two words'");
        form.env_text = String::from("SOME_CONFIG=a=1;b=2\nSPACED=two words\n");

        let spec = GameSpec::parse(&form, support.path(), Path::new(HOME)).unwrap();
        assert!(spec.name.is_none());
        let mut command_line = vec![String::from("gamelauncher")];
        command_line.extend(spec.override_args());

        let parsed = cli::parse_from(command_line).unwrap();
        assert_eq!(
            parsed.overrides,
            Settings {
                executable: Some(executable.display().to_string()),
                arguments: Some(vec![String::from("/S"), String::from("two words")]),
                env: BTreeMap::from([
                    (String::from("SOME_CONFIG"), String::from("a=1;b=2")),
                    (String::from("SPACED"), String::from("two words")),
                ]),
                ..Settings::default()
            }
        );
        assert_eq!(games::list(support.path()).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn the_form_is_checked_field_by_field() {
        let support = TempSupport::new("form-check");
        let executable = support.executable();
        let home = Path::new(HOME);
        let check = |edit: &dyn Fn(&mut GameForm)| {
            let mut form = form(&executable);
            edit(&mut form);
            GameSpec::parse(&form, support.path(), home)
        };

        assert!(check(&|_| {}).is_ok());
        assert!(
            check(&|f| f.executable = String::new())
                .unwrap_err()
                .contains("Pick")
        );
        assert!(check(&|f| f.executable = String::from("relative.exe")).is_err());
        assert!(
            check(&|f| f.executable = support.path().join("gone.exe").display().to_string())
                .unwrap_err()
                .contains("no file")
        );
        assert!(
            check(&|f| f.arguments = String::from("'open"))
                .unwrap_err()
                .contains("argument list")
        );
        assert!(
            check(&|f| f.env_text = String::from("NOEQUALS"))
                .unwrap_err()
                .contains("NAME=value")
        );
        assert!(
            check(&|f| f.env_text = String::from("BAD NAME=1"))
                .unwrap_err()
                .contains("not an environment variable name")
        );
        assert!(check(&|f| f.name = String::from(".hidden")).is_err());
        assert!(check(&|f| f.name = String::from("a/b")).is_err());

        games::create(support.path(), "Taken", "executable = \"/x.exe\"\n").unwrap();
        assert!(
            check(&|f| f.name = String::from("Taken"))
                .unwrap_err()
                .contains("exists already")
        );
        // The name only matters when saving.
        assert!(
            check(&|f| {
                f.name = String::from("Taken");
                f.save = false;
            })
            .is_ok()
        );
        // An empty name takes the executable's, kept free of clashes.
        games::create(support.path(), "Game", "executable = \"/x.exe\"\n").unwrap();
        assert_eq!(check(&|_| {}).unwrap().name.as_deref(), Some("Game-2"));
    }
}
