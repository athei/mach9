use std::{env, path::PathBuf, process::Command, sync::mpsc, thread, time::Duration};

use compatdb_core::{Dxgi, Origin, Scalar, VersionInfo};

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

fn variables(file: Option<&str>, inline: Option<&str>) -> Variables {
    Variables {
        file: file.map(str::to_owned),
        inline: inline.map(str::to_owned),
    }
}

fn path_line(path: &str) -> String {
    format!("  WINE_COMPATDB_FILE = {path}")
}

fn replaces_line() -> String {
    String::from("  warning: WINE_COMPATDB_FILE replaces WINE_COMPATDB, which is ignored")
}

fn not_read(reason: &str) -> String {
    format!("  warning: WINE_COMPATDB_FILE not read: {reason}; only the built-in rules apply")
}

/// A reader for the tests of [`choose`] alone.
fn reads_rule() -> impl FnOnce(&str) -> io::Result<String> {
    |_| Ok(String::from(RULE))
}

#[test]
fn each_variable_is_read_by_its_own_name() {
    let variables = Variables::read_with(|name| Some(name.to_owned()));
    assert_eq!(variables.file.as_deref(), Some("WINE_COMPATDB_FILE"));
    assert_eq!(variables.inline.as_deref(), Some("WINE_COMPATDB"));
}

#[test]
fn without_either_variable_there_are_no_rules_and_nothing_to_log() {
    let chosen = choose(variables(None, None), reads_rule());
    assert_eq!(chosen.text, None);
    assert_eq!(chosen.lines, NO_LINES);
}

#[test]
fn the_variable_alone_is_the_source_and_logs_nothing() {
    let chosen = choose(variables(None, Some("exe=a.exe")), reads_rule());
    assert_eq!(chosen.text.as_deref(), Some("exe=a.exe"));
    assert_eq!(chosen.lines, NO_LINES);
}

#[test]
fn an_empty_variable_counts_as_unset() {
    // An empty path reads nothing, so the other variable is the source.
    let chosen = choose(variables(Some(""), Some("exe=a.exe")), reads_rule());
    assert_eq!(chosen.text.as_deref(), Some("exe=a.exe"));
    assert_eq!(chosen.lines, NO_LINES);
    // An empty table does not count as set beside a file, so no warning.
    let chosen = choose(variables(Some("/rules"), Some("")), reads_rule());
    assert_eq!(chosen.text.as_deref(), Some(RULE));
    assert_eq!(chosen.lines, [path_line("/rules")]);
    let chosen = choose(variables(Some(""), Some("")), reads_rule());
    assert_eq!(chosen.text, None);
    assert_eq!(chosen.lines, NO_LINES);
}

#[test]
fn a_file_alone_is_the_source_and_its_path_is_logged() -> TestResult {
    let scratch = Scratch::new("file_alone")?;
    let path = scratch.file("rules", RULE.as_bytes())?;
    let chosen = choose(variables(Some(&path), None), read);
    assert_eq!(chosen.text.as_deref(), Some(RULE));
    assert_eq!(chosen.lines, [path_line(&path)]);
    Ok(())
}

#[test]
fn the_file_wins_over_the_variable_with_a_warning() -> TestResult {
    let scratch = Scratch::new("both_set")?;
    let path = scratch.file("rules", RULE.as_bytes())?;
    let chosen = choose(variables(Some(&path), Some("exe=a.exe")), read);
    assert_eq!(chosen.text.as_deref(), Some(RULE));
    assert_eq!(chosen.lines, [path_line(&path), replaces_line()]);
    Ok(())
}

#[test]
fn a_bad_key_in_the_file_warns_after_the_path_and_the_rule_keeps_its_label() -> TestResult {
    let scratch = Scratch::new("bad_key")?;
    let path = scratch.file("rules", b"exe=Game.exe;bogus=1;dpi_aware=false")?;
    let (table, lines) = choose(variables(Some(&path), None), read).into_table();
    assert_eq!(
        lines,
        [
            path_line(&path),
            String::from("  warning: WINE_COMPATDB line 1: ignoring unknown key \"bogus\""),
        ]
    );
    let game = table.resolve("Game.exe", &VersionInfo::default(), None);
    assert_eq!(
        game.scalar(Scalar::DpiAware)
            .set_by
            .as_ref()
            .map(Origin::label)
            .as_deref(),
        Some("WINE_COMPATDB line 1")
    );
    // The built-in rules stay in front of the file's.
    assert_eq!(game.dxgi(), Some(Dxgi::Dxmt));
    Ok(())
}

#[test]
fn no_chosen_text_is_the_built_in_table_with_the_lines_of_the_choice() {
    let chosen = choose(variables(Some("rules"), Some("exe=a.exe")), read);
    let (table, lines) = chosen.into_table();
    assert_eq!(table, Table::builtin());
    assert_eq!(
        lines,
        [
            path_line("rules"),
            replaces_line(),
            not_read("not an absolute path"),
        ]
    );
}

#[test]
fn an_empty_file_is_a_table_without_rules_not_a_fallback() -> TestResult {
    let scratch = Scratch::new("empty_file")?;
    let path = scratch.file("rules", b"")?;
    let chosen = choose(variables(Some(&path), Some("exe=a.exe")), read);
    assert_eq!(chosen.text.as_deref(), Some(""));
    Ok(())
}

#[test]
fn a_missing_file_gives_the_built_in_rules_and_no_fallback() -> TestResult {
    let scratch = Scratch::new("missing_file")?;
    let path = scratch.path("rules");
    let chosen = choose(variables(Some(&path), Some("exe=a.exe")), read);
    assert_eq!(chosen.text, None);
    assert_eq!(
        chosen.lines,
        [
            path_line(&path),
            replaces_line(),
            not_read("No such file or directory (os error 2)"),
        ]
    );
    Ok(())
}

#[test]
fn a_relative_path_is_refused() {
    let chosen = choose(variables(Some("rules"), None), read);
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
    let chosen = choose(variables(Some(&path), None), read);
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
    let fifo = variables(Some(&path), None);
    thread::spawn(move || sender.send(choose(fifo, read)));
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
    let chosen = choose(variables(Some(&path), None), read);
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
    let chosen = choose(variables(Some(&path), None), read);
    assert!(chosen.text.is_some());
    assert_eq!(chosen.lines, [path_line(&path)]);
    Ok(())
}

#[test]
fn a_file_that_grew_past_the_limit_is_refused_not_cut_off() -> TestResult {
    // What the read sees of a file that grew after its size was checked.
    let error = read_capped(io::repeat(b'\n'))
        .err()
        .ok_or("read past the limit")?;
    assert_eq!(error.to_string(), format!("over {MAX_BYTES} bytes"));
    let at_limit = read_capped(io::repeat(b'\n').take(MAX_BYTES))?;
    assert_eq!(u64::try_from(at_limit.len())?, MAX_BYTES);
    Ok(())
}

#[test]
fn invalid_utf8_is_replaced_as_in_the_variable() -> TestResult {
    let scratch = Scratch::new("lossy")?;
    let path = scratch.file("rules", b"exe=Caf\xE9.exe")?;
    assert_eq!(read(&path)?, "exe=Caf\u{FFFD}.exe");
    Ok(())
}
