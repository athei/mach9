//! Turns Ctrl-C, `SIGTERM` and `SIGHUP` into a flag the runner polls, so an
//! interrupted invocation ends the Wine processes of its prefix before it
//! exits.
//!
//! [`install`] gives the three signals a handler that stores to
//! [`REQUESTED`] and puts all three back to their default action. The
//! runner polls the flag while it waits for a process and between tests;
//! once it is set, it kills the running test's process group, ends the
//! prefix's session with `wineserver -k` and exits. A second signal, during
//! that cleanup or at any later point, ends the runner at once by the
//! default action; what the cleanup had not ended yet keeps running, and
//! the next invocation ends the server of its own prefix.
//!
//! The Wine processes run in process groups of their own, so a Ctrl-C at
//! the terminal reaches the runner and not them. A handler, and not a
//! blocked mask with a `sigwait` thread, because a child inherits the
//! signal mask through `exec` but not a handler: blocked signals would
//! reach every Wine process blocked.

use std::{
    ffi::c_int,
    sync::atomic::{AtomicBool, Ordering},
};

use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};

/// Set once one of the signals arrived.
pub static REQUESTED: AtomicBool = AtomicBool::new(false);

/// The signals that interrupt a run.
const SIGNALS: [Signal; 3] = [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP];

/// The handler: an atomic store, then the default action for the next
/// signal; both are async-signal-safe.
extern "C" fn on_signal(_: c_int) {
    REQUESTED.store(true, Ordering::SeqCst);
    let default = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
    for signal in SIGNALS {
        // SAFETY: the default action runs no code of this process.
        // sigaction(2) is async-signal-safe, and nix's wrapper only builds
        // its arguments on the stack before it calls it.
        let _ = unsafe { sigaction(signal, &default) };
    }
}

/// Install the handler for `SIGINT`, `SIGTERM` and `SIGHUP`.
///
/// `SA_RESTART` lets the reads and waits the signal interrupts go on, so
/// only the flag tells the runner about it.
///
/// # Errors
///
/// Returns a message naming the signal whose handler could not be set.
pub fn install() -> Result<(), String> {
    let action = SigAction::new(
        SigHandler::Handler(on_signal),
        SaFlags::SA_RESTART,
        SigSet::empty(),
    );
    for signal in SIGNALS {
        // SAFETY: `on_signal` only stores to an `AtomicBool` and calls
        // sigaction(2), both async-signal-safe, and touches no other state,
        // so it may run at any point of any thread.
        unsafe { sigaction(signal, &action) }
            .map_err(|e| format!("cannot handle {signal}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
