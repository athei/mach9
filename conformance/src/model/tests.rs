use super::*;

fn unit<'c>(config: &'c Config, exe: &str, name: &str) -> &'c Unit {
    config
        .exes
        .iter()
        .find(|e| e.name == exe)
        .and_then(|e| e.units.iter().find(|u| u.name == name))
        .expect("the unit is configured")
}

fn wrapping(unit: &Unit) -> &Wrapping {
    match &unit.selection {
        Selection::Wrapped(wrapping) => wrapping,
        Selection::Whole => panic!("{} runs as a whole", unit.name),
    }
}

/// The value of `NAME="..."` in a shell file.
fn shell_value<'t>(text: &'t str, name: &str) -> &'t str {
    let prefix = format!("{name}=\"");
    text.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or_else(|| panic!("profile.sh sets no {name}"))
}

/// The exes and their PE directories are what the bundle builds:
/// `GRAPHICS_TESTS` of `bin/lib/profile.sh` without d3d9, each restricted
/// to its `:<pe-dir>` or else built for every `TEST_PE_DIRS`.
#[test]
fn the_builtin_config_matches_the_graphics_tests_of_profile_sh() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../bin/lib/profile.sh");
    let profile = std::fs::read_to_string(path).unwrap();
    let all_dirs: Vec<&str> = shell_value(&profile, "TEST_PE_DIRS")
        .split_whitespace()
        .collect();
    let built: Vec<(&str, Vec<&str>)> = shell_value(&profile, "GRAPHICS_TESTS")
        .split_whitespace()
        .map(|entry| match entry.split_once(':') {
            Some((name, dir)) => (name, vec![dir]),
            None => (entry, all_dirs.clone()),
        })
        .filter(|(name, _)| *name != "d3d9")
        .collect();
    let config = Config::builtin().unwrap();
    let configured: Vec<(&str, Vec<&str>)> = config
        .exes
        .iter()
        .map(|exe| {
            (
                exe.name.as_str(),
                exe.pe_dirs.iter().map(PeDir::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(configured, built);
}

#[test]
fn defaults_fill_in_the_file_and_the_key_position() {
    let config = Config::builtin().unwrap();
    let d3d11 = unit(&config, "d3d11", "d3d11");
    assert_eq!(d3d11.file, "dlls/d3d11/tests/d3d11.c");
    let helpers = &wrapping(d3d11).dispatch;
    assert_eq!((helpers[0].args, helpers[0].key), (1, 1));
    let range = helpers
        .iter()
        .find(|h| h.name == "queue_for_each_feature_level_in_range")
        .unwrap();
    assert_eq!((range.args, range.key), (3, 3));
    assert_eq!(
        unit(&config, "d3d10", "reflection").file,
        "dlls/d3dcompiler_43/tests/reflection.c"
    );
    let prefixes: Vec<&str> = wrapping(unit(&config, "dxgi", "dxgi"))
        .dispatch
        .iter()
        .map(|h| h.prefix.as_str())
        .collect();
    assert_eq!(prefixes, ["", "d3d10:", "d3d12:"]);
}

/// A config with one exe and one unit whose body is `unit`.
fn one_unit(unit: &str) -> String {
    format!("[[exe]]\nname = \"x\"\ndir = \"dlls/x/tests\"\n\n[[exe.unit]]\nname = \"x\"\n{unit}\n")
}

#[test]
fn mistakes_in_the_config_are_errors() {
    for (unit, expected) in [
        ("colour = 1", "unit x/x: unknown key `colour`"),
        (
            "dispatch = [{ name = \"f\", args = 2, key = 3 }]",
            "`key` of f is 3, past its 2 arguments",
        ),
        (
            "dispatch = [{ name = \"f\", key = 0 }]",
            "`key` of f must be a positive integer",
        ),
        (
            "dispatch = [{ name = \"f\", prefix = \"a,b\" }]",
            "`prefix` of f must be a string",
        ),
        (
            "dispatch = [{ name = \"f\", argc = 1 }]",
            "unknown dispatch key `argc`",
        ),
        (
            "whole = true\nsetup = [\"f\"]",
            "a whole unit takes no `dispatch`, `setup` or `tests`",
        ),
        (
            "whole = true\ntests = [\"f\"]",
            "a whole unit takes no `dispatch`, `setup` or `tests`",
        ),
        ("setup = [\"f\", \"f\"]", "duplicate setup of unit x/x f"),
        (
            "setup = [\"not a name\"]",
            "`setup` holds a value that is not a valid name",
        ),
        ("tests = [\"f\", \"f\"]", "duplicate tests of unit x/x f"),
        (
            "dispatch = [{ name = \"f\" }]\nsetup = [\"f\"]",
            "f is both setup and a dispatch helper",
        ),
        (
            "setup = [\"f\"]\ntests = [\"f\"]",
            "f is in `tests` and also setup or a dispatch helper",
        ),
        (
            "dispatch = [{ name = \"f\" }]\ntests = [\"f\"]",
            "f is in `tests` and also setup or a dispatch helper",
        ),
    ] {
        let error = Config::parse(&one_unit(unit)).err().unwrap_or_default();
        assert!(error.contains(expected), "{unit}: {error}");
    }
}

#[test]
fn pe_dirs_take_only_the_bundle_directories() {
    let with = |pe_dirs: &str| {
        Config::parse(&format!(
            "[[exe]]\nname = \"x\"\ndir = \"d\"\npe_dirs = {pe_dirs}\n\n[[exe.unit]]\nname = \"x\"\n"
        ))
    };
    let config = with("[\"x86_64-windows\"]").unwrap();
    let dirs: Vec<&str> = config.exes[0].pe_dirs.iter().map(PeDir::as_str).collect();
    assert_eq!(dirs, ["x86_64-windows"]);
    for (pe_dirs, expected) in [
        ("[\"aarch64-windows\"]", "holds \"aarch64-windows\""),
        ("[]", "`pe_dirs` is empty"),
        (
            "[\"i386-windows\", \"i386-windows\"]",
            "duplicate pe_dirs of exe x i386-windows",
        ),
        ("\"i386-windows\"", "`pe_dirs` must be an array"),
    ] {
        let error = with(pe_dirs).err().unwrap_or_default();
        assert!(error.contains(expected), "{pe_dirs}: {error}");
    }
}

#[test]
fn duplicate_units_and_exes_are_errors() {
    let unit = one_unit("");
    let twice = format!("{unit}\n[[exe.unit]]\nname = \"x\"\n");
    let error = Config::parse(&twice).err().unwrap_or_default();
    assert!(error.contains("duplicate unit of exe x x"), "{error}");

    let error = Config::parse(&format!("{unit}\n{unit}"))
        .err()
        .unwrap_or_default();
    assert!(error.contains("duplicate executable x"), "{error}");

    let error = Config::parse("[[exe]]\nname = \"a b\"\ndir = \"d\"\n")
        .err()
        .unwrap_or_default();
    assert!(error.contains("a name holds only"), "{error}");
}
