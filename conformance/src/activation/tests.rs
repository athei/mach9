use objc2::runtime::AnyObject;
use objc2_foundation::NSDictionary;

use super::*;

/// The root entry's lines of `ioreg -n Root -d1`, cut down; the nested
/// dictionaries spell their keys without spaces around `=`.
const UNLOCKED: &str = r#"+-o Root  <class IORegistryEntry, id 0x100000100, retain 46>
    {
      "IOKitBuildVersion" = "Darwin Kernel Version 27.2.0"
      "IOConsoleLocked" = No
      "IOConsoleUsers" = ({"kCGSSessionOnConsoleKey"=Yes,"kCGSessionLoginDoneKey"=Yes})
    }
"#;

#[test]
fn the_lock_state_is_the_root_entrys_line() {
    assert_eq!(console_locked(UNLOCKED), Some(false));
    let locked = UNLOCKED.replace("\"IOConsoleLocked\" = No", "\"IOConsoleLocked\" = Yes");
    assert_eq!(console_locked(&locked), Some(true));
}

#[test]
fn no_lock_line_or_another_value_is_no_state() {
    let without = UNLOCKED.replace("      \"IOConsoleLocked\" = No\n", "");
    assert!(!without.contains("IOConsoleLocked"));
    assert_eq!(console_locked(&without), None);
    // A nested dictionary's key does not count.
    assert_eq!(
        console_locked("      \"IOConsoleUsers\" = ({\"IOConsoleLocked\"=Yes})\n"),
        None
    );
    assert_eq!(console_locked("      \"IOConsoleLocked\" = Maybe\n"), None);
}

#[test]
fn a_request_is_answered_only_for_another_process_of_the_prefix() {
    let prefix = "/root/build/conformance/prefix";
    assert!(answers(812, prefix, 100, prefix));
    assert!(!answers(100, prefix, 100, prefix), "the runner itself");
    assert!(!answers(0, prefix, 100, prefix), "no pid");
    assert!(!answers(-1, prefix, 100, prefix), "no pid");
    assert!(
        !answers(812, "/Users/me/.wine", 100, prefix),
        "another prefix"
    );
    assert!(!answers(812, "", 100, prefix), "no prefix");
}

#[test]
fn a_process_is_the_bundles_when_its_executable_lies_in_it() {
    let bundle = Path::new("/root/dist/wine");
    assert!(in_bundle(
        Path::new("/root/dist/wine/lib/wine/aarch64-unix/wine.app/Contents/MacOS/wine"),
        bundle
    ));
    assert!(!in_bundle(Path::new("/root/dist/wine2/bin/wine"), bundle));
    assert!(!in_bundle(
        Path::new("/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"),
        bundle
    ));
}

#[test]
fn an_active_runner_hands_over_a_pending_request_and_else_waits() {
    for front in [Front::Runner, Front::Bundle, Front::Other, Front::None] {
        assert_eq!(next_step(true, front, true), Step::HandOver, "{front:?}");
        assert_eq!(next_step(true, front, false), Step::Wait, "{front:?}");
    }
}

#[test]
fn an_inactive_runner_takes_the_activation_from_any_other_application() {
    for pending in [false, true] {
        assert_eq!(next_step(false, Front::Other, pending), Step::Take);
        assert_eq!(next_step(false, Front::None, pending), Step::Take);
    }
}

/// While a Wine process of the bundle is active, it answers the requests
/// itself.
#[test]
fn an_inactive_runner_leaves_an_active_wine_process_alone() {
    assert_eq!(next_step(false, Front::Bundle, false), Step::Wait);
    assert_eq!(next_step(false, Front::Bundle, true), Step::Drop);
}

/// Frontmost but not active: between a take-back and the activation, or
/// while a hand-over completes. A request from then waits for the turn
/// that finds the runner active, or a Wine process frontmost.
#[test]
fn a_frontmost_inactive_runner_keeps_a_request() {
    assert_eq!(next_step(false, Front::Runner, false), Step::Wait);
    assert_eq!(next_step(false, Front::Runner, true), Step::Wait);
    assert_eq!(next_step(true, Front::Runner, true), Step::HandOver);
}

#[test]
fn ctrl_c_and_command_q_stop_the_run() {
    let control = NSEventModifierFlags::Control;
    let command = NSEventModifierFlags::Command;
    assert!(stops_run(control, "c"));
    assert!(stops_run(command, "q"));
    assert!(!stops_run(command, "c"), "copy");
    assert!(!stops_run(control, "q"));
    assert!(!stops_run(command | NSEventModifierFlags::Option, "q"));
    assert!(!stops_run(NSEventModifierFlags::empty(), "c"));
    assert!(!stops_run(control, "x"));
}

/// A `WineAppWillActivateNotification` as winemac posts it, with `info` as
/// its user info.
fn note(info: &[(&NSString, &AnyObject)]) -> Retained<NSNotification> {
    let keys: Vec<&NSString> = info.iter().map(|(key, _)| *key).collect();
    let values: Vec<&AnyObject> = info.iter().map(|(_, value)| *value).collect();
    let info = NSDictionary::<NSString, AnyObject>::from_slices(&keys, &values);
    // SAFETY: a dictionary with string keys is one with object keys; the
    // notification only reads it.
    let info = unsafe { Retained::cast_unchecked::<NSDictionary>(info) };
    // SAFETY: the name is a string, the object is none and the user info a
    // dictionary, the types `notificationWithName:object:userInfo:` takes.
    unsafe {
        NSNotification::notificationWithName_object_userInfo(
            ns_string!("WineAppWillActivateNotification"),
            None,
            Some(&info),
        )
    }
}

#[test]
fn a_request_has_the_pid_and_the_prefix() {
    let pid = NSNumber::new_i32(812);
    let prefix = ns_string!("/root/build/conformance/prefix");
    let config = ns_string!("");
    let full = note(&[
        (ns_string!("ActivatingAppPID"), &pid),
        (ns_string!("ActivatingAppPrefix"), prefix),
        (ns_string!("ActivatingAppConfigDir"), config),
    ]);
    assert_eq!(
        request(&full),
        Some((812, String::from("/root/build/conformance/prefix")))
    );
}

#[test]
fn a_request_without_a_key_or_of_another_type_is_none() {
    let pid = NSNumber::new_i32(812);
    let prefix = ns_string!("/root/build/conformance/prefix");
    let without_pid = note(&[(ns_string!("ActivatingAppPrefix"), prefix)]);
    assert_eq!(request(&without_pid), None);
    let without_prefix = note(&[(ns_string!("ActivatingAppPID"), &pid)]);
    assert_eq!(request(&without_prefix), None);
    let pid_as_string = note(&[
        (ns_string!("ActivatingAppPID"), ns_string!("812")),
        (ns_string!("ActivatingAppPrefix"), prefix),
    ]);
    assert_eq!(request(&pid_as_string), None);
    let prefix_as_number = note(&[
        (ns_string!("ActivatingAppPID"), &pid),
        (ns_string!("ActivatingAppPrefix"), &pid),
    ]);
    assert_eq!(request(&prefix_as_number), None);
    // SAFETY: as in `note`, without user info.
    let bare = unsafe {
        NSNotification::notificationWithName_object_userInfo(
            ns_string!("WineAppWillActivateNotification"),
            None,
            None,
        )
    };
    assert_eq!(request(&bare), None);
}
