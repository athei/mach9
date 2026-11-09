//! The Dock's plug-in host after an update.
//!
//! The Dock loads the Dock tile plug-in into its host for third-party
//! plug-ins, `com.apple.dock.external.extra.arm64`, and that process keeps
//! the dylib it loaded for as long as it lives. An update replaces the
//! plug-in in the bundle, yet the Dock menu goes on showing the previous
//! version's items. The host ignores `SIGTERM`. After a `SIGKILL` the Dock
//! starts a new host right away, which loads the plug-in from the bundle,
//! and the Dock itself keeps running.
//!
//! So the first launch with a new plug-in binary kills the host once and
//! writes the binary's identity (inode and modification time) to
//! `dock-plugin-id` in the support directory. A bundle replaced by Sparkle,
//! from the disk image or by `make install` has a new plug-in file, so each
//! of them counts, a development build at an unchanged version included.
//! The host is shared, so the Dock tile plug-ins of other apps reload too,
//! once per new plug-in of ours.

use std::{
    fs, io,
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use gamelauncher_core::fsutil::write_atomic;
use log::{info, warn};

/// The process name of the Dock's host for third-party plug-ins.
const HOST: &str = "com.apple.dock.external.extra.arm64";

/// The file in the support directory that holds the identity of the plug-in
/// binary the host was last killed for.
const ID_FILE: &str = "dock-plugin-id";

/// The plug-in's name, as the Makefile installs it into the app.
const PLUGIN_NAME: &str = "GameLauncherDockTile";

/// The Dock tile plug-in's binary in the app whose `Resources` dir this is.
pub fn plugin_binary(resources_dir: &Path) -> PathBuf {
    resources_dir
        .with_file_name("PlugIns")
        .join(format!("{PLUGIN_NAME}.docktileplugin/Contents/MacOS"))
        .join(PLUGIN_NAME)
}

/// Run `kill` unless that was already done for the plug-in binary at
/// `plugin`. `kill` returns whether it found a host, see [`kill_host`]. Every
/// failure is logged and the launch goes on: at worst the Dock menu shows the
/// previous items until the Dock restarts. After a failed kill the identity
/// is not written, so the next launch tries again.
pub fn refresh_plugin_host(support: &Path, plugin: &Path, kill: impl FnOnce() -> io::Result<bool>) {
    let id = match plugin_id(plugin) {
        Ok(id) => id,
        Err(e) => {
            warn!(
                "the Dock plug-in host is left alone: failed to read {}: {e}",
                plugin.display()
            );
            return;
        }
    };
    let path = support.join(ID_FILE);
    let stored = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            warn!("failed to read {}: {e}", path.display());
            None
        }
    };
    if !needs_refresh(stored.as_deref(), &id) {
        return;
    }
    match kill() {
        Ok(true) => info!("restarted the Dock plug-in host for a new plug-in ({id})"),
        Ok(false) => info!("no Dock plug-in host to restart for a new plug-in ({id})"),
        Err(e) => {
            warn!("the Dock menu may show the previous items: {e}");
            return;
        }
    }
    if let Err(e) = write_atomic(&path, &format!("{id}\n")) {
        warn!("{e}");
    }
}

/// The identity of the file at `path`, one line: a file put in its place
/// gets another inode, and one written over in place another time. The
/// device is left out: it changes when the same volume is mounted again,
/// which would kill the host on every launch.
fn plugin_id(path: &Path) -> io::Result<String> {
    let meta = fs::metadata(path)?;
    Ok(format!(
        "ino={} mtime={}.{:09}",
        meta.ino(),
        meta.mtime(),
        meta.mtime_nsec()
    ))
}

/// Whether the host has to be restarted for the plug-in `id`, given the
/// content of the id file, `None` when there is none.
fn needs_refresh(stored: Option<&str>, id: &str) -> bool {
    stored.is_none_or(|stored| stored.trim() != id)
}

/// `SIGKILL` the host. `Ok(false)` when none was running.
pub fn kill_host() -> io::Result<bool> {
    let output = Command::new("/usr/bin/killall")
        .args(["-KILL", HOST])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| io::Error::new(e.kind(), format!("failed to run killall: {e}")))?;
    killed(output.status.code()).ok_or_else(|| {
        io::Error::other(format!(
            "killall -KILL {HOST} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    })
}

/// What killall's exit code says: `Some(true)` when it signalled a process,
/// `Some(false)` when none matched, `None` for any other outcome.
const fn killed(code: Option<i32>) -> Option<bool> {
    match code {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
