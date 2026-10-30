//! The caller's rules: the value of [`ENV_VAR`], then the rules of the file
//! [`FILE_ENV_VAR`] names. [`gather`] takes no input but its arguments;
//! [`read`] is the I/O, which `compatdb-core` does not do.

use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

use compatdb_core::{ENV_VAR, FILE_ENV_VAR, Table};

/// The largest rules file [`read`] takes, in bytes. A table is far smaller,
/// so a larger file is the wrong path, and every process would read it.
const MAX_BYTES: u64 = 1024 * 1024;

/// The values of the two variables the caller's rules can come from.
pub struct Variables {
    /// The value of [`FILE_ENV_VAR`].
    pub file: Option<String>,
    /// The value of [`ENV_VAR`].
    pub inline: Option<String>,
}

impl Variables {
    /// The value `get` gives for the name of each variable.
    pub fn read_with(get: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            file: get(FILE_ENV_VAR),
            inline: get(ENV_VAR),
        }
    }
}

/// The caller's rules for this process and what the log says about them.
pub struct Gathered {
    /// The value of [`ENV_VAR`], if it has one.
    pub inline: Option<String>,
    /// The text of the file, if there is one and it could be read.
    pub file: Option<String>,
    /// The lines for the block of the process, each as [`crate::log::line`]
    /// takes it.
    pub lines: Vec<String>,
}

impl Gathered {
    /// The built-in rules followed by the rules of the variable and those of
    /// the file, and the lines for the log, which end with a warning for
    /// each problem with either text.
    pub fn into_table(self) -> (Table, Vec<String>) {
        let Self {
            inline,
            file,
            mut lines,
        } = self;
        let (mut table, mut diagnostics) = Table::parse_lenient(&inline.unwrap_or_default());
        let (from_file, file_diagnostics) = Table::parse_file_lenient(&file.unwrap_or_default());
        table.append(from_file);
        diagnostics.extend(file_diagnostics);
        lines.extend(diagnostics.iter().map(|m| format!("  warning: {m}")));
        (table.with_builtin(), lines)
    }
}

/// The two sources of the caller's rules; an empty value counts as unset. A
/// set `file` is read with `read`, and when that fails it adds no rules, with
/// a warning: `inline` is a source of its own and not a fallback, so it
/// applies either way.
pub fn gather(variables: Variables, read: impl FnOnce(&str) -> io::Result<String>) -> Gathered {
    let Variables { file, inline } = variables;
    let inline = inline.filter(|text| !text.is_empty());
    let Some(path) = file.filter(|path| !path.is_empty()) else {
        return Gathered {
            inline,
            file: None,
            lines: Vec::new(),
        };
    };
    let mut lines = vec![format!("  {FILE_ENV_VAR} = {path}")];
    let file = match read(&path) {
        Ok(text) => Some(text),
        Err(error) => {
            lines.push(format!(
                "  warning: {FILE_ENV_VAR} not read: {error}; the file adds no rules"
            ));
            None
        }
    };
    Gathered {
        inline,
        file,
        lines,
    }
}

/// The text of the rules file at `path`, with invalid UTF-8 replaced as for
/// [`ENV_VAR`].
///
/// # Errors
///
/// A path that is not absolute, which would depend on the working directory
/// of each process, a path that is not a regular file, a file over
/// [`MAX_BYTES`], and any error of the file system.
pub fn read(path: &str) -> io::Result<String> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(io::Error::other("not an absolute path"));
    }
    // Checked before opening: opening a FIFO blocks until it has a writer,
    // and this runs in every Wine process.
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    if metadata.len() > MAX_BYTES {
        return Err(over_limit());
    }
    read_capped(File::open(path)?)
}

/// All of `reader` as text, or [`over_limit`] when it holds more than
/// [`MAX_BYTES`], as a file does that grew since [`read`] checked its size.
/// Cutting it off there would parse as a shorter table without a warning.
fn read_capped(reader: impl Read) -> io::Result<String> {
    // The byte past the limit tells a longer reader from one at the limit.
    let mut limited = reader.take(MAX_BYTES + 1);
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes)?;
    if limited.limit() == 0 {
        return Err(over_limit());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn over_limit() -> io::Error {
    io::Error::other(format!("over {MAX_BYTES} bytes"))
}

#[cfg(test)]
mod tests;
