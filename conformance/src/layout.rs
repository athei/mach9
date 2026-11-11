//! Where the runner finds the bundle and keeps its state.
//!
//! The paths follow `bin/lib/profile.sh`: the root is `WINE_BUILD_ROOT`
//! when it is set, else the directory that holds this repository's
//! checkout; the bundle is `<root>/dist/wine` and the state directory
//! `<root>/build/conformance`, beside the build trees.
//!
//! The state directory is a tree as `claim_tree` in `profile.sh` makes
//! them: it is used only when it is missing, empty or carries the marker
//! file `.wine-build-tree` naming it `conformance`, and then marked. The
//! directories inside it that a run deletes and makes again (the prefix,
//! the working directory, a recorded executable's raw logs) carry markers
//! of their own, and one without its marker, or a symbolic link in place of
//! one, is never used or deleted. The run logs
//! under `runs/` are never deleted.

use std::{
    ffi::OsStr,
    fmt::Write as _,
    fs::{self, File},
    io::{self, Read as _},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{model::PeDir, outcome::Key};

/// The file that marks a tree, as `TREE_MARKER` in `profile.sh`.
pub const TREE_MARKER: &str = ".wine-build-tree";

/// The tree name of the state directory.
pub const STATE_TREE: &str = "conformance";

/// The tree names of the directories in the state directory a run
/// replaces.
pub const PREFIX_TREE: &str = "conformance-prefix";
pub const WORK_TREE: &str = "conformance-work";
pub const BASELINE_RAW_TREE: &str = "conformance-baseline-raw";

/// The root of the build trees: `wine_build_root` when it is set and not
/// empty, else the parent of the checkout that holds `manifest_dir`, the
/// crate's directory.
#[must_use]
pub fn root(wine_build_root: Option<&OsStr>, manifest_dir: &Path) -> PathBuf {
    match wine_build_root {
        Some(root) if !root.is_empty() => PathBuf::from(root),
        _ => {
            let checkout = manifest_dir.parent().unwrap_or(manifest_dir);
            checkout.parent().unwrap_or(checkout).to_path_buf()
        }
    }
}

/// The default bundle under `root`.
#[must_use]
pub fn default_bundle(root: &Path) -> PathBuf {
    root.join("dist").join("wine")
}

/// The default state directory under `root`.
#[must_use]
pub fn default_state(root: &Path) -> PathBuf {
    root.join("build").join(STATE_TREE)
}

/// A bundle with its graphics tests.
pub struct Bundle {
    pub dir: PathBuf,
    /// `bin/wine`.
    pub wine: PathBuf,
    /// `bin/wineserver`.
    pub wineserver: PathBuf,
    /// `lib/wine/tests`.
    pub tests: PathBuf,
}

impl Bundle {
    /// The bundle in `dir`, made absolute.
    ///
    /// # Errors
    ///
    /// Returns a message when `bin/wine`, `bin/wineserver`,
    /// `lib/wine/tests/VERSION` or `lib/wine/tests/COMPONENTS` is missing:
    /// the tests come with a full bundle only.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let dir = std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let bundle = Self {
            wine: dir.join("bin").join("wine"),
            wineserver: dir.join("bin").join("wineserver"),
            tests: dir.join("lib").join("wine").join("tests"),
            dir,
        };
        for path in [
            &bundle.wine,
            &bundle.wineserver,
            &bundle.tests.join("VERSION"),
            &bundle.tests.join("COMPONENTS"),
        ] {
            if !path.is_file() {
                return Err(format!(
                    "{} is no bundle with graphics tests: {} is missing (a full bundle of \
                     bin/bundle-wine.sh has them)",
                    bundle.dir.display(),
                    path.display()
                ));
            }
        }
        Ok(bundle)
    }

    /// `lib/wine/tests/<leg>/<exe>_test.exe`.
    #[must_use]
    pub fn test_exe(&self, leg: PeDir, exe: &str) -> PathBuf {
        self.tests
            .join(leg.as_str())
            .join(format!("{exe}_test.exe"))
    }
}

/// The claimed state directory, locked for the invocation.
pub struct State {
    pub dir: PathBuf,
    /// The exclusive lock on `.lock`, held until the state is dropped.
    _lock: File,
}

impl State {
    /// Claim `dir` as the state directory ([`claim_dir`]) and lock it.
    ///
    /// # Errors
    ///
    /// Returns a message when the directory is not the runner's, cannot be
    /// made, or another invocation holds its lock.
    pub fn claim(dir: &Path) -> Result<Self, String> {
        let dir = std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        claim_dir(&dir, STATE_TREE)?;
        let path = dir.join(".lock");
        let lock = File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(format!(
                    "another conformance run uses {} (it holds {})",
                    dir.display(),
                    path.display()
                ));
            }
            Err(fs::TryLockError::Error(e)) => {
                return Err(format!("{}: cannot lock: {e}", path.display()));
            }
        }
        Ok(Self { dir, _lock: lock })
    }

    /// The Wine prefix of the run, made fresh by each invocation.
    #[must_use]
    pub fn prefix(&self) -> PathBuf {
        self.dir.join("prefix")
    }

    /// The working directory of every Wine process and vkd3d-proton's
    /// shader cache, made fresh by each invocation.
    #[must_use]
    pub fn work(&self) -> PathBuf {
        self.dir.join("work")
    }

    /// The raw logs of every run, `runs/<run id>`.
    #[must_use]
    pub fn runs(&self) -> PathBuf {
        self.dir.join("runs")
    }

    /// The resume files.
    #[must_use]
    pub fn resume(&self) -> PathBuf {
        self.dir.join("resume")
    }

    /// The raw logs of the run `exe`'s baseline was recorded from.
    #[must_use]
    pub fn baseline_raw(&self, exe: &str) -> PathBuf {
        self.dir.join("baseline-raw").join(exe)
    }
}

/// The tree name in the marker of `dir`, if it has one.
fn marker(dir: &Path) -> Option<String> {
    let mut text = String::new();
    File::open(dir.join(TREE_MARKER))
        .and_then(|mut file| file.read_to_string(&mut text))
        .ok()?;
    Some(text.trim_end().to_owned())
}

/// Whether `dir` exists and holds anything.
fn is_non_empty(dir: &Path) -> io::Result<bool> {
    match fs::read_dir(dir) {
        Ok(mut entries) => Ok(entries.next().is_some()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Make `dir` the tree `name` and mark it, as `claim_tree` in `profile.sh`
/// does without its wipe.
///
/// # Errors
///
/// Returns a message when `dir` carries the marker of another tree, is not
/// empty without a marker, or cannot be made or marked.
pub fn claim_dir(dir: &Path, name: &str) -> Result<(), String> {
    let at = |e: io::Error| format!("{}: {e}", dir.display());
    match marker(dir) {
        Some(found) if found == name => {}
        Some(found) => {
            return Err(format!(
                "{} is the tree '{found}', not '{name}'",
                dir.display()
            ));
        }
        None if is_non_empty(dir).map_err(at)? => {
            return Err(format!(
                "{} is not empty and has no {TREE_MARKER}, so the runner did not make it; remove \
                 it or name another directory",
                dir.display()
            ));
        }
        None => {}
    }
    fs::create_dir_all(dir).map_err(at)?;
    fs::write(dir.join(TREE_MARKER), format!("{name}\n")).map_err(at)
}

/// [`claim_dir`] for a tree inside the state directory, which must not be a
/// symbolic link: the marker and the run's files would go into the link's
/// target.
///
/// # Errors
///
/// As [`claim_dir`], and for a symbolic link.
pub fn claim_inner(dir: &Path, name: &str) -> Result<(), String> {
    if fs::symlink_metadata(dir).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(format!(
            "{} is a symbolic link; the runner makes it as a directory of its own",
            dir.display()
        ));
    }
    claim_dir(dir, name)
}

/// Make `dir`, a tree inside the state directory, the tree `name`, empty
/// but for its marker: a directory that carries the marker of `name` is
/// deleted first, a missing or empty one is made.
///
/// # Errors
///
/// As [`claim_inner`], and when the old tree cannot be deleted.
pub fn fresh_dir(dir: &Path, name: &str) -> Result<(), String> {
    claim_inner(dir, name)?;
    fs::remove_dir_all(dir).map_err(|e| format!("{}: cannot delete: {e}", dir.display()))?;
    claim_dir(dir, name)
}

/// The id of a run started at `time`: its UTC time as
/// `YYYYMMDD-HHMMSS`, which sorts as the runs started.
#[must_use]
pub fn run_id(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let mut id = String::new();
    let _ = write!(
        id,
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        of_day / 3600,
        of_day / 60 % 60,
        of_day % 60
    );
    id
}

/// The proleptic Gregorian date of `days` since 1970-01-01, after Howard
/// Hinnant's `civil_from_days`.
const fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// The file name of a test's raw log in its unit's directory: `<key>.log`,
/// or `whole.log` for a whole unit, which is the only test of its
/// directory.
#[must_use]
pub fn log_file(key: &Key) -> String {
    match key {
        Key::Whole => String::from("whole.log"),
        Key::Function(key) => format!("{key}.log"),
    }
}

/// The digest of a file's content for a resume identity.
///
/// FNV-1a over its bytes and its length, as `fnv1a64:<hex>:<length>`. Not
/// a security hash: it only tells a rebuilt test executable from the one a
/// resume file was written for.
///
/// # Errors
///
/// Returns the I/O error of reading the file.
pub fn file_digest(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut length: u64 = 0;
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for &byte in &buffer[..read] {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        length += u64::try_from(read).unwrap_or(u64::MAX);
    }
    Ok(format!("fnv1a64:{hash:016x}:{length}"))
}

#[cfg(test)]
mod tests;
