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
        // SAFETY: static input strings, null optional AppID, valid output buffer.
        let len = unsafe {
            compatdb_resolve_initial_display_name(
                c"C:\\Games\\Game.exe".as_ptr(),
                c"/missing-prefix".as_ptr(),
                std::ptr::null(),
                buf.as_mut_ptr().cast(),
                buf.len(),
            )
        };
        assert_eq!(len, 5);
        assert_eq!(buf, *b"Plain\0");
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
fn initial_paths_do_not_need_wine_state() {
    for (image, expected) in [
        ("/unix/path", Some("/unix/path")),
        ("game.exe", Some("game.exe")),
        (
            r"C:\games\game.exe",
            Some("/prefix/dosdevices/c:/games/game.exe"),
        ),
        (r"games\game.exe", Some("games/game.exe")),
        (r"D:game.exe", None),
        (r"\\host\share\game.exe", None),
    ] {
        assert_eq!(
            initial_image_path(image.as_bytes(), b"/prefix").as_deref(),
            expected.map(Path::new)
        );
    }
}
