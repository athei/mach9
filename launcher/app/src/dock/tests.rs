use std::cell::Cell;

use super::*;

/// A support directory and a plug-in binary of its own for one test, removed
/// when it is dropped.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("gamelauncher-dock-{name}-{}", std::process::id()));
        fs::create_dir_all(root.join("support")).unwrap();
        fs::write(root.join("plugin"), "dylib").unwrap();
        Self { root }
    }

    fn support(&self) -> PathBuf {
        self.root.join("support")
    }

    fn plugin(&self) -> PathBuf {
        self.root.join("plugin")
    }

    fn id_file(&self) -> PathBuf {
        self.support().join(ID_FILE)
    }

    /// Run [`refresh_plugin_host`] with a kill that returns `outcome`, and
    /// say whether the kill ran.
    fn refresh(&self, outcome: io::Result<bool>) -> bool {
        let called = Cell::new(false);
        refresh_plugin_host(&self.support(), &self.plugin(), || {
            called.set(true);
            outcome
        });
        called.get()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).ok();
    }
}

#[test]
fn the_plugin_sits_in_the_apps_plugins() {
    assert_eq!(
        plugin_binary(Path::new("/Applications/Mach9.app/Contents/Resources")),
        Path::new(
            "/Applications/Mach9.app/Contents/PlugIns/GameLauncherDockTile.docktileplugin/\
             Contents/MacOS/GameLauncherDockTile"
        )
    );
}

#[test]
fn restarts_the_host_once_per_plugin() {
    let id = "ino=2 mtime=3.000000004";
    assert!(needs_refresh(None, id));
    assert!(needs_refresh(Some(""), id));
    assert!(needs_refresh(Some("ino=5 mtime=3.000000004\n"), id));
    assert!(needs_refresh(Some("ino=2 mtime=3.000000005\n"), id));
    assert!(!needs_refresh(Some(&format!("{id}\n")), id));
    assert!(!needs_refresh(Some(id), id));
}

#[test]
fn a_new_plugin_kills_the_host_and_records_it() {
    for (name, outcome) in [("killed", true), ("absent", false)] {
        let scratch = Scratch::new(name);
        assert!(scratch.refresh(Ok(outcome)), "{name}");
        let id = plugin_id(&scratch.plugin()).unwrap();
        assert_eq!(
            fs::read_to_string(scratch.id_file()).unwrap(),
            format!("{id}\n"),
            "{name}"
        );
        // Done for this plug-in.
        assert!(!scratch.refresh(Ok(true)), "{name}");
    }
}

#[test]
fn a_failed_kill_leaves_the_record_for_the_next_launch() {
    let scratch = Scratch::new("failed");
    fs::write(scratch.id_file(), "ino=0 mtime=0.000000000\n").unwrap();
    assert!(scratch.refresh(Err(io::Error::other("killall failed"))));
    assert_eq!(
        fs::read_to_string(scratch.id_file()).unwrap(),
        "ino=0 mtime=0.000000000\n"
    );
    assert!(scratch.refresh(Ok(true)));
}

#[test]
fn a_replaced_plugin_counts_as_new() {
    let scratch = Scratch::new("replaced");
    assert!(scratch.refresh(Ok(true)));
    // A new file put in place of the old one, as every install does.
    let new = scratch.root.join("plugin.new");
    fs::write(&new, "dylib").unwrap();
    fs::rename(&new, scratch.plugin()).unwrap();
    assert!(scratch.refresh(Ok(true)));
}

#[test]
fn no_plugin_leaves_the_host_alone() {
    let scratch = Scratch::new("missing");
    fs::remove_file(scratch.plugin()).unwrap();
    assert!(!scratch.refresh(Ok(true)));
    assert!(!scratch.id_file().exists());
}

#[test]
fn killall_finding_no_process_is_no_failure() {
    assert_eq!(killed(Some(0)), Some(true));
    assert_eq!(killed(Some(1)), Some(false));
    assert_eq!(killed(Some(2)), None);
    // Killed by a signal itself.
    assert_eq!(killed(None), None);
}
