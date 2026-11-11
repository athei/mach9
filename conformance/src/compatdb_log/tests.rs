use super::*;

const PASS: &str = include_str!("../../tests/fixtures/output/pass.txt");
const CHILD_FAILURES: &str = include_str!("../../tests/fixtures/output/child-failures.txt");
const WHOLE: &str = include_str!("../../tests/fixtures/output/whole.txt");

#[test]
fn lines_are_the_compatdb_lines_without_prefix() {
    let windows = PASS.replace('\n', "\r\n");
    let lines = lines(&windows);
    assert_eq!(lines.len(), 17);
    assert_eq!(lines[0], "d3d11_test.exe [x86_64 (arm64ec)]");
    assert_eq!(lines[10], "  opengl = egl, built-in");
}

#[test]
fn each_process_has_its_trees_and_opengl() {
    let processes = Processes::parse(PASS);
    assert_eq!(processes.dxgi("d3d11_test.exe"), Ok(Dxgi::Dxmt));
    assert_eq!(processes.d3d9("D3D11_TEST.EXE"), Ok(D3d9::Mtld3d));
    assert_eq!(processes.opengl("d3d11_test.exe"), Ok(OpenGl::Egl));
    assert_eq!(processes.dxgi("services.exe"), Ok(Dxgi::Dxmt));
    assert_eq!(processes.opengl("services.exe"), Err(ImplError::NoLine));
    assert_eq!(processes.dxgi("explorer.exe"), Err(ImplError::NoProcess));

    let whole = Processes::parse(WHOLE);
    assert_eq!(whole.d3d9("ddraw_test.exe"), Ok(D3d9::Mtld3d));
}

#[test]
fn a_parent_and_its_child_must_agree() {
    let processes = Processes::parse(CHILD_FAILURES);
    assert_eq!(processes.dxgi("vulkan-1_test.exe"), Ok(Dxgi::Dxmt));
    let text = CHILD_FAILURES.replacen(
        "compatdb:   opengl = egl, built-in",
        "compatdb:   opengl = cgl, WINE_COMPATDB line 1",
        1,
    );
    assert_eq!(
        Processes::parse(&text).opengl("vulkan-1_test.exe"),
        Err(ImplError::Differ(vec![
            String::from("opengl = cgl, WINE_COMPATDB line 1"),
            String::from("opengl = egl, built-in"),
        ]))
    );
}

/// Two interleaved blocks can put two lines of one setting under one
/// header; when they disagree the processes differ.
#[test]
fn two_lines_of_one_setting_in_a_block_must_agree() {
    let text = "compatdb: a.exe [i386]\ncompatdb: b.exe [i386]\ncompatdb:   opengl = egl, \
                built-in\ncompatdb:   opengl = cgl, WINE_COMPATDB line 1\n";
    assert_eq!(
        Processes::parse(text).opengl("b.exe"),
        Err(ImplError::Differ(vec![
            String::from("opengl = egl, built-in"),
            String::from("opengl = cgl, WINE_COMPATDB line 1"),
        ]))
    );
    let same = text.replace("cgl, WINE_COMPATDB line 1", "egl, built-in");
    assert_eq!(Processes::parse(&same).opengl("b.exe"), Ok(OpenGl::Egl));
}

#[test]
fn a_tree_that_was_not_prepended_is_not_applied() {
    for line in [
        "dxgi = dxmt, built-in: /Users/me/dist/wine/lib/wine/dxgi/dxmt/x86_64-windows missing, \
         using the default dir",
        "dxgi = dxmt, built-in: ntdll lacks prepend_dll_path",
        "dxgi = wined3d, unset (upstream: wined3d): cannot locate lib/wine",
    ] {
        let text = format!("compatdb: d3d11_test.exe [i386]\ncompatdb:   {line}\n");
        assert_eq!(
            Processes::parse(&text).dxgi("d3d11_test.exe"),
            Err(ImplError::NotApplied(line.to_owned()))
        );
    }
    let text = "compatdb: d3d11_test.exe [i386]\ncompatdb:   dxgi = vulkan (from /x), built-in\n";
    assert_eq!(
        Processes::parse(text).dxgi("d3d11_test.exe"),
        Err(ImplError::Unknown(String::from(
            "dxgi = vulkan (from /x), built-in"
        )))
    );
}

#[test]
fn an_unset_opengl_is_apple_s_and_a_missing_hook_is_not_applied() {
    let unset = "compatdb: a.exe [aarch64] company=\"Valve\"\ncompatdb:   opengl = unset \
                 (upstream: Apple's OpenGL), reset by WINE_COMPATDB line 2\n";
    assert_eq!(Processes::parse(unset).opengl("a.exe"), Ok(OpenGl::Cgl));
    let line = "opengl = egl, built-in for exe=a.exe: ntdll lacks set_compat_opengl, ignored";
    let text = format!("compatdb: a.exe [aarch64]\ncompatdb:   {line}\n");
    assert_eq!(
        Processes::parse(&text).opengl("a.exe"),
        Err(ImplError::NotApplied(line.to_owned()))
    );
}

#[test]
fn the_wined3d_renderer_is_unset_or_named() {
    assert_eq!(Processes::parse(PASS).wined3d("d3d11_test.exe"), Ok(None));
    for (line, renderer) in [
        ("wined3d = vulkan, WINE_COMPATDB line 1", Wined3d::Vulkan),
        ("wined3d = gl, built-in for exe=a.exe", Wined3d::Gl),
        (
            "wined3d = vulkan, from the inherited WINE_D3D_CONFIG",
            Wined3d::Vulkan,
        ),
    ] {
        let text = format!("compatdb: a.exe [i386]\ncompatdb:   {line}\n");
        assert_eq!(
            Processes::parse(&text).wined3d("a.exe"),
            Ok(Some(renderer)),
            "{line}"
        );
    }
    let text = "compatdb: a.exe [i386]\ncompatdb:   wined3d = metal, built-in\n";
    assert_eq!(
        Processes::parse(text).wined3d("a.exe"),
        Err(ImplError::Unknown(String::from(
            "wined3d = metal, built-in"
        )))
    );
}

/// A line without indent that is no header ends the block, so the lines
/// after it belong to no process.
#[test]
fn a_line_that_is_no_header_ends_the_block() {
    let text = "compatdb: a.exe [i386]\ncompatdb: required ntdll symbols missing, doing \
                nothing\ncompatdb:   dxgi = dxmt (from /x), built-in\n";
    assert_eq!(Processes::parse(text).dxgi("a.exe"), Err(ImplError::NoLine));
    assert_eq!(
        header_exe("My Game.exe [x86_64 (arm64ec)]"),
        Some("My Game.exe")
    );
    assert_eq!(header_exe("[i386]"), None);
    assert_eq!(header_exe("a.exe [arm]"), None);
}
