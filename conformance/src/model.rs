//! The typed form of `wine-tests.toml`: the Wine test executables, their
//! units, and how each unit's test functions become selectable.
//!
//! [`Config::builtin`] reads the copy compiled into the binary.

use std::{collections::BTreeSet, str::FromStr};

use compatdb_core::{Dxgi, OpenGl, Wined3d};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

/// The `wine-tests.toml` next to `Cargo.toml`.
const WINE_TESTS: &str = include_str!("../wine-tests.toml");

/// Every test executable, in file order.
pub struct Config {
    pub exes: Vec<Exe>,
}

/// One test executable, `<name>_test.exe`.
pub struct Exe {
    pub name: String,
    /// The directory of its sources and its `Makefile.in`, relative to the
    /// Wine source root.
    pub dir: String,
    /// The PE directories the bundle builds it for; both unless the TOML
    /// restricts it.
    pub pe_dirs: Vec<PeDir>,
    /// What the `compatdb:` block of its test process must say before its
    /// results count.
    pub expect: Expect,
    pub units: Vec<Unit>,
}

/// The implementations an executable's tests must run on, as the
/// `compatdb:` block of the test process reports them; `None` checks
/// nothing.
pub struct Expect {
    /// The DXGI-family tree.
    pub dxgi: Option<Dxgi>,
    /// The OpenGL the Mac driver gives the process.
    pub opengl: Option<OpenGl>,
    /// The renderer of wined3d. A process whose `wined3d` line says
    /// `unset` gets wined3d's default, [`Wined3d::Gl`]: the runner's
    /// prefix is fresh and its environment has no `WINE_D3D_CONFIG`.
    pub wined3d: Option<Wined3d>,
}

/// A PE directory of the bundle's tests, `lib/wine/tests/<pe-dir>`.
///
/// A run of the tests of one directory is a leg; the baselines order the
/// legs as the variants are declared.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum PeDir {
    I386,
    X86_64,
}

impl PeDir {
    /// The directory name, as `TEST_PE_DIRS` in `bin/lib/profile.sh`
    /// spells it.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::I386 => "i386-windows",
            Self::X86_64 => "x86_64-windows",
        }
    }

    /// The directory [`Self::as_str`] spells `name`, if any.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "i386-windows" => Some(Self::I386),
            "x86_64-windows" => Some(Self::X86_64),
            _ => None,
        }
    }
}

/// One unit: a `START_TEST(name)` entry, the argument that selects it on
/// the test's command line.
pub struct Unit {
    pub name: String,
    /// The source file, relative to the Wine source root. Defaults to
    /// `<dir>/<name>.c`.
    pub file: String,
    pub selection: Selection,
}

/// How a unit's test functions are selected.
pub enum Selection {
    /// The unit runs as a whole: its `START_TEST` body mixes setup and tests,
    /// so it gets no generated block.
    Whole,
    /// Each test function the body runs is a key of its own.
    Wrapped(Wrapping),
}

/// How the calls of a wrapped unit's `START_TEST` body are classified.
pub struct Wrapping {
    /// Helpers that take a test function as an argument and run or queue it.
    pub dispatch: Vec<Dispatch>,
    /// Local functions and function-like macros the body calls that stay
    /// unwrapped: setup, child process entry points, the call that runs
    /// queued tests, and the file's check macros.
    pub setup: Vec<String>,
    /// Test functions the body calls directly whose names neither start
    /// with `test_` nor end with `_test`, or that a `return;` follows.
    pub tests: Vec<String>,
}

/// A dispatch helper: a local function one of whose arguments is the test
/// function, which names the key.
pub struct Dispatch {
    pub name: String,
    /// The number of arguments the helper takes.
    pub args: usize,
    /// The 1-based position of the test function among the arguments.
    pub key: usize,
    /// Put before the test function's name to form the key, for a test
    /// function the unit runs through two helpers: a C name and `:`, or
    /// empty.
    pub prefix: String,
}

impl Config {
    /// The configuration compiled into the binary.
    ///
    /// # Errors
    ///
    /// Returns a message when the compiled-in file does not parse; a unit
    /// test keeps that from shipping.
    pub fn builtin() -> Result<Self, String> {
        Self::parse(WINE_TESTS)
    }

    /// Parse a `wine-tests.toml` text.
    ///
    /// # Errors
    ///
    /// Returns a message naming the entry for a TOML syntax error, an unknown
    /// or missing key, a value of the wrong type, an `expect` value compatdb
    /// does not know, a name that is not a C
    /// identifier, an unknown PE directory, a duplicate executable, unit,
    /// helper or name, a key position outside the helper's arguments, a name
    /// in two of `dispatch`, `setup` and `tests`, or a whole unit with any of
    /// them.
    pub fn parse(text: &str) -> Result<Self, String> {
        let doc: DocumentMut = text.parse().map_err(|e| format!("wine-tests.toml: {e}"))?;
        check_keys(doc.as_table(), "the top level", &["exe"])?;
        let exes = match doc.get("exe") {
            Some(item) => tables(item, "exe")?
                .iter()
                .map(parse_exe)
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        };
        unique(exes.iter().map(|exe| exe.name.as_str()), "executable")?;
        Ok(Self { exes })
    }
}

fn parse_exe(table: &Table) -> Result<Exe, String> {
    let name = string(table, "name", "an exe")?;
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err(format!(
            "exe {name:?}: a name holds only letters, digits, `_` and `-`"
        ));
    }
    let context = format!("exe {name}");
    check_keys(
        table,
        &context,
        &["name", "dir", "pe_dirs", "expect", "unit"],
    )?;
    let dir = string(table, "dir", &context)?;
    let expect = match table.get("expect") {
        Some(item) => parse_expect(item, &context)?,
        None => Expect {
            dxgi: None,
            opengl: None,
            wined3d: None,
        },
    };
    let pe_dirs = match table.get("pe_dirs") {
        Some(item) => {
            let names = names(item, "pe_dirs", &context, |_| true)?;
            unique(
                names.iter().map(String::as_str),
                &format!("pe_dirs of {context}"),
            )?;
            if names.is_empty() {
                return Err(format!("{context}: `pe_dirs` is empty"));
            }
            names
                .iter()
                .map(|dir| {
                    PeDir::parse(dir).ok_or_else(|| {
                        format!(
                            "{context}: `pe_dirs` holds {dir:?}, not i386-windows or \
                             x86_64-windows"
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        None => vec![PeDir::I386, PeDir::X86_64],
    };
    let units = tables(required(table, "unit", &context)?, &context)?
        .iter()
        .map(|unit| parse_unit(unit, &name, &dir))
        .collect::<Result<Vec<_>, _>>()?;
    unique(
        units.iter().map(|unit| unit.name.as_str()),
        &format!("unit of exe {name}"),
    )?;
    Ok(Exe {
        name,
        dir,
        pe_dirs,
        expect,
        units,
    })
}

/// The `expect` table of an exe: `dxgi`, `opengl` and `wined3d`, each a
/// value in compatdb's spelling.
fn parse_expect(item: &Item, context: &str) -> Result<Expect, String> {
    let table = item
        .as_table_like()
        .ok_or_else(|| format!("{context}: `expect` must be a table"))?;
    if let Some((key, _)) = table
        .iter()
        .find(|(key, _)| !["dxgi", "opengl", "wined3d"].contains(key))
    {
        return Err(format!("{context}: unknown key `expect.{key}`"));
    }
    Ok(Expect {
        dxgi: expect_value(table, "dxgi", context)?,
        opengl: expect_value(table, "opengl", context)?,
        wined3d: expect_value(table, "wined3d", context)?,
    })
}

/// The value of `expect.<key>`, parsed as compatdb parses it.
fn expect_value<T: FromStr<Err: std::fmt::Display>>(
    table: &dyn toml_edit::TableLike,
    key: &str,
    context: &str,
) -> Result<Option<T>, String> {
    table.get(key).map_or(Ok(None), |item| {
        let text = item
            .as_str()
            .ok_or_else(|| format!("{context}: `expect.{key}` must be a string"))?;
        text.parse()
            .map(Some)
            .map_err(|e| format!("{context}: `expect.{key}`: {e}"))
    })
}

/// The strings of the array `item`, each passing `valid`.
fn names(
    item: &Item,
    key: &str,
    context: &str,
    valid: impl Fn(&str) -> bool,
) -> Result<Vec<String>, String> {
    item.as_array()
        .ok_or_else(|| format!("{context}: `{key}` must be an array of names"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|name| valid(name))
                .map(str::to_owned)
                .ok_or_else(|| format!("{context}: `{key}` holds a value that is not a valid name"))
        })
        .collect()
}

fn parse_unit(table: &Table, exe: &str, dir: &str) -> Result<Unit, String> {
    let name = ident(table, "name", &format!("a unit of exe {exe}"))?;
    let context = format!("unit {exe}/{name}");
    check_keys(
        table,
        &context,
        &["name", "file", "whole", "dispatch", "setup", "tests"],
    )?;
    let file = match table.get("file") {
        Some(_) => string(table, "file", &context)?,
        None => format!("{dir}/{name}.c"),
    };
    let whole = match table.get("whole") {
        Some(item) => item
            .as_bool()
            .ok_or_else(|| format!("{context}: `whole` must be true or false"))?,
        None => false,
    };
    let selection = if whole {
        if ["dispatch", "setup", "tests"]
            .iter()
            .any(|key| table.contains_key(key))
        {
            return Err(format!(
                "{context}: a whole unit takes no `dispatch`, `setup` or `tests`"
            ));
        }
        Selection::Whole
    } else {
        Selection::Wrapped(parse_wrapping(table, &context)?)
    };
    Ok(Unit {
        name,
        file,
        selection,
    })
}

fn parse_wrapping(table: &Table, context: &str) -> Result<Wrapping, String> {
    let dispatch = match table.get("dispatch") {
        Some(item) => item
            .as_array()
            .ok_or_else(|| format!("{context}: `dispatch` must be an array of inline tables"))?
            .iter()
            .map(|value| parse_dispatch(value, context))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    let list = |key: &str| {
        table.get(key).map_or_else(
            || Ok(Vec::new()),
            |item| names(item, key, context, is_ident),
        )
    };
    let setup = list("setup")?;
    let tests = list("tests")?;
    unique(
        dispatch.iter().map(|helper| helper.name.as_str()),
        &format!("dispatch helper of {context}"),
    )?;
    unique(
        setup.iter().map(String::as_str),
        &format!("setup of {context}"),
    )?;
    unique(
        tests.iter().map(String::as_str),
        &format!("tests of {context}"),
    )?;
    if let Some(both) = dispatch.iter().find(|helper| setup.contains(&helper.name)) {
        return Err(format!(
            "{context}: {} is both setup and a dispatch helper",
            both.name
        ));
    }
    if let Some(both) = tests
        .iter()
        .find(|name| setup.contains(name) || dispatch.iter().any(|h| &h.name == *name))
    {
        return Err(format!(
            "{context}: {both} is in `tests` and also setup or a dispatch helper"
        ));
    }
    Ok(Wrapping {
        dispatch,
        setup,
        tests,
    })
}

fn parse_dispatch(value: &Value, context: &str) -> Result<Dispatch, String> {
    let table = value
        .as_inline_table()
        .ok_or_else(|| format!("{context}: a `dispatch` entry must be an inline table"))?;
    if let Some((key, _)) = table
        .iter()
        .find(|(key, _)| !["name", "args", "key", "prefix"].contains(key))
    {
        return Err(format!("{context}: unknown dispatch key `{key}`"));
    }
    let name = table
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| is_ident(name))
        .ok_or_else(|| format!("{context}: a dispatch entry needs a `name` that is a C name"))?
        .to_owned();
    let count = |key: &str| -> Result<usize, String> {
        table.get(key).map_or(Ok(1), |value| {
            value
                .as_integer()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|&n| n >= 1)
                .ok_or_else(|| format!("{context}: `{key}` of {name} must be a positive integer"))
        })
    };
    let args = count("args")?;
    let key = count("key")?;
    if key > args {
        return Err(format!(
            "{context}: `key` of {name} is {key}, past its {args} arguments"
        ));
    }
    let prefix = match table.get("prefix") {
        Some(value) => value
            .as_str()
            .filter(|prefix| prefix.strip_suffix(':').is_some_and(is_ident))
            .ok_or_else(|| {
                format!("{context}: `prefix` of {name} must be a C name followed by `:`")
            })?
            .to_owned(),
        None => String::new(),
    };
    Ok(Dispatch {
        name,
        args,
        key,
        prefix,
    })
}

/// Whether `name` is a C identifier.
#[must_use]
pub fn is_ident(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn check_keys(table: &Table, context: &str, known: &[&str]) -> Result<(), String> {
    match table.iter().find(|(key, _)| !known.contains(key)) {
        Some((key, _)) => Err(format!("{context}: unknown key `{key}`")),
        None => Ok(()),
    }
}

fn required<'t>(table: &'t Table, key: &str, context: &str) -> Result<&'t Item, String> {
    table
        .get(key)
        .ok_or_else(|| format!("{context}: missing `{key}`"))
}

fn tables<'t>(item: &'t Item, context: &str) -> Result<&'t ArrayOfTables, String> {
    item.as_array_of_tables()
        .ok_or_else(|| format!("{context}: expected an array of tables ([[...]])"))
}

fn string(table: &Table, key: &str, context: &str) -> Result<String, String> {
    required(table, key, context)?
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("{context}: `{key}` must be a non-empty string"))
}

fn ident(table: &Table, key: &str, context: &str) -> Result<String, String> {
    let value = string(table, key, context)?;
    if is_ident(&value) {
        Ok(value)
    } else {
        Err(format!("{context}: `{key}` {value:?} is not a C name"))
    }
}

fn unique<'n>(names: impl Iterator<Item = &'n str>, what: &str) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(format!("duplicate {what} {name}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
