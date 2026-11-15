use std::os::unix::fs::PermissionsExt as _;

use super::*;

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A directory of its own under the system's temporary directory, removed
/// first, for one test.
fn scratch(name: &str) -> Result<PathBuf, std::io::Error> {
    let dir = std::env::temp_dir().join(format!("compatdb-capture-{}-{name}", std::process::id()));
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    std::fs::create_dir(&dir)?;
    Ok(dir)
}

#[test]
fn the_captures_folder_is_next_to_the_executable() -> TestResult {
    assert_eq!(
        captures_dir(Path::new("/Games/Game/bin/Game.exe"))?,
        Path::new("/Games/Game/bin/captures")
    );
    assert!(captures_dir(Path::new("/")).is_err());
    Ok(())
}

#[test]
fn the_folder_is_made_or_taken_when_it_exists() -> TestResult {
    let root = scratch("make")?;
    let dir = root.join(CAPTURES_DIR);
    make_writable_dir(&dir)?;
    assert!(dir.is_dir());
    make_writable_dir(&dir)?;
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

#[test]
fn a_folder_that_cannot_be_made_or_written_is_an_error() -> TestResult {
    let root = scratch("readonly")?;
    let file = root.join("file");
    std::fs::write(&file, "")?;
    // A file where the folder would be.
    assert!(make_writable_dir(&file).is_err_and(|e| e.starts_with("cannot make")));
    // A folder without write permission.
    let dir = root.join(CAPTURES_DIR);
    std::fs::create_dir(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500))?;
    assert!(make_writable_dir(&dir).is_err_and(|e| e.starts_with("cannot write into")));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    // A folder whose parent is missing.
    assert!(make_writable_dir(&root.join("missing").join(CAPTURES_DIR)).is_err());
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

#[test]
fn an_inherited_program_name_says_the_layer_records_nothing_here() {
    assert_eq!(
        inherited_note(&[PROCESS_NAME_VAR], |_| None, "the capture layer"),
        ""
    );
    assert_eq!(
        inherited_note(
            &[PROCESS_NAME_VAR],
            |_| Some(String::from("Game")),
            "the capture layer"
        ),
        "; GFXRECON_CAPTURE_PROCESS_NAME=Game inherited: the capture layer loads and records \
         nothing here"
    );
}

#[test]
fn inherited_metal_variables_say_the_layer_records_nothing_here() {
    let only_mtl = |name: &str| (name == "MTL_CAPTURE_ENABLED").then(|| String::from("1"));
    assert_eq!(
        inherited_note(&METAL_INHERITED_VARS, only_mtl, "Metal's capture layer"),
        "; MTL_CAPTURE_ENABLED=1 inherited: Metal's capture layer loads and records nothing here"
    );
    assert_eq!(
        inherited_note(
            &METAL_INHERITED_VARS,
            |_| Some(String::from("1")),
            "Metal's capture layer"
        ),
        "; MTL_CAPTURE_ENABLED=1, MESA_KK_GPU_CAPTURE=1 inherited: Metal's capture layer loads \
         and records nothing here"
    );
}

#[test]
fn the_program_name_is_libcs() {
    assert!(program_name().is_some_and(|name| !name.is_empty()));
}

#[test]
fn a_rule_for_every_process_or_an_app_id_is_ignored_with_a_line() -> TestResult {
    let table = compatdb_core::Table::parse_strict(
        "exe=*;vulkan_capture=true\nexe=steam:220;vulkan_capture=true\nexe=Game.exe;vulkan_capture=false",
    )?;
    let game = table.resolve(
        "Game.exe",
        &compatdb_core::VersionInfo::default(),
        Some(220),
    );
    assert_eq!(game.vulkan_capture(), Some(false));
    let lines: Vec<String> = game
        .needs_program_ignored
        .iter()
        .map(|(scalar, by)| ignored_line(*scalar, by))
        .collect();
    assert_eq!(
        lines,
        [
            "  vulkan_capture, WINE_COMPATDB line 1: ignored, only a rule that names one program \
             may set it",
            "  vulkan_capture, WINE_COMPATDB line 2: ignored, only a rule that names one program \
             may set it",
        ]
    );
    Ok(())
}
