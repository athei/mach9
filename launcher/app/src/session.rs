//! The wineserver session a launch runs in: reuse the one that is up, or boot a
//! fresh one.
//!
//! A session keeps the retina mode it booted with. winemac reads `RetinaMode`
//! once per process, and the desktop process (`explorer.exe /desktop`), which
//! owns the Win32 display state every other process reads, starts with the
//! session; it also records the display mode retina is judged against in a
//! volatile key at that moment. A `reg add` into a running session changes
//! the registry and nothing else: a game launched with retina on into a
//! session booted with it off gets the desktop in points, a short mode list,
//! and every frame upscaled. So a launch that wants the other mode stops the
//! running session, and everything in it, and boots a new one.
//!
//! The system DPI is kept the same way: every process reads it once at start,
//! and the desktop process gives each monitor the DPI it started with, which
//! is what a per-monitor-aware program reads. A launch that wants another DPI
//! restarts the session too.
//!
//! The live registry cannot say which mode a session booted with, so the
//! launcher writes it down: before every boot it stamps `prefix-session` in
//! the support directory with the mode, the DPI and the time. A running
//! wineserver that started at or after that time booted from the `user.reg`
//! the launcher wrote with the stamp. One that started before it, or a
//! missing stamp, means a session nobody vouches for, and it is restarted
//! like a mismatched one.

use std::{
    ffi::CStr,
    fs, io,
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gamelauncher_core::{bundle, fsutil::write_atomic};
use log::{info, warn};

use crate::on_off;

/// How long a stopped session may take to release its lock after
/// `wineserver -k` returns. The command itself already waits about ten
/// seconds for an orderly exit before it sends `SIGKILL`.
const EXIT_WAIT: Duration = Duration::from_secs(15);

/// The session a launch goes into.
pub enum Session {
    /// No wineserver is up; the lock is held so none can come up until the
    /// registry is written. Dropping it lets the game's session boot.
    Fresh(#[expect(dead_code, reason = "only held, for its drop")] ServerLock),
    /// A wineserver booted with the wanted retina mode and DPI is up.
    Running,
}

/// Reuse the running session when it booted with `retina` and `dpi`, otherwise
/// stop it with `wineserver`; either way, when a fresh session is next, stamp
/// it.
pub fn prepare(
    prefix: &Path,
    support: &Path,
    wineserver: &Path,
    retina: bool,
    dpi: u32,
) -> io::Result<Session> {
    let base = server_base_dir();
    let stamp_path = bundle::session_stamp_path(support);
    let pid = server_pid(&base, prefix)?;
    let server = pid.map_or(Server::Down, |pid| {
        process_start(pid).map_or(Server::Unknown, Server::StartedAt)
    });
    let stamp = fs::read_to_string(&stamp_path)
        .ok()
        .and_then(|text| Stamp::parse(&text));

    match decide(server, stamp.as_ref(), retina, dpi) {
        Decision::Reuse => {
            info!(
                "reusing the running wine session, booted with retina mode {} at {dpi} DPI",
                on_off(retina)
            );
            return Ok(Session::Running);
        }
        Decision::Restart(reason) => {
            let pid = pid.unwrap_or_default();
            warn!(
                "stopping the running wine session (wineserver PID {pid}): {reason}; this launch \
                 wants retina mode {} at {dpi} DPI",
                on_off(retina)
            );
            stop_server(wineserver)?;
        }
        Decision::Boot => {}
    }

    let lock = wait_for_lock(&base, prefix)?;
    let stamp = Stamp {
        retina,
        dpi,
        booted_after: now_secs(),
    };
    // Written while the lock is held: a session that boots from here on
    // started after this time and loads the user.reg written under this lock.
    write_atomic(&stamp_path, &stamp.format())?;
    info!(
        "booting a fresh wine session with retina mode {} at {dpi} DPI",
        on_off(retina)
    );
    Ok(Session::Fresh(lock))
}

/// What to do with the prefix's wineserver.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// None is up.
    Boot,
    /// One is up and booted with the wanted mode and DPI.
    Reuse,
    /// One is up that booted with the other mode or another DPI, or with
    /// unknown ones.
    Restart(&'static str),
}

/// The prefix's wineserver as the lock and the process table show it.
#[derive(Clone, Copy)]
enum Server {
    Down,
    /// Up, started at this many seconds since the Unix epoch.
    StartedAt(u64),
    /// Up, but its start time cannot be read or the lock holder is no
    /// wineserver.
    Unknown,
}

const fn decide(server: Server, stamp: Option<&Stamp>, retina: bool, dpi: u32) -> Decision {
    let started = match server {
        Server::Down => return Decision::Boot,
        Server::Unknown => return Decision::Restart("its start time cannot be read"),
        Server::StartedAt(started) => started,
    };
    let Some(stamp) = stamp else {
        return Decision::Restart("no session stamp records how it booted");
    };
    if started < stamp.booted_after {
        return Decision::Restart("it started before the last stamped boot, outside the launcher");
    }
    if stamp.retina != retina {
        return Decision::Restart("it booted with the other retina mode");
    }
    if stamp.dpi != dpi {
        return Decision::Restart("it booted with another DPI");
    }
    Decision::Reuse
}

/// The retina mode and DPI a session booted with, and the time before which
/// it cannot have started. A stamp written before the DPI was recorded parses
/// as none, so the session it describes is restarted once.
#[derive(Debug, PartialEq, Eq)]
struct Stamp {
    retina: bool,
    dpi: u32,
    /// Seconds since the Unix epoch, rounded down.
    booted_after: u64,
}

impl Stamp {
    fn parse(text: &str) -> Option<Self> {
        let mut retina = None;
        let mut dpi = None;
        let mut booted_after = None;
        for line in text.lines() {
            match line.split_once('=') {
                Some(("retina", "Y")) => retina = Some(true),
                Some(("retina", "N")) => retina = Some(false),
                Some(("dpi", value)) => dpi = value.parse().ok(),
                Some(("booted_after", secs)) => booted_after = secs.parse().ok(),
                _ => {}
            }
        }
        Some(Self {
            retina: retina?,
            dpi: dpi?,
            booted_after: booted_after?,
        })
    }

    fn format(&self) -> String {
        format!(
            "retina={}\ndpi={}\nbooted_after={}\n",
            if self.retina { "Y" } else { "N" },
            self.dpi,
            self.booted_after
        )
    }
}

/// Seconds since the Unix epoch, rounded down.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `wineserver -k` for the prefix in `WINEPREFIX`: an orderly shutdown that
/// ends every process and saves the registry, `SIGKILL` if that stalls. It
/// returns once the server has exited.
pub fn stop_server(wineserver: &Path) -> io::Result<()> {
    let status = Command::new(wineserver).arg("-k").status()?;
    if !status.success() {
        // It exits non-zero when the server was already gone; the lock wait
        // below is what decides.
        info!("{} -k exited with {status}", wineserver.display());
    }
    Ok(())
}

fn wait_for_lock(base: &Path, prefix: &Path) -> io::Result<ServerLock> {
    let deadline = Instant::now() + EXIT_WAIT;
    loop {
        if let Some(lock) = ServerLock::acquire(base, prefix)? {
            return Ok(lock);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!(
                "the running wine session did not exit within {} seconds",
                EXIT_WAIT.as_secs()
            )));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// The wineserver lock for a prefix, held for as long as the value lives.
///
/// A wineserver takes a write lock on byte 0 of `<server dir>/lock` when it
/// starts and keeps it until it exits (`server/request.c`, `acquire_lock`),
/// so holding that lock means no server is up and none can come up: one that
/// tries exits with status 2 and its client retries with a short back-off.
/// The directory is `/tmp/.wine-<uid>/server-<dev>-<inode>` from the prefix's
/// device and inode, created here the way wine creates it when missing.
pub struct ServerLock {
    /// Open for the lock's sake only: the lock ends when it is dropped.
    _file: fs::File,
}

impl ServerLock {
    /// `Ok(None)` when a wineserver holds the lock.
    fn acquire(base: &Path, prefix: &Path) -> io::Result<Option<Self>> {
        let file = open_lock_file(base, prefix)?;
        Ok(try_write_lock(&file)?.then_some(Self { _file: file }))
    }
}

fn open_lock_file(base: &Path, prefix: &Path) -> io::Result<fs::File> {
    let dir = server_dir(base, prefix)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join("lock"))
}

fn server_base_dir() -> PathBuf {
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/.wine-{uid}"))
}

fn server_dir(base: &Path, prefix: &Path) -> io::Result<PathBuf> {
    let meta = fs::metadata(prefix)?;
    Ok(base.join(format!("server-{:x}-{:x}", meta.dev(), meta.ino())))
}

/// The lock's byte range, byte 0, as wine takes it.
const fn lock_range(l_type: libc::c_short) -> libc::flock {
    libc::flock {
        l_start: 0,
        l_len: 1,
        l_pid: 0,
        l_type,
        // SEEK_SET: the range starts at the file's beginning.
        l_whence: 0,
    }
}

/// Try to take wine's lock without waiting. `Ok(true)` when it was free; the
/// lock goes away with `file`.
fn try_write_lock(file: &fs::File) -> io::Result<bool> {
    let mut fl = lock_range(libc::F_WRLCK);
    // SAFETY: fcntl with F_SETLK reads the flock struct and nothing else.
    let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &raw mut fl) };
    if rc == 0 {
        return Ok(true);
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::EAGAIN | libc::EACCES) => Ok(false),
        _ => Err(e),
    }
}

/// The PID of the wineserver holding the prefix's lock, `None` when none
/// does. This is how `wineserver -k` finds the server it stops.
fn server_pid(base: &Path, prefix: &Path) -> io::Result<Option<i32>> {
    let file = open_lock_file(base, prefix)?;
    let mut fl = lock_range(libc::F_WRLCK);
    // SAFETY: fcntl with F_GETLK reads and fills the flock struct and nothing
    // else.
    let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &raw mut fl) };
    if rc == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok((fl.l_type != libc::F_UNLCK && fl.l_pid > 0).then_some(fl.l_pid))
}

/// When the process started, in seconds since the Unix epoch, if it is a
/// wineserver. `None` when it cannot be read or is some other program, which
/// the caller treats as a session of unknown origin.
fn process_start(pid: i32) -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = i32::try_from(size_of::<libc::proc_bsdinfo>()).ok()?;
    // SAFETY: proc_pidinfo writes at most `size` bytes into the buffer, which
    // is a proc_bsdinfo of exactly that size.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // SAFETY: proc_pidinfo filled the whole struct, and zeroed plain C data
    // is a valid proc_bsdinfo anyway.
    let info = unsafe { info.assume_init() };
    // SAFETY: pbi_comm is a C char array inside the struct; reinterpreting it
    // as bytes of the same length reads nothing outside it.
    let comm: &[u8; libc::MAXCOMLEN] = unsafe { &*(&raw const info.pbi_comm).cast() };
    let name = CStr::from_bytes_until_nul(comm).map_or(&comm[..], CStr::to_bytes);
    (name == b"wineserver").then_some(info.pbi_start_tvsec)
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn stamp(retina: bool, booted_after: u64) -> Stamp {
        Stamp {
            retina,
            dpi: if retina { 192 } else { 96 },
            booted_after,
        }
    }

    #[test]
    fn boots_when_no_server_is_up() {
        assert_eq!(decide(Server::Down, None, true, 192), Decision::Boot);
        assert_eq!(
            decide(Server::Down, Some(&stamp(false, 10)), true, 192),
            Decision::Boot
        );
    }

    #[test]
    fn reuses_a_session_stamped_with_the_wanted_mode() {
        assert_eq!(
            decide(Server::StartedAt(10), Some(&stamp(true, 10)), true, 192),
            Decision::Reuse
        );
        assert_eq!(
            decide(Server::StartedAt(99), Some(&stamp(false, 10)), false, 96),
            Decision::Reuse
        );
    }

    #[test]
    fn restarts_a_session_booted_with_the_other_mode() {
        assert_eq!(
            decide(Server::StartedAt(12), Some(&stamp(false, 10)), true, 192),
            Decision::Restart("it booted with the other retina mode")
        );
    }

    #[test]
    fn restarts_a_session_booted_with_another_dpi() {
        assert_eq!(
            decide(Server::StartedAt(12), Some(&stamp(true, 10)), true, 144),
            Decision::Restart("it booted with another DPI")
        );
        let pinned = Stamp {
            dpi: 144,
            ..stamp(true, 10)
        };
        assert_eq!(
            decide(Server::StartedAt(12), Some(&pinned), true, 144),
            Decision::Reuse
        );
    }

    #[test]
    fn restarts_a_session_nobody_vouches_for() {
        let unread = decide(Server::Unknown, Some(&stamp(true, 10)), true, 192);
        assert_eq!(unread, Decision::Restart("its start time cannot be read"));
        let unstamped = decide(Server::StartedAt(10), None, true, 192);
        assert_eq!(
            unstamped,
            Decision::Restart("no session stamp records how it booted")
        );
        let older = decide(Server::StartedAt(9), Some(&stamp(true, 10)), true, 192);
        assert_eq!(
            older,
            Decision::Restart("it started before the last stamped boot, outside the launcher")
        );
    }

    #[test]
    fn a_stamp_reads_back_what_it_wrote() {
        for written in [stamp(true, 1_790_308_524), stamp(false, 0)] {
            assert_eq!(Stamp::parse(&written.format()), Some(written));
        }
    }

    #[test]
    fn a_stamp_missing_a_line_or_holding_garbage_is_no_stamp() {
        assert!(Stamp::parse("retina=Y\ndpi=96\nbooted_after=5\n").is_some());
        assert_eq!(Stamp::parse("dpi=96\nbooted_after=5\n"), None);
        assert_eq!(Stamp::parse("retina=Y\ndpi=96\n"), None);
        assert_eq!(Stamp::parse("retina=maybe\ndpi=96\nbooted_after=5\n"), None);
        assert_eq!(Stamp::parse("retina=Y\ndpi=96\nbooted_after=soon\n"), None);
        assert_eq!(Stamp::parse("retina=Y\ndpi=high\nbooted_after=5\n"), None);
        // Written before the DPI was recorded.
        assert_eq!(Stamp::parse("retina=Y\nbooted_after=5\n"), None);
        assert_eq!(Stamp::parse(""), None);
    }

    #[test]
    fn a_running_wineserver_is_one_by_name() {
        // This test binary is not a wineserver.
        assert_eq!(
            process_start(i32::try_from(std::process::id()).unwrap()),
            None
        );
        assert_eq!(process_start(-1), None);
    }

    /// The lock is taken in wine's directory, created like wine creates it,
    /// and a holder in another process keeps it from being taken again and is
    /// the PID the lock names.
    #[test]
    fn takes_wines_lock_and_yields_to_a_running_holder() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("gamelauncher-lock-{}", std::process::id()));
        let base = root.join("base");
        let prefix = root.join("prefix");
        fs::create_dir_all(&prefix).unwrap();

        let lock = ServerLock::acquire(&base, &prefix).unwrap().unwrap();
        let dir = server_dir(&base, &prefix).unwrap();
        assert!(dir.starts_with(&base));
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(dir.join("lock").is_file());
        drop(lock);
        assert_eq!(server_pid(&base, &prefix).unwrap(), None);

        // fcntl locks are per process, so the holder has to be another one.
        let holder = LockHolder::start(&dir.join("lock"));
        assert!(ServerLock::acquire(&base, &prefix).unwrap().is_none());
        assert_eq!(server_pid(&base, &prefix).unwrap(), Some(holder.pid()));
        // A holder that is not a wineserver has no start time to trust.
        assert_eq!(process_start(holder.pid()), None);
        drop(holder);

        assert!(ServerLock::acquire(&base, &prefix).unwrap().is_some());
        fs::remove_dir_all(&root).unwrap();
    }

    /// Names the lock file to the copy of this test binary that runs
    /// `hold_wines_lock`.
    const HOLD_LOCK_ENV: &str = "GAMELAUNCHER_TEST_HOLD_LOCK";

    /// What `hold_wines_lock` prints once it has tried to take the lock,
    /// followed by `held` or `refused`.
    const HOLD_LOCK_SAYS: &str = "lock holder: ";

    /// A process holding wine's lock, killed and reaped when this is dropped,
    /// also when a failing assert unwinds past it.
    struct LockHolder(std::process::Child);

    impl LockHolder {
        /// Start a copy of this test binary that takes wine's lock on `path`
        /// the way a wineserver does, and return once it holds the lock.
        ///
        /// It is a new process image rather than a fork of this one. The
        /// harness runs other tests on other threads, and a fork taken while
        /// one of them is inside a one-time libSystem initializer (libnotify's,
        /// for one) aborts in the child, in fork's own child handlers, before
        /// any code of the test runs.
        fn start(path: &Path) -> Self {
            use std::io::BufRead;
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "session::tests::hold_wines_lock"])
                .args(["--ignored", "--nocapture"])
                .env(HOLD_LOCK_ENV, path)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let stdout = child.stdout.take().unwrap();
            let holder = Self(child);
            let said = io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
                .find_map(|line| {
                    line.split_once(HOLD_LOCK_SAYS)
                        .map(|(_, word)| word.to_owned())
                });
            match said.as_deref() {
                Some("held") => holder,
                Some(word) => panic!("the lock holder could not take the lock: {word}"),
                None => panic!("the lock holder exited without saying"),
            }
        }

        fn pid(&self) -> i32 {
            i32::try_from(self.0.id()).unwrap()
        }
    }

    impl Drop for LockHolder {
        fn drop(&mut self) {
            // Either can only fail when the holder has already exited, and
            // then its lock is gone too.
            self.0.kill().ok();
            self.0.wait().ok();
        }
    }

    /// The holder's side of `LockHolder::start`, which runs it in a copy of
    /// this binary and names the lock file in `HOLD_LOCK_ENV`; without that
    /// it does nothing. It takes the lock, says whether it got it, and keeps
    /// it until it is killed or its stdin closes, which also happens when the
    /// test's process ends.
    #[test]
    #[ignore = "the lock holder of takes_wines_lock_and_yields_to_a_running_holder"]
    fn hold_wines_lock() {
        let Some(path) = std::env::var_os(HOLD_LOCK_ENV) else {
            return;
        };
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        let word = if try_write_lock(&file).unwrap() {
            "held"
        } else {
            "refused"
        };
        println!("\n{HOLD_LOCK_SAYS}{word}");
        io::copy(&mut io::stdin(), &mut io::sink()).unwrap();
    }
}
