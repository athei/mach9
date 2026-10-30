//! Where the caller's rules come from: the file [`FILE_ENV_VAR`] names, or
//! else the value of [`ENV_VAR`]. [`choose`] decides and takes no input but
//! its arguments; [`read`] is the I/O, which `compatdb-core` does not do.

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
pub struct Chosen {
    /// The text to parse, or `None` for the built-in rules alone.
    pub text: Option<String>,
    /// The lines for the block of the process, each as [`crate::log::line`]
    /// takes it.
    pub lines: Vec<String>,
}

impl Chosen {
    /// The built-in rules followed by the chosen ones, and the lines for the
    /// log, which end with a warning for each problem with the chosen text.
    pub fn into_table(self) -> (Table, Vec<String>) {
        let Self { text, mut lines } = self;
        let Some(text) = text else {
            return (Table::builtin(), lines);
        };
        let (rules, diagnostics) = Table::parse_lenient(&text);
        lines.extend(diagnostics.iter().map(|m| format!("  warning: {m}")));
        (rules.with_builtin(), lines)
    }
}

/// The source of the caller's rules; an empty value counts as unset. A set
/// `file` is the only source, read with `read`: when that fails the process
/// gets the built-in rules alone, since falling back to `inline` would hide a
/// broken path behind rules that may be stale.
pub fn choose(variables: Variables, read: impl FnOnce(&str) -> io::Result<String>) -> Chosen {
    let Variables { file, inline } = variables;
    let inline = inline.filter(|text| !text.is_empty());
    let Some(path) = file.filter(|path| !path.is_empty()) else {
        return Chosen {
            text: inline,
            lines: Vec::new(),
        };
    };
    let mut lines = vec![format!("  {FILE_ENV_VAR} = {path}")];
    if inline.is_some() {
        lines.push(format!(
            "  warning: {FILE_ENV_VAR} replaces {ENV_VAR}, which is ignored"
        ));
    }
    let text = match read(&path) {
        Ok(text) => Some(text),
        Err(error) => {
            lines.push(format!(
                "  warning: {FILE_ENV_VAR} not read: {error}; only the built-in rules apply"
            ));
            None
        }
    };
    Chosen { text, lines }
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
