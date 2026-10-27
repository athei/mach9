use std::process::Command;

use compatdb_core::{DebugChannel, Exe, Rule};

use super::*;

/// Set for the copy of this test binary that runs
/// `apply_in_an_inherited_environment`, naming what it applies: `empty` for a
/// table without rules, `rules` for one with a rule.
const APPLY_ENV: &str = "GAMELAUNCHER_TEST_APPLY";

/// What `apply_in_an_inherited_environment` prints, followed by the value
/// `apply` left [`COMPATDB_ENV`] at, or `<unset>`.
const APPLY_SAYS: &str = "apply left the rules at: ";

/// The table of the one rule `apply_in_an_inherited_environment` applies.
fn one_rule() -> Table {
    let mut table = Table::new();
    table.push(Rule::new(Exe::new("Old.exe").unwrap()));
    table
}

/// What `apply_in_an_inherited_environment` prints, followed by the
/// variables of [`inherited_conflicts`] that `apply` left in place.
const APPLY_KEPT: &str = "apply kept: ";

/// Run [`apply`] in a copy of this test binary that inherited
/// [`COMPATDB_ENV`] set to `inherited` and every variable of
/// [`inherited_conflicts`], and return what it left [`COMPATDB_ENV`] at and
/// which of the others it kept.
///
/// A new process rather than `set_var` here, which is unsound while the
/// harness runs other tests on other threads, and rather than a fork, which
/// can abort in the child for the same reason.
fn apply_with_an_inherited_table(table: &str) -> (String, String) {
    let output = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "wine_env::tests::apply_in_an_inherited_environment",
        ])
        .args(["--ignored", "--nocapture", "--test-threads=1"])
        .env(APPLY_ENV, table)
        .env(COMPATDB_ENV, "inherited")
        .envs(inherited_conflicts().iter().map(|name| (name, "inherited")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let said = |what: &str| {
        stdout
            .lines()
            .find_map(|line| line.split_once(what).map(|(_, v)| v.to_owned()))
            .unwrap_or_else(|| panic!("the child said nothing: {stdout}"))
    };
    (said(APPLY_SAYS), said(APPLY_KEPT))
}

#[test]
fn apply_removes_an_inherited_table_when_the_config_has_no_rules() {
    assert_eq!(apply_with_an_inherited_table("empty").0, "<unset>");
    assert_eq!(
        apply_with_an_inherited_table("rules").0,
        one_rule().to_string()
    );
}

#[test]
fn apply_removes_the_inherited_variables_that_conflict_with_the_rules() {
    assert_eq!(apply_with_an_inherited_table("rules").1, "");
}

/// The child's side of `apply_with_an_inherited_table`; without `APPLY_ENV`
/// it does nothing.
#[test]
#[ignore = "the child of the apply_removes_ tests"]
fn apply_in_an_inherited_environment() {
    let Some(table) = env::var_os(APPLY_ENV) else {
        return;
    };
    let rules = if table == "rules" {
        one_rule()
    } else {
        Table::new()
    };
    apply(Path::new("/tmp/prefix"), &rules);
    let value = env::var(COMPATDB_ENV).unwrap_or_else(|_| String::from("<unset>"));
    let kept: Vec<&str> = inherited_conflicts()
        .iter()
        .copied()
        .filter(|name| env::var_os(name).is_some())
        .collect();
    println!("\n{APPLY_SAYS}{value}\n{APPLY_KEPT}{}", kept.join(","));
}

fn channel(text: &str) -> DebugChannel {
    DebugChannel::new(text).unwrap()
}

/// The rules' entries come after the launcher's channels, so they win for
/// the same channel.
#[test]
fn the_rules_channels_come_after_the_launchers() {
    let mut table = Table::new();
    table.push(
        Rule::new(Exe::Any)
            .with_debug_channel(channel("+seh"))
            .with_debug_channel(channel("+dbghelp_pdb")),
    );
    table.push(Rule::new(Exe::new("Tool.exe").unwrap()).with_debug_channel(channel("+relay")));
    let mut expected = vec![String::from(WINEDEBUG_CHANNELS)];
    expected.extend(table.winedebug());
    assert_eq!(winedebug(&table), expected.join(","));
    assert!(
        winedebug(&table).starts_with(&format!("{WINEDEBUG_CHANNELS},")),
        "{}",
        winedebug(&table)
    );
}

#[test]
fn no_channels_leave_the_launchers_alone() {
    assert_eq!(winedebug(&Table::new()), WINEDEBUG_CHANNELS);
    assert_eq!(winedebug(&one_rule()), WINEDEBUG_CHANNELS);
}

fn launcher_vars(rules: &Table) -> [(&'static str, Option<String>); 3] {
    launcher_env(Path::new("/tmp/prefix"), rules)
        .map(|(name, value)| (name, value.map(|value| value.into_string().unwrap())))
}

/// No rules means no table: the variable is removed, an inherited one
/// included, rather than set empty.
#[test]
fn the_launcher_sets_its_own_variables_and_no_empty_table() {
    assert_eq!(
        launcher_vars(&Table::new()),
        [
            ("WINEPREFIX", Some(String::from("/tmp/prefix"))),
            ("WINEMSYNC", Some(String::from("1"))),
            (COMPATDB_ENV, None),
        ]
    );
}

/// The table writes the value itself.
#[test]
fn a_config_with_rules_exports_them() {
    let table = one_rule();
    assert_eq!(
        launcher_vars(&table)[2],
        (COMPATDB_ENV, Some(table.to_string()))
    );
}
