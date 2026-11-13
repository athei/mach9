//! Hands the activation to each test process of a run.
//!
//! macOS activates a process that shows a window only when the active
//! application lets it: a test process started from the frontmost terminal
//! becomes the active application, one started while another application
//! is active does not. A test process that stays inactive gets no
//! activation or focus events, which the ddraw `test_coop_level_mode_set`
//! tests depend on (mach9 #101).
//!
//! So `run` and `record` make the runner a regular application ([`Host`])
//! that takes the activation when it starts and hands it to every Wine
//! process of its prefix that asks. winemac asks the other Wine processes
//! with the distributed notification `WineAppWillActivateNotification`
//! before it activates itself (`-tryToActivateIgnoringOtherApps:` in
//! `dlls/winemac.drv/cocoa_app.m`), and the runner answers as an active
//! Wine process of the same prefix does (`-otherWineAppWillActivate:`).
//! When neither the runner nor a process of the bundle is active, the
//! runner takes the activation back, from whichever application has it
//! ([`next_step`]).
//!
//! The run itself goes to a thread of its own, since `AppKit` and the
//! notifications need the main thread ([`Host::serve`]).
//!
//! The runner has no window, and the activation is the test processes' or
//! its own, so the terminal it was started from gets no key presses. A
//! quit request (Quit in the runner's Dock menu) stops the run as a signal
//! does: it sets the stop flag, and the run ends its Wine processes and
//! exits with 130. The runner also takes a Ctrl-C or Command-Q key event
//! that reaches it as a stop request, but a key press during a run does not
//! reach it reliably.
//!
//! On a locked screen no process becomes active, so a run refuses to start
//! there ([`check_unlocked`]).

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::{Retained, autoreleasepool},
    runtime::{NSObject, ProtocolObject},
    sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy,
    NSApplicationDelegate, NSApplicationTerminateReply, NSEvent, NSEventMask, NSEventModifierFlags,
    NSEventType, NSRunningApplication, NSWorkspace,
};
use objc2_foundation::{
    NSDate, NSDefaultRunLoopMode, NSDistributedNotificationCenter, NSNotification,
    NSNotificationSuspensionBehavior, NSNumber, NSObjectProtocol, NSString, ns_string,
};

use crate::{procs, session::Failure};

/// macOS's registry dump, whose root entry has `IOConsoleLocked`.
const IOREG_TOOL: &str = "/usr/sbin/ioreg";

/// How long one turn of the runner's event loop waits for an event.
const TURN: Duration = Duration::from_millis(200);

/// How long the runner may take to become active when it starts.
const START_LIMIT: Duration = Duration::from_secs(3);

/// After how many turns of failing to take the activation back the runner
/// says so.
const TAKE_WARNING: u32 = 10;

/// Refuse a locked screen.
///
/// # Errors
///
/// Returns a message when the screen is locked or the lock state cannot be
/// read.
pub fn check_unlocked() -> Result<(), String> {
    let dump = procs::output_limited(IOREG_TOOL, &["-n", "Root", "-d1"], procs::HELPER_LIMIT)?;
    match console_locked(&dump) {
        Some(false) => Ok(()),
        Some(true) => Err(String::from(
            "the screen is locked (IOConsoleLocked = Yes): no test process becomes the active \
             application there; unlock the screen and start again",
        )),
        None => Err(format!(
            "cannot tell whether the screen is locked: `{IOREG_TOOL} -n Root -d1` has no \
             IOConsoleLocked"
        )),
    }
}

/// The value of the root entry's `"IOConsoleLocked" = Yes|No` line in the
/// output of `ioreg -n Root -d1`.
#[must_use]
pub fn console_locked(dump: &str) -> Option<bool> {
    dump.lines().find_map(
        |line| match line.trim().strip_prefix("\"IOConsoleLocked\" = ")? {
            "Yes" => Some(true),
            "No" => Some(false),
            _ => None,
        },
    )
}

/// Who the frontmost application is, as the runner sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Front {
    /// The runner.
    Runner,
    /// A process of the bundle: a Wine process, which answers the requests
    /// of the others itself.
    Bundle,
    /// Any other application.
    Other,
    /// No application is frontmost.
    None,
}

/// What a turn of the runner's event loop does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Wait,
    /// Take the activation from the frontmost application.
    Take,
    /// Hand the activation to the process whose request came while the
    /// runner was not active.
    HandOver,
    /// Forget that request: the Wine process that is active answers it.
    Drop,
}

/// The step of a turn: whether the runner is `active`, who is frontmost,
/// and whether a request came while the runner was not active.
///
/// The runner is frontmost but not yet active between a take-back and the
/// activation that follows it; a request from then is kept for the turn
/// that finds the runner active. Only an active Wine process of the bundle
/// answers it in the runner's place.
#[must_use]
pub const fn next_step(active: bool, front: Front, pending: bool) -> Step {
    match (pending, active, front) {
        (true, true, _) => Step::HandOver,
        (true, false, Front::Bundle) => Step::Drop,
        (_, false, Front::Other | Front::None) => Step::Take,
        (_, false, Front::Runner) | (false, true, _) | (false, false, Front::Bundle) => Step::Wait,
    }
}

/// Whether the runner answers a request of the process `pid` for the
/// prefix `prefix`: one of another process of the runner's prefix.
#[must_use]
pub fn answers(pid: i32, prefix: &str, own_pid: i32, own_prefix: &str) -> bool {
    pid > 0 && pid != own_pid && prefix == own_prefix
}

/// Whether `executable` lies in the bundle `bundle`.
#[must_use]
pub fn in_bundle(executable: &Path, bundle: &Path) -> bool {
    executable.starts_with(bundle)
}

/// Whether a key press stops the run: Ctrl-C, as in a terminal, or
/// Command-Q, as in any application.
///
/// `modifiers` are the device independent ones, `key` the event's
/// `charactersIgnoringModifiers`, which keep the case Shift gives:
/// Ctrl-Shift-C is "C" and does not stop the run.
#[must_use]
pub fn stops_run(modifiers: NSEventModifierFlags, key: &str) -> bool {
    let only = |flag: NSEventModifierFlags| {
        modifiers
            & (NSEventModifierFlags::Control
                | NSEventModifierFlags::Command
                | NSEventModifierFlags::Option)
            == flag
    };
    match key {
        "c" => only(NSEventModifierFlags::Control),
        "q" => only(NSEventModifierFlags::Command),
        _ => false,
    }
}

/// The pid and prefix of a `WineAppWillActivateNotification`: its
/// `ActivatingAppPID` number and `ActivatingAppPrefix` string.
#[must_use]
pub fn request(note: &NSNotification) -> Option<(i32, String)> {
    let info = note.userInfo()?;
    let pid = info
        .objectForKey(ns_string!("ActivatingAppPID"))?
        .downcast::<NSNumber>()
        .ok()?
        .intValue();
    let prefix = info
        .objectForKey(ns_string!("ActivatingAppPrefix"))?
        .downcast::<NSString>()
        .ok()?
        .to_string();
    Some((pid, prefix))
}

/// The runner's view of the notifications and of quit requests, on the
/// main thread.
struct ObserverIvars {
    prefix: String,
    own_pid: i32,
    /// The run's stop flag.
    stop: &'static AtomicBool,
    /// The hand-overs so far.
    handed: Cell<usize>,
    /// The last request that came while the runner was not active.
    pending: Cell<Option<i32>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, the class is only
    // created through `Observer::new`, and the ivars are plain Rust values
    // the runtime never touches. It is main-thread-only: `Host::start`
    // creates it on the main thread and registers it there with the
    // distributed notification center, which delivers on the main run
    // loop, the one `Host::pump` runs; `NSApplication` calls its delegate
    // on the main thread.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ObserverIvars]
    struct Observer;

    unsafe impl NSObjectProtocol for Observer {}

    unsafe impl NSApplicationDelegate for Observer {
        /// A quit request (the Dock menu's Quit, an Apple event) stops the
        /// run instead of `exit`, which would leave its Wine processes up.
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _sender: &NSApplication) -> NSApplicationTerminateReply {
            self.ivars().stop.store(true, Ordering::SeqCst);
            NSApplicationTerminateReply::TerminateCancel
        }
    }

    impl Observer {
        /// A Wine process is about to activate itself.
        #[unsafe(method(wineWillActivate:))]
        fn wine_will_activate(&self, note: &NSNotification) {
            let Some((pid, prefix)) = request(note) else {
                return;
            };
            let ivars = self.ivars();
            if !answers(pid, &prefix, ivars.own_pid, &ivars.prefix) {
                return;
            }
            if NSApplication::sharedApplication(self.mtm()).isActive() {
                self.hand_over(pid);
            } else {
                ivars.pending.set(Some(pid));
            }
        }
    }
);

impl Observer {
    fn new(mtm: MainThreadMarker, ivars: ObserverIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: plain NSObject init on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }

    /// Yield the activation to `pid` and activate it, as winemac's
    /// `-otherWineAppWillActivate:` does; the second call covers a request
    /// whose sender activated itself before the yield.
    fn hand_over(&self, pid: i32) {
        let Some(other) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
            return;
        };
        NSApplication::sharedApplication(self.mtm()).yieldActivationToApplication(&other);
        other.activateFromApplication_options(
            &NSRunningApplication::currentApplication(),
            NSApplicationActivationOptions::empty(),
        );
        let handed = &self.ivars().handed;
        handed.set(handed.get() + 1);
    }
}

/// The runner as a regular application that hands the activation to the
/// test processes.
pub struct Host {
    app: Retained<NSApplication>,
    observer: Retained<Observer>,
    bundle: PathBuf,
    own_pid: i32,
    /// The times the runner took the activation back after it started.
    taken: usize,
    /// The turns in a row that tried to take the activation.
    taking: u32,
}

impl Host {
    /// Check that the screen is unlocked, make the runner a regular
    /// application, listen to the requests of the Wine processes of
    /// `prefix` and take the activation. `bundle` is the bundle whose
    /// processes answer such requests themselves. A quit request or a stop
    /// key sets `stop`.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::Setup`] off the main thread, on a locked screen,
    /// and when the runner does not become active.
    pub fn start(bundle: &Path, prefix: &Path, stop: &'static AtomicBool) -> Result<Self, Failure> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            Failure::Setup(String::from("the runner has to run on the main thread"))
        })?;
        check_unlocked().map_err(Failure::Setup)?;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let own_pid = NSRunningApplication::currentApplication().processIdentifier();
        let observer = Observer::new(
            mtm,
            ObserverIvars {
                prefix: prefix.to_string_lossy().into_owned(),
                own_pid,
                stop,
                handed: Cell::new(0),
                pending: Cell::new(None),
            },
        );
        // The delegate is a weak reference; `Host` keeps the observer and
        // `Drop` clears it.
        app.setDelegate(Some(ProtocolObject::from_ref(&*observer)));
        app.finishLaunching();
        // SAFETY: the observer is an `Observer`, which implements the
        // selector with one `NSNotification` argument, and `Drop` removes
        // it from the center before it is released.
        unsafe {
            NSDistributedNotificationCenter::defaultCenter()
                .addObserver_selector_name_object_suspensionBehavior(
                    &observer,
                    sel!(wineWillActivate:),
                    Some(ns_string!("WineAppWillActivateNotification")),
                    None,
                    NSNotificationSuspensionBehavior::DeliverImmediately,
                );
        }
        let mut host = Self {
            app,
            observer,
            bundle: std::fs::canonicalize(bundle).unwrap_or_else(|_| bundle.to_path_buf()),
            own_pid,
            taken: 0,
            taking: 0,
        };
        let deadline = Instant::now() + START_LIMIT;
        while !host.app.isActive() {
            if Instant::now() >= deadline {
                return Err(Failure::Setup(format!(
                    "the runner cannot become the active application, {} stays active; the \
                     test processes would not become active either",
                    host.frontmost_name()
                )));
            }
            host.take();
            host.pump(TURN);
        }
        host.taking = 0;
        println!(
            "conformance: the runner (pid {own_pid}) hands the activation to the test \
             processes; to stop the run, choose Quit in its Dock menu or send it SIGINT"
        );
        Ok(host)
    }

    /// Run `work` on a thread of its own and handle the window server's
    /// events and the requests meanwhile; return what `work` returns.
    pub fn serve<T: Send>(mut self, work: impl FnOnce() -> T + Send) -> T {
        let result = thread::scope(|scope| {
            let worker = scope.spawn(work);
            while !worker.is_finished() {
                self.turn();
            }
            worker
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        });
        println!(
            "conformance: handed the activation to {} Wine processes, took it back {} times",
            self.observer.ivars().handed.get(),
            self.taken
        );
        result
    }

    /// One turn: the events of up to [`TURN`], then [`next_step`].
    fn turn(&mut self) {
        self.pump(TURN);
        let ivars = self.observer.ivars();
        let (front, _) = self.frontmost();
        let step = next_step(self.app.isActive(), front, ivars.pending.get().is_some());
        if step != Step::Take {
            self.taking = 0;
        }
        match step {
            Step::Wait => {}
            Step::Drop => ivars.pending.set(None),
            Step::HandOver => {
                if let Some(pid) = ivars.pending.take() {
                    self.observer.hand_over(pid);
                }
            }
            Step::Take => {
                if self.taking == 0 {
                    self.taken += 1;
                }
                if self.taking == TAKE_WARNING {
                    println!(
                        "conformance: cannot take the activation back, {} stays active; a test \
                         process that starts now does not become active",
                        self.frontmost_name()
                    );
                }
                self.taking = self.taking.saturating_add(1);
                self.take();
            }
        }
    }

    /// Ask for the activation from the frontmost application. In probes of
    /// a process without a window, `-activateFromApplication:` with the
    /// frontmost application as the source took the activation both from
    /// the terminal that started the process and from an application that
    /// `LaunchServices` started; `-activate` took it from neither. With no
    /// frontmost application `-activate` is the only call left, and no
    /// probe showed it working: the runner then says after
    /// [`TAKE_WARNING`] turns, or at start, that it cannot take the
    /// activation.
    fn take(&self) {
        match NSWorkspace::sharedWorkspace().frontmostApplication() {
            Some(front) if front.processIdentifier() != self.own_pid => {
                NSRunningApplication::currentApplication().activateFromApplication_options(
                    &front,
                    NSApplicationActivationOptions::empty(),
                );
            }
            _ => self.app.activate(),
        }
    }

    /// The frontmost application and who it is.
    fn frontmost(&self) -> (Front, Option<Retained<NSRunningApplication>>) {
        let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
            return (Front::None, None);
        };
        let front = if app.processIdentifier() == self.own_pid {
            Front::Runner
        } else if app
            .executableURL()
            .and_then(|url| url.path())
            .is_some_and(|path| in_bundle(Path::new(&path.to_string()), &self.bundle))
        {
            Front::Bundle
        } else {
            Front::Other
        };
        (front, Some(app))
    }

    /// The frontmost application's name and pid, for a message.
    fn frontmost_name(&self) -> String {
        self.frontmost().1.map_or_else(
            || String::from("no application"),
            |app| {
                format!(
                    "{} (pid {})",
                    app.localizedName()
                        .map_or_else(|| String::from("an application"), |name| name.to_string()),
                    app.processIdentifier()
                )
            },
        )
    }

    /// Handle the window server's events: wait up to `timeout` for the
    /// first, then take what else is queued. The distributed notifications
    /// come in while it waits. A stop key sets the stop flag; the runner
    /// has no window or menu that would take it.
    fn pump(&self, timeout: Duration) {
        // SAFETY: an immutable constant of Foundation.
        let mode = unsafe { NSDefaultRunLoopMode };
        autoreleasepool(|_| {
            let mut until = NSDate::dateWithTimeIntervalSinceNow(timeout.as_secs_f64());
            while let Some(event) = self.app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&until),
                mode,
                true,
            ) {
                if is_stop_key(&event) {
                    self.observer.ivars().stop.store(true, Ordering::SeqCst);
                }
                self.app.sendEvent(&event);
                until = NSDate::distantPast();
            }
        });
    }
}

/// Whether `event` is a key press that stops the run ([`stops_run`]).
fn is_stop_key(event: &NSEvent) -> bool {
    event.r#type() == NSEventType::KeyDown
        && event.charactersIgnoringModifiers().is_some_and(|key| {
            stops_run(
                event.modifierFlags() & NSEventModifierFlags::DeviceIndependentFlagsMask,
                &key.to_string(),
            )
        })
}

impl Drop for Host {
    fn drop(&mut self) {
        self.app.setDelegate(None);
        // SAFETY: the observer was added in `start` and is still alive.
        unsafe { NSDistributedNotificationCenter::defaultCenter().removeObserver(&self.observer) };
    }
}

#[cfg(test)]
mod tests;
