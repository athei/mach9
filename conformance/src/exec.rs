//! Runs one Wine process tree under a watchdog and collects its output.
//!
//! [`run_tree`] starts the process in a process group of its own, with
//! stdout and stderr on the write end of one pipe, so the output of the
//! whole tree arrives as one stream in the order it was written: the
//! parser needs the `wt-run` marker of a key before the lines it caused.
//! The watching loop reads the pipe itself, with `poll`, between its looks
//! at the root process, the time since the last output and the total time,
//! and the interrupt flag; no thread stands between the pipe and the
//! output.
//!
//! Wine starts helper processes (`explorer.exe /desktop`, a wineserver)
//! that inherit the pipe and put themselves in process groups of their
//! own, so the end of the pipe may never come. Once the root has exited
//! its group is killed, and then everything the root and the group wrote
//! is in the pipe: it is read until the pipe is empty or ends, without
//! waiting for a quiet gap. The end is then awaited for [`DRAIN_QUIET`]
//! while members of the group die, and [`DRAIN_CAP`] bounds a holder that
//! keeps writing. A pipe still open after that is handed to a thread that
//! reads and discards until its end, so a helper's later writes go on
//! succeeding as they do when a game runs, instead of failing on a closed
//! pipe; the thread ends with the last holder, at the latest when the
//! session ends.
//!
//! [`run_step`] runs a step of the session (boot, `reg add`, `wineserver`)
//! with its output in a log file, which no helper can hold open.
//!
//! Limits:
//!
//! - A process of a test that left the process group (`setsid`, `setpgid`)
//!   survives the group kill and runs on into later keys; only a stall or
//!   a key that was not reached restarts the session, which ends it.
//! - Output of a helper outside the group that keeps writing resets the
//!   idle limit, so a hang under such a helper is caught by the total
//!   limit only.

use std::{
    ffi::OsString,
    fmt::{self, Write as _},
    fs::{self, File},
    io::{self, PipeReader, Read as _},
    os::{
        fd::AsFd as _,
        unix::process::{CommandExt as _, ExitStatusExt as _},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use nix::{
    poll::{PollFd, PollFlags, PollTimeout, poll},
    sys::signal::{Signal, killpg},
    unistd::Pid,
};

use crate::{output::Ending, procs};

/// How often the loops look at the root process and the flag.
const POLL: Duration = Duration::from_millis(50);

/// Once the pipe is empty after the root exited, how long its end is
/// awaited from members of the killed group that are still dying.
pub const DRAIN_QUIET: Duration = Duration::from_millis(200);

/// After the root exited, the pipe is read for at most this long.
pub const DRAIN_CAP: Duration = Duration::from_secs(2);

/// How long the samples of a stalled test may take together.
const SAMPLE_BUDGET: Duration = Duration::from_secs(60);

/// How long `sample` watches a process.
const SAMPLE_SECONDS: &str = "2";

/// The size of one read from the pipe.
const CHUNK: usize = 1 << 16;

/// The watchdog's limits for one process tree.
#[derive(Clone, Copy)]
pub struct Limits {
    /// No output for this long is a stall.
    pub idle: Duration,
    /// Running for longer than this is a stall.
    pub total: Duration,
}

/// What to run: the program, its arguments, its whole environment and its
/// working directory.
pub struct Spec<'a> {
    pub program: &'a Path,
    pub args: &'a [OsString],
    pub env: &'a [(OsString, OsString)],
    pub cwd: &'a Path,
}

impl Spec<'_> {
    /// The command, in a process group of its own, with no input.
    fn command(&self) -> Command {
        let mut command = Command::new(self.program);
        command
            .args(self.args)
            .env_clear()
            .envs(self.env.iter().cloned())
            .current_dir(self.cwd)
            .stdin(Stdio::null())
            .process_group(0);
        command
    }
}

/// How a stalled tree is sampled before it is killed.
pub struct Sampler {
    /// The `sample` tool; nothing is sampled when it does not exist.
    pub tool: PathBuf,
    /// Where the samples go.
    pub dir: PathBuf,
    /// The samples are `<stem>.sample.txt` and
    /// `<stem>.wineserver-sample.txt`.
    pub stem: String,
    /// The wineserver executable and the prefix whose server to sample as
    /// well.
    pub wineserver: Option<(PathBuf, PathBuf)>,
}

/// Which limit a stall hit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stall {
    Idle,
    Total,
}

/// A process tree that ran to its end or was killed for a stall.
pub struct Finished {
    /// The merged stdout and stderr.
    pub output: Vec<u8>,
    pub ending: Ending,
    /// The limit the tree hit, for [`Ending::Stalled`].
    pub stall: Option<Stall>,
    /// From the start to the end of the root process.
    pub elapsed: Duration,
    /// Whether the pipe reached its end; `false` when a process outside
    /// the group still held it after the drain.
    pub pipe_closed: bool,
    /// The samples taken before a stall's kill.
    pub samples: Vec<PathBuf>,
}

impl Finished {
    /// The lines that say how the tree ended, for the end of its raw log.
    #[must_use]
    pub fn trailer(&self, limits: &Limits) -> String {
        let seconds = self.elapsed.as_secs_f64();
        let mut text = match (self.ending, self.stall) {
            (Ending::Exited(code), _) => {
                format!("[conformance] exited with code {code} after {seconds:.1} s\n")
            }
            (Ending::Signaled(signal), _) => {
                format!("[conformance] killed by signal {signal} after {seconds:.1} s\n")
            }
            (Ending::Stalled, Some(Stall::Total)) => format!(
                "[conformance] stall: still running after {} s; the process group was killed\n",
                limits.total.as_secs()
            ),
            (Ending::Stalled, _) => format!(
                "[conformance] stall: no output for {} s; the process group was killed after \
                 {seconds:.1} s\n",
                limits.idle.as_secs()
            ),
        };
        if !self.samples.is_empty() {
            let samples: Vec<String> = self
                .samples
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            let _ = writeln!(text, "[conformance] samples: {}", samples.join(", "));
        }
        if !self.pipe_closed {
            text.push_str(
                "[conformance] a process outside the test's process group still held the output \
                 pipe after the test ended; what it wrote later is not in this log\n",
            );
        }
        text
    }
}

/// Why [`run_tree`] or [`run_step`] gave no result.
#[derive(Debug)]
pub enum Error {
    /// The program could not be started.
    Spawn(io::Error),
    /// The root process could not be waited for, or the pipe not read; the
    /// process group was killed.
    Wait(io::Error),
    /// A step ran past its limit and was killed.
    TimedOut(Duration),
    /// The interrupt flag was set; the process group was killed.
    Interrupted,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "cannot be started: {e}"),
            Self::Wait(e) => write!(f, "cannot be waited for: {e}"),
            Self::TimedOut(limit) => {
                write!(
                    f,
                    "was still running after {} s and was killed",
                    limit.as_secs()
                )
            }
            Self::Interrupted => f.write_str("was interrupted"),
        }
    }
}

/// What one look at the pipe found.
enum Read {
    /// Bytes, appended to the output.
    Data,
    /// Nothing within the wait: the pipe is empty.
    Empty,
    /// The end of the pipe: every write end is closed.
    End,
}

/// Run `spec` as a process tree under the watchdog `limits`.
///
/// A tree that prints nothing for `limits.idle`, or runs longer than
/// `limits.total`, is sampled with `sampler` and its group killed; it
/// ends as [`Ending::Stalled`]. The group is killed after the root exits
/// too, for the processes it left behind, and on every error after the
/// spawn. The flag `stop` is read every 50 ms, also while the tree is
/// sampled; once it is set the group is killed and the result is
/// [`Error::Interrupted`].
///
/// # Errors
///
/// Returns [`Error::Spawn`], [`Error::Wait`] or [`Error::Interrupted`].
pub fn run_tree(
    spec: &Spec<'_>,
    limits: &Limits,
    sampler: Option<&Sampler>,
    stop: &AtomicBool,
) -> Result<Finished, Error> {
    let (mut reader, writer) = io::pipe().map_err(Error::Spawn)?;
    let error_writer = writer.try_clone().map_err(Error::Spawn)?;
    let mut command = spec.command();
    command.stdout(writer).stderr(error_writer);
    let spawned = command.spawn();
    // The command holds the write ends until it is dropped, and the pipe
    // ends only once every write end is closed.
    drop(command);
    let mut child = spawned.map_err(Error::Spawn)?;
    let mut output = Vec::new();
    let result = watch(&mut child, &mut reader, &mut output, limits, sampler, stop);
    kill_group(&child);
    let (status, stall, elapsed, taken) = match result {
        Ok(watched) => watched,
        Err(e) => {
            let _ = child.wait();
            return Err(e);
        }
    };
    let ending = if let Some(status) = status {
        ending(status)
    } else {
        child.wait().map_err(Error::Wait)?;
        Ending::Stalled
    };
    let closed = drain(&mut reader, &mut output);
    if !closed {
        discard_until_end(reader);
    }
    Ok(Finished {
        output,
        ending,
        stall,
        elapsed,
        pipe_closed: closed,
        samples: taken,
    })
}

/// What [`watch`] saw: the root's exit status (none for a stall), the
/// stall, the run time and the samples.
type Watched = (Option<ExitStatus>, Option<Stall>, Duration, Vec<PathBuf>);

/// Read the pipe into `output` until the root exits or the watchdog fires,
/// and sample a stalled tree. The caller kills the group.
fn watch(
    child: &mut Child,
    reader: &mut PipeReader,
    output: &mut Vec<u8>,
    limits: &Limits,
    sampler: Option<&Sampler>,
    stop: &AtomicBool,
) -> Result<Watched, Error> {
    let start = Instant::now();
    let mut last_output = start;
    let mut ended = false;
    let (status, stall) = loop {
        if stop.load(Ordering::SeqCst) {
            return Err(Error::Interrupted);
        }
        if let Some(status) = child.try_wait().map_err(Error::Wait)? {
            break (Some(status), None);
        }
        let now = Instant::now();
        if now.duration_since(last_output) >= limits.idle {
            break (None, Some(Stall::Idle));
        }
        if now.duration_since(start) >= limits.total {
            break (None, Some(Stall::Total));
        }
        if ended {
            thread::sleep(POLL);
            continue;
        }
        match read_pipe(reader, POLL, output).map_err(Error::Wait)? {
            Read::Data => last_output = Instant::now(),
            Read::Empty => {}
            Read::End => ended = true,
        }
    };
    let elapsed = start.elapsed();
    let taken = match (stall, sampler) {
        (Some(_), Some(sampler)) => sample(child.id(), sampler, stop),
        _ => Vec::new(),
    };
    if stop.load(Ordering::SeqCst) {
        return Err(Error::Interrupted);
    }
    Ok((status, stall, elapsed, taken))
}

/// Wait up to `wait` for the pipe to be readable, then read once.
fn read_pipe(reader: &mut PipeReader, wait: Duration, output: &mut Vec<u8>) -> io::Result<Read> {
    let ready = {
        let mut fds = [PollFd::new(reader.as_fd(), PollFlags::POLLIN)];
        poll(
            &mut fds,
            PollTimeout::try_from(wait).unwrap_or(PollTimeout::MAX),
        )
    };
    match ready {
        Ok(0) | Err(nix::errno::Errno::EINTR) => return Ok(Read::Empty),
        Ok(_) => {}
        Err(e) => return Err(io::Error::from(e)),
    }
    let mut buffer = vec![0u8; CHUNK];
    match reader.read(&mut buffer) {
        Ok(0) => Ok(Read::End),
        Ok(read) => {
            output.extend_from_slice(&buffer[..read]);
            Ok(Read::Data)
        }
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(Read::Empty),
        Err(e) => Err(e),
    }
}

/// Read what is left in the pipe after the root exited and its group was
/// killed. Whether the pipe ended.
///
/// Everything the root and its group wrote is in the pipe by then, so it
/// is read without waiting until the pipe is empty or ends. Then the end
/// is awaited for [`DRAIN_QUIET`] after the last byte, for members of the
/// group that are still dying; [`DRAIN_CAP`] bounds both for a holder that
/// keeps writing.
fn drain(reader: &mut PipeReader, output: &mut Vec<u8>) -> bool {
    let cap = Instant::now() + DRAIN_CAP;
    loop {
        match read_pipe(reader, Duration::ZERO, output) {
            Ok(Read::Data) if Instant::now() < cap => {}
            Ok(Read::Data) => return false,
            Ok(Read::Empty) => break,
            Ok(Read::End) | Err(_) => return true,
        }
    }
    let mut last = Instant::now();
    loop {
        let wait = (last + DRAIN_QUIET)
            .min(cap)
            .saturating_duration_since(Instant::now());
        if wait.is_zero() {
            return false;
        }
        // `poll` takes whole milliseconds, so an empty look can come back
        // before `wait` passed; the next turn checks the time again.
        match read_pipe(reader, wait, output) {
            Ok(Read::Data) => last = Instant::now(),
            Ok(Read::Empty) => {}
            Ok(Read::End) | Err(_) => return true,
        }
    }
}

/// Read and discard `reader` on a thread until the pipe ends, so its
/// holders can go on writing.
fn discard_until_end(mut reader: PipeReader) {
    thread::spawn(move || {
        let mut buffer = vec![0u8; CHUNK];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return,
                Err(e) if e.kind() != io::ErrorKind::Interrupted => return,
                _ => {}
            }
        }
    });
}

/// Run one step of the session with its stdout and stderr appended to
/// `log`, for at most `limit`.
///
/// # Errors
///
/// Returns [`Error::Spawn`], [`Error::Wait`], [`Error::TimedOut`] when the
/// step ran past `limit`, or [`Error::Interrupted`]; the step's group is
/// killed on each but the first.
pub fn run_step(
    spec: &Spec<'_>,
    log: &File,
    limit: Duration,
    stop: &AtomicBool,
) -> Result<ExitStatus, Error> {
    let mut command = spec.command();
    command
        .stdout(log.try_clone().map_err(Error::Spawn)?)
        .stderr(log.try_clone().map_err(Error::Spawn)?);
    let mut child = command.spawn().map_err(Error::Spawn)?;
    let deadline = Instant::now() + limit;
    let error = loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(e) => break Error::Wait(e),
        }
        if stop.load(Ordering::SeqCst) {
            break Error::Interrupted;
        }
        if Instant::now() >= deadline {
            break Error::TimedOut(limit);
        }
        thread::sleep(POLL);
    };
    kill_group(&child);
    let _ = child.wait();
    Err(error)
}

/// How the root process ended, as [`crate::output::parse_run`] takes it.
fn ending(status: ExitStatus) -> Ending {
    status.code().map_or_else(
        || Ending::Signaled(status.signal().unwrap_or(0)),
        Ending::Exited,
    )
}

/// `SIGKILL` the process group of `child`, its own: it was spawned as the
/// leader of a new group. A group that is gone is no error.
fn kill_group(child: &Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
    }
}

/// Sample `pid`, and the wineserver of the prefix when the sampler names
/// one, at the same time, within [`SAMPLE_BUDGET`]; a set `stop` ends the
/// samples at once. The files written.
fn sample(pid: u32, sampler: &Sampler, stop: &AtomicBool) -> Vec<PathBuf> {
    if !sampler.tool.is_file() || fs::create_dir_all(&sampler.dir).is_err() {
        return Vec::new();
    }
    let mut targets = vec![(pid, format!("{}.sample.txt", sampler.stem))];
    if let Some((exe, prefix)) = &sampler.wineserver {
        let name = format!("{}.wineserver-sample.txt", sampler.stem);
        if let Some(server) = procs::server_of(exe, prefix) {
            targets.push((server, name));
        } else {
            let note = format!("no {} serves {}\n", exe.display(), prefix.display());
            let _ = fs::write(sampler.dir.join(name), note);
        }
    }
    let deadline = Instant::now() + SAMPLE_BUDGET;
    let mut running = Vec::new();
    for (pid, name) in targets {
        let path = sampler.dir.join(name);
        let Ok(file) = File::create(&path) else {
            continue;
        };
        let Ok(error_file) = file.try_clone() else {
            continue;
        };
        let child = Command::new(&sampler.tool)
            .args([pid.to_string().as_str(), SAMPLE_SECONDS, "-mayDie"])
            .stdin(Stdio::null())
            .stdout(file)
            .stderr(error_file)
            .spawn();
        if let Ok(child) = child {
            running.push((child, path));
        }
    }
    let mut written = Vec::new();
    for (mut child, path) in running {
        loop {
            let over = stop.load(Ordering::SeqCst) || Instant::now() >= deadline;
            match child.try_wait() {
                Ok(None) if !over => thread::sleep(POLL),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                _ => break,
            }
        }
        written.push(path);
    }
    written
}

#[cfg(test)]
mod tests;
