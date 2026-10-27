//! The update check, run in the launcher process before it launches wine.
//!
//! Sparkle is built for a long-lived app with a run loop and a menu bar, and
//! this process has neither: every launch execs into wine, a Finder launch
//! with a child process left to pump wine's PTY output into the log. That
//! cannot host an `NSApplication` for an updater. Nor can the check be pushed
//! into a helper: a child process cannot take focus, so its windows draw but
//! never accept a click, which is the whole reason `ui` runs its dialogs in
//! this process.
//!
//! So the check goes where the dialogs already go: here, before the exec.
//! Two consequences follow from the process being short-lived. Sparkle's own
//! scheduling timer never gets to fire, so the interval is enforced by hand
//! against `lastUpdateCheckDate`; and there is no run loop turning, so the
//! check is driven by pumping one until `sessionInProgress` goes false.
//!
//! Even a silent check can create an `NSApplication` when Sparkle schedules
//! its next check. Every dispatched check therefore becomes a setup run:
//! `main` starts a fresh instance instead of going on to launch wine, because
//! a process that has talked to the window server stays registered with it.
//!
//! A local build installed into a bundle checks like any installed copy. It
//! carries the version and the public key committed in
//! `launcher/app/Info.plist`, the ones the latest release was cut from, so it
//! is never behind the feed it polls.

use std::{
    ffi::CString,
    os::unix::ffi::OsStrExt as _,
    path::Path,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use log::{Level, info, log};
use objc2::{
    AnyThread, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol},
};
use objc2_foundation::{NSBundle, NSDate, NSDefaultRunLoopMode, NSError, NSRunLoop, NSString};

use crate::{bundle, ui};

/// How long to wait for a silent check before giving up and starting the game.
/// Only applies while nothing is on screen: once the user is looking at an
/// update window, the launch waits on them, not on a clock.
const SILENT_TIMEOUT: Duration = Duration::from_secs(10);

/// One turn of the run loop.
const PUMP_INTERVAL: f64 = 0.05;

/// Set from the user driver delegate the moment Sparkle commits to showing an
/// update window, which is before the window actually goes up.
static SHOWING_UI: AtomicBool = AtomicBool::new(false);

define_class!(
    // SAFETY: NSObject has no subclassing requirements, the class holds no
    // ivars, and the one method is an optional protocol method Sparkle only
    // ever calls on the main thread.
    #[unsafe(super(NSObject))]
    #[name = "GameLauncherUpdaterDriverDelegate"]
    struct DriverDelegate;

    unsafe impl NSObjectProtocol for DriverDelegate {}

    impl DriverDelegate {
        #[unsafe(method(standardUserDriverWillHandleShowingUpdate:forUpdate:state:))]
        fn will_handle_showing_update(
            &self,
            handle_showing_update: bool,
            _update: &AnyObject,
            _state: &AnyObject,
        ) {
            if !handle_showing_update {
                return;
            }
            SHOWING_UI.store(true, Ordering::Relaxed);
            // Check in now, while the window is still being built: an app that
            // counts as still starting up paints the wait cursor over
            // everything outside its own window. Focus comes later, once there
            // is something on screen to focus; see the note in `ui`.
            ui::check_in_as_app();
        }
    }
);

impl DriverDelegate {
    fn new() -> Retained<Self> {
        // SAFETY: the designated initializer of the superclass, no ivars to set.
        unsafe { msg_send![Self::alloc(), init] }
    }
}

/// Whether the caller can continue launching in this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No check ran, and Sparkle was never started, so nothing of it runs in
    /// the background. The only other threads are the idle libdispatch
    /// workers that reading the last check date from the user defaults may
    /// have started. The caller goes on to set the wine environment and
    /// launch.
    Continue,
    /// A check ran. The caller must start a fresh instance before launching a
    /// game, whether or not Sparkle showed a window.
    RestartRequired,
}

/// Check for an update.
///
/// `force` is the check the user asked for from the Dock menu: it ignores the
/// interval and always ends in a window, even when the answer is that there is
/// nothing new. A normal launch passes `false` and shows nothing unless there
/// is an update to show.
///
/// Every reason not to check, the interval included, is settled before
/// `startUpdater:`. Starting the updater sets Sparkle's own work going on
/// other threads, and a launch that goes on in this process sets environment
/// variables afterwards, which is only sound while no other thread reads
/// them. Reading the interval does start threads: the user defaults are
/// fetched from cfprefsd, which brings up libdispatch workers. They sit idle,
/// and nothing of Sparkle runs.
pub fn check(force: bool) -> Outcome {
    let bundle = NSBundle::mainBundle();

    let path = bundle.bundlePath().to_string();
    if on_read_only_volume(Path::new(&path)) {
        return skip(force, Level::Info, "running from a read-only volume");
    }
    if !has_public_key(&bundle) {
        return skip(force, Level::Info, "no Sparkle public key in this build");
    }

    let (Some(driver_class), Some(updater_class)) = (
        AnyClass::get(c"SPUStandardUserDriver"),
        AnyClass::get(c"SPUUpdater"),
    ) else {
        return skip(force, Level::Warn, "Sparkle is not loaded");
    };

    let delegate = DriverDelegate::new();
    // SAFETY: both initializers are the ones declared in Sparkle's headers,
    // called on the main thread with a live host bundle. The updater takes no
    // delegate of its own: everything this needs is on the user driver.
    let (driver, updater) = unsafe {
        let driver: Retained<AnyObject> = msg_send![
            msg_send![driver_class, alloc],
            initWithHostBundle: &*bundle,
            delegate: &*delegate,
        ];
        let updater: Retained<AnyObject> = msg_send![
            msg_send![updater_class, alloc],
            initWithHostBundle: &*bundle,
            applicationBundle: &*bundle,
            userDriver: &*driver,
            delegate: ptr::null::<AnyObject>(),
        ];
        (driver, updater)
    };

    // Settled before the updater is started. Both values come from the host's
    // user defaults and Info.plist, which the updater reads from as soon as
    // it exists. The defaults read starts libdispatch workers that stay idle;
    // nothing of Sparkle runs until `startUpdater:`.
    if !force && !interval_elapsed(&updater) {
        return Outcome::Continue;
    }

    let mut error: Option<Retained<NSError>> = None;
    // SAFETY: `startUpdater:` takes an autoreleasing out-parameter.
    let started: bool = unsafe { msg_send![&*updater, startUpdater: &mut error] };
    if !started {
        // A start fails on Sparkle's check of its own configuration, which
        // comes before it schedules anything, so this still counts as no
        // check.
        let message = error.map_or_else(
            || String::from("no reason given"),
            |e| e.localizedDescription().to_string(),
        );
        return skip(
            force,
            Level::Warn,
            &format!("Sparkle would not start ({message})"),
        );
    }

    if force {
        // There will be a window either way, so check in before Sparkle
        // starts building one and let the pump take focus once it is up.
        SHOWING_UI.store(true, Ordering::Relaxed);
        ui::check_in_as_app();
    }

    info!("checking for updates");
    if force {
        // SAFETY: the user-initiated check, which reports its own result.
        unsafe { msg_send![&*updater, checkForUpdates] }
    } else {
        // SAFETY: shows nothing unless there is an update.
        unsafe { msg_send![&*updater, checkForUpdatesInBackground] }
    }

    pump_until_done(&updater);
    // The updater, the driver and its delegate all live until here, after the
    // session: dropping the driver earlier would tear the window out from
    // under the user.
    drop(driver);

    Outcome::RestartRequired
}

/// A check that does not run. The log says why, and so does a dialog when
/// the user asked for the check from the Dock menu: that launch shows nothing
/// else, so the menu item would otherwise seem to do nothing at all.
fn skip(force: bool, level: Level, reason: &str) -> Outcome {
    log!(level, "update check skipped: {reason}");
    if force {
        ui::error(
            &bundle::app_name(),
            &format!("Cannot check for updates: {reason}."),
        );
    }
    Outcome::Continue
}

/// Whether enough time has passed since the last check. Sparkle records the
/// date but never gets to act on it here, because the timer it would schedule
/// dies with the process a few hundred milliseconds later.
fn interval_elapsed(updater: &AnyObject) -> bool {
    // SAFETY: both are readonly properties on SPUUpdater.
    let (last, interval): (Option<Retained<NSDate>>, f64) = unsafe {
        (
            msg_send![updater, lastUpdateCheckDate],
            msg_send![updater, updateCheckInterval],
        )
    };
    let Some(last) = last else {
        return true;
    };
    // SAFETY: NSDate, in the past, so the interval comes back negative.
    let since: f64 = unsafe { msg_send![&*last, timeIntervalSinceNow] };
    if -since < interval {
        info!(
            "update check skipped: last one was {:.0}s ago, interval is {interval:.0}s",
            -since
        );
        return false;
    }
    true
}

/// Turn the run loop until Sparkle is done, or until a silent check has taken
/// long enough that the game should just start.
fn pump_until_done(updater: &AnyObject) {
    let run_loop = NSRunLoop::mainRunLoop();
    let started = Instant::now();
    let mut activated = false;

    loop {
        let until = NSDate::dateWithTimeIntervalSinceNow(PUMP_INTERVAL);
        // SAFETY: main thread, default mode, a date in the near future.
        unsafe { run_loop.runMode_beforeDate(NSDefaultRunLoopMode, &until) };

        let showing = SHOWING_UI.load(Ordering::Relaxed);
        if showing && !activated {
            // The window is up now, so this is the moment focus lands.
            ui::activate_app();
            activated = true;
        }

        // SAFETY: readonly property on SPUUpdater.
        let in_progress: bool = unsafe { msg_send![updater, sessionInProgress] };
        if !in_progress {
            return;
        }
        if !showing && started.elapsed() > SILENT_TIMEOUT {
            info!(
                "update check gave up after {SILENT_TIMEOUT:?}; a fresh instance will start the game"
            );
            return;
        }
    }
}

/// Whether the bundle sits on a read-only mount, which is what running from
/// the distribution image looks like. Updating that is not a thing.
fn on_read_only_volume(path: &Path) -> bool {
    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: statfs is plain C data, for which all zeroes is a valid value.
    let mut buf: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path and a buffer of the right type.
    if unsafe { libc::statfs(c_path.as_ptr(), &raw mut buf) } != 0 {
        return false;
    }
    buf.f_flags & libc::MNT_RDONLY as u32 != 0
}

/// Whether the bundle carries a Sparkle public key. `launcher/app/Info.plist`
/// has one, so every bundle built with it does; without it nothing the check
/// downloaded could be verified.
fn has_public_key(bundle: &NSBundle) -> bool {
    let key = NSString::from_str("SUPublicEDKey");
    // SAFETY: NSBundle's Info.plist accessor; the value is a string or nil.
    let value: Option<Retained<NSString>> =
        unsafe { msg_send![bundle, objectForInfoDictionaryKey: &*key] };
    value.is_some_and(|v| v.length() > 0)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn the_system_volume_is_read_only_and_the_temporary_directory_is_not() {
        assert!(on_read_only_volume(Path::new("/System")));
        assert!(!on_read_only_volume(&std::env::temp_dir()));
        // A path statfs cannot read is not taken for a disk image.
        assert!(!on_read_only_volume(Path::new("/nonexistent/gamelauncher")));
    }

    #[test]
    fn a_bundle_needs_a_non_empty_public_key() {
        let root = std::env::temp_dir().join(format!("gamelauncher-update-{}", std::process::id()));
        let bundle_with = |name: &str, key: Option<&str>| {
            let contents = root.join(format!("{name}.app/Contents"));
            fs::create_dir_all(&contents).unwrap();
            let entry = key.map_or_else(String::new, |key| {
                format!("<key>SUPublicEDKey</key><string>{key}</string>")
            });
            fs::write(
                contents.join("Info.plist"),
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                     <key>CFBundleIdentifier</key><string>app.mach9.test.{name}</string>{entry}\
                     </dict></plist>\n"
                ),
            )
            .unwrap();
            let path = root.join(format!("{name}.app"));
            NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy())).unwrap()
        };
        assert!(has_public_key(&bundle_with(
            "Keyed",
            Some("I3lqkTv6m+kA6p2XlMNfNItk32X19llHw9qFJc+3DX8=")
        )));
        assert!(!has_public_key(&bundle_with("Empty", Some(""))));
        assert!(!has_public_key(&bundle_with("Keyless", None)));
        fs::remove_dir_all(&root).unwrap();
    }
}
