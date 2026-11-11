//! The Wine session of an invocation: a fresh prefix with the crash dialog
//! off and the launcher's display settings, under one persistent wineserver
//! that every test of the invocation attaches to.
//!
//! [`Session::start`] refuses to run beside a wineserver of the same
//! bundle, which would share the display and the GPU with the tests, and
//! ends a leftover server of the runner's own prefix first. It then makes
//! the prefix fresh, boots it, sets the registry values, and starts the
//! session again ([`Session::restart`]) so that the desktop starts with
//! them: win32u reads `EmulateModeset` and the Mac driver its retina mode
//! once per process, and the session's display modes are computed when
//! its desktop starts. A restart is also what the runner does after a
//! stall or a key that was not reached. [`Session::end`], or the drop of
//! the session, ends every process of the prefix with `wineserver -k`.
//!
//! A build with `panic = "abort"`, as the workspace's release profile is,
//! does not run the drop on a panic, so the session of a panicking runner
//! stays up; the next invocation on the same state directory ends it before
//! it makes the prefix fresh.

use std::{
    ffi::OsString,
    fmt,
    fs::File,
    io::Write as _,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

use crate::{
    exec::{self, Spec},
    layout::{self, Bundle, PREFIX_TREE, State, TREE_MARKER, WORK_TREE},
    procs,
};

/// How long the boot of a fresh prefix may take: `wineboot --init` creates
/// it.
const BOOT_LIMIT: Duration = Duration::from_secs(600);

/// How long any other step may take.
const STEP_LIMIT: Duration = Duration::from_secs(120);

/// How long `wineserver -k` and `wineserver -w` may take each: the server
/// kills its clients with `SIGKILL` and exits.
const STOP_LIMIT: Duration = Duration::from_secs(30);

/// The stop flag of the steps that end a session, which run after an
/// interrupt too.
static NEVER: AtomicBool = AtomicBool::new(false);

/// The registry values the prefix gets before the session that runs the
/// tests starts.
///
/// Each is the `reg add` key, value name, type and data. The display
/// values are those `launcher/app/src/registry.rs` keeps in a game's
/// prefix, with what the launcher's default `auto` gives on a Retina
/// display: retina mode on (`retina.rs`) and twice the unscaled 96 DPI,
/// 192 (`dpi.rs`). The tests run as a game does on such a display, and the
/// baselines are recorded on one. The launcher's detection of the display
/// lives in its binary, so these are the values it produces, not a call of
/// it.
pub const SETTINGS: [(&str, &str, &str, &str); 4] = [
    (
        r"HKCU\Software\Wine\WineDbg",
        "ShowCrashDialog",
        "REG_DWORD",
        "0",
    ),
    (
        r"HKCU\Software\Wine\X11 Driver",
        "EmulateModeset",
        "REG_SZ",
        "Y",
    ),
    (
        r"HKCU\Software\Wine\Mac Driver",
        "RetinaMode",
        "REG_SZ",
        "Y",
    ),
    (
        r"HKCU\Control Panel\Desktop",
        "LogPixels",
        "REG_DWORD",
        "192",
    ),
];

/// Why a command stopped before it had a result.
#[derive(Debug)]
pub enum Failure {
    /// The setup failed or a result cannot be trusted; the message says
    /// why. The command exits with 2.
    Setup(String),
    /// A signal asked the runner to stop.
    Interrupted,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Setup(message) => f.write_str(message),
            Self::Interrupted => f.write_str(
                "interrupted; the finished tests are kept for the next run with the same \
                 arguments",
            ),
        }
    }
}

impl Failure {
    /// The exit code: 2 for a setup failure, 130 for an interrupt.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Setup(_) => 2,
            Self::Interrupted => 130,
        }
    }
}

/// A running session of the runner's prefix.
pub struct Session<'a> {
    bundle: &'a Bundle,
    prefix: PathBuf,
    work: PathBuf,
    env: &'a [(OsString, OsString)],
    log: File,
    log_path: PathBuf,
    stop: &'a AtomicBool,
    /// Whether a server may run that [`Self::end`] has not ended.
    live: bool,
}

impl<'a> Session<'a> {
    /// Check the running wineservers, make the prefix and the working
    /// directory fresh, boot the prefix, set [`SETTINGS`] and start the
    /// persistent session. Every step's output goes to `log_path`.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::Setup`] when a wineserver of the bundle serves
    /// another prefix, a directory is not the runner's, or a step fails,
    /// and [`Failure::Interrupted`] when `stop` is set meanwhile.
    pub fn start(
        bundle: &'a Bundle,
        state: &State,
        env: &'a [(OsString, OsString)],
        log_path: &Path,
        stop: &'a AtomicBool,
    ) -> Result<Self, Failure> {
        let log = File::create(log_path)
            .map_err(|e| Failure::Setup(format!("{}: {e}", log_path.display())))?;
        let mut session = Self {
            bundle,
            prefix: state.prefix(),
            work: state.work(),
            env,
            log,
            log_path: log_path.to_path_buf(),
            stop,
            live: false,
        };
        layout::fresh_dir(&session.work, WORK_TREE).map_err(Failure::Setup)?;
        session.check_servers()?;
        layout::fresh_dir(&session.prefix, PREFIX_TREE).map_err(Failure::Setup)?;
        session.live = true;
        session.step(
            "the prefix boot",
            &bundle.wine,
            &["cmd", "/c", "exit"],
            BOOT_LIMIT,
        )?;
        for (key, name, kind, data) in SETTINGS {
            session.step(
                &format!("setting {key}\\{name}"),
                &bundle.wine,
                &["reg", "add", key, "/v", name, "/t", kind, "/d", data, "/f"],
                STEP_LIMIT,
            )?;
        }
        session.restart()?;
        Ok(session)
    }

    /// End the session and start it again: `wineserver -k` and `-w`, a
    /// persistent `wineserver -p`, `wine wineboot` to start the desktop
    /// and services, and `wineserver -k0` to check the server stayed.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::Setup`] when the new session does not start, and
    /// [`Failure::Interrupted`] when `stop` is set meanwhile.
    pub fn restart(&mut self) -> Result<(), Failure> {
        self.stop_server(self.stop)?;
        self.live = true;
        let (server, wine) = (&self.bundle.wineserver, &self.bundle.wine);
        self.step("the persistent wineserver", server, &["-p"], STEP_LIMIT)?;
        self.step("the session boot", wine, &["wineboot"], STEP_LIMIT)?;
        self.step("the check for the wineserver", server, &["-k0"], STEP_LIMIT)
    }

    /// End every process of the prefix and its wineserver.
    ///
    /// # Errors
    ///
    /// Returns a message when a server of the prefix is still running
    /// afterwards.
    pub fn end(&mut self) -> Result<(), String> {
        let _ = self.stop_server(&NEVER);
        self.live = false;
        match procs::server_of(&self.bundle.wineserver, &self.prefix) {
            Some(pid) => Err(format!(
                "the wineserver {pid} of {} still runs after `wineserver -k`",
                self.prefix.display()
            )),
            None => Ok(()),
        }
    }

    /// The wineserver executable and the prefix, for sampling the server.
    #[must_use]
    pub fn server(&self) -> (PathBuf, PathBuf) {
        (self.bundle.wineserver.clone(), self.prefix.clone())
    }

    /// `wineserver -k`, then `wineserver -w`; their exit status is not
    /// checked, since both fail when no server runs.
    fn stop_server(&self, stop: &AtomicBool) -> Result<(), Failure> {
        for arg in ["-k", "-w"] {
            let _ = writeln!(&self.log, "[conformance] wineserver {arg}");
            let args = [OsString::from(arg)];
            let spec = self.spec(&self.bundle.wineserver, &args);
            let result = exec::run_step(&spec, &self.log, STOP_LIMIT, stop);
            if matches!(result, Err(exec::Error::Interrupted)) {
                return Err(Failure::Interrupted);
            }
        }
        Ok(())
    }

    /// Refuse a wineserver of the bundle that serves another prefix, and
    /// end one that serves the runner's prefix, left by an invocation that
    /// did not end it.
    fn check_servers(&self) -> Result<(), Failure> {
        let pids = procs::pids_of(&self.bundle.wineserver).map_err(Failure::Setup)?;
        if pids.is_empty() {
            return Ok(());
        }
        let ours = self
            .prefix
            .join(TREE_MARKER)
            .is_file()
            .then(|| procs::server_dir_name(&self.prefix).ok())
            .flatten();
        let mut leftover = false;
        for &pid in &pids {
            if ours.as_deref().is_some_and(|dir| procs::serves(pid, dir)) {
                leftover = true;
                continue;
            }
            let serving = procs::cwd_of(pid).map_or_else(
                || String::from("an unknown prefix"),
                |dir| dir.display().to_string(),
            );
            return Err(Failure::Setup(format!(
                "a wineserver of {} runs (pid {pid}, in {serving}); end what runs in that \
                 prefix first, the tests share the display and the GPU with it",
                self.bundle.dir.display()
            )));
        }
        if leftover {
            println!(
                "conformance: ending the wineserver a previous run left in {}",
                self.prefix.display()
            );
            self.stop_server(self.stop)?;
            if let Some(pid) = procs::server_of(&self.bundle.wineserver, &self.prefix) {
                return Err(Failure::Setup(format!(
                    "the wineserver {pid} a previous run left in {} does not end",
                    self.prefix.display()
                )));
            }
        }
        Ok(())
    }

    /// The spec of a step: the run's environment, in the working directory.
    fn spec<'s>(&'s self, program: &'s Path, args: &'s [OsString]) -> Spec<'s> {
        Spec {
            program,
            args,
            env: self.env,
            cwd: &self.work,
        }
    }

    /// Run `program` with `args` as the step `what`, which must succeed.
    fn step(
        &self,
        what: &str,
        program: &Path,
        args: &[&str],
        limit: Duration,
    ) -> Result<(), Failure> {
        let _ = writeln!(
            &self.log,
            "[conformance] {what}: {} {}",
            program.display(),
            args.join(" ")
        );
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let spec = self.spec(program, &args);
        let log = self.log_path.display();
        match exec::run_step(&spec, &self.log, limit, self.stop) {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(Failure::Setup(format!(
                "{what} ended with {status}; its output is in {log}"
            ))),
            Err(exec::Error::Interrupted) => Err(Failure::Interrupted),
            Err(e) => Err(Failure::Setup(format!(
                "{what} {e}; its output is in {log}"
            ))),
        }
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        if self.live {
            let _ = self.end();
        }
    }
}

#[cfg(test)]
mod tests;
