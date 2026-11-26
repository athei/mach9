use std::{
    fs,
    os::unix::fs::symlink,
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// An app bundle in a temporary directory, removed on drop.
struct TempApp(PathBuf);

impl TempApp {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gamelauncher-licenses-{name}-{}-{}/Mach9.app",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// The runtime's licence folder, with a notice when `notice` is set.
    fn runtime(&self, notice: bool) {
        let dir = self.0.join("Contents/Resources/wine/share/licenses");
        fs::create_dir_all(&dir).unwrap();
        if notice {
            fs::write(
                dir.join("SOURCES.md"),
                "# Sources of the components under a GNU licence\n",
            )
            .unwrap();
        }
    }

    /// The links `populate` in launcher/Makefile makes, by the same relative
    /// targets.
    fn links(&self) {
        let dir = licenses_dir(&self.0);
        fs::create_dir_all(&dir).unwrap();
        symlink("../wine/share/licenses", dir.join("engine")).unwrap();
        symlink("engine/SOURCES.md", dir.join(NOTICE)).unwrap();
    }
}

impl Drop for TempApp {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }
}

#[test]
fn the_licence_folder_is_in_the_app_resources() {
    assert_eq!(
        licenses_dir(Path::new("/Applications/Mach9.app")),
        Path::new("/Applications/Mach9.app/Contents/Resources/licenses")
    );
}

#[test]
fn reveals_the_runtime_notice_through_the_links() {
    let app = TempApp::new("notice");
    app.runtime(true);
    app.links();
    let notice = licenses_dir(&app.0).join(NOTICE);
    assert_eq!(what_to_show(&app.0), Some(Show::Notice(notice.clone())));
    assert_eq!(
        fs::read_to_string(&notice).unwrap(),
        "# Sources of the components under a GNU licence\n"
    );
    assert!(
        licenses_dir(&app.0)
            .join("engine")
            .join("SOURCES.md")
            .is_file()
    );
}

#[test]
fn shows_the_folder_when_the_notice_does_not_resolve() {
    let app = TempApp::new("dangling");
    app.runtime(false);
    app.links();
    assert_eq!(
        what_to_show(&app.0),
        Some(Show::Folder(licenses_dir(&app.0)))
    );
}

#[test]
fn shows_nothing_without_a_licence_folder() {
    let app = TempApp::new("bare");
    assert_eq!(what_to_show(&app.0), None);
}
