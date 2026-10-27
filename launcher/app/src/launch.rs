use std::{
    convert::Infallible,
    env,
    ffi::{CStr, CString},
    io, mem,
    os::{
        fd::{AsRawFd, RawFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
    sync::atomic::{AtomicI32, Ordering},
};

use nix::{
    errno::Errno,
    pty::{OpenptyResult, openpty},
    spawn::{PosixSpawnAttr, PosixSpawnFileActions, PosixSpawnFlags, posix_spawn},
    sys::signal::SigSet,
    unistd::{Pid, dup2_stderr, dup2_stdin, dup2_stdout, execv, getpid, getppid},
};

/// How much of wine's output the pump reads at a time.
const CHUNK: usize = 8192;

/// How long the pump waits on a quiet pty, in milliseconds, before it checks
/// again whether wine is still running. Also the longest it can go on after
/// wine has exited, and how often a pump that ended early checks whether
/// wine is still there.
const WINE_CHECK_MS: libc::c_int = 500;

/// How many reads the pump makes, at most, of what is waiting in the pty once
/// wine has exited. A bound, so a process left writing cannot keep it going.
const DRAIN_CHUNKS: u32 = 16;

/// The wine process the pump's SIGTERM handler kills, 0 until the pump sets
/// it. A static because the handler gets nothing but the signal number.
static SIGTERM_KILLS: AtomicI32 = AtomicI32::new(0);

/// The log the pump's SIGTERM handler reports the kill in.
static SIGTERM_LOG: AtomicI32 = AtomicI32::new(-1);

/// What the pump's SIGTERM handler writes to the log before it kills wine.
/// It starts with a newline, since it can come in the middle of a line of
/// wine's output.
const SIGTERM_MESSAGE: &[u8] =
    b"\ngamelauncher: the log pump got SIGTERM, killing wine with SIGKILL\n";

/// Replace this process with wine, for a launch from a terminal. Returns only
/// with an error.
pub fn exec_direct(
    wine_bin: &Path,
    game_dir: &Path,
    exe_arg: &str,
    arguments: &[String],
) -> io::Result<Infallible> {
    env::set_current_dir(game_dir)?;
    let argv_c = build_argv(wine_bin, exe_arg, arguments)?;
    let argv = argv_refs(&argv_c);
    Ok(execv(argv[0], &argv)?)
}

/// Replace this process with wine, with a pty as its stdin, stdout and
/// stderr, after starting a child, this executable in its `--log-pump` mode,
/// that copies what wine writes there into the log. For a launch from Finder
/// or the Dock. Returns only with an error.
///
/// Wine runs in this process and the child does the copying, not the other
/// way round, because macOS gives Game Mode to the app Launch Services
/// started, and knows that app by this process. A child is a process of no
/// app, and a game running in one never gets Game Mode.
///
/// The pty is needed because wine only writes some of its output when stderr
/// is a terminal. It does not have to be wine's controlling terminal, so
/// there is no `setsid` here, which fails in a process group leader, as an
/// app started by Launch Services usually is.
///
/// The child also makes Force Quit work. Launch Services knows the app by
/// this pid until winemac checks in after the exec; from then on its record
/// of the app has no pid. Force Quit (Activity Monitor, Command-Option-Escape,
/// the Dock) kills the pid in that record, so nothing, and sends SIGTERM to
/// every other process started under the app, but not to this one, which it
/// still counts as the app. wineserver exits on SIGTERM without killing its
/// clients, so the game would be left running without a server, hung. The
/// child is one of the processes that get the SIGTERM, and it answers it by
/// killing wine with SIGKILL, see [`run_pump`].
///
/// The child is a new process image started with `posix_spawn`, not a fork
/// of this process. By now this process can have other threads (see `launch`
/// in main.rs), and a fork taken while one of them is inside a one-time
/// libSystem initializer can abort in the child, in fork's own child
/// handlers, which would leave the game without its log and without Force
/// Quit. A spawned process runs none of those handlers.
pub fn exec_via_pty(
    wine_bin: &Path,
    game_dir: &Path,
    exe_arg: &str,
    arguments: &[String],
    log_fd: RawFd,
) -> io::Result<Infallible> {
    env::set_current_dir(game_dir)?;
    let argv_c = build_argv(wine_bin, exe_arg, arguments)?;
    let argv = argv_refs(&argv_c);
    let OpenptyResult { master, slave } = openpty(None, None)?;
    let exe = env::current_exe().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("cannot find the launcher's own executable to start the log pump: {e}"),
        )
    })?;
    // This process is wine from the exec on, and the pid stays the same.
    pump_argv(&exe, getpid())
        .and_then(|argv| spawn_pump(&argv, &[], master.as_raw_fd(), log_fd))
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("cannot start the log pump {}: {e}", exe.display()),
            )
        })?;

    // Wine gets the slave as fds 0, 1 and 2 and nothing else of the pty or
    // the log: the master and the slave's own fd from openpty are not
    // close-on-exec. The child is not waited for: it ends shortly after
    // wine, its parent from here on, has exited, and launchd reaps it.
    drop(master);
    dup2_stdin(&slave)?;
    dup2_stdout(&slave)?;
    dup2_stderr(&slave)?;
    drop(slave);
    // SAFETY: this process does not use the descriptor again; the child has
    // its own copy.
    unsafe { libc::close(log_fd) };
    Ok(execv(argv[0], &argv)?)
}

/// The command line of `exe`, this executable, in its `--log-pump` mode, for
/// the wine process `wine`.
fn pump_argv(exe: &Path, wine: Pid) -> io::Result<Vec<CString>> {
    Ok(vec![
        CString::new(exe.as_os_str().as_bytes())
            .map_err(|_| io::Error::other("the path contains NUL"))?,
        CString::from(c"--log-pump"),
        CString::new(wine.to_string()).map_err(|_| io::Error::other("pid contains NUL"))?,
    ])
}

/// Start `argv` with `master` as its stdin, `log_fd` as its stdout and
/// stderr, and `envp` as its whole environment, and return its pid. It gets
/// no other descriptor of this process, the slave included, and an empty
/// signal mask. Both descriptors have to be above 2.
///
/// The launcher passes an empty environment. Its own is set up for wine by
/// now, and holds what the config's `[env]` gives wine, which could reach the
/// dynamic loader and the frameworks this executable loads (`DYLD_*`,
/// `OBJC_*`, `Malloc*`). The pump reads nothing from its environment.
fn spawn_pump(argv: &[CString], envp: &[CString], master: RawFd, log_fd: RawFd) -> io::Result<Pid> {
    debug_assert!(
        master > 2 && log_fd > 2,
        "the pump's descriptors {master} and {log_fd} would be overwritten by its own dup2s"
    );
    let mut actions = PosixSpawnFileActions::init()?;
    actions.add_dup2(master, libc::STDIN_FILENO)?;
    actions.add_dup2(log_fd, libc::STDOUT_FILENO)?;
    actions.add_dup2(log_fd, libc::STDERR_FILENO)?;
    let mut attr = PosixSpawnAttr::init()?;
    // The mask of the thread that spawns would be inherited otherwise, and a
    // blocked SIGTERM would never reach the pump's handler.
    attr.set_sigmask(&SigSet::empty())?;
    attr.set_flags(
        PosixSpawnFlags::POSIX_SPAWN_SETSIGMASK
            | PosixSpawnFlags::from_bits_retain(libc::POSIX_SPAWN_CLOEXEC_DEFAULT),
    )?;
    Ok(posix_spawn(
        argv[0].as_c_str(),
        &actions,
        &attr,
        argv,
        envp,
    )?)
}

/// The `--log-pump` mode, in which [`exec_via_pty`] starts this executable:
/// copy wine's output from stdin, the pty's master, to stdout, the log,
/// until `wine`, this process's parent, has exited, and kill wine with
/// SIGKILL on SIGTERM. Touches nothing of `AppKit` or Launch Services, so the
/// process never shows up as an app.
pub fn run_pump(wine: Pid) {
    let (master, log) = (libc::STDIN_FILENO, libc::STDOUT_FILENO);
    if !kill_wine_on_sigterm(wine, log) {
        write_all(
            log,
            b"gamelauncher: the log pump has no SIGTERM handler, Force Quit will not stop the game\n",
        );
    }
    pump(master, log, wine);
    // Closed before the wait: a pump that ended early leaves wine running,
    // and a master nobody reads would block wine's writes once the pty's
    // buffer is full. Closed, they fail with `EIO` instead. No SIGHUP comes
    // of it, since the pty is nobody's controlling terminal. The log stays
    // open for the handler.
    // SAFETY: nothing in this process uses stdin again.
    unsafe { libc::close(master) };
    wait_for_wine(wine);
}

fn build_argv(wine_bin: &Path, exe_arg: &str, arguments: &[String]) -> io::Result<Vec<CString>> {
    let mut argv = Vec::with_capacity(arguments.len() + 2);
    argv.push(
        CString::new(wine_bin.as_os_str().as_bytes())
            .map_err(|_| io::Error::other("wine path contains NUL"))?,
    );
    argv.push(CString::new(exe_arg).map_err(|_| io::Error::other("executable name contains NUL"))?);
    for (index, argument) in arguments.iter().enumerate() {
        argv.push(CString::new(argument.as_str()).map_err(|_| {
            io::Error::other(format!("executable argument {} contains NUL", index + 1))
        })?);
    }
    Ok(argv)
}

fn argv_refs(argv: &[CString]) -> Vec<&CStr> {
    argv.iter().map(CString::as_c_str).collect()
}

/// Make a SIGTERM to this process kill `wine` with SIGKILL, and say so in the
/// log. `false` when the handler could not be installed.
///
/// For the pump, whose parent is wine; [`exec_via_pty`] says why. The
/// handler only kills while `wine` is still this process's parent, so a pid
/// that wine left behind and the system gave to another process is never
/// hit. SIGKILL rather than SIGTERM, since wine is the game and the user asked
/// for it to stop. The pump goes on after the handler returns: it sees that
/// wine has exited, copies what is left in the pty into the log and ends as
/// it does after any session. The handler makes only async-signal-safe
/// calls.
fn kill_wine_on_sigterm(wine: Pid, log_fd: RawFd) -> bool {
    SIGTERM_KILLS.store(wine.as_raw(), Ordering::Relaxed);
    SIGTERM_LOG.store(log_fd, Ordering::Relaxed);
    // SAFETY: an all-zero `sigaction` is a valid value: no flags and an empty
    // mask. The handler is set below.
    let mut action: libc::sigaction = unsafe { mem::zeroed() };
    action.sa_sigaction = on_sigterm as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: empties the mask in `action` and installs `action`, whose
    // handler makes only async-signal-safe calls.
    unsafe {
        libc::sigemptyset(&raw mut action.sa_mask);
        libc::sigaction(libc::SIGTERM, &raw const action, std::ptr::null_mut()) == 0
    }
}

/// Return once `wine`, this process's parent, has exited. For the pump,
/// which can end while wine still runs, when the pty reports its end or an
/// error: the process stays until wine is gone, so that Force Quit's SIGTERM
/// still finds the handler [`kill_wine_on_sigterm`] installed.
fn wait_for_wine(wine: Pid) {
    while getppid() == wine {
        // SAFETY: a poll on no descriptors, which only sleeps, and which a
        // signal cuts short.
        unsafe { libc::poll(std::ptr::null_mut(), 0, WINE_CHECK_MS) };
    }
}

/// The SIGTERM handler [`kill_wine_on_sigterm`] installs.
extern "C" fn on_sigterm(_: libc::c_int) {
    let wine = SIGTERM_KILLS.load(Ordering::Relaxed);
    // The pump reads errno after an interrupted call, which this signal can
    // be the reason for.
    let errno = Errno::last_raw();
    // SAFETY: getppid, write and kill are async-signal-safe; the write reads
    // only the bytes of the static message.
    unsafe {
        if wine > 1 && libc::getppid() == wine {
            libc::write(
                SIGTERM_LOG.load(Ordering::Relaxed),
                SIGTERM_MESSAGE.as_ptr().cast(),
                SIGTERM_MESSAGE.len(),
            );
            libc::kill(wine, libc::SIGKILL);
        }
    }
    Errno::set_raw(errno);
}

/// Pump pty master → log fd. ptys translate output `\n` to `\r\n`; strip the
/// CR before LF so the log stays free of stray ^M bytes, also when a read
/// ends between the two.
///
/// Runs until `wine`, the pump's parent, has exited, then copies what is
/// still waiting in the pty, up to [`DRAIN_CHUNKS`] reads, and stops. The
/// pty is not wine's controlling terminal, so wine's exit does not take the
/// slave away from the processes that still hold it: wineserver keeps it as
/// its stderr for a few seconds, and so do processes the game leaves
/// running. Waiting for all of them would keep this process writing into a
/// log that the next launch has already truncated. Parenthood is checked
/// every [`WINE_CHECK_MS`] while the pty is quiet and after every read.
///
/// It also stops when the pty reports end of file or an error, which on
/// macOS is `EIO` once everything has closed the slave side; [`run_pump`]
/// then closes the master and waits for wine in [`wait_for_wine`] before it
/// returns. `EINTR` is retried, so a signal cannot end the copy early, the
/// SIGTERM that makes the pump kill wine included. Once the log takes no
/// more, the output is still read and dropped: wine keeps a working terminal
/// for the whole session, where a closed master would fail every write it
/// makes with `EIO`.
fn pump(master_fd: RawFd, log_fd: RawFd, wine: Pid) {
    let mut buf = [0u8; CHUNK];
    // A CR held back from the previous read can come out ahead of the bytes
    // of this one.
    let mut out = [0u8; CHUNK + 1];
    let mut strip = StripCrBeforeLf::default();
    let mut logging = true;
    let mut drain_left = DRAIN_CHUNKS;
    loop {
        let wine_gone = getppid() != wine;
        if wine_gone {
            if drain_left == 0 {
                break;
            }
            drain_left -= 1;
        }
        let mut poll_fd = libc::pollfd {
            fd: master_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout = if wine_gone { 0 } else { WINE_CHECK_MS };
        // SAFETY: polls the one `pollfd` on this stack.
        match unsafe { libc::poll(&raw mut poll_fd, 1, timeout) } {
            0 if wine_gone => break,
            0 => continue,
            ready if ready < 0 && interrupted() => continue,
            ready if ready < 0 => break,
            // Output, or a hangup, which the read reports. Also `POLLNVAL`:
            // poll(2) on macOS warns that it may not support devices, and a
            // descriptor that is really bad fails the read with `EBADF`. On a
            // system whose poll cannot watch a pty, the pump then learns that
            // wine has exited only after its next read.
            _ => {}
        }
        // SAFETY: reads at most `buf.len()` bytes into `buf`.
        let n = unsafe { libc::read(master_fd, buf.as_mut_ptr().cast(), buf.len()) };
        let n = match usize::try_from(n) {
            Ok(0) => break,
            Ok(v) => v,
            Err(_) if interrupted() => continue,
            Err(_) => break,
        };
        if logging {
            logging = write_all(log_fd, strip.feed(&buf[..n], &mut out));
        }
    }
    if logging {
        write_all(log_fd, strip.finish());
    }
}

/// Write all of `bytes` to the log. `false` when it takes no more.
fn write_all(log_fd: RawFd, bytes: &[u8]) -> bool {
    let mut remaining = bytes;
    while !remaining.is_empty() {
        // SAFETY: writes from the `remaining.len()` bytes `remaining` points
        // at, and reads nothing else.
        let w = unsafe { libc::write(log_fd, remaining.as_ptr().cast(), remaining.len()) };
        let w = match usize::try_from(w) {
            Ok(0) => return false,
            Ok(v) => v,
            Err(_) if interrupted() => continue,
            Err(_) => return false,
        };
        remaining = &remaining[w..];
    }
    true
}

/// Whether the last failed call was cut short by a signal.
fn interrupted() -> bool {
    io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
}

/// Drops every CR that comes right before an LF, across the chunks it is fed.
/// A CR at the end of a chunk is held back until the next byte shows whether
/// an LF follows.
#[derive(Default)]
struct StripCrBeforeLf {
    pending_cr: bool,
}

impl StripCrBeforeLf {
    /// Write what `input` leaves for the log into `out`, which needs room for
    /// one byte more than `input` has, and return that part of `out`.
    fn feed<'o>(&mut self, input: &[u8], out: &'o mut [u8]) -> &'o [u8] {
        let mut len = 0;
        for &b in input {
            if mem::take(&mut self.pending_cr) && b != b'\n' {
                out[len] = b'\r';
                len += 1;
            }
            if b == b'\r' {
                self.pending_cr = true;
            } else {
                out[len] = b;
                len += 1;
            }
        }
        &out[..len]
    }

    /// A CR held back at the very end of the output, which no LF followed.
    const fn finish(self) -> &'static [u8] {
        if self.pending_cr { b"\r" } else { b"" }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        process::Command,
        sync::mpsc,
        thread,
        time::Duration,
    };

    use super::*;

    /// What the log gets from output that arrives in `chunks`, each fed with
    /// exactly the room `feed` asks for.
    fn strip(chunks: &[&[u8]]) -> Vec<u8> {
        let mut strip = StripCrBeforeLf::default();
        let mut out = Vec::new();
        for chunk in chunks {
            let mut room = vec![0; chunk.len() + 1];
            out.extend_from_slice(strip.feed(chunk, &mut room));
        }
        out.extend_from_slice(strip.finish());
        out
    }

    #[test]
    fn strips_cr_before_lf_only() {
        assert_eq!(strip(&[b"hello\r\nworld\r\n"]), b"hello\nworld\n");
        assert_eq!(strip(&[b"a\r\r\nb"]), b"a\r\nb");
    }

    #[test]
    fn preserves_lone_cr() {
        assert_eq!(strip(&[b"a\rb"]), b"a\rb");
        assert_eq!(strip(&[b"a\r", b"b"]), b"a\rb");
    }

    #[test]
    fn preserves_trailing_cr() {
        assert_eq!(strip(&[b"x\r"]), b"x\r");
        // Held back until it is clear no LF follows.
        let mut held = StripCrBeforeLf::default();
        assert_eq!(held.feed(b"x\r", &mut [0; 3]), b"x");
        assert_eq!(held.finish(), b"\r");
    }

    #[test]
    fn strips_a_crlf_split_between_reads() {
        assert_eq!(strip(&[b"one\r", b"\ntwo\r", b"\n"]), b"one\ntwo\n");
        assert_eq!(strip(&[b"\r", b"", b"\n"]), b"\n");
    }

    #[test]
    fn a_held_back_cr_takes_the_one_extra_byte() {
        let mut held = StripCrBeforeLf::default();
        assert_eq!(held.feed(b"\r", &mut [0; 2]), b"");
        assert_eq!(held.feed(b"ab", &mut [0; 3]), b"\rab");
    }

    #[test]
    fn pump_copies_wine_output_into_the_log_without_the_cr_before_lf() {
        let (master, mut wine) = io::pipe().unwrap();
        let (mut log, log_write) = io::pipe().unwrap();
        wine.write_all(b"one\r\ntwo\r").unwrap();
        drop(wine);

        // The test's parent stands in for wine and outlives the pump.
        pump(master.as_raw_fd(), log_write.as_raw_fd(), getppid());
        drop(log_write);

        let mut logged = Vec::new();
        log.read_to_end(&mut logged).unwrap();
        assert_eq!(logged, b"one\ntwo\r");
    }

    /// On a real pty, which turns each LF written to the slave into CR LF.
    /// The slave stays open while the pump reads, because macOS discards
    /// what nobody has read when the last slave descriptor closes; the pump
    /// ends on the drain after wine's exit instead, with this test's own pid
    /// standing in for a wine that is not its parent. Then, with the slave
    /// closed, the pump has to see the pty's end and return.
    #[test]
    fn pump_copies_a_pty_into_the_log_without_the_cr_the_pty_adds() {
        let OpenptyResult { master, slave } = openpty(None, None).unwrap();
        let (mut log, log_write) = io::pipe().unwrap();
        let mut slave = std::fs::File::from(slave);
        slave.write_all(b"one\ntwo\n").unwrap();

        pump(master.as_raw_fd(), log_write.as_raw_fd(), getpid());
        drop(slave);
        let (done, ended) = mpsc::channel();
        thread::spawn(move || {
            // The test's parent stands in for wine and outlives the pump.
            pump(master.as_raw_fd(), log_write.as_raw_fd(), getppid());
            drop(log_write);
            done.send(()).unwrap();
        });
        assert!(
            ended.recv_timeout(Duration::from_secs(5)).is_ok(),
            "the pump still runs 5 s after the slave was closed"
        );

        let mut logged = Vec::new();
        log.read_to_end(&mut logged).unwrap();
        assert_eq!(logged, b"one\ntwo\n");
    }

    /// More output than a pipe holds, so the writer only finishes when the
    /// pump goes on reading after the log failed. Once the pump returns its
    /// end is closed, so a writer left behind fails instead of hanging.
    #[test]
    fn pump_keeps_reading_once_the_log_takes_no_more() {
        let (master, mut wine) = io::pipe().unwrap();
        let writer = thread::spawn(move || wine.write_all(&vec![b'x'; 16 * CHUNK]));
        // The read end of a pipe, which every write fails on.
        let (log, _) = io::pipe().unwrap();

        pump(master.as_raw_fd(), log.as_raw_fd(), getppid());
        drop(master);

        assert!(writer.join().unwrap().is_ok());
    }

    /// Wine exits while the slave is still held, as wineserver holds it for a
    /// while after a session; here the holder is this test, so no end of file
    /// ever comes. The pump has to copy what was written and then end.
    #[test]
    fn pump_ends_once_wine_has_exited_while_the_slave_is_still_held() {
        let (master, mut slave) = io::pipe().unwrap();
        let (mut log, log_write) = io::pipe().unwrap();
        slave.write_all(b"last words\r\n").unwrap();
        let mut wine = StandInWine::start(master, log_write);
        let mut logged = Vec::new();
        read_log_until(&mut log, &mut logged, b"last words\n");

        wine.child.kill().unwrap();
        wine.child.wait().unwrap();
        read_log_to_end(&mut log, &mut logged);
        assert!(
            contains(&logged, b"last words\n"),
            "log: {:?}",
            String::from_utf8_lossy(&logged)
        );
        drop(slave);
    }

    /// Whether `fd` has something to read, or its end of file, within `ms`.
    fn readable_within(fd: RawFd, ms: libc::c_int) -> bool {
        let mut poll_fd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: polls the one `pollfd` on this stack.
        unsafe { libc::poll(&raw mut poll_fd, 1, ms) == 1 }
    }

    /// Turns the helper tests below on, in the copies of this test binary
    /// that the tests start. The stand-in for wine passes it on to its pump,
    /// as the pump's only variable.
    const HELPER_ENV: &str = "GAMELAUNCHER_TEST_LOG_PUMP";

    /// The pid of the process the handler is installed for, in
    /// `sigterm_with_the_handler_for_another_process`.
    const OTHER_PID_ENV: &str = "GAMELAUNCHER_TEST_OTHER_PID";

    /// What `stand_in_wine` prints before the pid of the pump it started.
    const PUMP_PID_SAYS: &str = "stand-in wine started the pump: ";

    /// A descriptor `stand_in_wine` holds open without close-on-exec when it
    /// starts the pump, as `exec_via_pty` holds the slave, and which the pump
    /// must not get. High, so the pump cannot have opened one of its own
    /// under that number.
    const STRAY_FD: RawFd = 200;

    /// What `sigterm_with_the_handler_for_another_process` prints once the
    /// SIGTERM has been handled.
    const SURVIVED_SAYS: &str = "survived SIGTERM";

    /// The argv of this test binary running only the ignored helper test
    /// `name`, the way the tests start their helpers.
    fn helper_argv(name: &str) -> Vec<CString> {
        use std::os::unix::ffi::OsStringExt;
        let exe = env::current_exe().unwrap().into_os_string().into_vec();
        [
            exe.as_slice(),
            b"--exact",
            name.as_bytes(),
            b"--ignored",
            b"--nocapture",
        ]
        .into_iter()
        .map(|arg| CString::new(arg).unwrap())
        .collect()
    }

    /// `helper_argv` as a command, with the helpers turned on.
    fn helper(name: &str) -> Command {
        let argv = helper_argv(name);
        let mut command = Command::new(std::ffi::OsStr::from_bytes(argv[0].as_bytes()));
        command
            .args(
                argv[1..]
                    .iter()
                    .map(|arg| std::ffi::OsStr::from_bytes(arg.as_bytes())),
            )
            .env(HELPER_ENV, "1");
        command
    }

    /// A copy of this test binary standing in for wine: it starts a pump with
    /// `spawn_pump`, as `exec_via_pty` does, so the pump is its child. Each
    /// start checks that the pump got none of the stand-in's other
    /// descriptors. It is
    /// a new process image rather than a fork of this one. The harness runs
    /// other tests on other threads, and a fork taken while one of them is
    /// inside a one-time libSystem initializer can abort in the child, in
    /// fork's own child handlers. Killed and reaped when this is dropped,
    /// also when a failing assert unwinds past it.
    struct StandInWine {
        child: std::process::Child,
        pump: libc::pid_t,
    }

    impl StandInWine {
        /// Start the stand-in with `master` as the pump's pty and `log` as
        /// its log, and return once it has started the pump.
        fn start(master: io::PipeReader, log: io::PipeWriter) -> Self {
            use std::io::BufRead;
            // The command is dropped at the end of the statement, so this
            // process keeps no copy of `log`.
            let mut child = helper("launch::tests::stand_in_wine")
                .stdin(master)
                .stdout(std::process::Stdio::piped())
                .stderr(log)
                .spawn()
                .unwrap();
            let stdout = child.stdout.take().unwrap();
            let mut wine = Self { child, pump: 0 };
            let said = io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
                .find_map(|line| {
                    line.split_once(PUMP_PID_SAYS)
                        .map(|(_, pid)| pid.to_owned())
                });
            wine.pump = said
                .expect("the stand-in for wine exited without starting the pump")
                .parse()
                .unwrap();
            assert!(
                !has_fd(wine.pump, STRAY_FD),
                "the pump got a descriptor of the stand-in it was not given"
            );
            wine
        }

        /// Send the pump SIGTERM and check that the stand-in dies of
        /// SIGKILL.
        fn sigterm_the_pump(&mut self) {
            use std::os::unix::process::ExitStatusExt;
            // SAFETY: signals the pump, which is alive: it stays until its
            // parent is gone, and its parent has not been reaped yet.
            assert_eq!(unsafe { libc::kill(self.pump, libc::SIGTERM) }, 0);
            let status = self.child.wait().unwrap();
            assert_eq!(
                status.signal(),
                Some(libc::SIGKILL),
                "wine did not die of SIGKILL: {status}"
            );
        }
    }

    impl Drop for StandInWine {
        fn drop(&mut self) {
            // Either can only fail when the stand-in has already been reaped.
            self.child.kill().ok();
            self.child.wait().ok();
        }
    }

    /// Whether `bytes` holds `part`.
    fn contains(bytes: &[u8], part: &[u8]) -> bool {
        bytes.windows(part.len()).any(|window| window == part)
    }

    /// Read the log into `logged` until it holds `part`. Fails on the end of
    /// the log, or after 5 s without anything to read.
    fn read_log_until(log: &mut io::PipeReader, logged: &mut Vec<u8>, part: &[u8]) {
        let mut chunk = [0u8; 256];
        while !contains(logged, part) {
            assert!(
                readable_within(log.as_raw_fd(), 5000),
                "nothing in the log for 5 s, log: {:?}",
                String::from_utf8_lossy(logged)
            );
            match log.read(&mut chunk).unwrap() {
                0 => panic!("the log ended, log: {:?}", String::from_utf8_lossy(logged)),
                n => logged.extend_from_slice(&chunk[..n]),
            }
        }
    }

    /// Read the rest of the log into `logged`. The pump holds the last write
    /// end, so the end of the log is the pump's exit; fails when it has not
    /// come after 5 s without anything to read.
    fn read_log_to_end(log: &mut io::PipeReader, logged: &mut Vec<u8>) {
        let mut chunk = [0u8; 256];
        loop {
            assert!(
                readable_within(log.as_raw_fd(), 5000),
                "the pump still runs 5 s after wine exited"
            );
            match log.read(&mut chunk).unwrap() {
                0 => return,
                n => logged.extend_from_slice(&chunk[..n]),
            }
        }
    }

    /// Whether the process `pid` has the descriptor `fd` open.
    fn has_fd(pid: libc::pid_t, fd: RawFd) -> bool {
        let mut fds = [libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0,
        }; 256];
        let size = libc::c_int::try_from(mem::size_of_val(&fds)).unwrap();
        // SAFETY: fills at most `size` bytes of `fds`, which has that many.
        let bytes = unsafe {
            libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, fds.as_mut_ptr().cast(), size)
        };
        assert!(bytes > 0, "cannot list the descriptors of {pid}");
        let count = usize::try_from(bytes).unwrap() / mem::size_of::<libc::proc_fdinfo>();
        fds[..count].iter().any(|info| info.proc_fd == fd)
    }

    /// Force Quit's SIGTERM reaches the pump and not wine. The stand-in for
    /// wine has to die of SIGKILL, and the pump has to go on to copy the
    /// output into the log, add the handler's message and end. The output's
    /// write end stays open, as wineserver keeps the slave, so only wine's
    /// death can end the pump. The handler is in place once the output is in
    /// the log, since the pump installs it before it starts copying.
    #[test]
    fn sigterm_to_the_pump_kills_wine_with_sigkill() {
        let (master, mut output) = io::pipe().unwrap();
        let (mut log, log_write) = io::pipe().unwrap();
        output.write_all(b"before\r\n").unwrap();
        let mut wine = StandInWine::start(master, log_write);
        let mut logged = Vec::new();
        read_log_until(&mut log, &mut logged, b"before\n");

        wine.sigterm_the_pump();
        read_log_to_end(&mut log, &mut logged);
        let message = logged
            .windows(SIGTERM_MESSAGE.len())
            .position(|window| window == SIGTERM_MESSAGE);
        let before = logged.windows(7).position(|window| window == b"before\n");
        assert!(
            matches!((before, message), (Some(before), Some(message)) if before < message),
            "log: {:?}",
            String::from_utf8_lossy(&logged)
        );
        drop(output);
    }

    /// A pump whose pty ended while wine still runs stays and still answers
    /// SIGTERM. Nothing holds the output's write end, so the pump reads the
    /// end of file right after the output. The SIGTERM comes once the pump
    /// has closed the master, which it does after the copying and before the
    /// wait for wine.
    #[test]
    fn a_pump_that_ended_early_still_kills_wine_on_sigterm() {
        let (master, mut output) = io::pipe().unwrap();
        let (mut log, log_write) = io::pipe().unwrap();
        output.write_all(b"early\r\n").unwrap();
        drop(output);
        let mut wine = StandInWine::start(master, log_write);
        let mut logged = Vec::new();
        read_log_until(&mut log, &mut logged, b"early\n");
        let waited = std::time::Instant::now();
        while has_fd(wine.pump, libc::STDIN_FILENO) {
            assert!(
                waited.elapsed() < Duration::from_secs(5),
                "the pump still has the master open 5 s after its end"
            );
            thread::sleep(Duration::from_millis(10));
        }

        wine.sigterm_the_pump();
        read_log_to_end(&mut log, &mut logged);
        let tail = [b"early\n".as_slice(), SIGTERM_MESSAGE].concat();
        assert!(
            contains(&logged, &tail),
            "log: {:?}",
            String::from_utf8_lossy(&logged)
        );
    }

    /// The handler kills only the pump's parent, so a pid the system handed
    /// to another process after wine exited is safe. Here the pid belongs to
    /// a sibling of the process that installs the handler, and the SIGTERM
    /// must neither kill the sibling nor end the process that got it.
    #[test]
    fn sigterm_to_the_pump_spares_a_process_that_is_not_its_parent() {
        let mut other = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let output = helper("launch::tests::sigterm_with_the_handler_for_another_process")
            .env(OTHER_PID_ENV, other.id().to_string())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains(SURVIVED_SAYS),
            "the process did not live through SIGTERM: {}, {stdout}",
            output.status
        );
        // Time for a stray SIGKILL to arrive, should the handler have sent
        // one.
        thread::sleep(Duration::from_millis(200));
        assert!(
            other.try_wait().unwrap().is_none(),
            "the sibling was killed"
        );
        other.kill().unwrap();
        other.wait().unwrap();
    }

    /// The stand-in for wine, which `StandInWine::start` runs in a copy of
    /// this binary with the pty on stdin and the log on stderr; without
    /// `HELPER_ENV` it does nothing. It starts `stand_in_pump` with
    /// `spawn_pump`, from a thread that has SIGTERM blocked, so the pump only
    /// gets the signal because `spawn_pump` clears the mask. Then it waits to
    /// be killed; when that never happens it exits after 30 s, which fails
    /// the test instead of hanging it.
    #[test]
    #[ignore = "the stand-in for wine of the log pump tests"]
    fn stand_in_wine() {
        use std::os::fd::AsFd;
        if env::var_os(HELPER_ENV).is_none() {
            return;
        }
        // Copies above 2, as `spawn_pump` needs them.
        let master = io::stdin().as_fd().try_clone_to_owned().unwrap();
        let log = io::stderr().as_fd().try_clone_to_owned().unwrap();
        // SAFETY: duplicates stdout under a number nothing else uses; dup2
        // leaves the copy without close-on-exec.
        assert_eq!(
            unsafe { libc::dup2(libc::STDOUT_FILENO, STRAY_FD) },
            STRAY_FD
        );
        let mut term = SigSet::empty();
        term.add(nix::sys::signal::Signal::SIGTERM);
        term.thread_block().unwrap();
        // Only the variable that turns the helper on, as the launcher gives
        // the pump an empty environment.
        let envp = [CString::new(format!("{HELPER_ENV}=1")).unwrap()];
        let pump = spawn_pump(
            &helper_argv("launch::tests::stand_in_pump"),
            &envp,
            master.as_raw_fd(),
            log.as_raw_fd(),
        )
        .unwrap();
        println!("\n{PUMP_PID_SAYS}{pump}");
        drop((master, log));
        thread::sleep(Duration::from_secs(30));
    }

    /// The pump `stand_in_wine` starts, the `--log-pump` mode for the
    /// stand-in, its parent; without `HELPER_ENV` it does nothing. The log
    /// also gets what the harness prints, around the pump's output.
    #[test]
    #[ignore = "the log pump of the log pump tests"]
    fn stand_in_pump() {
        if env::var_os(HELPER_ENV).is_none() {
            return;
        }
        run_pump(getppid());
    }

    /// Installs the SIGTERM handler for the process named in `OTHER_PID_ENV`,
    /// which is not this one's parent, sends itself SIGTERM and says it
    /// survived; without `HELPER_ENV` it does nothing.
    #[test]
    #[ignore = "the process sigterm_to_the_pump_spares_a_process_that_is_not_its_parent signals"]
    fn sigterm_with_the_handler_for_another_process() {
        if env::var_os(HELPER_ENV).is_none() {
            return;
        }
        let other = env::var(OTHER_PID_ENV).unwrap().parse().unwrap();
        assert!(kill_wine_on_sigterm(Pid::from_raw(other), -1));
        // SAFETY: the handler is installed, and makes only
        // async-signal-safe calls.
        assert_eq!(unsafe { libc::raise(libc::SIGTERM) }, 0);
        println!("\n{SURVIVED_SAYS}");
    }

    #[test]
    fn builds_wine_executable_and_literal_arguments_in_order() {
        let arguments = vec![
            String::from("-console"),
            String::from("value with spaces"),
            String::from("name=value;still-one-argument"),
            String::new(),
        ];
        let argv = build_argv(Path::new("/bundle/wine"), "Game.exe", &arguments).unwrap();
        let values: Vec<&[u8]> = argv.iter().map(CString::as_bytes).collect();

        assert_eq!(
            values,
            [
                b"/bundle/wine".as_slice(),
                b"Game.exe".as_slice(),
                b"-console".as_slice(),
                b"value with spaces".as_slice(),
                b"name=value;still-one-argument".as_slice(),
                b"".as_slice(),
            ]
        );
    }

    #[test]
    fn rejects_nul_in_an_executable_argument() {
        let error = build_argv(
            Path::new("/bundle/wine"),
            "Game.exe",
            &[String::from("valid"), String::from("bad\0value")],
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "executable argument 2 contains NUL");
    }
}
