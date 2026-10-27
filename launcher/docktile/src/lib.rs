//! The Dock icon's context menu: the saved games with the selected one
//! checked, "New Executable", "Open Config Folder" and "Check for Updates".
//! Clicking a game writes the `game` file and starts the app; "New
//! Executable" opens the app with `--add-game`, which shows the window that
//! saves a new game or runs something once; "Open Config Folder" shows the
//! support directory in Finder; "Check for Updates" opens the app with
//! `--check-updates`.
//!
//! This is an `NSDockTilePlugIn` bundle. The Dock loads it into its plug-in
//! host while the app is pinned and not running, so the launcher itself never
//! has to stay around, and the menu works without a process of ours alive.
//!
//! Every pinned app's plug-in loads into that one host, which has a single
//! Objective-C class namespace, so the class a plug-in's `NSPrincipalClass`
//! names has to be ours alone: `GameLauncherDockTile_` followed by the bundle
//! id, written into the plug-in's `Info.plist` when it is installed into the
//! app (see `gamelauncher_core::docktile`). The behaviour lives in one
//! `define_class!` class whose name objc2 generates, so a second copy of this
//! dylib in the host does not clash with it; the principal class is an empty
//! subclass of it, registered here when the dylib loads under whatever name
//! the plug-in's `Info.plist` gives. An instance then knows which app it
//! serves from its own class name, matched against the loaded plug-in
//! bundles.

use std::{
    cell::RefCell,
    ffi::CString,
    fs,
    path::{Path, PathBuf},
};

use gamelauncher_core::{
    bundle,
    docktile::{CLASS_PREFIX, class_name, principal_class},
    games,
};
use objc2::{
    ClassType, MainThreadMarker, MainThreadOnly, define_class,
    rc::Retained,
    runtime::{AnyClass, AnyObject, ClassBuilder, NSObject, NSObjectProtocol},
    sel,
};
use objc2_app_kit::{
    NSControlStateValueOn, NSDockTile, NSDockTilePlugIn, NSMenu, NSMenuItem, NSWorkspace,
    NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{NSArray, NSBundle, NSString, NSURL, ns_string};

/// Tags of the fixed items; the games take 1, 2, 3, ... A click comes back
/// from the Dock as a menu command carrying the item's tag, and the host looks
/// the item up by it, so every item needs a tag of its own and none may be 0.
const RUN_EXECUTABLE_TAG: isize = 1_000_000;
const OPEN_CONFIG_TAG: isize = 1_000_001;
const CHECK_UPDATES_TAG: isize = 1_000_002;

thread_local! {
    /// The menu most recently handed to the Dock. The Dock shows a serialized
    /// copy and reports a click by tag; the host then finds the item and its
    /// target in the original, which therefore has to outlive the call.
    static LAST_MENU: RefCell<Option<Retained<NSMenu>>> = const { RefCell::new(None) };
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and DockTile has no
    // `Drop` and no ivars, so the host creating instances with plain
    // `alloc`/`init` is fine.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct DockTile;

    unsafe impl NSObjectProtocol for DockTile {}

    unsafe impl NSDockTilePlugIn for DockTile {
        #[unsafe(method(setDockTile:))]
        fn set_dock_tile(&self, _dock_tile: Option<&NSDockTile>) {}

        #[unsafe(method_id(dockMenu))]
        fn dock_menu(&self) -> Option<Retained<NSMenu>> {
            self.build_menu(self.mtm())
        }
    }

    impl DockTile {
        #[unsafe(method(selectGame:))]
        fn select_game(&self, sender: Option<&NSMenuItem>) {
            let Some(item) = sender else {
                log("selectGame: called without a menu item");
                return;
            };
            let name = item.title().to_string();
            let Some(host) = self.host() else {
                return;
            };
            match games::select(&host.support, &name) {
                Ok(()) => {
                    log(&format!("selected {name} for {}", host.app.display()));
                    open_app(&host.app, &[]);
                }
                Err(e) => log(&format!("selecting {name}: {e}")),
            }
        }

        #[unsafe(method(runExecutable:))]
        fn run_executable(&self, _sender: Option<&NSMenuItem>) {
            if let Some(host) = self.host() {
                open_app(&host.app, &["--add-game"]);
            }
        }

        #[unsafe(method(checkUpdates:))]
        fn check_updates(&self, _sender: Option<&NSMenuItem>) {
            if let Some(host) = self.host() {
                open_app(&host.app, &["--check-updates"]);
            }
        }

        #[unsafe(method(openConfig:))]
        fn open_config(&self, _sender: Option<&NSMenuItem>) {
            let Some(host) = self.host() else {
                return;
            };
            // Finder has nothing to show before the first launch has created it.
            if let Err(e) = fs::create_dir_all(&host.support) {
                log(&format!("creating {}: {e}", host.support.display()));
                return;
            }
            let url = NSURL::fileURLWithPath(&NSString::from_str(&host.support.to_string_lossy()));
            if !NSWorkspace::sharedWorkspace().openURL(&url) {
                log(&format!("Finder did not open {}", host.support.display()));
            }
        }
    }
);

/// Start the app through Launch Services. Always a new instance: without
/// that, Launch Services only brings forward an instance that is still
/// around, such as one showing an error for the game selected before, and
/// nothing starts. The launch is asynchronous and its completion is not
/// waited for: the host stays alive, and a failure is the launcher's to
/// report.
fn open_app(app: &Path, args: &[&str]) {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&app.to_string_lossy()));
    let configuration = NSWorkspaceOpenConfiguration::configuration();
    configuration.setCreatesNewApplicationInstance(true);
    let args: Vec<Retained<NSString>> = args.iter().map(|arg| NSString::from_str(arg)).collect();
    configuration.setArguments(&NSArray::from_retained_slice(&args));
    NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(
        &url,
        &configuration,
        None,
    );
}

impl DockTile {
    /// Built afresh on every right-click, so the list follows the files: a
    /// game file copied into `games/` by hand is there the next time.
    fn build_menu(&self, mtm: MainThreadMarker) -> Option<Retained<NSMenu>> {
        let host = self.host()?;
        let current = games::current(&host.support);
        let names = games::list(&host.support).unwrap_or_else(|e| {
            log(&format!("listing games: {e}"));
            Vec::new()
        });

        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        for (index, name) in names.iter().enumerate() {
            let item = self.game_item(mtm, name, index);
            if current.as_deref() == Some(name.as_str()) {
                item.setState(NSControlStateValueOn);
            }
            menu.addItem(&item);
        }
        if !names.is_empty() {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        }
        menu.addItem(&self.item(
            mtm,
            "New Executable\u{2026}",
            sel!(runExecutable:),
            RUN_EXECUTABLE_TAG,
        ));
        menu.addItem(&self.item(
            mtm,
            "Open Config Folder",
            sel!(openConfig:),
            OPEN_CONFIG_TAG,
        ));
        menu.addItem(&self.item(
            mtm,
            "Check for Updates\u{2026}",
            sel!(checkUpdates:),
            CHECK_UPDATES_TAG,
        ));
        LAST_MENU.with(|last| *last.borrow_mut() = Some(menu.clone()));
        Some(menu)
    }

    fn game_item(&self, mtm: MainThreadMarker, name: &str, index: usize) -> Retained<NSMenuItem> {
        let tag = isize::try_from(index).unwrap_or(0) + 1;
        self.item(mtm, name, sel!(selectGame:), tag)
    }

    fn item(
        &self,
        mtm: MainThreadMarker,
        title: &str,
        action: objc2::runtime::Sel,
        tag: isize,
    ) -> Retained<NSMenuItem> {
        let item = NSMenuItem::new(mtm);
        item.setTitle(&NSString::from_str(title));
        item.setTag(tag);
        let target: &AnyObject = self;
        // SAFETY: the action selectors name methods defined above, taking the
        // menu item as their one argument, and the host keeps this instance
        // alive for as long as the tile exists.
        unsafe {
            item.setTarget(Some(target));
            item.setAction(Some(action));
        }
        item
    }

    /// The app this instance serves, found through its class name.
    fn host(&self) -> Option<Host> {
        let own_class = self.class().name().to_str().ok()?.to_owned();
        let plugin = plugin_bundles()
            .into_iter()
            .find(|plugin| principal_class(plugin).is_ok_and(|name| name == own_class));
        let Some(plugin) = plugin else {
            log(&format!("no loaded plug-in bundle names {own_class}"));
            return None;
        };
        match Host::of_plugin(&plugin) {
            Ok(host) => Some(host),
            Err(e) => {
                log(&format!("{}: {e}", plugin.display()));
                None
            }
        }
    }
}

struct Host {
    app: PathBuf,
    support: PathBuf,
}

impl Host {
    fn of_plugin(plugin: &Path) -> Result<Self, String> {
        let app = bundle::enclosing_app(plugin)
            .ok_or_else(|| String::from("the plug-in is not inside an app's Contents/PlugIns"))?;
        let id = bundle::bundle_id_of_app(&app).map_err(|e| e.to_string())?;
        let support = bundle::support_dir(&id).map_err(|e| e.to_string())?;
        Ok(Self { app, support })
    }
}

/// Runs when the dylib is loaded, inside `-[NSBundle load]`, before the host
/// looks up the principal class.
#[used]
#[unsafe(link_section = "__DATA,__mod_init_func")]
static INIT: extern "C" fn() = init;

extern "C" fn init() {
    let base = DockTile::class();
    for plugin in plugin_bundles() {
        let name = match principal_class(&plugin) {
            Ok(name) => name,
            Err(e) => {
                log(&format!("{e}"));
                continue;
            }
        };
        if !name.starts_with(CLASS_PREFIX) {
            continue;
        }
        register_principal_class(&name, base, &plugin);
    }
}

/// Register `name` as an empty subclass of `base`, unless a copy of this
/// dylib loaded earlier already did.
fn register_principal_class(name: &str, base: &AnyClass, plugin: &Path) {
    let Ok(c_name) = CString::new(name) else {
        log(&format!("{name}: not a class name"));
        return;
    };
    if AnyClass::get(&c_name).is_some() {
        return;
    }
    match ClassBuilder::new(&c_name, base) {
        Some(builder) => {
            builder.register();
            log(&format!("registered {name} for {}", plugin.display()));
        }
        None => log(&format!(
            "could not register {name} for {}",
            plugin.display()
        )),
    }
    // Only a warning: the plist is what the host resolves, so it wins.
    if let Some(expected) = bundle::enclosing_app(plugin)
        .and_then(|app| bundle::bundle_id_of_app(&app).ok())
        .map(|id| class_name(&id))
        .filter(|expected| expected != name)
    {
        log(&format!(
            "{}: NSPrincipalClass is {name}, expected {expected}; was NSPrincipalClass set when the plug-in was installed?",
            plugin.display()
        ));
    }
}

/// Every Dock tile plug-in bundle known to this process.
fn plugin_bundles() -> Vec<PathBuf> {
    // SAFETY: the binding is unsafe because NSBundle objects are not thread
    // safe; the host calls into the plug-in on its main thread only, and the
    // bundles are read, not mutated.
    let bundles = unsafe { NSBundle::allBundles() };
    bundles
        .iter()
        .map(|bundle| PathBuf::from(bundle.bundlePath().to_string()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "docktileplugin"))
        .collect()
}

unsafe extern "C" {
    fn NSLog(format: &NSString, ...);
}

/// Into the unified log, the one place the plug-in host can reach:
/// `log stream --predicate 'process CONTAINS "com.apple.dock.external.extra"' --info`.
fn log(message: &str) {
    let message = NSString::from_str(message);
    // SAFETY: the format has one `%@` and is given one object for it.
    unsafe { NSLog(ns_string!("GameLauncherDockTile: %@"), &*message) };
}
