//! The check-in that hands this process to a profiler.
//!
//! The hardened `wine.app` drops `DYLD_INSERT_LIBRARIES`, so a profiler starts
//! the process with the name of its bootstrap service in
//! [`PROF_SERVICE_VAR`] and this library sends the service one message with
//! the task port and the image name. Without the variable nothing here runs.
//! On a failure the process logs a line and carries on unprofiled; it never
//! retries and no answer comes back.
//!
//! The message is the control port by `COPY_SEND`. A task's read port is not
//! movable: sending it ends the process with `EXC_GUARD`.

use std::ffi::CString;

use compatdb_core::PROF_SERVICE_VAR;
use mach2::{
    bootstrap::{BOOTSTRAP_MAX_NAME_LEN, BOOTSTRAP_SUCCESS, bootstrap_look_up, bootstrap_port},
    mach_port::mach_port_deallocate,
    message::{
        MACH_MSG_SUCCESS, MACH_MSG_TYPE_COPY_SEND, MACH_MSGH_BITS, MACH_MSGH_BITS_COMPLEX,
        MACH_SEND_MSG, MACH_SEND_TIMED_OUT, MACH_SEND_TIMEOUT, mach_msg, mach_msg_body_t,
        mach_msg_destroy, mach_msg_header_t, mach_msg_id_t, mach_msg_port_descriptor_t,
        mach_msg_return_t,
    },
    port::{MACH_PORT_NULL, mach_port_t},
    traps::mach_task_self,
};

use crate::{log, read_env};

/// Identifies the message.
const MESSAGE_ID: mach_msg_id_t = 0x5750_0001;
/// The version of the layout of [`CheckIn`], which a change to it raises.
const VERSION: u32 = 1;
/// The size of [`CheckIn`] on the wire.
const MESSAGE_BYTES: u32 = 176;
/// The size of the field for the image name, its terminator included.
const EXE_BYTES: usize = 128;
/// How long a send may wait, which it does only on a full queue.
const SEND_TIMEOUT_MS: u32 = 50;

const _: () = assert!(size_of::<CheckIn>() == MESSAGE_BYTES as usize);

/// The message. It is copied from the profiler's protocol, not shared with it.
#[repr(C)]
struct CheckIn {
    header: mach_msg_header_t,
    body: mach_msg_body_t,
    task: mach_msg_port_descriptor_t,
    version: u32,
    pid: u32,
    exe: [u8; EXE_BYTES],
}

impl CheckIn {
    /// The message that hands `task`, of the process `pid` running the image
    /// `exe`, to `service`.
    fn new(service: mach_port_t, task: mach_port_t, pid: u32, exe: &str) -> Self {
        Self {
            header: mach_msg_header_t {
                msgh_bits: MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0) | MACH_MSGH_BITS_COMPLEX,
                msgh_size: MESSAGE_BYTES,
                msgh_remote_port: service,
                msgh_local_port: MACH_PORT_NULL,
                msgh_voucher_port: MACH_PORT_NULL,
                msgh_id: MESSAGE_ID,
            },
            body: mach_msg_body_t {
                msgh_descriptor_count: 1,
            },
            task: mach_msg_port_descriptor_t::new(task, MACH_MSG_TYPE_COPY_SEND),
            version: VERSION,
            pid,
            exe: exe_field(exe),
        }
    }
}

/// `exe`, zero-padded and NUL-terminated, cut on a character boundary so the
/// field stays UTF-8.
fn exe_field(exe: &str) -> [u8; EXE_BYTES] {
    let mut field = [0; EXE_BYTES];
    let end = exe.floor_char_boundary(EXE_BYTES - 1);
    for (to, from) in field.iter_mut().zip(exe.bytes().take(end)) {
        *to = from;
    }
    field
}

/// `name` as `bootstrap_look_up` takes it, if it can be a bootstrap name at
/// all.
fn bootstrap_name(name: &str) -> Option<CString> {
    if name.len() < BOOTSTRAP_MAX_NAME_LEN as usize {
        CString::new(name).ok()
    } else {
        None
    }
}

/// Send `service` the message for this process.
fn send(service: mach_port_t, exe: &str) -> mach_msg_return_t {
    // SAFETY: both only read process state and have no preconditions.
    let (task, pid) = unsafe { (mach_task_self(), libc::getpid()) };
    let mut message = CheckIn::new(service, task, pid.cast_unsigned(), exe);
    let header = (&raw mut message).cast::<mach_msg_header_t>();
    // SAFETY: `header` starts a whole message of MESSAGE_BYTES that lives past
    // the call, and MACH_SEND_TIMEOUT bounds the wait.
    let result = unsafe {
        mach_msg(
            header,
            MACH_SEND_MSG | MACH_SEND_TIMEOUT,
            MESSAGE_BYTES,
            0,
            MACH_PORT_NULL,
            SEND_TIMEOUT_MS,
            MACH_PORT_NULL,
        )
    };
    if result == MACH_SEND_TIMED_OUT {
        // SAFETY: the kernel put the rights it had taken back into `message`,
        // one reference each on `service` and the task, which this releases.
        // After any other error the rights are still in the sender's hands,
        // so there is nothing to destroy.
        unsafe { mach_msg_destroy(header) };
    }
    result
}

/// Look up the service `name` and send it the message.
fn hand_over(name: &str, exe: &str) -> Result<(), String> {
    let name = bootstrap_name(name).ok_or("not a bootstrap name")?;
    let mut service = MACH_PORT_NULL;
    // SAFETY: `name` is NUL-terminated and `service` takes the send right.
    let result = unsafe { bootstrap_look_up(bootstrap_port, name.as_ptr(), &raw mut service) };
    if result != BOOTSTRAP_SUCCESS {
        return Err(format!("bootstrap_look_up returned {result:#x}"));
    }
    let result = send(service, exe);
    // SAFETY: the lookup gave this function the right to `service`.
    unsafe { mach_port_deallocate(mach_task_self(), service) };
    if result == MACH_MSG_SUCCESS {
        Ok(())
    } else {
        Err(format!("mach_msg returned {result:#x}"))
    }
}

/// Check in with the service [`PROF_SERVICE_VAR`] names, if it names one. An
/// empty value counts as unset, as for the rules. The process is `exe`.
pub fn check_in(exe: &str) {
    let Some(name) = read_env(PROF_SERVICE_VAR).filter(|name| !name.is_empty()) else {
        return;
    };
    match hand_over(&name, exe) {
        Ok(()) => log::line(&format!("  {PROF_SERVICE_VAR} = {name}: checked in")),
        Err(why) => log::line(&format!(
            "  {PROF_SERVICE_VAR} = {name}: {why}, not profiled"
        )),
    }
}

#[cfg(test)]
mod tests;
