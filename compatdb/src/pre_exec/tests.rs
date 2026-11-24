use super::*;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn version_constraints_and_reset_follow_normal_rule_precedence() -> TestResult {
    let table = Table::parse_strict(
        "exe=steam:440;display_name=Title\n\
         exe=GAME.EXE;display_name=By exe\n\
         exe=game.exe;company=;display_name=Empty pin\n\
         exe=game.exe;company=Acme;display_name=default",
    )?;
    let steam = Some(440);
    let version = || {
        Some(VersionInfo {
            company: "ACME Games".into(),
            ..VersionInfo::default()
        })
    };
    let name =
        |r: Option<compatdb_core::Resolution>| r.and_then(|r| r.display_name().map(str::to_owned));
    assert_eq!(name(resolve(&table, "game.exe", steam, version)), None);
    assert_eq!(
        name(resolve(&table, "game.exe", steam, || None)).as_deref(),
        Some("By exe")
    );
    assert_eq!(
        name(resolve(&table, "other.exe", steam, || None)).as_deref(),
        Some("Title")
    );
    assert!(resolve(&table, "other.exe", None, || None).is_none());
    Ok(())
}

fn query(image: &[u16], env: &[u16], out: &mut [u8]) -> usize {
    // SAFETY: all three slices are valid and the output is disjoint.
    unsafe {
        compatdb_resolve_display_name(
            image.as_ptr(),
            image.len(),
            env.as_ptr(),
            env.len(),
            out.as_mut_ptr().cast(),
            out.len(),
        )
    }
}

#[test]
fn ffi_reads_the_current_file_and_uses_counted_utf8_output() -> TestResult {
    // Set the Unix environment in a subprocess, not the parallel test runner.
    const CHILD: &str = "MACH9_COMPATDB_TEST_PRE_EXEC_CHILD";
    if let Some(path) = std::env::var_os(CHILD) {
        let image = wide(r"C:\Games\Game.exe");
        let env = wide("SteamAppId=440\0\0");
        let mut buf = [0xff; 6];
        std::fs::write(&path, "exe=steam:440;display_name=Game")?;
        // No Wine state is initialized here: direct lookup takes Unix inputs only.
        for image in [c"C:\\Games\\Game.exe", c"/games/Game.exe", c"Game.exe"] {
            // SAFETY: static C strings and a disjoint output buffer of the specified size.
            let len = unsafe {
                compatdb_resolve_initial_display_name(
                    image.as_ptr(),
                    c"/prefix".as_ptr(),
                    c"440".as_ptr(),
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                )
            };
            assert_eq!(len, 4);
            assert_eq!(buf, *b"Game\0\xff");
        }
        // An unset Unix AppID is not the child's Windows SteamAppId. Without
        // accessible PE metadata the direct resolver must skip fingerprinted rules.
        std::fs::write(
            &path,
            "exe=Game.exe;display_name=Plain\nexe=Game.exe;company=Acme;display_name=Pinned",
        )?;
        for image in [
            c"C:\\Games\\Game.exe",
            c"C:\\..\\Game.exe. ",
            c"Game.exe.",
            c"D:Game.exe",
            c"\\\\host\\share\\Game.exe",
        ] {
            // SAFETY: static input strings, null optional AppID, valid output buffer.
            let len = unsafe {
                compatdb_resolve_initial_display_name(
                    image.as_ptr(),
                    c"/missing-prefix".as_ptr(),
                    std::ptr::null(),
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                )
            };
            assert_eq!(len, 5, "{image:?}");
            assert_eq!(buf, *b"Plain\0");
        }
        buf.fill(0xff);
        std::fs::write(&path, "exe=steam:440;display_name=Game")?;
        assert_eq!(query(&image, &env, &mut buf), 4);
        assert_eq!(buf, *b"Game\0\xff");

        std::fs::write(&path, "exe=steam:440;display_name=café")?;
        let mut short = [0xff; 5];
        assert_eq!(query(&image, &env, &mut short), 0);
        assert_eq!(short, [0xff; 5]);
        assert_eq!(query(&image, &env, &mut buf), 5);
        assert_eq!(buf.as_slice(), "café\0".as_bytes());
        assert_eq!(query(&image, &wide("SteamAppId=440\0"), &mut buf), 0);
        assert_eq!(
            query(&image, &wide("SteamAppId=440\0\0junk\0\0"), &mut buf),
            0
        );
        assert_eq!(query(&wide("Game.exe\0"), &env, &mut buf), 0);
        assert_eq!(query(&image, &[], &mut buf), 0);
        assert_eq!(buf.as_slice(), "café\0".as_bytes());
        return Ok(());
    }
    let dir = std::env::temp_dir().join(format!("compatdb-pre-exec-{}", std::process::id()));
    std::fs::create_dir(&dir)?;
    let path = dir.join("rules");
    let status = std::process::Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "pre_exec::tests::ffi_reads_the_current_file_and_uses_counted_utf8_output",
        ])
        .env(CHILD, &path)
        .env(compatdb_core::FILE_ENV_VAR, &path)
        .env(compatdb_core::ENV_VAR, "exe=steam:440;display_name=Inline")
        .status();
    std::fs::remove_dir_all(dir)?;
    assert!(status?.success());
    Ok(())
}

#[test]
fn initial_dos_names_and_paths_use_wines_normalized_spelling() {
    for (image, exe, host) in [
        ("Game.exe. ", "Game.exe", Some("Game.exe")),
        (
            r"C:\games\Game.exe",
            "Game.exe",
            Some("/prefix/dosdevices/c:/games/Game.exe"),
        ),
        (
            r"C:\..\..\Game.exe.",
            "Game.exe",
            Some("/prefix/dosdevices/c:/Game.exe"),
        ),
        (
            r"D:/\games./gone\.././/Game.exe. ",
            "Game.exe",
            Some("/prefix/dosdevices/d:/games/Game.exe"),
        ),
        (
            r"D:\games..\Game.exe",
            "Game.exe",
            Some("/prefix/dosdevices/d:/games./Game.exe"),
        ),
        (r"games\Game.exe", "Game.exe", Some("games/Game.exe")),
        (r"games/../Game.exe", "Game.exe", Some("Game.exe")),
        (r"..\Game.exe", "Game.exe", None),
        (r".\games\..\..\Game.exe", "Game.exe", None),
        (r"D:Game.exe", "Game.exe", None),
        (r"\Game.exe", "Game.exe", None),
        (r"\\host\share\Game.exe", "Game.exe", None),
        (r"\\?\C:\Game.exe", "Game.exe", None),
        (r"\??\C:\Game.exe", "Game.exe", None),
        (r"C:\...\Game.exe", "Game.exe", None),
        (r"C:\odd?\Game.exe", "Game.exe", None),
        (r"C:\games\Game.exe:stream", "Game.exe:stream", None),
        (r"C:\games\COM1.exe", "COM1.exe", None),
    ] {
        assert_eq!(
            initial_image(image.as_bytes(), b"/prefix"),
            Some((exe, host.map(PathBuf::from))),
            "{image}"
        );
    }
    for image in [
        "",
        r"C:\",
        r"C:\games\",
        r"C:\games\.",
        r"C:\games\..",
        "... ",
    ] {
        assert!(
            initial_image(image.as_bytes(), b"/prefix").is_none(),
            "{image}"
        );
    }
    assert_eq!(
        initial_image(b"C:\\\xff\\Game.exe", b"/prefix"),
        Some(("Game.exe", None))
    );
}

#[test]
fn absolute_unix_paths_keep_their_literal_bytes() -> TestResult {
    for (image, exe) in [
        (&b"/unix/./link/../Game.exe. "[..], "Game.exe. "),
        (&b"/unix/\xff/dir\\Game.exe"[..], r"dir\Game.exe"),
    ] {
        let (actual_exe, path) = initial_image(image, b"/prefix").ok_or("missing image")?;
        assert_eq!(actual_exe, exe);
        assert_eq!(path.ok_or("missing path")?.as_os_str().as_bytes(), image);
    }
    Ok(())
}

#[test]
fn dos_parent_components_are_resolved_before_following_host_symlinks() -> TestResult {
    let dir = std::env::temp_dir().join(format!("compatdb-pre-exec-path-{}", std::process::id()));
    std::fs::create_dir(&dir)?;
    let check = || -> TestResult {
        let drives = dir.join("dosdevices");
        let root = dir.join("drive_c");
        std::fs::create_dir(&drives)?;
        std::fs::create_dir(&root)?;
        std::os::unix::fs::symlink(&root, drives.join("c:"))?;
        std::os::unix::fs::symlink(&drives, root.join("link"))?;
        std::fs::write(dir.join("Game.exe"), b"wrong image")?;
        std::fs::write(root.join("Game.exe"), b"drive image")?;
        for image in [r"C:\..\Game.exe", r"C:\link\..\Game.exe. "] {
            let (exe, path) = initial_image(image.as_bytes(), dir.as_os_str().as_bytes())
                .ok_or("missing image")?;
            assert_eq!(exe, "Game.exe");
            assert_eq!(std::fs::read(path.ok_or("missing path")?)?, b"drive image");
        }
        Ok(())
    };
    let result = check();
    std::fs::remove_dir_all(dir)?;
    result
}
