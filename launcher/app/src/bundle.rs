use std::{
    env,
    fs::OpenOptions,
    io,
    os::fd::{AsRawFd, IntoRawFd, RawFd},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    thread,
    time::Duration,
};

pub use gamelauncher_core::bundle::*;
use log::info;

pub fn resources_dir() -> io::Result<PathBuf> {
    let exe = env::current_exe()?;
    let macos_dir = exe.parent().ok_or_else(|| {
        io::Error::other(format!("executable path has no parent: {}", exe.display()))
    })?;
    let contents = macos_dir.parent().ok_or_else(|| {
        io::Error::other(format!("MacOS dir has no parent: {}", macos_dir.display()))
    })?;
    let resources = contents.join("Resources");
    if !resources.is_dir() {
        return Err(io::Error::other(format!(
            "Resources dir not found at {}",
            resources.display()
        )));
    }
    Ok(resources)
}

/// Name of the enclosing `.app`, used as the dialog title. Falls back to the
/// binary's own name when the launcher runs outside a bundle.
pub fn app_name() -> String {
    let exe = env::current_exe().ok();
    app_bundle()
        .as_deref()
        .and_then(Path::file_stem)
        .or_else(|| exe.as_deref().and_then(Path::file_stem))
        .and_then(|name| name.to_str())
        .map_or_else(|| String::from("gamelauncher"), str::to_owned)
}

/// The enclosing `.app`, or `None` when the launcher runs outside a bundle.
fn app_bundle() -> Option<PathBuf> {
    enclosing_app(&env::current_exe().ok()?)
}

/// Start a fresh instance of this app through Launch Services, with `args`
/// on its command line, once this process has exited.
///
/// Not an exec: a process that has shown a dialog is registered with the
/// window server for good, an exec keeps the process, and running wine in one
/// that has already checked in with the window server and Launch Services is
/// untested, so it is avoided as a precaution. And not a launch from here
/// either: while this process is alive it owns the app's Dock tile, and an
/// instance started next to it gets a tile of its own that stays for the
/// whole session. So the launcher binary is started as a plain child in
/// `--relaunch-after` mode (no Launch Services, so no tile), it waits for
/// this process to go away and then opens the app, which lands on the pinned
/// tile as a Dock click does. The caller returns right after.
pub fn relaunch(args: &[String]) -> io::Result<()> {
    let app = app_bundle().ok_or_else(|| io::Error::other("not running inside an app bundle"))?;
    let helper = env::current_exe()?;
    info!(
        "starting a fresh instance after exit: open -n {} {}",
        app.display(),
        args.join(" ")
    );
    Command::new(helper)
        .arg("--relaunch-after")
        .arg(process::id().to_string())
        .arg("--")
        .args(args)
        .stdin(Stdio::null())
        .spawn()?;
    Ok(())
}

/// The `--relaunch-after` mode: wait for `pid` to exit, then open this app
/// with `args`. Errors go to stderr, which is the log of the process that
/// started the helper.
pub fn relaunch_after(pid: u32, args: &[String]) -> io::Result<()> {
    let app = app_bundle().ok_or_else(|| io::Error::other("not running inside an app bundle"))?;
    if !wait_for_exit(pid, Duration::from_mins(1)) {
        eprintln!("gamelauncher: process {pid} is still running after 60s; opening the app anyway");
    }
    // Launch Services notices the exit a moment later; opening before that
    // still counts as a second instance.
    thread::sleep(Duration::from_millis(200));
    let mut command = Command::new("/usr/bin/open");
    command.arg("-n").arg(&app);
    if !args.is_empty() {
        command.arg("--args").args(args);
    }
    let status = command.status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "open {} failed with {status}",
            app.display()
        )));
    }
    Ok(())
}

/// Wait until the process has exited or `timeout` is up. Returns whether it
/// exited. Through kqueue's process filter: it fires on exit itself, whether
/// or not the parent has reaped the process, and a pid that is already gone
/// is reported as such when the filter is added.
fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    // SAFETY: plain kqueue calls on a descriptor this function owns; the
    // kevent structs are fully initialised and only read by the kernel.
    unsafe {
        let kq = libc::kqueue();
        if kq < 0 {
            eprintln!("gamelauncher: kqueue: {}", io::Error::last_os_error());
            return false;
        }
        let change = libc::kevent {
            ident: pid as usize,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let mut event: libc::kevent = std::mem::zeroed();
        let wait = libc::timespec {
            tv_sec: libc::time_t::try_from(timeout.as_secs()).unwrap_or(libc::time_t::MAX),
            tv_nsec: libc::c_long::from(timeout.subsec_nanos()),
        };
        let n = libc::kevent(kq, &raw const change, 1, &raw mut event, 1, &raw const wait);
        let error = io::Error::last_os_error();
        libc::close(kq);
        if n < 0 {
            eprintln!("gamelauncher: kevent: {error}");
            return false;
        }
        if n == 0 {
            return false;
        }
        // A registration failure comes back as an event: ESRCH means the
        // process was gone before the filter was added.
        event.flags & libc::EV_ERROR == 0 || event.data == libc::ESRCH as isize
    }
}

/// `CFBundleIdentifier` of the app whose `Resources` dir this is.
pub fn bundle_id(resources_dir: &Path) -> io::Result<String> {
    let app = enclosing_app(resources_dir).ok_or_else(|| {
        io::Error::other(format!(
            "{} is not inside an app bundle",
            resources_dir.display()
        ))
    })?;
    bundle_id_of_app(&app)
}

/// Whether Launch Services started this process: a launch from Finder or the
/// Dock, which sets `__CFBundleIdentifier` to the app's own id. Comparing the
/// value matters: Terminal.app exports `__CFBundleIdentifier` to the shells
/// it starts, so mere presence would also be true for terminal launches.
/// False when the bundle id cannot be read, as on an error path that failed
/// to read it.
pub fn launched_via_launch_services() -> bool {
    let Some(value) = env::var_os("__CFBundleIdentifier") else {
        return false;
    };
    resources_dir()
        .and_then(|resources| bundle_id(&resources))
        .is_ok_and(|id| value == *id)
}

/// Open the log file and dup2 it onto stdout/stderr. Returns the underlying
/// fd so the pty pump can write to it directly. Once both dup2 calls worked
/// the fd is taken out of its `File` and stays open for the rest of the
/// process; when one fails, the `File` closes it.
pub fn redirect_stdio_to(log_path: &Path) -> io::Result<RawFd> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(log_path)?;
    for target in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        // SAFETY: duplicates a descriptor `file` owns onto a standard one,
        // which Rust's stdout and stderr go on using by number.
        if unsafe { libc::dup2(file.as_raw_fd(), target) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(file.into_raw_fd())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// The child is not reaped until the end, so the exit has to be seen on
    /// a zombie; and a pid that is already gone counts as exited.
    #[test]
    fn waits_for_a_process_to_exit() {
        let mut child = Command::new("/bin/sleep").arg("0.3").spawn().unwrap();
        let started = Instant::now();
        assert!(wait_for_exit(child.id(), Duration::from_secs(5)));
        assert!(started.elapsed() >= Duration::from_millis(250));
        child.wait().unwrap();
        assert!(wait_for_exit(child.id(), Duration::from_secs(5)));
    }

    #[test]
    fn gives_up_after_the_timeout() {
        let mut child = Command::new("/bin/sleep").arg("5").spawn().unwrap();
        assert!(!wait_for_exit(child.id(), Duration::from_millis(150)));
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
