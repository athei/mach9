//! Reads the `compatdb:` lines of a test's output: which Direct3D tree and
//! which OpenGL each process got.
//!
//! `compatdb.so` writes one block per process to stderr when it loads
//! (`docs/compatdb.md`, "Logging and verification"): a header line
//! `compatdb: <exe> [<arch>]<fingerprint>`, then one line per setting,
//! indented by two spaces. The lines read here are those of `apply::tree`
//! and `apply::opengl` in `compatdb/src/apply.rs` and the unset line of
//! `scalar_unset_line` in `compatdb/src/lib.rs`:
//!
//! - `dxgi = <impl> (from <dir>), <source>` when the tree was prepended, and
//!   `dxgi = <impl>, <source>: <why not>` when it was not; `d3d9` alike.
//! - `opengl = <impl>, <source>` when ntdll took the value,
//!   `opengl = <impl>, <source>: ntdll lacks set_compat_opengl, ignored`
//!   when it did not, and `opengl = unset (upstream: ...)` when no rule set
//!   it, which leaves the process on Apple's OpenGL.
//! - `wined3d = <renderer>, <source>` when a rule or a `WINE_D3D_CONFIG`
//!   names a renderer, and `wined3d = unset (upstream: ...)` when neither
//!   does (`wined3d_line` in `compatdb/src/lib.rs`).
//!
//! Each `write` of the library is one line, but the blocks of processes
//! that start together can interleave, and their lines carry no process id.
//! A setting line belongs to the latest header before it.

use std::fmt;

use compatdb_core::{D3d9, Dxgi, Key, OpenGl, Wined3d};

/// The prefix of every line `compatdb.so` writes.
const PREFIX: &str = "compatdb: ";

/// The arch labels of a header line, from `arch_name` in
/// `compatdb/src/lib.rs`.
const ARCHES: [&str; 3] = ["x86_64 (arm64ec)", "i386", "aarch64"];

/// The `compatdb:` lines of `output`, without the prefix and a trailing
/// `\r`, in order.
#[must_use]
pub fn lines(output: &str) -> Vec<&str> {
    output
        .split('\n')
        .filter_map(|line| line.strip_suffix('\r').unwrap_or(line).strip_prefix(PREFIX))
        .collect()
}

/// The blocks of the `compatdb:` lines of one output.
pub struct Processes<'a> {
    blocks: Vec<Block<'a>>,
}

/// The block of one process.
struct Block<'a> {
    /// The image name of the header line.
    exe: &'a str,
    /// Its setting lines, without their indent.
    settings: Vec<&'a str>,
}

/// Why [`Processes`] cannot tell which implementation a process got.
#[derive(PartialEq, Eq, Debug)]
pub enum ImplError {
    /// No block has a header for the executable.
    NoProcess,
    /// A block of the executable has no line for the setting.
    NoLine,
    /// A line says the tree was not prepended or ntdll did not take the
    /// value; the line.
    NotApplied(String),
    /// A line names a value this parser does not know; the line.
    Unknown(String),
    /// The blocks of the executable disagree; their lines.
    Differ(Vec<String>),
}

impl fmt::Display for ImplError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProcess => f.write_str("no compatdb block for the process"),
            Self::NoLine => f.write_str("a compatdb block of the process lacks the setting"),
            Self::NotApplied(line) => write!(f, "not applied: {line}"),
            Self::Unknown(line) => write!(f, "unknown value: {line}"),
            Self::Differ(lines) => write!(f, "the processes differ: {}", lines.join("; ")),
        }
    }
}

impl<'a> Processes<'a> {
    /// Split the `compatdb:` lines of `output` into blocks.
    ///
    /// A line without indent that is not a header, such as
    /// `required ntdll symbols missing, doing nothing`, ends the block
    /// before it and starts none.
    #[must_use]
    pub fn parse(output: &'a str) -> Self {
        let mut blocks: Vec<Block<'a>> = Vec::new();
        let mut open = false;
        for line in lines(output) {
            if let Some(setting) = line.strip_prefix("  ") {
                if open && let Some(block) = blocks.last_mut() {
                    block.settings.push(setting);
                }
                continue;
            }
            open = false;
            if let Some(exe) = header_exe(line) {
                blocks.push(Block {
                    exe,
                    settings: Vec::new(),
                });
                open = true;
            }
        }
        Self { blocks }
    }

    /// The DXGI-family tree every process of `exe` got.
    ///
    /// # Errors
    ///
    /// Returns an [`ImplError`] when no block is for `exe`, a block lacks
    /// the line, a tree was not prepended, a value is unknown, or two
    /// blocks disagree.
    pub fn dxgi(&self, exe: &str) -> Result<Dxgi, ImplError> {
        self.agreed(exe, Key::Dxgi.as_str(), |value, line| {
            tree(value, line, |name| name.parse().ok())
        })
    }

    /// The D3D9 tree every process of `exe` got, as for [`Self::dxgi`].
    ///
    /// # Errors
    ///
    /// As [`Self::dxgi`].
    pub fn d3d9(&self, exe: &str) -> Result<D3d9, ImplError> {
        self.agreed(exe, Key::D3d9.as_str(), |value, line| {
            tree(value, line, |name| name.parse().ok())
        })
    }

    /// The OpenGL every process of `exe` got: the value ntdll took, or
    /// [`OpenGl::Cgl`], Apple's, when no rule set one.
    ///
    /// # Errors
    ///
    /// As [`Self::dxgi`], with ntdll lacking the hook as not applied.
    pub fn opengl(&self, exe: &str) -> Result<OpenGl, ImplError> {
        self.agreed(exe, Key::OpenGl.as_str(), |value, line| {
            if value.starts_with("unset (upstream: ") {
                return Ok(OpenGl::Cgl);
            }
            if value.ends_with(": ntdll lacks set_compat_opengl, ignored") {
                return Err(ImplError::NotApplied(line.to_owned()));
            }
            value
                .split_once(", ")
                .and_then(|(name, _)| name.parse().ok())
                .ok_or_else(|| ImplError::Unknown(line.to_owned()))
        })
    }

    /// The wined3d renderer every process of `exe` got: the one a rule or
    /// a `WINE_D3D_CONFIG` names, or `None` for `wined3d = unset (...)`,
    /// where wined3d's registry settings or its default decide.
    ///
    /// # Errors
    ///
    /// As [`Self::dxgi`].
    pub fn wined3d(&self, exe: &str) -> Result<Option<Wined3d>, ImplError> {
        self.agreed(exe, Key::Wined3d.as_str(), |value, line| {
            if value.starts_with("unset (upstream: ") {
                return Ok(None);
            }
            value
                .split_once(", ")
                .and_then(|(name, _)| name.parse().ok())
                .map(Some)
                .ok_or_else(|| ImplError::Unknown(line.to_owned()))
        })
    }

    /// The value of the setting `key` in every block of `exe`, read by
    /// `parse` from the text after `<key> = ` and the whole line, when all
    /// its lines agree. A block with two lines for the setting holds the
    /// lines of two processes that started together, and both count.
    fn agreed<T: PartialEq>(
        &self,
        exe: &str,
        key: &str,
        parse: impl Fn(&str, &str) -> Result<T, ImplError>,
    ) -> Result<T, ImplError> {
        let prefix = format!("{key} = ");
        let mut found: Vec<(T, &str)> = Vec::new();
        for block in self
            .blocks
            .iter()
            .filter(|block| block.exe.eq_ignore_ascii_case(exe))
        {
            let before = found.len();
            for line in block
                .settings
                .iter()
                .filter(|setting| setting.starts_with(&prefix))
            {
                found.push((parse(&line[prefix.len()..], line)?, line));
            }
            if found.len() == before {
                return Err(ImplError::NoLine);
            }
        }
        let mut found = found.into_iter();
        let (first, first_line) = found.next().ok_or(ImplError::NoProcess)?;
        let others: Vec<&str> = found
            .filter(|(value, _)| *value != first)
            .map(|(_, line)| line)
            .collect();
        if others.is_empty() {
            Ok(first)
        } else {
            Err(ImplError::Differ(
                std::iter::once(first_line)
                    .chain(others)
                    .map(str::to_owned)
                    .collect(),
            ))
        }
    }
}

/// The tree of a `dxgi` or `d3d9` line from the text after `<family> = `:
/// `<impl> (from <dir>), ...` when it was prepended.
fn tree<T>(value: &str, line: &str, parse: impl Fn(&str) -> Option<T>) -> Result<T, ImplError> {
    let end = value.find([' ', ',']).unwrap_or(value.len());
    let name = parse(&value[..end]).ok_or_else(|| ImplError::Unknown(line.to_owned()))?;
    if value[end..].starts_with(" (from ") {
        Ok(name)
    } else {
        Err(ImplError::NotApplied(line.to_owned()))
    }
}

/// The image name of a header line `<exe> [<arch>]<fingerprint>`.
fn header_exe(line: &str) -> Option<&str> {
    ARCHES.iter().find_map(|arch| {
        let marker = format!(" [{arch}]");
        let at = line.find(&marker)?;
        let after = &line[at + marker.len()..];
        (at > 0 && (after.is_empty() || after.starts_with(' '))).then(|| &line[..at])
    })
}

#[cfg(test)]
mod tests;
