//! The app's opener: a helper app inside it, at
//! `gamelauncher_core::bundle::opener_app`, that declares the Windows file
//! types, so Finder's double-click and Open With reach the app.
//!
//! Launch Services hands an opened file over only as an Apple Event, and only
//! to a process that has checked in with `AppKit`. The launcher avoids that
//! before it execs into wine, and cannot tell a launch with a file from one
//! without checking in, so the file comes here instead. The opener opens the
//! app with the file as a one-shot `executable` override, the way the New
//! Executable window runs something without saving it, and quits.

use std::{path::Path, process::Command};

use gamelauncher_core::{bundle, words};
use objc2::{
    MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{NSObject, NSObjectProtocol, ProtocolObject},
};
use objc2_app_kit::{NSApplication, NSApplicationDelegate};
use objc2_foundation::{NSArray, NSNotification, NSURL};

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Opener has no
    // `Drop` and no ivars.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct Opener;

    unsafe impl NSObjectProtocol for Opener {}

    unsafe impl NSApplicationDelegate for Opener {
        /// A launch with files gets them before `applicationDidFinishLaunching`.
        /// Each file runs in an instance of its own.
        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, app: &NSApplication, urls: &NSArray<NSURL>) {
            for file in urls
                .iter()
                .filter(|url| url.isFileURL())
                .filter_map(|url| url.path())
            {
                open_in_app(Path::new(&file.to_string()));
            }
            app.terminate(None);
        }

        /// Without a file there is nothing to hand on.
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, notification: &NSNotification) {
            let _ = notification;
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

impl Opener {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: plain NSObject init on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

/// Open the app this opener is in to run `file` once. A new instance, as the
/// Dock menu starts one, so a game that is running keeps its own.
fn open_in_app(file: &Path) {
    let Some(file) = file.to_str() else {
        eprintln!("gamelauncher-opener: {} is not UTF-8", file.display());
        return;
    };
    let Some(app) = std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(bundle::enclosing_app)
        .as_deref()
        .and_then(bundle::enclosing_app)
    else {
        eprintln!("gamelauncher-opener: not inside an app's opener");
        return;
    };
    let status = Command::new("/usr/bin/open")
        .arg("-n")
        .arg(&app)
        .arg("--args")
        .arg("--config")
        .arg(format!("executable={}", words::quote(file)))
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!(
            "gamelauncher-opener: open {} failed with {status}",
            app.display()
        ),
        Err(e) => eprintln!("gamelauncher-opener: open {}: {e}", app.display()),
    }
}

fn main() {
    let mtm = MainThreadMarker::new().expect("main runs on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let opener = Opener::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*opener)));
    app.run();
}
