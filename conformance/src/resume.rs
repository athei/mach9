//! Keeps the finished outcomes of a run of one leg of one executable, so a
//! run that was interrupted continues where it stopped.
//!
//! The state is one file in a directory the caller names, per mode,
//! executable and leg. Its first lines are the [`Identity`] of the run,
//! then one [`Record`] per line:
//!
//! ```text
//! conformance resume
//! mode=record
//! exe=d3d11
//! leg=x86_64-windows
//! digest=<the caller's digest of the test executables>
//! scope=<the caller's spelling of the legs and keys it runs>
//! wine=wine-11.21-37-g53def89293a
//! component=dxmt=8c0b5f1
//! end
//! first d3d11 test_create_device ok failed=0 sites=0 todo_succeeded=0 skipped=0
//! ```
//!
//! The identity is written to a temporary file that is renamed into place,
//! and each record is appended with one write that ends in its newline. A
//! kill at any point leaves either no file, or the identity and complete
//! lines with at most a partial last line, which [`Resume::open`] drops.

use std::{
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use crate::{
    header::Header,
    model::PeDir,
    outcome::{Key, Outcome},
};

/// The first line of a resume file.
const MAGIC: &str = "conformance resume";
/// The line that ends the identity.
const END: &str = "end";

/// Which command the state belongs to.
#[derive(Clone, Copy)]
pub enum Mode {
    Run,
    Record,
}

impl Mode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Record => "record",
        }
    }
}

/// What the state is the state of: a resumed run must match it exactly.
pub struct Identity {
    pub mode: Mode,
    /// The executable's name without `_test.exe`.
    pub exe: String,
    pub leg: PeDir,
    /// The caller's digest of the test executables, so a rebuilt executable
    /// does not reuse the outcomes of the old one. It must not hold a line
    /// break.
    pub digest: String,
    /// The caller's spelling of the invocation's scope, the legs and the
    /// selected keys, so the state of an invocation with another scope is
    /// not reused. It must not hold a line break.
    pub scope: String,
    pub header: Header,
}

impl Identity {
    /// The identity lines of the file, each ending in a newline.
    fn text(&self) -> String {
        let mut text = format!(
            "{MAGIC}\nmode={}\nexe={}\nleg={}\ndigest={}\nscope={}\nwine={}\n",
            self.mode.as_str(),
            self.exe,
            self.leg.as_str(),
            self.digest,
            self.scope,
            self.header.wine_version
        );
        for (name, commit) in &self.header.components {
            let _ = writeln!(text, "component={name}={commit}");
        }
        text.push_str(END);
        text.push('\n');
        text
    }

    /// The name of the state file: one per mode, executable and leg.
    fn file_name(&self) -> String {
        format!(
            "{}-{}-{}.txt",
            self.mode.as_str(),
            self.exe,
            self.leg.as_str()
        )
    }
}

/// Which pass of a run a record is from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pass {
    /// The run of every key.
    First,
    /// A record's second run of a key whose outcome changed.
    Confirm,
}

impl Pass {
    const fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Confirm => "confirm",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Self::First, Self::Confirm]
            .into_iter()
            .find(|pass| pass.as_str() == text)
    }
}

/// One finished test of the leg.
#[derive(PartialEq, Eq, Debug)]
pub struct Record {
    pub pass: Pass,
    pub unit: String,
    pub key: Key,
    pub outcome: Outcome,
}

impl Record {
    fn parse(line: &str) -> Option<Self> {
        let tokens: Vec<&str> = line.split(' ').collect();
        let [pass, unit, key, outcome @ ..] = &tokens[..] else {
            return None;
        };
        Some(Self {
            pass: Pass::parse(pass)?,
            unit: crate::model::is_ident(unit).then(|| (*unit).to_owned())?,
            key: Key::parse(key)?,
            outcome: Outcome::parse(outcome).ok()?,
        })
    }
}

/// The open state of a run, to append records to.
pub struct Resume {
    path: PathBuf,
    file: File,
}

/// What [`Resume::open`] found.
pub struct Opened {
    pub resume: Resume,
    /// The records of the earlier run, in the order it wrote them.
    pub records: Vec<Record>,
    /// Why the state of an earlier run was discarded, if there was one.
    pub discarded: Option<String>,
}

impl Resume {
    /// Open the state of the run `identity` in `dir`, creating `dir`.
    ///
    /// A file with another identity, or with a complete line that is not a
    /// record, is discarded and replaced by a fresh one. A partial last
    /// line, the trace of a kill during an append, is cut off.
    ///
    /// # Errors
    ///
    /// Returns the I/O error of creating, reading, writing or renaming the
    /// file, or an `InvalidInput` error for a digest or a scope that holds
    /// a line break.
    pub fn open(dir: &Path, identity: &Identity) -> io::Result<Opened> {
        if identity.digest.contains(['\n', '\r']) || identity.scope.contains(['\n', '\r']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the digest or the scope holds a line break",
            ));
        }
        fs::create_dir_all(dir)?;
        let path = dir.join(identity.file_name());
        let expected = identity.text();
        let (records, discarded) = match fs::read(&path) {
            Ok(bytes) => match read_records(&bytes, &expected) {
                Ok((records, complete)) => {
                    if complete < bytes.len() {
                        OpenOptions::new()
                            .write(true)
                            .open(&path)?
                            .set_len(u64::try_from(complete).unwrap_or(u64::MAX))?;
                    }
                    (Some(records), None)
                }
                Err(why) => (None, Some(why)),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (None, None),
            Err(e) => return Err(e),
        };
        if records.is_none() {
            let temporary = dir.join(format!("{}.new", identity.file_name()));
            fs::write(&temporary, &expected)?;
            fs::rename(&temporary, &path)?;
        }
        let records = records.unwrap_or_default();
        let file = OpenOptions::new().append(true).open(&path)?;
        Ok(Opened {
            resume: Self { path, file },
            records,
            discarded,
        })
    }

    /// Append `record` with one write that ends in its newline.
    ///
    /// # Errors
    ///
    /// Returns the I/O error of the write, or an `InvalidInput` error for a
    /// unit that is not a C name.
    pub fn append(&mut self, record: &Record) -> io::Result<()> {
        if !crate::model::is_ident(&record.unit) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unit {:?} is not a C name", record.unit),
            ));
        }
        let line = format!(
            "{} {} {} {}\n",
            record.pass.as_str(),
            record.unit,
            record.key,
            record.outcome
        );
        self.file.write_all(line.as_bytes())
    }

    /// Remove the state once the run finished.
    ///
    /// # Errors
    ///
    /// Returns the I/O error of removing the file.
    pub fn finish(self) -> io::Result<()> {
        drop(self.file);
        fs::remove_file(&self.path)
    }
}

/// The records of a resume file's `bytes` and the length of its complete
/// part, when its identity is `expected`; else why it is discarded.
fn read_records(bytes: &[u8], expected: &str) -> Result<(Vec<Record>, usize), String> {
    let body = bytes
        .strip_prefix(expected.as_bytes())
        .ok_or_else(|| String::from("it is the state of another run"))?;
    let complete = body
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |newline| newline + 1);
    let text =
        std::str::from_utf8(&body[..complete]).map_err(|_| String::from("a line is not UTF-8"))?;
    let records = text
        .lines()
        .enumerate()
        .map(|(index, line)| {
            Record::parse(line).ok_or_else(|| {
                format!(
                    "line {} is not a record: {line:?}",
                    expected.lines().count() + index + 1
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((records, expected.len() + complete))
}

#[cfg(test)]
mod tests;
