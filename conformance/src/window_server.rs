//! Watches the Mach port count of WindowServer during `run` and `record`.
//!
//! WindowServer loses Mach ports as applications register with it, and
//! every test process is an application, as is the AutoFill helper `AppKit`
//! starts for it. When its port table reaches the kernel's limit (305834 on
//! the Mac where it happened), the kernel kills WindowServer and loginwindow
//! logs the session out (mach9 #190). Only a new WindowServer, after a log
//! out and in or a restart of the Mac, starts again from a few thousand
//! ports.
//!
//! So `run` and `record` read the count before they start, before each
//! unit and before a second pass, and stop above [`PORT_THRESHOLD`]. A
//! reading that fails stops a run before it starts ([`check`]) and only
//! warns later ([`between_units`]). The resume files stay, so the same
//! command continues after the new login.
//!
//! WindowServer runs as another user, and neither `task_read_for_pid` nor
//! `proc_pidinfo` works on it without privileges. `top` has the entitlement
//! to read other tasks' ports, so the count is the `#PORTS` column of
//! `top -l 1 -pid <pid> -stats pid,ports` ([`ports`]); a reading takes about
//! a quarter of a second.

use crate::procs;

/// The count above which `run` and `record` stop. It leaves room for
/// about 20000 more test processes below the kernel's limit, far more
/// than one unit runs.
pub const PORT_THRESHOLD: u64 = 250_000;

/// The size of WindowServer's port table when the kernel killed it.
pub const PORT_LIMIT: u64 = 305_834;

/// The file name of WindowServer's executable, which `ps` reports as a
/// full path.
const NAME: &str = "WindowServer";

/// The tool that reads the count.
const TOP_TOOL: &str = "/usr/bin/top";

/// The Mach port count of WindowServer: the largest one when more than one
/// runs.
///
/// # Errors
///
/// Returns a message when no WindowServer runs or `ps` or `top` fails, runs
/// past its limit or prints no count for it.
pub fn ports() -> Result<u64, String> {
    let table = procs::output_limited("ps", &["-Ao", "pid=,comm="], procs::HELPER_LIMIT)?;
    let pids = pids(&table);
    if pids.is_empty() {
        return count_of(&pids, &[]);
    }
    let pid_args: Vec<String> = pids.iter().map(u32::to_string).collect();
    let mut args = vec!["-l", "1", "-stats", "pid,ports"];
    for pid in &pid_args {
        args.extend(["-pid", pid.as_str()]);
    }
    let report = procs::output_limited(TOP_TOOL, &args, procs::HELPER_LIMIT)?;
    count_of(&pids, &parse_top(&report))
        .map_err(|e| format!("`{TOP_TOOL} {}`: {e}", args.join(" ")))
}

/// The largest count of `counts`, the pid and count pairs of [`parse_top`],
/// over the WindowServer processes `pids`.
///
/// # Errors
///
/// Returns a message when `pids` is empty or `counts` lacks one of them.
pub fn count_of(pids: &[u32], counts: &[(u32, u64)]) -> Result<u64, String> {
    if pids.is_empty() {
        return Err(String::from("no WindowServer runs"));
    }
    let mut max = 0;
    for pid in pids {
        let (_, count) = counts
            .iter()
            .find(|(listed, _)| listed == pid)
            .ok_or_else(|| format!("no port count of WindowServer {pid}"))?;
        max = max.max(*count);
    }
    Ok(max)
}

/// Read the count before a run starts and refuse one above
/// [`PORT_THRESHOLD`].
///
/// # Errors
///
/// Returns the message of [`ports`] or [`over_threshold`].
pub fn check() -> Result<u64, String> {
    let count = ports().map_err(|e| format!("cannot read WindowServer's Mach ports: {e}"))?;
    over_threshold(count).map_or(Ok(count), Err)
}

/// What a `reading` of [`ports`] between units does.
///
/// Nothing at or below [`PORT_THRESHOLD`], and a warning to print when the
/// reading failed, since the run started below the threshold and one unit
/// adds far fewer ports than the margin to the limit.
///
/// # Errors
///
/// Returns the message of [`over_threshold`], which stops the run.
pub fn between_units(reading: Result<u64, String>) -> Result<Option<String>, String> {
    match reading {
        Ok(count) => over_threshold(count).map_or(Ok(None), Err),
        Err(e) => Ok(Some(format!(
            "conformance: cannot read WindowServer's Mach ports, the run goes on: {e}"
        ))),
    }
}

/// The message that stops a run at `count` ports, or `None` at or below
/// [`PORT_THRESHOLD`].
#[must_use]
pub fn over_threshold(count: u64) -> Option<String> {
    (count > PORT_THRESHOLD).then(|| {
        format!(
            "WindowServer holds {count} Mach ports, more than the {PORT_THRESHOLD} the runner \
             allows: it loses ports with every application that starts, and at {PORT_LIMIT} the \
             kernel kills it and the session is logged out; log out and in, or restart the Mac, \
             to start a new WindowServer, then run the same command again, which continues with \
             the tests not finished yet"
        )
    })
}

/// The line that ends a run which started at `start` ports, with the
/// reading `end` after it.
#[must_use]
pub fn summary(start: u64, end: Result<u64, String>) -> String {
    match end {
        Ok(end) => format!(
            "conformance: WindowServer holds {end} Mach ports, {:+} since the start",
            i128::from(end) - i128::from(start)
        ),
        Err(e) => format!("conformance: cannot read WindowServer's Mach ports: {e}"),
    }
}

/// The pids of the WindowServer lines of a `ps -Ao pid=,comm=` table.
#[must_use]
pub fn pids(table: &str) -> Vec<u32> {
    procs::parse_ps(table)
        .into_iter()
        .filter(|(_, command)| command.rsplit('/').next() == Some(NAME))
        .map(|(pid, _)| pid)
        .collect()
}

/// The pid and port count of each process line of the output of
/// `top -l <n> -stats pid,ports`: the lines after a `PID #PORTS` header. A
/// count that changed since the previous sample ends in `+` or `-`.
#[must_use]
pub fn parse_top(report: &str) -> Vec<(u32, u64)> {
    let mut counts = Vec::new();
    let mut in_table = false;
    for line in report.lines() {
        let mut fields = line.split_whitespace();
        let (Some(first), Some(second)) = (fields.next(), fields.next()) else {
            in_table = false;
            continue;
        };
        if first == "PID" {
            in_table = second == "#PORTS";
            continue;
        }
        if !in_table {
            continue;
        }
        let count = second.trim_end_matches(['+', '-']);
        if let (Ok(pid), Ok(count)) = (first.parse(), count.parse()) {
            counts.push((pid, count));
        }
    }
    counts
}

#[cfg(test)]
mod tests;
