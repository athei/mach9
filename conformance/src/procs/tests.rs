use std::os::unix::{fs::symlink, process::CommandExt as _};

use super::*;

#[test]
fn a_ps_table_gives_pids_and_argv0() {
    let table = "    1 /sbin/launchd\n  812 wineserver\nbad line\n 9 /Applications/My \
                 App.app/Contents/MacOS/My App\n";
    assert_eq!(
        parse_ps(table),
        [
            (1, "/sbin/launchd"),
            (812, "wineserver"),
            (9, "/Applications/My App.app/Contents/MacOS/My App"),
        ]
    );
}

#[test]
fn the_server_directory_is_device_and_inode_in_hex() {
    assert_eq!(
        server_dir_for(0x0100_0012, 0x2f_a3b1),
        "server-1000012-2fa3b1"
    );
    let report = "p812\nftxt\nn/Users/me/dist/wine/bin/wineserver\nftxt\nn/usr/lib/dyld\n";
    assert_eq!(
        lsof_names(report).collect::<Vec<_>>(),
        ["/Users/me/dist/wine/bin/wineserver", "/usr/lib/dyld"]
    );
    assert_eq!(lsof_names("p812\n").next(), None);
}

/// A process whose `argv[0]` is a bare name, as for a server started
/// through `PATH`, is found by its executable, also through a link to the
/// executable; the test binary is found too.
#[test]
fn processes_are_found_by_their_executable_not_their_argv0() {
    let me = std::env::current_exe().unwrap();
    assert!(pids_of(&me).unwrap().contains(&std::process::id()));
    assert!(pids_of(Path::new("/nonexistent/bin/wineserver")).is_err());

    let dir = me.parent().unwrap().join("conformance-test-procs");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let link = dir.join("sleep");
    symlink("/bin/sleep", &link).unwrap();
    let mut child = Command::new("/bin/sleep")
        .arg0("sleep")
        .arg("5")
        .spawn()
        .unwrap();
    let found = pids_of(&link);
    let _ = child.kill();
    let _ = child.wait();
    assert!(found.unwrap().contains(&child.id()));

    // A link with another name than its target, started by the link's
    // path: argv[0] has the link's name.
    let renamed = dir.join("wineserver");
    symlink("/bin/sleep", &renamed).unwrap();
    let mut child = Command::new(&renamed).arg("5").spawn().unwrap();
    let found = pids_of(&renamed);
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_dir_all(&dir);
    assert!(found.unwrap().contains(&child.id()));
}

#[test]
fn a_helper_past_its_limit_is_killed() {
    let start = Instant::now();
    let error = output_limited("sleep", &["5"], Duration::from_millis(300))
        .err()
        .unwrap_or_default();
    assert!(error.contains("was still running after"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(
        output_limited("echo", &["hi"], HELPER_LIMIT).unwrap(),
        "hi\n"
    );
}
