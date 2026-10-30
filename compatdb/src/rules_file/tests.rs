use std::{env, path::PathBuf, process::Command, sync::mpsc, thread, time::Duration};

use super::*;

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

const RULE: &str = "exe=Game.exe;dpi_aware=false";

const NO_LINES: [String; 0] = [];

/// A directory of one test beside the test binary, in Cargo's target
/// directory, removed when the test ends however it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> io::Result<Self> {
        let dir = env::current_exe()?.with_extension(test);
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    /// The absolute path of `name` in the directory.
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }

    /// Create the file `name` holding `bytes`, and return its path.
    fn file(&self, name: &str, bytes: &[u8]) -> io::Result<String> {
        let path = self.path(name);
        fs::write(&path, bytes)?;
        Ok(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn path_line(path: &str) -> String {
    format!("  WINE_COMPATDB_FILE = {path}")
}

fn not_read(reason: &str) -> String {
    format!("  warning: WINE_COMPATDB_FILE not read: {reason}; only the built-in rules apply")
}

/// A reader for the tests of [`choose`] alone.
fn reads_rule() -> impl FnOnce(&str) -> io::Result<String> {
    |_| Ok(String::from(RULE))
}

#[test]
fn without_either_variable_there_are_no_rules_and_nothing_to_log() {
    let chosen = choose(None, None, reads_rule());
    assert_eq!(chosen.text, None);
    assert_eq!(chosen.lines, NO_LINES);
}

#[test]
fn the_variable_alone_is_the_source_and_logs_nothing() {
    let chosen = choose(None, Some(String::from("exe=a.exe")), reads_rule());
    assert_eq!(chosen.text.as_deref(), Some("exe=a.exe"));
    assert_eq!(chosen.lines, NO_LINES);
}

#[test]
fn an_empty_variable_counts_as_unset() {
    let empty = || Some(String::new());
    // An empty path reads nothing, so the other variable is the source.
    let chosen = choose(empty(), Some(String::from("exe=a.exe")), reads_rule());
    assert_eq!(chosen.text.as_deref(), Some("exe=a.exe"));
    assert_eq!(chosen.lines, NO_LINES);
    // An empty table does not count as set beside a file, so no warning.
    let chosen = choose(Some(String::from("/rules")), empty(), reads_rule());
    assert_eq!(chosen.text.as_deref(), Some(RULE));
    assert_eq!(chosen.lines, [path_line("/rules")]);
    let chosen = choose(empty(), empty(), reads_rule());
    assert_eq!(chosen.text, None);
    assert_eq!(chosen.lines, NO_LINES);
}

#[test]
fn a_file_alone_is_the_source_and_its_path_is_logged() -> TestResult {
    let scratch = Scratch::new("file_alone")?;
    let path = scratch.file("rules", RULE.as_bytes())?;
    let chosen = choose(Some(path.clone()), None, read);
    assert_eq!(chosen.text.as_deref(), Some(RULE));
    assert_eq!(chosen.lines, [path_line(&path)]);
    Ok(())
}

#[test]
fn the_file_wins_over_the_variable_with_a_warning() -> TestResult {
    let scratch = Scratch::new("both_set")?;
    let path = scratch.file("rules", RULE.as_bytes())?;
    let chosen = choose(Some(path.clone()), Some(String::from("exe=a.exe")), read);
    assert_eq!(chosen.text.as_deref(), Some(RULE));
    assert_eq!(
        chosen.lines,
        [
            path_line(&path),
            String::from("  warning: WINE_COMPATDB_FILE replaces WINE_COMPATDB, which is ignored"),
        ]
    );
    Ok(())
}

#[test]
fn an_empty_file_is_a_table_without_rules_not_a_fallback() -> TestResult {
    let scratch = Scratch::new("empty_file")?;
    let path = scratch.file("rules", b"")?;
    let chosen = choose(Some(path), Some(String::from("exe=a.exe")), read);
    assert_eq!(chosen.text.as_deref(), Some(""));
    Ok(())
}

#[test]
fn a_missing_file_gives_the_built_in_rules_and_no_fallback() -> TestResult {
    let scratch = Scratch::new("missing_file")?;
    let path = scratch.path("rules");
    let chosen = choose(Some(path.clone()), Some(String::from("exe=a.exe")), read);
    assert_eq!(chosen.text, None);
    assert_eq!(
        chosen.lines,
        [
            path_line(&path),
            String::from("  warning: WINE_COMPATDB_FILE replaces WINE_COMPATDB, which is ignored"),
            not_read("No such file or directory (os error 2)"),
        ]
    );
    Ok(())
}

#[test]
fn a_relative_path_is_refused() {
    let chosen = choose(Some(String::from("rules")), None, read);
    assert_eq!(chosen.text, None);
    assert_eq!(
        chosen.lines,
        [path_line("rules"), not_read("not an absolute path")]
    );
}

#[test]
fn a_directory_is_refused() -> TestResult {
    let scratch = Scratch::new("directory")?;
    let path = scratch.path("");
    let chosen = choose(Some(path.clone()), None, read);
    assert_eq!(chosen.text, None);
    assert_eq!(
        chosen.lines,
        [path_line(&path), not_read("not a regular file")]
    );
    Ok(())
}

#[test]
fn a_fifo_is_refused_without_blocking() -> TestResult {
    let scratch = Scratch::new("fifo")?;
    let path = scratch.path("rules");
    assert!(Command::new("mkfifo").arg(&path).status()?.success());
    // On a thread, so a read that blocks fails this test instead of hanging
    // the run.
    let (sender, receiver) = mpsc::channel();
    let fifo = path.clone();
    thread::spawn(move || sender.send(choose(Some(fifo), None, read)));
    let chosen = receiver.recv_timeout(Duration::from_secs(5))?;
    assert_eq!(chosen.text, None);
    assert_eq!(
        chosen.lines,
        [path_line(&path), not_read("not a regular file")]
    );
    Ok(())
}

#[test]
fn a_file_over_the_limit_is_refused_and_one_at_the_limit_is_not() -> TestResult {
    let scratch = Scratch::new("oversize")?;
    let path = scratch.file("rules", b"")?;
    File::options()
        .write(true)
        .open(&path)?
        .set_len(MAX_BYTES + 1)?;
    let chosen = choose(Some(path.clone()), None, read);
    assert_eq!(chosen.text, None);
    assert_eq!(
        chosen.lines,
        [
            path_line(&path),
            not_read(&format!("over {MAX_BYTES} bytes"))
        ]
    );
    File::options()
        .write(true)
        .open(&path)?
        .set_len(MAX_BYTES)?;
    let chosen = choose(Some(path.clone()), None, read);
    assert!(chosen.text.is_some());
    assert_eq!(chosen.lines, [path_line(&path)]);
    Ok(())
}

#[test]
fn invalid_utf8_is_replaced_as_in_the_variable() -> TestResult {
    let scratch = Scratch::new("lossy")?;
    let path = scratch.file("rules", b"exe=Caf\xE9.exe")?;
    assert_eq!(read(&path)?, "exe=Caf\u{FFFD}.exe");
    Ok(())
}
