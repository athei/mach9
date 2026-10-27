//! Fixtures the config tests share: a throwaway support directory and the
//! files that go in it.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use gamelauncher_core::games;

use super::{Raw, Source, files::GAME_TEMPLATE, template::is_option};
use crate::{bundle, ui::GameForm};

pub(super) const HOME: &str = "/Users/tester";
static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

pub(super) struct TempSupport(PathBuf);

impl TempSupport {
    pub(super) fn new(test_name: &str) -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "gamelauncher-{test_name}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }

    pub(super) fn executable(&self) -> PathBuf {
        let path = self.0.join("Game.exe");
        fs::write(&path, b"test executable").unwrap();
        path
    }

    /// A game file holding `text`, selected for the next launch.
    pub(super) fn game(&self, name: &str, text: &str) -> PathBuf {
        let path = games::create(&self.0, name, text).unwrap();
        games::select(&self.0, name).unwrap();
        path
    }

    pub(super) fn defaults(&self, text: &str) -> PathBuf {
        let path = bundle::config_path(&self.0);
        fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for TempSupport {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn executable_line(exe: &Path) -> String {
    format!("executable = \"{}\"\n", exe.display())
}

/// The game template with the example executable replaced by `line`, so a
/// file that is expected to stay untouched needs no top-up.
pub(super) fn game_file(line: &str) -> String {
    let mut text = String::new();
    for template_line in GAME_TEMPLATE.lines() {
        text.push_str(if is_option(template_line, "executable") {
            line.trim_end()
        } else {
            template_line
        });
        text.push('\n');
    }
    text
}

pub(super) fn raw_of(text: &str) -> Raw {
    Raw::from_document(&text.parse().unwrap(), Source::Game).unwrap()
}

pub(super) fn form(executable: &Path) -> GameForm {
    GameForm {
        name: String::new(),
        executable: executable.display().to_string(),
        arguments: String::new(),
        env_text: String::new(),
        save: true,
    }
}
