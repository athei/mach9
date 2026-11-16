//! The scratch directories of the crate's tests, the unit tests and those
//! under `tests/`, which take this file in with `#[path]`.
//!
//! A directory is `conformance-test-<suite>/<name>-<pid>` beside the test
//! binary, in Cargo's target directory. The process id keeps two runs of
//! one test binary, which share that directory, out of each other's trees;
//! the name keeps the tests of one run apart.

use std::{
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    process, thread,
};

/// The empty directory of one test, removed when the test passes and kept,
/// with its path printed, when it fails.
pub struct Scratch(PathBuf);

impl Scratch {
    /// The directory of the test `name` among the tests of `suite`.
    ///
    /// A directory of that path is left from an ended process that had the
    /// same id, since a failed test keeps its directory, so it is deleted
    /// first.
    #[must_use]
    pub fn new(suite: &str, name: &str) -> Self {
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("conformance-test-{suite}"))
            .join(format!("{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if thread::panicking() {
            eprintln!("the test's directory is kept: {}", self.0.display());
        } else {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
