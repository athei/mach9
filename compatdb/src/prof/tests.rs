use mach2::{
    kern_return::{KERN_SUCCESS, kern_return_t},
    mach_port::{mach_port_allocate, mach_port_insert_right, mach_port_mod_refs},
    message::MACH_MSG_TYPE_MAKE_SEND,
    port::{MACH_PORT_RIGHT_RECEIVE, MACH_PORT_RIGHT_SEND, mach_port_right_t},
};

use super::*;

/// What a test returns, so a failed call fails it through `?` rather than a
/// panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

unsafe extern "C" {
    fn mach_port_get_refs(
        task: mach_port_t,
        name: mach_port_t,
        right: mach_port_right_t,
        refs: *mut u32,
    ) -> kern_return_t;
}

/// The bytes of `message`.
const fn bytes(message: &CheckIn) -> &[u8] {
    // SAFETY: `message` is valid for its size, and CheckIn is exactly its
    // fields with no padding (the assertion on its size), all initialized.
    unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref(message).cast::<u8>(),
            size_of::<CheckIn>(),
        )
    }
}

/// `words` in the byte order of the wire.
fn le(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

#[test]
fn the_message_has_the_bytes_of_the_protocol() {
    let mut want = le(&[
        0x8000_0013, // complex, remote COPY_SEND
        176,
        0x1234, // the service
        0,
        0,
        0x5750_0001,
        1,      // one descriptor
        0x5678, // the task
        0,
    ]);
    want.extend([0, 0, 19, 0]); // pad2, COPY_SEND, a port descriptor
    want.extend(le(&[1, 4242]));
    want.extend(b"game.exe");
    want.resize(176, 0);

    assert_eq!(
        bytes(&CheckIn::new(0x1234, 0x5678, 4242, "game.exe")),
        want.as_slice()
    );
}

#[test]
fn an_exe_that_fits_stays_whole_and_terminated() {
    let exe = "x".repeat(EXE_BYTES - 1);
    let field = exe_field(&exe);
    assert_eq!(field.get(..EXE_BYTES - 1), Some(exe.as_bytes()));
    assert_eq!(field.last(), Some(&0));
    assert_eq!(exe_field(""), [0; EXE_BYTES]);
}

#[test]
fn a_long_exe_is_cut_to_a_whole_character_and_stays_terminated() -> TestResult {
    let field = exe_field(&"x".repeat(EXE_BYTES * 2));
    assert_eq!(exe_field(&"x".repeat(EXE_BYTES - 1)), field);

    // The two bytes of `é` would span the end of the name.
    let field = exe_field(&format!("{}é", "x".repeat(EXE_BYTES - 2)));
    let name = core::ffi::CStr::from_bytes_until_nul(&field)?.to_str()?;
    assert_eq!(name, "x".repeat(EXE_BYTES - 2));
    Ok(())
}

#[test]
fn a_bootstrap_name_has_at_most_127_bytes_and_no_nul() {
    assert!(bootstrap_name(&"x".repeat(127)).is_some());
    assert!(bootstrap_name(&"x".repeat(128)).is_none());
    assert!(bootstrap_name("x\0y").is_none());
}

/// The send rights this task holds to `port`, in user references.
fn send_refs(port: mach_port_t) -> Result<u32, String> {
    let mut refs = 0;
    // SAFETY: `refs` is written and both ports are names of this task.
    let result =
        unsafe { mach_port_get_refs(mach_task_self(), port, MACH_PORT_RIGHT_SEND, &raw mut refs) };
    if result == KERN_SUCCESS {
        Ok(refs)
    } else {
        Err(format!("mach_port_get_refs returned {result:#x}"))
    }
}

#[test]
fn a_send_that_times_out_keeps_no_reference() -> TestResult {
    // SAFETY: reads libSystem's task port.
    let task = unsafe { mach_task_self() };
    let mut service = MACH_PORT_NULL;
    // SAFETY: `service` takes the name of the new port.
    let result = unsafe { mach_port_allocate(task, MACH_PORT_RIGHT_RECEIVE, &raw mut service) };
    assert_eq!(result, KERN_SUCCESS);
    // SAFETY: `service` is the receive right allocated above.
    let result = unsafe { mach_port_insert_right(task, service, service, MACH_MSG_TYPE_MAKE_SEND) };
    assert_eq!(result, KERN_SUCCESS);

    let before = (send_refs(service)?, send_refs(task)?);
    // Nothing receives, so the queue fills and the last send times out. A
    // message in the queue holds kernel references, not user ones.
    let failure = (0..64)
        .map(|_| send(service, "game.exe"))
        .find(|&result| result != MACH_MSG_SUCCESS);
    assert_eq!(failure, Some(MACH_SEND_TIMED_OUT));
    assert_eq!((send_refs(service)?, send_refs(task)?), before);

    // SAFETY: both rights are the ones made above.
    unsafe {
        mach_port_mod_refs(task, service, MACH_PORT_RIGHT_RECEIVE, -1);
        mach_port_deallocate(task, service);
    }
    Ok(())
}
