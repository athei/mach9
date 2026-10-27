//! Which game file a launch loads.

use std::{io, path::Path};

use gamelauncher_core::games;
use log::{info, warn};

use super::Settings;

/// Which game file to load: the one asked for on the command line, else the
/// selected one, else the only one there is. With no game at all `main` has
/// already opened the New Executable window, so that case is an error here.
pub(super) fn choose_game(support_dir: &Path, requested: Option<&str>) -> io::Result<String> {
    if let Some(name) = requested {
        if !games::game_path(support_dir, name).is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "invalid `--config` game: {} does not exist",
                    games::game_path(support_dir, name).display()
                ),
            ));
        }
        return Ok(name.to_owned());
    }

    let selected = games::current(support_dir);
    if let Some(name) = &selected {
        if games::game_path(support_dir, name).is_file() {
            return Ok(name.clone());
        }
        warn!(
            "the selected game {name} has no file at {}",
            games::game_path(support_dir, name).display()
        );
    }

    let names = games::list(support_dir)?;
    match names.as_slice() {
        [] => Err(io::Error::other(format!(
            "no games saved: right-click the Dock icon and choose New Executable, or run with \
             --add-game, or create a game file under {} and name it in {}",
            games::games_dir(support_dir).display(),
            games::selection_path(support_dir).display()
        ))),
        [only] => {
            games::select(support_dir, only)?;
            info!("selected {only}, the only saved game");
            Ok(only.clone())
        }
        many => Err(io::Error::other(format!(
            "no game selected: right-click the Dock icon to choose one, or write one of {} into {}",
            many.join(", "),
            games::selection_path(support_dir).display()
        ))),
    }
}

/// Whether a plain launch has nothing to start, so the New Executable window
/// is the right first thing to show.
pub fn needs_first_game(support_dir: &Path, overrides: &Settings) -> io::Result<bool> {
    if overrides.executable.is_some() || overrides.game.is_some() {
        return Ok(false);
    }
    Ok(games::list(support_dir)?.is_empty())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::config::{Config, test_support::*};

    #[test]
    fn the_only_game_is_selected_on_its_own() {
        let support = TempSupport::new("only-game");
        let executable = support.executable();
        games::create(support.path(), "Solo", &executable_line(&executable)).unwrap();
        assert!(games::current(support.path()).is_none());

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        assert_eq!(config.game.as_deref(), Some("Solo"));
        assert_eq!(games::current(support.path()).as_deref(), Some("Solo"));
    }

    #[test]
    fn a_stale_selection_falls_back_to_the_only_game() {
        let support = TempSupport::new("stale-selection");
        let executable = support.executable();
        games::create(support.path(), "Solo", &executable_line(&executable)).unwrap();
        fs::write(games::selection_path(support.path()), "Gone\n").unwrap();

        let config = Config::load_or_setup(support.path(), Settings::default()).unwrap();

        assert_eq!(config.game.as_deref(), Some("Solo"));
        assert_eq!(games::current(support.path()).as_deref(), Some("Solo"));
    }

    #[test]
    fn several_games_without_a_selection_are_fatal() {
        let support = TempSupport::new("no-selection");
        let executable = support.executable();
        games::create(support.path(), "One", &executable_line(&executable)).unwrap();
        games::create(support.path(), "Two", &executable_line(&executable)).unwrap();

        let error = Config::load_or_setup(support.path(), Settings::default()).unwrap_err();

        assert!(
            error.to_string().contains("no game selected")
                && error.to_string().contains("One, Two"),
            "unexpected error: {error}"
        );
        assert!(games::current(support.path()).is_none());
    }

    /// The first launch is `main`'s to handle with the New Executable window;
    /// by the time the config loads, no games is an error, not a dialog.
    #[test]
    fn no_games_at_all_is_fatal_here() {
        let support = TempSupport::new("no-games");
        assert!(needs_first_game(support.path(), &Settings::default()).unwrap());

        let error = Config::load_or_setup(support.path(), Settings::default()).unwrap_err();

        assert!(
            error.to_string().contains("no games saved"),
            "unexpected error: {error}"
        );
        assert_eq!(games::list(support.path()).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn overrides_that_name_what_to_run_need_no_first_game() {
        let support = TempSupport::new("needs-first-game");
        let with_executable = Settings {
            executable: Some(String::from("/Games/Game.exe")),
            ..Settings::default()
        };
        let with_game = Settings {
            game: Some(String::from("MyGame")),
            ..Settings::default()
        };
        assert!(!needs_first_game(support.path(), &with_executable).unwrap());
        assert!(!needs_first_game(support.path(), &with_game).unwrap());

        let executable = support.executable();
        games::create(support.path(), "MyGame", &executable_line(&executable)).unwrap();
        assert!(!needs_first_game(support.path(), &Settings::default()).unwrap());
    }

    #[test]
    fn cli_game_picks_a_file_without_changing_the_selection() {
        let support = TempSupport::new("cli-game");
        let executable = support.executable();
        support.game("Selected", &executable_line(&executable));
        games::create(
            support.path(),
            "Other",
            &format!("{}\nretina_mode = false\n", executable_line(&executable)),
        )
        .unwrap();

        let config = Config::load_or_setup(
            support.path(),
            Settings {
                game: Some(String::from("Other")),
                ..Settings::default()
            },
        )
        .unwrap();

        assert_eq!(config.game.as_deref(), Some("Other"));
        assert_eq!(config.retina_mode, crate::retina::RetinaMode::Off);
        assert_eq!(games::current(support.path()).as_deref(), Some("Selected"));

        let error = Config::load_or_setup(
            support.path(),
            Settings {
                game: Some(String::from("Missing")),
                ..Settings::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
