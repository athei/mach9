use super::*;
use crate::outcome::Status;

/// A directory under the target directory, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("conformance-test-resume")
            .join(name);
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn identity() -> Identity {
    Identity {
        mode: Mode::Record,
        exe: String::from("d3d11"),
        leg: PeDir::X86_64,
        digest: String::from("sha256:0f1e2d"),
        scope: String::from("legs=i386-windows,x86_64-windows only=all"),
        header: Header::parse(
            "wine-11.21-37-g53def89293a\n",
            "dxmt=8c0b5f1\nfex=0123456\n",
        )
        .unwrap(),
    }
}

fn record(pass: Pass, key: &str, failed: u32) -> Record {
    Record {
        pass,
        unit: String::from("d3d11"),
        key: Key::Function(key.to_owned()),
        outcome: Outcome {
            status: if failed == 0 {
                Status::Ok
            } else {
                Status::Fail
            },
            failed,
            sites: failed,
            todo_succeeded: 0,
            skipped: 1,
        },
    }
}

fn records() -> Vec<Record> {
    vec![
        record(Pass::First, "test_a", 0),
        record(Pass::First, "test_b", 3),
        record(Pass::Confirm, "test_b", 2),
    ]
}

/// Open the state, append `records` and drop it without finishing.
fn interrupted(dir: &Path, records: &[Record]) {
    let mut opened = Resume::open(dir, &identity()).unwrap();
    for record in records {
        opened.resume.append(record).unwrap();
    }
}

#[test]
fn an_interrupted_run_resumes_with_its_records() {
    let dir = Dir::new("resume");
    let opened = Resume::open(&dir.0, &identity()).unwrap();
    assert_eq!(opened.records, []);
    assert_eq!(opened.discarded, None);
    drop(opened);
    interrupted(&dir.0, &records());
    let mut opened = Resume::open(&dir.0, &identity()).unwrap();
    assert_eq!(opened.records, records());
    assert_eq!(opened.discarded, None);
    opened
        .resume
        .append(&record(Pass::First, "test_c", 0))
        .unwrap();
    drop(opened);
    let opened = Resume::open(&dir.0, &identity()).unwrap();
    assert_eq!(opened.records.len(), 4);
    let path = dir.0.join("record-d3d11-x86_64-windows.txt");
    assert!(path.is_file());
    opened.resume.finish().unwrap();
    assert!(!path.exists());
}

#[test]
fn another_identity_discards_the_state() {
    let dir = Dir::new("identity");
    let changes: [fn(&mut Identity); 5] = [
        |identity| identity.digest.push('0'),
        |identity| identity.scope = String::from("legs=x86_64-windows only=d3d11/test_a"),
        |identity| identity.header.wine_version.push('0'),
        |identity| {
            identity
                .header
                .components
                .insert(String::from("mesa"), String::from("1a2b3c4"));
        },
        |identity| {
            identity.header.components.remove("fex");
        },
    ];
    for change in changes {
        interrupted(&dir.0, &records());
        let mut other = identity();
        change(&mut other);
        let opened = Resume::open(&dir.0, &other).unwrap();
        assert_eq!(opened.records, []);
        assert_eq!(
            opened.discarded.as_deref(),
            Some("it is the state of another run")
        );
        drop(opened);
        let opened = Resume::open(&dir.0, &identity()).unwrap();
        assert_eq!(opened.records, [], "the other run replaced the state");
    }
    let mut run = identity();
    run.mode = Mode::Run;
    let opened = Resume::open(&dir.0, &run).unwrap();
    assert_eq!(opened.discarded, None, "run and record keep separate files");
}

/// A kill can stop an append after any byte: every cut of the last record
/// reads as the records before it, and the next append starts a clean line.
#[test]
fn a_torn_last_line_is_cut_off() {
    let dir = Dir::new("torn");
    interrupted(&dir.0, &records());
    let path = dir.0.join("record-d3d11-x86_64-windows.txt");
    let full = fs::read(&path).unwrap();
    let last_start = full[..full.len() - 1]
        .iter()
        .rposition(|&b| b == b'\n')
        .unwrap()
        + 1;
    for cut in last_start..full.len() {
        fs::write(&path, &full[..cut]).unwrap();
        let mut opened = Resume::open(&dir.0, &identity()).unwrap();
        assert_eq!(opened.records, records()[..2], "cut at {cut}");
        assert_eq!(opened.discarded, None);
        opened.resume.append(&records()[2]).unwrap();
        drop(opened);
        assert_eq!(fs::read(&path).unwrap(), full, "cut at {cut}");
    }
}

/// A file cut inside its identity, or with a complete line that is not a
/// record, is discarded.
#[test]
fn a_broken_file_is_discarded() {
    let dir = Dir::new("broken");
    interrupted(&dir.0, &records());
    let path = dir.0.join("record-d3d11-x86_64-windows.txt");
    let full = fs::read_to_string(&path).unwrap();
    let identity_len = identity().text().len();
    for broken in [
        full[..identity_len - 1].to_owned(),
        full.replacen("first d3d11 test_b", "first d3d11 test b", 1),
        full.replacen("confirm", "second", 1),
        format!("{full}\n"),
    ] {
        fs::write(&path, &broken).unwrap();
        let opened = Resume::open(&dir.0, &identity()).unwrap();
        assert_eq!(opened.records, [], "{broken:?}");
        assert!(opened.discarded.is_some(), "{broken:?}");
        drop(opened);
        assert_eq!(fs::read_to_string(&path).unwrap(), identity().text());
    }
}

#[test]
fn bad_input_is_refused() {
    let dir = Dir::new("refused");
    let mut bad = identity();
    bad.digest = String::from("a\nend");
    let Err(error) = Resume::open(&dir.0, &bad) else {
        panic!("a digest with a line break was taken");
    };
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    let mut bad = identity();
    bad.scope = String::from("all\r");
    assert!(Resume::open(&dir.0, &bad).is_err());
    let mut opened = Resume::open(&dir.0, &identity()).unwrap();
    let mut record = record(Pass::First, "test_a", 0);
    record.unit = String::from("d3d 11");
    let error = opened.resume.append(&record).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}
