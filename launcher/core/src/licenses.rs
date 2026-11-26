//! Where Mach9.app keeps its licence texts and the source notice, which the
//! Dock menu's "Licenses" item shows in Finder.
//!
//! `make app` and `make install` put the texts of the launcher's parts into
//! [`LICENSES_DIR`] (`APP_LICENSES` in `launcher/Makefile`), and link there
//! the Wine runtime's `share/licenses`, with its texts and the source notice
//! `bin/bundle-wine.sh` writes, as `engine`, and that notice as [`NOTICE`]. One
//! folder then leads to every text and the notice, without a copy of either
//! and without writing into the signed bundle at run time.

use std::path::{Path, PathBuf};

/// The licence folder of an app, relative to the app bundle.
pub const LICENSES_DIR: &str = "Contents/Resources/licenses";

/// The link to the runtime's source notice in [`LICENSES_DIR`].
pub const NOTICE: &str = "SOURCES.md";

/// The licence folder of the app at `app`.
#[must_use]
pub fn licenses_dir(app: &Path) -> PathBuf {
    app.join(LICENSES_DIR)
}

/// What the "Licenses" item shows in Finder.
#[derive(Debug, PartialEq, Eq)]
pub enum Show {
    /// The source notice, selected in the licence folder.
    Notice(PathBuf),
    /// The licence folder alone, for an app whose runtime was bundled before
    /// the runtime had a notice (`make install` into such an app links none).
    Folder(PathBuf),
}

/// What the "Licenses" item of the app at `app` shows, or `None` when the app
/// has no licence folder.
#[must_use]
pub fn what_to_show(app: &Path) -> Option<Show> {
    let dir = licenses_dir(app);
    let notice = dir.join(NOTICE);
    if notice.is_file() {
        Some(Show::Notice(notice))
    } else if dir.is_dir() {
        Some(Show::Folder(dir))
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
