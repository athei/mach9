//! The bundle's files outside the app: everything the launcher writes hangs
//! off one directory under `~/Library/Application Support`, named after the
//! bundle identifier.

use std::{
    env,
    ffi::CStr,
    io,
    path::{Path, PathBuf},
};

/// Launcher-owned files that live outside the bundle, next to the wine prefix.
pub const CONFIG_FILE: &str = "gamelauncher.toml";
pub const LOG_FILE: &str = "gamelauncher.log";
/// The retina mode and DPI the running wine session booted with.
pub const SESSION_STAMP_FILE: &str = "prefix-session";

/// `$HOME`, or the password database when the environment lacks it, which is
/// the case inside the Dock's plug-in host.
///
/// # Errors
///
/// When `HOME` is unset or empty and the password database has no home
/// directory for the user.
pub fn home_dir() -> io::Result<PathBuf> {
    if let Some(home) = env::var_os("HOME").filter(|home| !home.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    passwd_home()
        .ok_or_else(|| io::Error::other("HOME is unset and the password database has no entry"))
}

fn passwd_home() -> Option<PathBuf> {
    // SAFETY: getpwuid returns a pointer into static storage, or null. The
    // pw_dir string is copied out before anything else could overwrite it.
    unsafe {
        let entry = libc::getpwuid(libc::getuid());
        if entry.is_null() || (*entry).pw_dir.is_null() {
            return None;
        }
        let dir = CStr::from_ptr((*entry).pw_dir).to_str().ok()?;
        (!dir.is_empty()).then(|| PathBuf::from(dir))
    }
}

/// Everything the launcher writes lives here.
///
/// The config, the game files, the log and the wine prefix. Named after
/// `CFBundleIdentifier`, which is how the Dock tile plug-in, running in the
/// Dock's process, finds the same directory from the app it belongs to.
///
/// # Errors
///
/// When the home directory is unknown ([`home_dir`]).
pub fn support_dir(bundle_id: &str) -> io::Result<PathBuf> {
    Ok(home_dir()?
        .join("Library/Application Support")
        .join(bundle_id))
}

#[must_use]
pub fn prefix_dir(support_dir: &Path) -> PathBuf {
    support_dir.join("prefix")
}

/// The session stamp, see `session.rs` in the launcher.
#[must_use]
pub fn session_stamp_path(support_dir: &Path) -> PathBuf {
    support_dir.join(SESSION_STAMP_FILE)
}

#[must_use]
pub fn config_path(support_dir: &Path) -> PathBuf {
    support_dir.join(CONFIG_FILE)
}

#[must_use]
pub fn log_path(support_dir: &Path) -> PathBuf {
    support_dir.join(LOG_FILE)
}

/// `CFBundleIdentifier` of the `.app` at `app`.
///
/// # Errors
///
/// As [`plist_string`] for the app's `Contents/Info.plist`.
pub fn bundle_id_of_app(app: &Path) -> io::Result<String> {
    plist_string(&app.join("Contents/Info.plist"), "CFBundleIdentifier")
}

/// The `.app` whose `Contents` holds `path`, such as the app of its own
/// executable, its `Resources` directory or a plug-in in `Contents/PlugIns`.
/// `None` outside an app bundle.
#[must_use]
pub fn enclosing_app(path: &Path) -> Option<PathBuf> {
    let contents = path
        .ancestors()
        .skip(1)
        .find(|dir| dir.file_name().is_some_and(|name| name == "Contents"))?;
    let app = contents.parent()?;
    (app.extension().is_some_and(|ext| ext == "app")).then(|| app.to_path_buf())
}

/// A string value out of an `Info.plist`.
///
/// # Errors
///
/// [`io::ErrorKind::InvalidData`] when the file cannot be read as a
/// property list, or its top-level dictionary has no string under `key`.
pub fn plist_string(info_plist: &Path, key: &str) -> io::Result<String> {
    let value: plist::Value = plist::from_file(info_plist).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to read {}: {e}", info_plist.display()),
        )
    })?;
    value
        .as_dictionary()
        .and_then(|d| d.get(key))
        .and_then(|v| v.as_string())
        .map(str::to_owned)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{key} missing from {}", info_plist.display()),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_app_that_holds_a_path_in_its_contents() {
        let app = Some(Path::new("/Applications/Game.app"));
        for inside in [
            "/Applications/Game.app/Contents/PlugIns/Tile.docktileplugin",
            "/Applications/Game.app/Contents/MacOS/gamelauncher",
            "/Applications/Game.app/Contents/Resources",
            "/Applications/Game.app/Contents/Resources/wine/bin/wine",
        ] {
            assert_eq!(enclosing_app(Path::new(inside)).as_deref(), app, "{inside}");
        }
        for outside in [
            "/tmp/Tile.docktileplugin",
            "/Library/Contents/PlugIns/Tile.docktileplugin",
            "/Applications/Game.app",
            "/Applications/Game.app/Contents",
            "/Applications/Game.app/Other/gamelauncher",
            "target/release/gamelauncher",
        ] {
            assert!(enclosing_app(Path::new(outside)).is_none(), "{outside}");
        }
    }

    #[test]
    fn the_session_stamp_sits_next_to_the_prefix() {
        let support = Path::new("/support");
        assert_eq!(prefix_dir(support), Path::new("/support/prefix"));
        assert_eq!(
            session_stamp_path(support),
            Path::new("/support/prefix-session")
        );
    }

    #[test]
    fn home_comes_from_the_environment_or_passwd() {
        assert!(home_dir().unwrap().is_absolute());
        assert!(passwd_home().is_some_and(|home| home.is_absolute()));
    }
}
