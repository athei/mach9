//! The saved games: one TOML file each under `games/`, and a one-line `game`
//! file naming the one the next launch starts. A game's name is its file stem,
//! which is also what the Dock menu shows.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::fsutil::write_atomic;

pub const GAMES_DIR: &str = "games";
pub const SELECTION_FILE: &str = "game";

#[must_use]
pub fn games_dir(support_dir: &Path) -> PathBuf {
    support_dir.join(GAMES_DIR)
}

#[must_use]
pub fn selection_path(support_dir: &Path) -> PathBuf {
    support_dir.join(SELECTION_FILE)
}

#[must_use]
pub fn game_path(support_dir: &Path, name: &str) -> PathBuf {
    games_dir(support_dir).join(format!("{name}.toml"))
}

/// Whether `name` can be a game's name. It becomes a file name under
/// `games/`, so it must not be empty, hide the file with a leading dot, or
/// reach into another directory with a slash.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains('/')
}

/// Names of the saved games, sorted without regard to case. A missing
/// directory is an empty list. Hidden files, leftovers of an atomic write and
/// anything that is not a `.toml` file are skipped.
///
/// # Errors
///
/// When the directory exists but cannot be read.
pub fn list(support_dir: &Path) -> io::Result<Vec<String>> {
    let dir = games_dir(support_dir);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(io::Error::new(
                e.kind(),
                format!("failed to list {}: {e}", dir.display()),
            ));
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|ext| ext != "toml") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !valid_name(name) {
            continue;
        }
        names.push(name.to_owned());
    }
    names.sort_by_key(|name| (name.to_lowercase(), name.clone()));
    Ok(names)
}

/// The selected game, or `None` when nothing has been selected yet.
#[must_use]
pub fn current(support_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(selection_path(support_dir)).ok()?;
    let name = text.lines().next()?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Make `name` the game the next launch starts. It has to exist.
///
/// # Errors
///
/// [`io::ErrorKind::NotFound`] when no game file of that name exists, or
/// the error of writing the selection file.
pub fn select(support_dir: &Path, name: &str) -> io::Result<()> {
    let path = game_path(support_dir, name);
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no game named {name}: {} does not exist", path.display()),
        ));
    }
    write_atomic(&selection_path(support_dir), &format!("{name}\n"))
}

/// A name for a new game derived from `stem`, the executable's file stem:
/// `WoW`, then `WoW-2`, `WoW-3`, whatever is free.
#[must_use]
pub fn unique_name(support_dir: &Path, stem: &str) -> String {
    let stem = stem.trim();
    let stem = if stem.is_empty() { "Game" } else { stem };
    if !game_path(support_dir, stem).exists() {
        return stem.to_owned();
    }
    let mut n = 2u32;
    loop {
        let name = format!("{stem}-{n}");
        if !game_path(support_dir, &name).exists() {
            return name;
        }
        n += 1;
    }
}

/// Write a new game file with the given text. Returns its path.
///
/// # Errors
///
/// When the games directory cannot be created or the file cannot be
/// written.
pub fn create(support_dir: &Path, name: &str, text: &str) -> io::Result<PathBuf> {
    let dir = games_dir(support_dir);
    fs::create_dir_all(&dir).map_err(|e| {
        io::Error::new(e.kind(), format!("failed to create {}: {e}", dir.display()))
    })?;
    let path = game_path(support_dir, name);
    write_atomic(&path, text)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct TempSupport(PathBuf);

    impl TempSupport {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gamelauncher-games-{name}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn game(&self, name: &str) {
            fs::create_dir_all(games_dir(&self.0)).unwrap();
            fs::write(game_path(&self.0, name), "executable = \"/Games/x.exe\"\n").unwrap();
        }
    }

    impl Drop for TempSupport {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn lists_games_sorted_without_regard_to_case() {
        let support = TempSupport::new("list");
        for name in ["beta", "Gamma", "Alpha"] {
            support.game(name);
        }
        let dir = games_dir(&support.0);
        fs::write(dir.join(".hidden.toml"), "").unwrap();
        fs::write(dir.join("Beta.tmp"), "").unwrap();
        fs::write(dir.join("notes.txt"), "").unwrap();
        fs::create_dir(dir.join("folder.toml")).unwrap();

        assert_eq!(list(&support.0).unwrap(), ["Alpha", "beta", "Gamma"]);
    }

    #[test]
    fn a_missing_games_directory_is_an_empty_list() {
        let support = TempSupport::new("empty");
        assert_eq!(list(&support.0).unwrap(), Vec::<String>::new());
        assert!(current(&support.0).is_none());
    }

    #[test]
    fn selects_an_existing_game_and_reads_it_back() {
        let support = TempSupport::new("select");
        support.game("MyGame");

        select(&support.0, "MyGame").unwrap();
        assert_eq!(current(&support.0).as_deref(), Some("MyGame"));
        assert!(!selection_path(&support.0).with_extension("tmp").exists());

        let error = select(&support.0, "Missing").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(current(&support.0).as_deref(), Some("MyGame"));
    }

    #[test]
    fn a_blank_selection_counts_as_none() {
        let support = TempSupport::new("blank");
        fs::write(selection_path(&support.0), "  \n").unwrap();
        assert!(current(&support.0).is_none());
        fs::write(selection_path(&support.0), " MyGame \nignored\n").unwrap();
        assert_eq!(current(&support.0).as_deref(), Some("MyGame"));
    }

    #[test]
    fn picks_a_free_name_for_a_new_game() {
        let support = TempSupport::new("unique");
        assert_eq!(unique_name(&support.0, "MyGame"), "MyGame");
        support.game("MyGame");
        assert_eq!(unique_name(&support.0, "MyGame"), "MyGame-2");
        support.game("MyGame-2");
        assert_eq!(unique_name(&support.0, "MyGame"), "MyGame-3");
        assert_eq!(unique_name(&support.0, "  "), "Game");
    }

    #[test]
    fn creates_the_games_directory_on_demand() {
        let support = TempSupport::new("create");
        let path = create(&support.0, "MyGame", "executable = \"/Games/Game.exe\"\n").unwrap();
        assert_eq!(path, game_path(&support.0, "MyGame"));
        assert_eq!(list(&support.0).unwrap(), ["MyGame"]);
    }

    #[test]
    fn a_name_has_to_be_a_plain_file_name() {
        for name in ["MyGame", "My Game", "MyGame-2", "a.b"] {
            assert!(valid_name(name), "{name}");
        }
        for name in ["", ".hidden", "a/b", "/abs", ".."] {
            assert!(!valid_name(name), "{name}");
        }
    }
}
