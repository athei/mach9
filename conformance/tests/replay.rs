//! Parses the raw logs of earlier runs again, to check a change to the
//! output parser against real output: every run of a test's log must give
//! the outcome its `[conformance] parsed:` line holds.
//!
//! It needs logs of this machine, so it is ignored by default:
//!
//! ```text
//! CONFORMANCE_REPLAY=<dir>[:<dir>...] cargo test -p conformance --test replay -- --ignored --nocapture
//! ```
//!
//! A directory is a run's directory, `runs/<run id>`, or `baseline-raw`, or
//! any directory above test logs. A parsed line without `unexplained=`
//! means none, also in a log older than the field, so the comparison is
//! exact. The test prints how many runs it read and each run with
//! unexplained failures.

use std::{
    fs,
    path::{Path, PathBuf},
};

use conformance::{
    outcome::Key,
    output::{Ending, parse_run},
};

/// The test logs under `dir`: every `<unit>/<key>.log` but the listings.
fn test_logs(dir: &Path, logs: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            test_logs(&path, logs);
        } else if path.extension().is_some_and(|ext| ext == "log")
            && path.file_name().is_some_and(|name| {
                name != "session.log" && !name.to_string_lossy().ends_with(".list.log")
            })
        {
            logs.push(path);
        }
    }
}

/// How a run ended, from its trailer line.
fn ending(line: &str) -> Option<Ending> {
    let number = |rest: &str| rest.split(' ').next()?.parse().ok();
    if let Some(rest) = line.strip_prefix("[conformance] exited with code ") {
        return number(rest).map(Ending::Exited);
    }
    if let Some(rest) = line.strip_prefix("[conformance] killed by signal ") {
        return number(rest).map(Ending::Signaled);
    }
    line.starts_with("[conformance] stall: ")
        .then_some(Ending::Stalled)
}

/// One run of a test as its log holds it.
struct Stored {
    output: String,
    ending: Ending,
    /// The `[conformance] parsed:` line without its prefix.
    parsed: String,
}

/// The runs of a test log's `text`: each a `[conformance] <pass> run,
/// attempt <n>:` line, the output, the trailer and the parsed line.
fn runs(text: &str) -> Vec<Stored> {
    let mut runs = Vec::new();
    let mut output: Option<String> = None;
    let mut ended: Option<(String, Ending)> = None;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches('\n');
        if bare.starts_with("[conformance] ") && bare.contains(" run, attempt ") {
            output = Some(String::new());
            ended = None;
        } else if output.is_some()
            && let Some(ending) = ending(bare)
        {
            ended = output.take().map(|output| (output, ending));
        } else if let Some(text) = output.as_mut() {
            text.push_str(line);
        } else if let Some(parsed) = bare.strip_prefix("[conformance] parsed: ")
            && let Some((output, ending)) = ended.take()
        {
            runs.push(Stored {
                output,
                ending,
                parsed: parsed.to_owned(),
            });
        }
    }
    runs
}

#[test]
#[ignore = "needs CONFORMANCE_REPLAY, the raw logs of earlier runs"]
fn stored_runs_parse_to_their_parsed_lines() {
    let dirs = std::env::var("CONFORMANCE_REPLAY")
        .expect("CONFORMANCE_REPLAY names the directories of raw logs to read");
    let mut differ = Vec::new();
    for dir in dirs.split(':') {
        let mut logs = Vec::new();
        test_logs(Path::new(dir), &mut logs);
        logs.sort();
        let mut count = 0;
        let mut unexplained = 0;
        for log in &logs {
            let unit = log
                .parent()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let stem = log
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            let key = if stem == "whole" {
                Key::Whole
            } else {
                Key::Function(stem)
            };
            let text = String::from_utf8_lossy(&fs::read(log).unwrap()).into_owned();
            for stored in runs(&text) {
                count += 1;
                let run = parse_run(&stored.output, &unit, &key, stored.ending);
                let now = run.outcome;
                let crash = run
                    .crash
                    .as_ref()
                    .map_or_else(String::new, |crash| format!(", {crash:?}"));
                if format!("{now}{crash}") != stored.parsed {
                    differ.push(format!(
                        "{}: parsed {}, now {now}{crash}",
                        log.display(),
                        stored.parsed
                    ));
                }
                if now.unexplained > 0 {
                    unexplained += 1;
                    println!("unexplained {}: {now}", log.display());
                }
            }
        }
        println!(
            "{dir}: {} logs, {count} runs, {unexplained} with unexplained failures",
            logs.len()
        );
        assert!(count > 0, "{dir} holds no run of a test");
    }
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}
