use super::*;

/// The action of `signal` now, read by setting it to itself.
fn current(signal: Signal) -> SigHandler {
    let probe = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
    // SAFETY: the default action runs no code of this process; the old
    // action is put back right after.
    let old = unsafe { sigaction(signal, &probe) }.unwrap();
    // SAFETY: puts back the action that was set before the probe.
    unsafe { sigaction(signal, &old) }.unwrap();
    old.handler()
}

/// After the first signal the next one takes the default action, which
/// ends the runner at once, for each of the three signals.
#[test]
fn the_first_signal_puts_the_default_action_back() {
    install().unwrap();
    for signal in SIGNALS {
        assert!(
            matches!(current(signal), SigHandler::Handler(_)),
            "{signal}"
        );
    }
    on_signal(Signal::SIGINT as c_int);
    assert!(REQUESTED.load(Ordering::SeqCst));
    for signal in SIGNALS {
        assert!(matches!(current(signal), SigHandler::SigDfl), "{signal}");
    }
}
