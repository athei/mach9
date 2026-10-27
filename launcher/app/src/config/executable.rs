//! The executable a launch runs: resolving the configured path, and asking
//! for one when the game file names none or a stale one.

use std::{
    io,
    path::{Path, PathBuf},
};

use gamelauncher_core::fsutil::write_atomic;
use log::{info, warn};

use super::template::set_executable;
use crate::{bundle, ui};

/// The game file's executable when it still resolves, or else the one the
/// user picks, which is written back into the game file at `path`.
pub(super) fn resolve_game_executable(
    file_executable: Option<&str>,
    path: &Path,
    text: &str,
    home: &Path,
) -> io::Result<PathBuf> {
    let file_exe = file_executable.map(Path::new);
    let reason = match resolve_configured(file_exe, home) {
        Ok(exe) => return Ok(exe),
        Err(reason) => reason,
    };
    match reason.as_deref() {
        Some(reason) => warn!("{reason}; asking for the game executable"),
        None => info!("`executable` is unset; asking for the game executable"),
    }
    // Start the picker where the stale path pointed, when that directory still
    // exists; the game has usually just moved.
    let start_dir = file_exe
        .map(|configured| expand_tilde(configured, home))
        .and_then(|configured| configured.parent().map(Path::to_path_buf))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| home.to_path_buf());
    pick_executable(path, text, reason.as_deref(), &start_dir, home)
}

/// Validate the configured `executable`. The error is a human-readable reason
/// the value cannot be used, shown before the file picker opens.
fn resolve_configured(configured: Option<&Path>, home: &Path) -> Result<PathBuf, Option<String>> {
    let configured = configured.ok_or(None)?;
    let exe = resolve_exe(configured, home).map_err(|e| Some(e.to_string()))?;
    if !exe.is_file() {
        return Err(Some(format!("no game executable at {}", exe.display())));
    }
    Ok(exe)
}

/// Resolve an `executable` that came from `--config`. Unlike the file value,
/// an unusable one is fatal: it was asked for explicitly, so the picker would
/// be answering a question nobody posed.
pub(super) fn resolve_cli_executable(configured: &str, home: &Path) -> io::Result<PathBuf> {
    resolve_configured(Some(Path::new(configured)), home).map_err(|reason| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "invalid `--config` executable: {}",
                reason.unwrap_or_else(|| String::from("the value is unset"))
            ),
        )
    })
}

/// Explain what is needed, ask for the executable, then write the answer back
/// into the game file, keeping its comments and layout intact.
fn pick_executable(
    path: &Path,
    text: &str,
    reason: Option<&str>,
    start_dir: &Path,
    home: &Path,
) -> io::Result<PathBuf> {
    let app = bundle::app_name();
    let headline = if reason.is_some() {
        format!("{app} cannot find the game where it was.")
    } else {
        format!("{app} needs to know where the game is installed.")
    };
    let body = format!(
        "Pick the game executable, for example Game.exe. Its folder becomes the \
         game directory, and the choice is saved to {}.",
        path.display()
    );

    let picked = ui::explain_and_pick(&headline, &body, start_dir).ok_or_else(|| {
        io::Error::other(format!(
            "no game executable selected; set `executable` in {} to an absolute path",
            path.display()
        ))
    })?;
    let exe = resolve_exe(&picked, home)?;

    write_atomic(path, &set_executable(text, &exe))?;
    info!("saved executable {} to {}", exe.display(), path.display());
    Ok(exe)
}

/// Expand a leading `~`, then require an absolute UTF-8 path with a file name.
pub(super) fn resolve_exe(raw: &Path, home: &Path) -> io::Result<PathBuf> {
    let exe = expand_tilde(raw, home);
    if !exe.is_absolute() {
        return Err(io::Error::other(format!(
            "`executable` must be an absolute path, got {}",
            raw.display()
        )));
    }
    if exe.file_name().is_none() {
        return Err(io::Error::other(format!(
            "`executable` does not name a file: {}",
            raw.display()
        )));
    }
    if exe.to_str().is_none() {
        return Err(io::Error::other(format!(
            "`executable` is not valid UTF-8: {}",
            exe.display()
        )));
    }
    Ok(exe)
}

fn expand_tilde(raw: &Path, home: &Path) -> PathBuf {
    match raw.to_str() {
        Some("~") => home.to_path_buf(),
        Some(s) => s
            .strip_prefix("~/")
            .map_or_else(|| raw.to_path_buf(), |rest| home.join(rest)),
        None => raw.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gamelauncher_core::games;

    use super::*;
    use crate::config::{Config, Settings, files::TEMPLATE, test_support::*};

    #[test]
    fn expands_leading_tilde() {
        let exe = resolve_exe(Path::new("~/Games/Game.exe"), Path::new(HOME)).unwrap();
        assert_eq!(exe, Path::new("/Users/tester/Games/Game.exe"));
    }

    #[test]
    fn keeps_absolute_paths() {
        let exe = resolve_exe(Path::new("/Games/Game.exe"), Path::new(HOME)).unwrap();
        assert_eq!(exe, Path::new("/Games/Game.exe"));
    }

    #[test]
    fn rejects_relative_paths() {
        assert!(resolve_exe(Path::new("game/Game.exe"), Path::new(HOME)).is_err());
    }

    #[test]
    fn rejects_paths_without_a_file_name() {
        assert!(resolve_exe(Path::new("/"), Path::new(HOME)).is_err());
        assert!(resolve_exe(Path::new("~"), Path::new("/")).is_err());
    }

    /// A `--config` executable needs no game file and persists nothing.
    #[test]
    fn cli_executable_needs_no_game_file_and_persists_nothing() {
        let support = TempSupport::new("command-override");
        let executable = support.executable();
        let game = support.game(
            "Game",
            &game_file(&executable_line(Path::new("/missing/File.exe"))),
        );
        let existing = fs::read_to_string(&game).unwrap();

        let config = Config::load_or_setup(
            support.path(),
            Settings {
                executable: Some(executable.display().to_string()),
                arguments: Some(vec![String::from("-console"), String::from("two words")]),
                ..Settings::default()
            },
        )
        .unwrap();

        assert_eq!(config.executable(), executable);
        assert_eq!(config.arguments(), ["-console", "two words"]);
        assert!(config.game.is_none());
        assert_eq!(config.display_name(), "Game");
        assert_eq!(fs::read_to_string(game).unwrap(), existing);
        assert_eq!(
            fs::read_to_string(bundle::config_path(support.path())).unwrap(),
            *TEMPLATE
        );
    }

    #[test]
    fn cli_executable_needs_no_game_at_all() {
        let support = TempSupport::new("no-games-cli");
        let executable = support.executable();

        let config = Config::load_or_setup(
            support.path(),
            Settings {
                executable: Some(executable.display().to_string()),
                ..Settings::default()
            },
        )
        .unwrap();

        assert_eq!(config.executable(), executable);
        assert_eq!(config.arguments(), Vec::<String>::new());
        assert_eq!(games::list(support.path()).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn invalid_cli_executable_is_fatal() {
        let support = TempSupport::new("invalid-cli-executable");

        let error = Config::load_or_setup(
            support.path(),
            Settings {
                executable: Some(support.path().join("Missing.exe").display().to_string()),
                ..Settings::default()
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("invalid `--config` executable"));
    }
}
