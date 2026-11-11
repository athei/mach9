//! Per-function test selection in Wine's test sources.
//!
//! A Wine test unit runs every test function of its `START_TEST` body in one
//! process. For each wrapped unit of the [`Config`], a generated block right
//! before `START_TEST` defines function-like macros that shadow the body's
//! calls, so each test function runs only when `winetest_selected()` of
//! `include/wine/test.h` selects its key. The body itself stays as upstream
//! wrote it.
//!
//! [`check`] reports a block that is missing or out of date and every call it
//! cannot classify; [`write()`] writes the blocks.

mod lex;
mod scan;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    ops::Range,
    path::Path,
};

use scan::{Call, Source, StartTest};

use crate::model::{Config, Dispatch, Exe, Selection, Unit, Wrapping};

/// The first line of a generated block.
const BEGIN: &str =
    "/* Test selection through WT_LIST, WT_ONLY and WT_SKIP, generated: do not edit. */";
/// The last line of a generated block.
const END: &str = "/* End of the generated test selection. */";

/// The keys the source scan finds in one unit's `START_TEST` body.
///
/// This is not what `WT_LIST` prints: the scan reads every branch of an
/// `#if` and every conditional call and names each key once, while
/// `WT_LIST` prints the keys of the calls the build compiles and the run
/// reaches, once per call.
pub struct UnitKeys {
    pub exe: String,
    pub unit: String,
    /// The keys in source order of their first call; `None` for a unit that
    /// runs as a whole.
    pub keys: Option<Vec<String>>,
}

/// Check every block of the Wine source tree at `root` against `config`.
///
/// # Errors
///
/// Returns one message per problem, each naming the file and line: a file
/// or unit the config names that does not exist, a `START_TEST` the config
/// does not name, a local call that is neither wrapped nor setup, a setup or
/// dispatch helper the body does not call, a dispatch call that does not
/// match its entry, a call after the body that a block would shadow, and a
/// block that is missing, out of date, or present on a whole unit.
pub fn check(config: &Config, root: &Path) -> Result<Vec<UnitKeys>, Vec<String>> {
    let plans = plan(config, root)?;
    let stale: Vec<String> = plans.iter().filter_map(|plan| plan.stale.clone()).collect();
    if stale.is_empty() {
        Ok(plans.into_iter().map(|plan| plan.keys).collect())
    } else {
        Err(stale)
    }
}

/// Write or replace the block of every unit of `config` in the Wine source
/// tree at `root`, and remove the block of a whole unit.
///
/// Returns the files it changed, relative to `root`. Writes nothing unless
/// every unit can be generated, and leaves a file that is already up to date
/// untouched, so a second run changes nothing.
///
/// # Errors
///
/// Returns the messages of [`check`] other than those about the blocks
/// themselves, or the error of a file that cannot be written.
pub fn write(config: &Config, root: &Path) -> Result<Vec<String>, Vec<String>> {
    let plans = plan(config, root)?;
    let mut written = Vec::new();
    for plan in plans {
        if plan.current == plan.updated || written.contains(&plan.file) {
            continue;
        }
        fs::write(root.join(&plan.file), &plan.updated)
            .map_err(|e| vec![format!("{}: {e}", plan.file)])?;
        written.push(plan.file);
    }
    Ok(written)
}

/// What [`write()`] would make of one unit's file.
struct Plan {
    /// The file, relative to the source root.
    file: String,
    current: String,
    updated: String,
    /// Why `current` is out of date, if it is.
    stale: Option<String>,
    keys: UnitKeys,
}

/// Plan every unit of `config`, or return every problem found.
fn plan(config: &Config, root: &Path) -> Result<Vec<Plan>, Vec<String>> {
    let mut errors = Vec::new();
    let mut plans = Vec::new();
    for exe in &config.exes {
        check_units_listed(exe, root, &mut errors);
        for unit in &exe.units {
            if let Some(plan) = plan_unit(exe, unit, root, &mut errors) {
                plans.push(plan);
            }
        }
    }
    let mut by_file: BTreeMap<&str, &Plan> = BTreeMap::new();
    for plan in &plans {
        match by_file.get(plan.file.as_str()) {
            Some(first) if first.updated != plan.updated => errors.push(format!(
                "{}: units {}/{} and {}/{} share the file but generate different blocks",
                plan.file, first.keys.exe, first.keys.unit, plan.keys.exe, plan.keys.unit
            )),
            Some(_) => {}
            None => {
                by_file.insert(&plan.file, plan);
            }
        }
    }
    if errors.is_empty() {
        Ok(plans)
    } else {
        Err(errors)
    }
}

/// Compare the units the executable's `Makefile.in` builds with the
/// config: report every `START_TEST` of its `SOURCES` the config does not
/// name, and every configured unit its `SOURCES` do not build from the
/// configured file.
fn check_units_listed(exe: &Exe, root: &Path, errors: &mut Vec<String>) {
    let files = match source_files(exe, root) {
        Ok(files) => files,
        Err(e) => {
            errors.push(e);
            return;
        }
    };
    let mut built: Vec<(String, &str)> = Vec::new();
    for file in &files {
        let text = match fs::read_to_string(root.join(file)) {
            Ok(text) => text,
            Err(e) => {
                errors.push(format!("{file}: {e}"));
                continue;
            }
        };
        let source = match Source::new(&text) {
            Ok(source) => source,
            Err(e) => {
                if !exe.units.iter().any(|unit| unit.file == *file) {
                    errors.push(format!("{file}:{e}"));
                }
                continue;
            }
        };
        for test in source.start_tests() {
            if !exe.units.iter().any(|unit| unit.name == test.name) {
                errors.push(format!(
                    "{file}: START_TEST({}) has no unit under exe {} in wine-tests.toml",
                    test.name, exe.name
                ));
            }
            built.push((test.name, file));
        }
    }
    for unit in &exe.units {
        if !built
            .iter()
            .any(|(name, file)| *name == unit.name && *file == unit.file)
        {
            errors.push(format!(
                "{}/Makefile.in: its SOURCES do not build START_TEST({}) from {}, the file of \
                 unit {}/{}",
                exe.dir, unit.name, unit.file, exe.name, unit.name
            ));
        }
    }
}

/// The `.c` files of the `SOURCES` of the executable's `Makefile.in`,
/// relative to the source root, each found in `dir` or else in a directory
/// of `PARENTSRC`, as make finds them.
fn source_files(exe: &Exe, root: &Path) -> Result<Vec<String>, String> {
    let makefile = format!("{}/Makefile.in", exe.dir);
    let text = fs::read_to_string(root.join(&makefile)).map_err(|e| format!("{makefile}: {e}"))?;
    let text = text.replace("\\\n", " ");
    let variable = |name: &str| -> Vec<&str> {
        text.lines()
            .filter_map(|line| line.split_once('='))
            .find(|(key, _)| key.trim() == name)
            .map(|(_, value)| value.split_whitespace().collect())
            .unwrap_or_default()
    };
    let dirs: Vec<String> = std::iter::once(exe.dir.clone())
        .chain(
            variable("PARENTSRC")
                .into_iter()
                .map(|parent| join_relative(&exe.dir, parent)),
        )
        .collect();
    let mut files = Vec::new();
    for name in variable("SOURCES") {
        if Path::new(name).extension().is_none_or(|ext| ext != "c") {
            continue;
        }
        let file = dirs
            .iter()
            .map(|dir| format!("{dir}/{name}"))
            .find(|file| root.join(file).is_file())
            .ok_or_else(|| {
                format!(
                    "{makefile}: {name} of SOURCES is in none of {}",
                    dirs.join(", ")
                )
            })?;
        files.push(file);
    }
    Ok(files)
}

/// `dir/relative` with `.` and `..` components resolved, for paths below
/// the source root.
fn join_relative(dir: &str, relative: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|part| !part.is_empty()).collect();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

fn plan_unit(exe: &Exe, unit: &Unit, root: &Path, errors: &mut Vec<String>) -> Option<Plan> {
    match fs::read_to_string(root.join(&unit.file)) {
        Ok(current) => plan_source(&exe.name, unit, current, errors),
        Err(e) => {
            errors.push(format!(
                "{}: {e} (unit {}/{})",
                unit.file, exe.name, unit.name
            ));
            None
        }
    }
}

/// Plan one unit of the executable `exe` whose file holds `current`.
fn plan_source(exe: &str, unit: &Unit, current: String, errors: &mut Vec<String>) -> Option<Plan> {
    let file = unit.file.as_str();
    let source = match Source::new(&current) {
        Ok(source) => source,
        Err(e) => {
            errors.push(format!("{file}:{e}"));
            return None;
        }
    };
    let Some(test) = source
        .start_tests()
        .into_iter()
        .find(|test| test.name == unit.name)
    else {
        errors.push(format!(
            "{file}: no START_TEST({}) for unit {exe}/{}",
            unit.name, unit.name
        ));
        return None;
    };
    let line = source.lexed.line(test.token);
    let line_start = source.line_start(test.token);
    let block = match existing_block(&current, line_start) {
        Ok(block) => block,
        Err(e) => {
            errors.push(format!("{file}:{line}: {e}"));
            return None;
        }
    };
    let found = usize::from(block.is_some());
    if current.matches(BEGIN).count() != found {
        errors.push(format!(
            "{file}: a generated block that is not right before START_TEST({})",
            unit.name
        ));
        return None;
    }
    let replaced = block.clone().unwrap_or(line_start..line_start);
    let (inserted, keys) = match &unit.selection {
        Selection::Whole => (String::new(), None),
        Selection::Wrapped(wrapping) => {
            let generated = generate(&source, &test, &replaced, wrapping, file, errors)?;
            (generated.block, Some(generated.keys))
        }
    };
    let mut updated = String::with_capacity(current.len() + inserted.len());
    updated.push_str(&current[..replaced.start]);
    updated.push_str(&inserted);
    updated.push_str(&current[replaced.end..]);
    let stale = (updated != current).then(|| match (&unit.selection, block) {
        (Selection::Whole, _) => format!(
            "{file}:{line}: START_TEST({}) runs as a whole but has a generated block",
            unit.name
        ),
        (Selection::Wrapped(_), None) => format!(
            "{file}:{line}: no generated block before START_TEST({})",
            unit.name
        ),
        (Selection::Wrapped(_), Some(_)) => format!(
            "{file}:{line}: the block before START_TEST({}) differs from what --write generates",
            unit.name
        ),
    });
    Some(Plan {
        file: file.to_owned(),
        current,
        updated,
        stale,
        keys: UnitKeys {
            exe: exe.to_owned(),
            unit: unit.name.clone(),
            keys,
        },
    })
}

/// The lines of the source before `end`, which is the start of a line, from
/// the nearest: each line's start and text.
fn lines_before(src: &str, end: usize) -> impl Iterator<Item = (usize, &str)> {
    let mut cursor = end;
    std::iter::from_fn(move || {
        if cursor == 0 {
            return None;
        }
        let text_end = cursor - 1;
        let start = src[..text_end].rfind('\n').map_or(0, |newline| newline + 1);
        cursor = start;
        Some((start, &src[start..text_end]))
    })
}

/// The byte range of the generated block that ends right before the line at
/// `line_start`, with at most one blank line between them.
fn existing_block(src: &str, line_start: usize) -> Result<Option<Range<usize>>, String> {
    let mut lines = lines_before(src, line_start).peekable();
    if lines.next_if(|(_, text)| text.is_empty()).is_some() && lines.peek().is_none() {
        return Ok(None);
    }
    if lines.next_if(|(_, text)| *text == END).is_none() {
        return Ok(None);
    }
    for (start, text) in lines {
        if text == BEGIN {
            return Ok(Some(start..line_start));
        }
    }
    Err(String::from(
        "the end of a generated block without its beginning",
    ))
}

/// A unit's generated block and its keys.
struct Generated {
    /// The block with a blank line after it, ready to put before the
    /// `START_TEST` line.
    block: String,
    keys: Vec<String>,
}

/// Generate the block of a wrapped unit, or report why the body cannot be
/// wrapped. `block` is the byte range of the block the file has now.
fn generate(
    source: &Source<'_>,
    test: &StartTest,
    block: &Range<usize>,
    wrapping: &Wrapping,
    file: &str,
    errors: &mut Vec<String>,
) -> Option<Generated> {
    let errors_before = errors.len();
    let calls = classify_calls(source, test, block, wrapping, file, errors);
    for name in &wrapping.setup {
        if !calls.called.contains(name.as_str()) {
            errors.push(format!(
                "{file}: setup {name} is not a local function or macro START_TEST({}) calls",
                test.name
            ));
        }
    }
    for name in &wrapping.tests {
        if !calls.direct.contains(&name.as_str()) {
            errors.push(format!(
                "{file}: test {name} is not a local function START_TEST({}) calls as a \
                 statement",
                test.name
            ));
        }
    }
    for helper in &wrapping.dispatch {
        if !calls.called.contains(helper.name.as_str()) {
            errors.push(format!(
                "{file}: dispatch helper {} is not a local function START_TEST({}) calls",
                helper.name, test.name
            ));
        }
    }
    let shadowed: BTreeSet<&str> = wrapping
        .dispatch
        .iter()
        .map(|helper| helper.name.as_str())
        .chain(calls.direct.iter().copied())
        .collect();
    check_after_body(source, test, &shadowed, file, errors);
    if errors.len() > errors_before {
        return None;
    }
    let lines: Vec<String> = std::iter::once(String::from(BEGIN))
        .chain(wrapping.dispatch.iter().map(dispatch_macro))
        .chain(calls.direct.iter().map(|name| direct_macro(name)))
        .chain(std::iter::once(String::from(END)))
        .collect();
    Some(Generated {
        block: format!("{}\n\n", lines.join("\n")),
        keys: calls.keys,
    })
}

/// What the local calls of a `START_TEST` body are.
struct Calls<'a> {
    /// Every local function the body calls.
    called: BTreeSet<&'a str>,
    /// The keys, in the order the body first runs them.
    keys: Vec<String>,
    /// The test functions the body calls directly, in source order.
    direct: Vec<&'a str>,
}

/// Classify every call of a local function or function-like macro of the
/// file in the body as setup, a dispatch helper or a direct test call, and
/// report each call that is none of them. The macros the generated block at
/// `block` defines do not count.
fn classify_calls<'a>(
    source: &Source<'a>,
    test: &StartTest,
    block: &Range<usize>,
    wrapping: &Wrapping,
    file: &str,
    errors: &mut Vec<String>,
) -> Calls<'a> {
    let local = source.local_functions();
    let macros: BTreeSet<&str> = source
        .lexed
        .macros
        .iter()
        .filter(|definition| !block.contains(&definition.offset))
        .map(|definition| definition.name)
        .collect();
    let mut calls = Calls {
        called: BTreeSet::new(),
        keys: Vec::new(),
        direct: Vec::new(),
    };
    for call in source.calls(test.open, test.close) {
        let is_macro = macros.contains(call.name);
        if !is_macro && !local.contains(call.name) {
            continue;
        }
        calls.called.insert(call.name);
        if wrapping.setup.iter().any(|name| name == call.name) {
            continue;
        }
        let at = format!("{file}:{}: {}()", call.line, call.name);
        if is_macro {
            if call.statement {
                errors.push(format!(
                    "{at}: neither wrapped nor setup: a function-like macro of the file, which \
                     may run a test that no key selects; name it as setup"
                ));
            }
            continue;
        }
        let key = if let Some(helper) = wrapping.dispatch.iter().find(|h| h.name == call.name) {
            match dispatch_key(source, &local, &call, helper) {
                Ok(key) => key,
                Err(e) => {
                    errors.push(format!("{at}: {e}"));
                    continue;
                }
            }
        } else {
            if let Some(e) = direct_error(source, &local, &call, wrapping) {
                errors.push(format!("{at}: neither wrapped nor setup: {e}"));
                continue;
            }
            if !calls.direct.contains(&call.name) {
                calls.direct.push(call.name);
            }
            call.name.to_owned()
        };
        if !calls.keys.contains(&key) {
            calls.keys.push(key);
        }
    }
    calls
}

/// Why a call of a local function that is neither setup nor a dispatch
/// helper cannot be wrapped as a direct test call, if it cannot.
fn direct_error(
    source: &Source<'_>,
    local: &BTreeSet<&str>,
    call: &Call<'_>,
    wrapping: &Wrapping,
) -> Option<String> {
    if !call.statement {
        return Some(String::from(
            "the call's value is used, so it cannot be wrapped; name it as setup",
        ));
    }
    if let Some(function) = call
        .args
        .iter()
        .filter_map(|arg| source.single_ident(arg))
        .find(|arg| local.contains(arg))
    {
        return Some(format!(
            "it takes the local function {function}; name it as a dispatch helper or as setup"
        ));
    }
    if wrapping.tests.iter().any(|name| name == call.name) {
        return None;
    }
    if !(call.name.starts_with("test_") || call.name.ends_with("_test")) {
        return Some(String::from(
            "the name is not test_* or *_test; name it in `tests` or as setup",
        ));
    }
    let lexed = &source.lexed;
    if lexed.text(call.end + 2) == "return" && lexed.text(call.end + 3) == ";" {
        return Some(String::from(
            "`return;` follows the call, the shape of a child process entry point; name it as \
             setup, or in `tests` if it is a test",
        ));
    }
    None
}

/// The key of a call of a dispatch helper: the prefix and the name of the
/// local test function it passes.
fn dispatch_key(
    source: &Source<'_>,
    local: &BTreeSet<&str>,
    call: &Call<'_>,
    helper: &Dispatch,
) -> Result<String, String> {
    if !call.statement {
        return Err(String::from(
            "the value of this dispatch call is used, so it cannot be wrapped",
        ));
    }
    if call.args.len() != helper.args {
        return Err(format!(
            "called with {} arguments, wine-tests.toml says {}",
            call.args.len(),
            helper.args
        ));
    }
    source
        .single_ident(&call.args[helper.key - 1])
        .filter(|function| local.contains(function))
        .map(|function| format!("{}{function}", helper.prefix))
        .ok_or_else(|| {
            format!(
                "argument {} is not a function the file defines, so it cannot be the key",
                helper.key
            )
        })
}

/// Report every call after the `START_TEST` body of a name the block
/// shadows: the macro would apply to it too.
fn check_after_body(
    source: &Source<'_>,
    test: &StartTest,
    shadowed: &BTreeSet<&str>,
    file: &str,
    errors: &mut Vec<String>,
) {
    let lexed = &source.lexed;
    for index in test.close + 1..lexed.tokens.len() {
        if lexed.is_ident(index)
            && lexed.text(index + 1) == "("
            && shadowed.contains(lexed.text(index))
        {
            errors.push(format!(
                "{file}:{}: {}() after START_TEST({}) would expand the generated macro",
                lexed.line(index),
                lexed.text(index),
                test.name
            ));
        }
    }
}

/// The macro that shadows a direct call of a test function.
fn direct_macro(name: &str) -> String {
    format!(
        "#define {name}(...) do {{ if (winetest_selected(\"{name}\")) {name}(__VA_ARGS__); }} \
         while (0)"
    )
}

/// The macro that shadows a dispatch helper: it runs the helper only when the
/// key, the prefix and the test function's name, is selected.
fn dispatch_macro(helper: &Dispatch) -> String {
    let params: Vec<String> = (1..=helper.args)
        .map(|position| {
            if position == helper.key {
                String::from("f")
            } else {
                format!("a{position}")
            }
        })
        .collect();
    let params = params.join(", ");
    let key = if helper.prefix.is_empty() {
        String::from("#f")
    } else {
        format!("\"{}\" #f", helper.prefix)
    };
    let name = &helper.name;
    format!(
        "#define {name}({params}) do {{ if (winetest_selected({key})) {name}({params}); }} \
         while (0)"
    )
}

#[cfg(test)]
mod tests;
