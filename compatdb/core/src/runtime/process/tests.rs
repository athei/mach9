use super::*;

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn u16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn env_block(entries: &[&str]) -> Vec<u16> {
    let mut out = Vec::new();
    for e in entries {
        out.extend(e.encode_utf16());
        out.push(0);
    }
    out.push(0);
    out
}

#[test]
fn the_steam_app_id_is_a_decimal_value_in_the_block() {
    assert_eq!(
        steam_app(&env_block(&["A=1", "SteamAppId=220", "B=2"])),
        Some(220)
    );
    assert_eq!(
        steam_app(&env_block(&["SteamAppId=4294967295"])),
        Some(u32::MAX)
    );
    // Windows names have no case.
    for name in ["STEAMAPPID", "steamappid", "SteamAPPID"] {
        let entry = format!("{name}=220");
        assert_eq!(steam_app(&env_block(&[&entry])), Some(220), "{name}");
    }
}

#[test]
fn a_value_that_is_no_app_id_reads_as_none() {
    assert_eq!(steam_app(&env_block(&[])), None);
    assert_eq!(steam_app(&[]), None);
    assert_eq!(steam_app(&env_block(&["PATH=C:\\windows"])), None);
    // Steam's own processes have the variable empty.
    for value in [
        "",
        "0",
        "000",
        "abc",
        "220a",
        " 220",
        "220 ",
        "+220",
        "-220",
        "2.5",
        "4294967296",
    ] {
        let entry = format!("{STEAM_APP_VAR}={value}");
        assert_eq!(steam_app(&env_block(&[&entry])), None, "{value:?}");
    }
    // Another variable that holds the name is not this one.
    let others = ["SteamGameId=220", "XSteamAppId=220", "SteamAppId2=220"];
    assert_eq!(steam_app(&env_block(&others)), None);
    // A drive entry is no variable.
    assert_eq!(steam_app(&env_block(&["=SteamAppId=220"])), None);
}

#[test]
fn append_cmdline_is_idempotent() -> TestResult {
    let base = u16s("cmd.exe");
    let appends = vec!["--in-process-gpu".to_string()];
    let once = append_cmdline(&base, &appends).ok_or("should append")?;
    assert_eq!(String::from_utf16_lossy(&once), "cmd.exe --in-process-gpu");
    assert!(append_cmdline(&once, &appends).is_none());
    Ok(())
}

#[test]
fn append_cmdline_keeps_the_existing_arguments() -> TestResult {
    // The launcher's `arguments` knob and any switches the parent passed
    // are in the command line already; appending must never drop them.
    let base = u16s("game.exe -console --width 1280");
    let out = append_cmdline(&base, &["--in-process-gpu".to_string()]).ok_or("appended")?;
    assert_eq!(
        String::from_utf16_lossy(&out),
        "game.exe -console --width 1280 --in-process-gpu"
    );
    Ok(())
}

#[test]
fn append_cmdline_applies_each_new_text_once() -> TestResult {
    let base = u16s("app.exe --foo");
    let appends = vec!["--foo".to_string(), "--bar".to_string()];
    let out = append_cmdline(&base, &appends).ok_or("should append --bar")?;
    assert_eq!(String::from_utf16_lossy(&out), "app.exe --foo --bar");
    // Each text says whether it went in, for the log.
    let (out, appended) = append_cmdline_each(&base, &appends);
    assert_eq!(appended, [false, true]);
    assert!(out.is_some());
    let (out, appended) = append_cmdline_each(&base, &["--foo".to_string()]);
    assert_eq!((out, appended), (None, vec![false]));
    Ok(())
}

#[test]
fn merge_env_replaces_appends_and_removes() -> TestResult {
    let block = env_block(&["PATH=C:\\windows", "FOO=old", "=C:=C:\\cur"]);
    let out = merge_env(
        &block,
        &[
            EnvEntry::literal("foo", "new"),   // case-insensitive replace
            EnvEntry::literal("BAR", "added"), // append
            EnvEntry::literal("PATH", ""),     // remove
        ],
    )
    .ok_or("changed")?;
    let segs = split_env(&out);
    let text: Vec<String> = segs.iter().map(|s| String::from_utf16_lossy(s)).collect();
    assert_eq!(text, vec!["foo=new", "=C:=C:\\cur", "BAR=added"]);
    // Terminated with a double NUL.
    assert_eq!(out.last(), Some(&0));
    Ok(())
}

#[test]
fn merge_env_reports_no_change() {
    let block = env_block(&["FOO=bar"]);
    assert!(merge_env(&block, &[EnvEntry::literal("FOO", "bar")]).is_none());
    assert!(merge_env(&block, &[EnvEntry::literal("MISSING", "")]).is_none());
}

#[test]
fn basename_handles_both_separators_and_none() {
    assert_eq!(basename_of(&u16s("C:\\a\\b\\Foo.exe")), "Foo.exe");
    assert_eq!(basename_of(&u16s("/a/b/Bar.exe")), "Bar.exe");
    assert_eq!(basename_of(&u16s("Baz.exe")), "Baz.exe");
}

#[test]
fn a_64_bit_process_loads_aarch64_builtins() {
    assert_eq!(Arch::X86_64.pe_dir(), "aarch64-windows");
    assert_eq!(Arch::Aarch64.pe_dir(), "aarch64-windows");
    assert_eq!(Arch::I386.pe_dir(), "i386-windows");
}

#[test]
fn the_process_arch_comes_from_wow64_and_the_main_image() {
    const AMD64: u16 = 0x8664;
    for machine in [None, Some(AMD64), Some(IMAGE_FILE_MACHINE_ARM64)] {
        assert_eq!(Arch::of_process(true, machine), Arch::I386);
    }
    assert_eq!(
        Arch::of_process(false, Some(IMAGE_FILE_MACHINE_ARM64)),
        Arch::Aarch64
    );
    // An x64 program under emulation, an ARM64EC one and an unreadable
    // header all stay x86_64.
    for machine in [None, Some(AMD64), Some(0)] {
        assert_eq!(Arch::of_process(false, machine), Arch::X86_64);
    }
}
