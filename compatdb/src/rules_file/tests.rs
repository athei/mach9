use std::{env, path::PathBuf, process::Command, sync::mpsc, thread, time::Duration};

use compatdb_core::{Dxgi, Origin, Scalar, Source, VersionInfo};

use super::*;

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

const RULE: &str = "exe=Game.exe;dpi_aware=false";

/// A rule for the value of [`ENV_VAR`] that no test puts in a file.
const INLINE: &str = "exe=Game.exe;gcmouse=false";

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

fn not_read(reason: &str) -> String {
    format!("  warning: WINE_COMPATDB_FILE not read: {reason}; the file adds no rules")
}

/// A reader for the tests of [`gather`] alone.
fn reads_rule() -> impl FnOnce(&str) -> io::Result<String> {
    |_| Ok(String::from(RULE))
}

/// The label of the rule that gives a `Game.exe` the setting `scalar`.
fn set_by(table: &Table, scalar: Scalar) -> Option<String> {
    table
        .resolve("Game.exe", &VersionInfo::default(), None)
        .scalar(scalar)
        .set_by
        .as_ref()
        .map(Origin::label)
}

#[test]
fn each_variable_is_read_by_its_own_name() {
    let variables = Variables::read_with(|name| Some(name.to_owned()));
    assert_eq!(variables.file.as_deref(), Some("WINE_COMPATDB_FILE"));
    assert_eq!(variables.inline.as_deref(), Some("WINE_COMPATDB"));
}

#[test]
fn without_either_variable_there_are_no_rules_and_nothing_to_log() {
    let gathered = gather(variables(None, None), reads_rule());
    assert_eq!(gathered.inline, None);
    assert_eq!(gathered.file, None);
    assert_eq!(gathered.lines, NO_LINES);
}

#[test]
fn the_variable_alone_is_the_source_and_logs_nothing() {
    let gathered = gather(variables(None, Some(INLINE)), reads_rule());
    assert_eq!(gathered.inline.as_deref(), Some(INLINE));
    assert_eq!(gathered.file, None);
    assert_eq!(gathered.lines, NO_LINES);
}

#[test]
fn an_empty_variable_counts_as_unset() {
    // An empty path reads nothing, so the other variable is the only source.
    let gathered = gather(variables(Some(""), Some(INLINE)), reads_rule());
    assert_eq!(gathered.inline.as_deref(), Some(INLINE));
    assert_eq!(gathered.file, None);
    assert_eq!(gathered.lines, NO_LINES);
    let gathered = gather(variables(Some("/rules"), Some("")), reads_rule());
    assert_eq!(gathered.inline, None);
    assert_eq!(gathered.file.as_deref(), Some(RULE));
    assert_eq!(gathered.lines, [path_line("/rules")]);
    let gathered = gather(variables(Some(""), Some("")), reads_rule());
    assert_eq!(gathered.inline, None);
    assert_eq!(gathered.file, None);
    assert_eq!(gathered.lines, NO_LINES);
}

#[test]
fn a_file_alone_is_the_source_and_its_path_is_logged() -> TestResult {
    let scratch = Scratch::new("file_alone")?;
    let path = scratch.file("rules", RULE.as_bytes())?;
    let gathered = gather(variables(Some(&path), None), read);
    assert_eq!(gathered.inline, None);
    assert_eq!(gathered.file.as_deref(), Some(RULE));
    assert_eq!(gathered.lines, [path_line(&path)]);
    Ok(())
}

#[test]
fn with_both_set_the_rules_of_each_apply_and_nothing_warns() -> TestResult {
    let scratch = Scratch::new("both_set")?;
    let path = scratch.file("rules", RULE.as_bytes())?;
    let (table, lines) = gather(variables(Some(&path), Some(INLINE)), read).into_table();
    assert_eq!(lines, [path_line(&path)]);
    // Each is line 1 of its text, and each keeps its own label.
    assert_eq!(
        set_by(&table, Scalar::Gcmouse).as_deref(),
        Some("WINE_COMPATDB line 1")
    );
    assert_eq!(
        set_by(&table, Scalar::DpiAware).as_deref(),
        Some("WINE_COMPATDB_FILE line 1")
    );
    Ok(())
}

#[test]
fn the_table_is_the_built_in_rules_then_the_variable_then_the_file() {
    let (table, _) = gather(
        variables(Some("/rules"), Some("exe=a.exe\nexe=b.exe")),
        |_| Ok(String::from("exe=c.exe")),
    )
    .into_table();
    let builtin = Table::builtin().rules().len();
    assert_eq!(
        table.sources().get(builtin..),
        Some(&[Source::Line(1), Source::Line(2), Source::FileLine(1)][..])
    );
    assert_eq!(table.rules().get(..builtin), Some(Table::builtin().rules()));
}

#[test]
fn a_file_rule_wins_over_a_variable_rule_of_equal_specificity() {
    let (table, _) = gather(
        variables(Some("/rules"), Some("exe=Game.exe;dpi_aware=true")),
        reads_rule(),
    )
    .into_table();
    let game = table.resolve("Game.exe", &VersionInfo::default(), None);
    assert_eq!(game.dpi_aware(), Some(false));
    assert_eq!(
        set_by(&table, Scalar::DpiAware).as_deref(),
        Some("WINE_COMPATDB_FILE line 1")
    );
}

#[test]
fn a_star_rule_of_the_file_loses_to_a_variable_rule_for_the_exe() {
    let (table, _) = gather(
        variables(Some("/rules"), Some("exe=Game.exe;dpi_aware=true")),
        |_| Ok(String::from("exe=*;dpi_aware=false")),
    )
    .into_table();
    let game = table.resolve("Game.exe", &VersionInfo::default(), None);
    assert_eq!(game.dpi_aware(), Some(true));
    assert_eq!(
        set_by(&table, Scalar::DpiAware).as_deref(),
        Some("WINE_COMPATDB line 1")
    );
}

#[test]
fn a_rules_file_keeps_the_settings_the_variable_carries() {
    // A launcher sends its config's rules in the variable, and a file set in
    // that config's `[env]` adds to them.
    let (table, lines) = gather(
        variables(Some("/rules"), Some("exe=*;metal_hud=true")),
        reads_rule(),
    )
    .into_table();
    assert_eq!(lines, [path_line("/rules")]);
    let game = table.resolve("Game.exe", &VersionInfo::default(), None);
    assert_eq!(game.metal_hud(), Some(true));
    assert_eq!(
        set_by(&table, Scalar::MetalHud).as_deref(),
        Some("WINE_COMPATDB line 1")
    );
}

#[test]
fn a_bad_key_warns_with_the_label_of_its_source() -> TestResult {
    let scratch = Scratch::new("bad_key")?;
    let path = scratch.file("rules", b"exe=Game.exe;bogus=1;dpi_aware=false")?;
    let (table, lines) = gather(
        variables(Some(&path), Some("exe=Game.exe;odd=2;gcmouse=false")),
        read,
    )
    .into_table();
    assert_eq!(
        lines,
        [
            path_line(&path),
            String::from("  warning: WINE_COMPATDB line 1: ignoring unknown key \"odd\""),
            String::from("  warning: WINE_COMPATDB_FILE line 1: ignoring unknown key \"bogus\""),
        ]
    );
    assert_eq!(
        set_by(&table, Scalar::DpiAware).as_deref(),
        Some("WINE_COMPATDB_FILE line 1")
    );
    assert_eq!(
        set_by(&table, Scalar::Gcmouse).as_deref(),
        Some("WINE_COMPATDB line 1")
    );
    // The built-in rules stay in front of the caller's.
    let game = table.resolve("Game.exe", &VersionInfo::default(), None);
    assert_eq!(game.dxgi(), Some(Dxgi::Dxmt));
    Ok(())
}

#[test]
fn no_text_at_all_is_the_built_in_table_with_the_lines_of_the_file() {
    let (table, lines) = gather(variables(Some("rules"), None), read).into_table();
    assert_eq!(table, Table::builtin());
    assert_eq!(
        lines,
        [path_line("rules"), not_read("not an absolute path")]
    );
}

#[test]
fn an_empty_file_adds_no_rules_and_no_warning() -> TestResult {
    let scratch = Scratch::new("empty_file")?;
    let path = scratch.file("rules", b"")?;
    let gathered = gather(variables(Some(&path), Some(INLINE)), read);
    assert_eq!(gathered.file.as_deref(), Some(""));
    let (table, lines) = gathered.into_table();
    assert_eq!(lines, [path_line(&path)]);
    assert_eq!(
        set_by(&table, Scalar::Gcmouse).as_deref(),
        Some("WINE_COMPATDB line 1")
    );
    Ok(())
}

/// An unusable file named `path` warns with `reason` and adds no rules, and
/// the rules of the variable still apply.
fn keeps_the_variable(path: &str, reason: &str) {
    let gathered = gather(variables(Some(path), Some(INLINE)), read);
    assert_eq!(gathered.file, None, "{path}");
    let (table, lines) = gathered.into_table();
    assert_eq!(lines, [path_line(path), not_read(reason)]);
    assert_eq!(
        set_by(&table, Scalar::Gcmouse).as_deref(),
        Some("WINE_COMPATDB line 1"),
        "{path}"
    );
    let builtin = Table::builtin().rules().len();
    assert_eq!(table.rules().len(), builtin + 1, "{path}");
}

#[test]
fn a_missing_file_keeps_the_rules_of_the_variable() -> TestResult {
    let scratch = Scratch::new("missing_file")?;
    keeps_the_variable(
        &scratch.path("rules"),
        "No such file or directory (os error 2)",
    );
    Ok(())
}

#[test]
fn a_relative_path_is_refused_and_keeps_the_rules_of_the_variable() {
    keeps_the_variable("rules", "not an absolute path");
}

#[test]
fn a_directory_is_refused_and_keeps_the_rules_of_the_variable() -> TestResult {
    let scratch = Scratch::new("directory")?;
    keeps_the_variable(&scratch.path(""), "not a regular file");
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
    thread::spawn(move || sender.send(gather(fifo, read)));
    let gathered = receiver.recv_timeout(Duration::from_secs(5))?;
    assert_eq!(gathered.file, None);
    assert_eq!(
        gathered.lines,
        [path_line(&path), not_read("not a regular file")]
    );
    keeps_the_variable(&path, "not a regular file");
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
    keeps_the_variable(&path, &format!("over {MAX_BYTES} bytes"));
    File::options()
        .write(true)
        .open(&path)?
        .set_len(MAX_BYTES)?;
    let gathered = gather(variables(Some(&path), None), read);
    assert!(gathered.file.is_some());
    assert_eq!(gathered.lines, [path_line(&path)]);
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
