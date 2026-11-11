//! Finds the running wineservers of a bundle and the prefix each serves.
//!
//! `ps -o comm` reports a process's `argv[0]`, which for a server started
//! through `PATH` is a bare `wineserver`. So `ps` only names the
//! candidates, the processes whose `argv[0]` has the file name of the
//! bundle's `bin/wineserver` or, when that is a symbolic link, of the file
//! it resolves to; a candidate is the bundle's when `lsof`
//! lists that file, with symlinks and `..` resolved, among its `txt`
//! entries (the executable and the libraries it maps). A server serves a
//! prefix when its working directory is the prefix's server directory,
//! `/tmp/.wine-<uid>/server-<dev>-<ino>` with the prefix's device and
//! inode in hex, which a wineserver works from for its whole life.
//!
//! Every helper runs for at most [`HELPER_LIMIT`].

use std::{
    fs,
    io::Read as _,
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// How long `ps` or `lsof` may run before it is killed.
pub const HELPER_LIMIT: Duration = Duration::from_secs(10);

/// The pids of the running processes whose executable is `exe`.
///
/// # Errors
///
/// Returns a message when `exe` does not exist or `ps` fails or runs past
/// its limit.
pub fn pids_of(exe: &Path) -> Result<Vec<u32>, String> {
    let wanted = fs::canonicalize(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let table = output_limited("ps", &["-Ao", "pid=,comm="], HELPER_LIMIT)?;
    let names = [exe.file_name(), wanted.file_name()];
    Ok(parse_ps(&table)
        .into_iter()
        .filter(|(_, argv0)| names.contains(&Path::new(argv0).file_name()))
        .filter(|&(pid, _)| {
            txt_paths(pid)
                .iter()
                .any(|path| fs::canonicalize(path).is_ok_and(|path| path == wanted))
        })
        .map(|(pid, _)| pid)
        .collect())
}

/// Run `program` with `args` for at most `limit` and return its stdout.
///
/// # Errors
///
/// Returns a message when it cannot be started, fails, or runs past
/// `limit` and is killed.
pub fn output_limited(program: &str, args: &[&str], limit: Duration) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{program} cannot be run: {e}"))?;
    let reader = child.stdout.take().map(|mut stdout| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            bytes
        })
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{program} was still running after {} s and was killed",
                    limit.as_secs()
                ));
            }
            Err(e) => return Err(format!("{program} cannot be waited for: {e}")),
        }
    };
    let bytes = reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    if !status.success() {
        return Err(format!("{program} ended with {status}"));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The pid and `argv[0]` of each line of a `ps -Ao pid=,comm=` table.
fn parse_ps(table: &str) -> Vec<(u32, &str)> {
    table
        .lines()
        .filter_map(|line| {
            let (pid, command) = line.trim_start().split_once(' ')?;
            Some((pid.parse().ok()?, command.trim()))
        })
        .collect()
}

/// The files `lsof` lists as the `txt` entries of `pid`: its executable
/// and the libraries it maps.
fn txt_paths(pid: u32) -> Vec<PathBuf> {
    output_limited(
        "lsof",
        &["-w", "-a", "-p", &pid.to_string(), "-d", "txt", "-Fn"],
        HELPER_LIMIT,
    )
    .map(|report| lsof_names(&report).map(PathBuf::from).collect())
    .unwrap_or_default()
}

/// The name of the server directory of the prefix `prefix`.
///
/// # Errors
///
/// Returns a message when the prefix cannot be read.
pub fn server_dir_name(prefix: &Path) -> Result<String, String> {
    let meta = fs::metadata(prefix).map_err(|e| format!("{}: {e}", prefix.display()))?;
    Ok(server_dir_for(meta.dev(), meta.ino()))
}

/// The server directory name wineserver derives from a prefix's device
/// and inode.
fn server_dir_for(dev: u64, ino: u64) -> String {
    format!("server-{dev:x}-{ino:x}")
}

/// The working directory of `pid`, as `lsof` reports it.
#[must_use]
pub fn cwd_of(pid: u32) -> Option<PathBuf> {
    let report = output_limited(
        "lsof",
        &["-w", "-a", "-d", "cwd", "-Fn", "-p", &pid.to_string()],
        HELPER_LIMIT,
    )
    .ok()?;
    lsof_names(&report).next().map(PathBuf::from)
}

/// The paths of an `lsof -Fn` report.
fn lsof_names(report: &str) -> impl Iterator<Item = &str> {
    report.lines().filter_map(|line| line.strip_prefix('n'))
}

/// Whether the process `pid` works from the server directory `dir_name`.
#[must_use]
pub fn serves(pid: u32, dir_name: &str) -> bool {
    cwd_of(pid).is_some_and(|cwd| cwd.file_name().is_some_and(|name| name == dir_name))
}

/// The pid of the wineserver `exe` that serves `prefix`, if one runs.
#[must_use]
pub fn server_of(exe: &Path, prefix: &Path) -> Option<u32> {
    let dir = server_dir_name(prefix).ok()?;
    pids_of(exe)
        .ok()?
        .into_iter()
        .find(|&pid| serves(pid, &dir))
}

#[cfg(test)]
mod tests;
