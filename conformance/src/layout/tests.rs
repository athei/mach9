use super::*;
use crate::scratch::Scratch;

#[test]
fn the_root_is_wine_build_root_or_the_checkout_s_parent() {
    let manifest = Path::new("/Users/me/wine/scripts/conformance");
    assert_eq!(root(None, manifest), Path::new("/Users/me/wine"));
    assert_eq!(
        root(Some(OsStr::new("")), manifest),
        Path::new("/Users/me/wine")
    );
    assert_eq!(
        root(Some(OsStr::new("/elsewhere")), manifest),
        Path::new("/elsewhere")
    );
    let root = Path::new("/r");
    assert_eq!(default_bundle(root), Path::new("/r/dist/wine"));
    assert_eq!(default_state(root), Path::new("/r/build/conformance"));
}

#[test]
fn a_bundle_needs_its_tests() {
    let dir = Scratch::new("layout", "bundle");
    let error = Bundle::open(&dir).err().unwrap_or_default();
    assert!(error.contains("bin/wine is missing"), "{error}");
    fs::create_dir_all(dir.join("bin")).unwrap();
    fs::create_dir_all(dir.join("lib/wine/tests")).unwrap();
    for file in ["bin/wine", "bin/wineserver", "lib/wine/tests/VERSION"] {
        fs::write(dir.join(file), "").unwrap();
    }
    let error = Bundle::open(&dir).err().unwrap_or_default();
    assert!(error.contains("COMPONENTS is missing"), "{error}");
    fs::write(dir.join("lib/wine/tests/COMPONENTS"), "").unwrap();
    let bundle = Bundle::open(&dir).unwrap();
    assert_eq!(
        bundle.test_exe(PeDir::I386, "d3d8"),
        dir.join("lib/wine/tests/i386-windows/d3d8_test.exe")
    );
}

#[test]
fn a_directory_is_claimed_only_when_missing_empty_or_marked_so() {
    let dir = Scratch::new("layout", "claim");
    let tree = dir.join("state");
    claim_dir(&tree, STATE_TREE).unwrap();
    assert_eq!(
        fs::read_to_string(tree.join(TREE_MARKER)).unwrap(),
        "conformance\n"
    );
    fs::write(tree.join("kept"), "").unwrap();
    claim_dir(&tree, STATE_TREE).unwrap();
    assert!(tree.join("kept").exists());

    let error = claim_dir(&tree, PREFIX_TREE).err().unwrap_or_default();
    assert!(error.contains("is the tree 'conformance', not"), "{error}");

    let foreign = dir.join("foreign");
    fs::create_dir_all(&foreign).unwrap();
    claim_dir(&foreign, STATE_TREE).unwrap();
    fs::remove_file(foreign.join(TREE_MARKER)).unwrap();
    fs::write(foreign.join("data"), "").unwrap();
    let error = claim_dir(&foreign, STATE_TREE).err().unwrap_or_default();
    assert!(error.contains("has no .wine-build-tree"), "{error}");
    let error = fresh_dir(&foreign, STATE_TREE).err().unwrap_or_default();
    assert!(error.contains("has no .wine-build-tree"), "{error}");
    assert!(
        foreign.join("data").exists(),
        "an unmarked tree was deleted"
    );
}

#[test]
fn a_fresh_directory_replaces_only_its_own_tree() {
    let dir = Scratch::new("layout", "fresh");
    let prefix = dir.join("prefix");
    fresh_dir(&prefix, PREFIX_TREE).unwrap();
    fs::write(prefix.join("system.reg"), "").unwrap();
    fresh_dir(&prefix, PREFIX_TREE).unwrap();
    assert!(!prefix.join("system.reg").exists());
    assert!(prefix.join(TREE_MARKER).exists());
    let error = fresh_dir(&prefix, WORK_TREE).err().unwrap_or_default();
    assert!(error.contains("not 'conformance-work'"), "{error}");
}

/// A symbolic link in place of an inner tree is refused, and its target is
/// neither marked nor emptied.
#[test]
fn an_inner_tree_is_never_a_symbolic_link() {
    let dir = Scratch::new("layout", "link");
    let target = dir.join("target");
    fs::create_dir_all(&target).unwrap();
    let link = dir.join("prefix");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    for result in [
        fresh_dir(&link, PREFIX_TREE),
        claim_inner(&link, BASELINE_RAW_TREE),
    ] {
        let error = result.err().unwrap_or_default();
        assert!(error.contains("is a symbolic link"), "{error}");
    }
    assert!(!target.join(TREE_MARKER).exists());
    claim_dir(&link, STATE_TREE).unwrap();
}

#[test]
fn a_state_directory_has_one_user_at_a_time() {
    let dir = Scratch::new("layout", "lock");
    let state = State::claim(&dir.join("state")).unwrap();
    let error = State::claim(&dir.join("state")).err().unwrap_or_default();
    assert!(error.contains("another conformance run uses"), "{error}");
    drop(state);
    let state = State::claim(&dir.join("state")).unwrap();
    assert_eq!(state.prefix(), state.dir.join("prefix"));
    assert_eq!(
        state.baseline_raw("d3d12"),
        state.dir.join("baseline-raw/d3d12")
    );
}

/// A copy of the lock's descriptor, as a child spawned by another thread
/// holds until its exec, does not keep the state claimed once the claim is
/// dropped.
#[test]
fn a_dropped_claim_frees_the_state_while_a_copy_of_its_descriptor_is_open() {
    let dir = Scratch::new("layout", "copy");
    let state = State::claim(&dir.join("state")).unwrap();
    let copy = state.lock.try_clone().unwrap();
    drop(state);
    let again = State::claim(&dir.join("state"));
    drop(copy);
    assert!(again.is_ok(), "{:?}", again.err());
}

#[test]
fn run_ids_are_utc_times() {
    let at = |seconds| run_id(UNIX_EPOCH + std::time::Duration::from_secs(seconds));
    assert_eq!(at(0), "19700101-000000");
    assert_eq!(at(951_782_400), "20000229-000000");
    assert_eq!(at(1_793_069_563), "20261027-025243");
}

#[test]
fn logs_are_named_by_their_key() {
    assert_eq!(
        log_file(&Key::Function(String::from("d3d10:test_a"))),
        "d3d10:test_a.log"
    );
    assert_eq!(log_file(&Key::Whole), "whole.log");
}

#[test]
fn a_digest_names_the_content() {
    let dir = Scratch::new("layout", "digest");
    let file = dir.join("a.exe");
    fs::write(&file, "a").unwrap();
    assert_eq!(file_digest(&file).unwrap(), "fnv1a64:af63dc4c8601ec8c:1");
    fs::write(&file, "").unwrap();
    assert_eq!(file_digest(&file).unwrap(), "fnv1a64:cbf29ce484222325:0");
    assert!(file_digest(&dir.join("missing")).is_err());
}
