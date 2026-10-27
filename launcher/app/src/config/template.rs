//! The templates of the config files, and keeping a config file in step with
//! its template: topping it up with the options it does not mention yet, and
//! writing a value under the comment that explains it.

use std::{collections::BTreeMap, io, path::Path};

use compatdb_core::{DocSection, Key, KeyKind, ScalarValue};
use gamelauncher_core::fsutil::write_atomic;
use log::info;
use toml_edit::Item;

use crate::words;

/// The width of a comment line the expanded templates wrap their text at.
const WIDTH: usize = 79;

/// The text of a template with its directives expanded from compatdb-core,
/// so that the template names none of the rules' keys, values or links
/// itself. A directive is a line `#@ <what>`:
///
/// - `keys`: the sentence that lists the keys `--config` takes.
/// - `settings`: one block per key of the rules the top level takes, its
///   summary, the link to its documentation and an example assignment.
/// - `doc <section>`: the link to that [`DocSection`] of the crate's docs.
/// - `env`: an example entry of the `[env]` table.
/// - `rule`: the fields of the example `[process]` rule, a key of each kind.
///
/// In any other line, [`EXE`] stands for the matcher key [`Key::Exe`] in
/// backticks.
pub(super) fn expand(template: &str) -> String {
    let exe = format!("`{}`", Key::Exe);
    let mut text = String::with_capacity(template.len().saturating_mul(2));
    for line in template.lines() {
        if let Some(directive) = line.strip_prefix("#@ ") {
            text.push_str(&directive_text(directive));
        } else {
            text.push_str(&line.replace(EXE, &exe));
            text.push('\n');
        }
    }
    text
}

/// The placeholder of [`expand`] for the matcher key.
const EXE: &str = "{exe}";

/// The lines a directive of [`expand`] stands for.
fn directive_text(directive: &str) -> String {
    let (what, argument) = directive.split_once(' ').unwrap_or((directive, ""));
    match what {
        "keys" => comment(&format!(
            "Supported keys are {}.",
            words::listing(&super::command_line_keys(), "")
        )),
        "settings" => super::launch_wide_keys()
            .into_iter()
            .map(setting_block)
            .collect::<Vec<_>>()
            .join("\n"),
        "doc" => {
            let section = DocSection::ALL
                .iter()
                .find(|section| format!("{section:?}") == argument)
                .unwrap_or_else(|| unreachable!("no doc section {argument}"));
            format!("# {}\n", section.url())
        }
        "env" => {
            let (name, value) = env_example();
            format!("# {}\n", text_assignment(name, value))
        }
        "rule" => rule_example()
            .into_iter()
            .fold(String::new(), |mut lines, key| {
                lines.push_str("# ");
                lines.push_str(&assignment(key));
                lines.push('\n');
                lines
            }),
        _ => unreachable!("no template directive {directive}"),
    }
}

/// The block of a key of the rules: its summary, its link and an example.
fn setting_block(key: Key) -> String {
    format!(
        "{}# {}\n# {}\n",
        comment(key.summary()),
        key.docs_url(),
        assignment(key)
    )
}

/// `text` as comment lines, wrapped at [`WIDTH`].
fn comment(text: &str) -> String {
    let mut lines = String::new();
    let mut line = String::from("#");
    for word in text.split_whitespace() {
        if line.len() > 1 && line.len() + 1 + word.len() > WIDTH {
            lines.push_str(&line);
            lines.push('\n');
            line = String::from("#");
        }
        line.push(' ');
        line.push_str(word);
    }
    lines.push_str(&line);
    lines.push('\n');
    lines
}

/// The name and the value of the crate's example environment entry.
fn env_example() -> (&'static str, &'static str) {
    let example = Key::Env.example();
    example.split_once('=').unwrap_or((example, ""))
}

/// `name = "value"`.
fn text_assignment(name: &str, value: &str) -> String {
    format!("{name} = {}", toml_edit::Value::from(value))
}

/// The field `key` with the crate's example value, as a file writes it: a
/// boolean, a list of one entry, an `env` entry under its name, or a string.
pub(super) fn assignment(key: Key) -> String {
    let value = key.example();
    match key.kind() {
        KeyKind::List => {
            let array: toml_edit::Array = std::iter::once(value).collect();
            format!("{key} = {array}")
        }
        KeyKind::Env => {
            let (name, value) = env_example();
            text_assignment(&format!("{key}.{name}"), value)
        }
        _ => match key
            .scalar()
            .and_then(|scalar| scalar.parse_value(value).ok())
        {
            Some(ScalarValue::Bool(on)) => format!("{key} = {on}"),
            _ => text_assignment(key.as_str(), value),
        },
    }
}

/// The keys of the example rule: [`Key::Exe`], one fingerprint, and the first
/// key of each other kind.
fn rule_example() -> Vec<Key> {
    let mut kinds = Vec::new();
    let mut matchers = 0_u8;
    Key::ALL
        .iter()
        .copied()
        .filter(|key| {
            let kind = key.kind();
            if kind == KeyKind::Matcher {
                matchers = matchers.saturating_add(1);
                return matchers <= 2;
            }
            if kinds.contains(&kind) {
                return false;
            }
            kinds.push(kind);
            true
        })
        .collect()
}

/// Append the template's options the file does not mention yet, and write the
/// result back when there were any. Returns the file's text.
pub(super) fn top_up(path: &Path, text: String, template: &str) -> io::Result<String> {
    match sync_template(&text, template) {
        Some(synced) => {
            write_atomic(path, &synced.text)?;
            info!(
                "added options to {}: {}",
                path.display(),
                synced.added.join(", ")
            );
            Ok(synced.text)
        }
        None => Ok(text),
    }
}

/// Write the path into a game file, see `set_option`.
pub(super) fn set_executable(text: &str, exe: &Path) -> String {
    set_option(
        text,
        "executable",
        toml_edit::value(exe.to_str().expect("resolve_exe rejects non-UTF-8")),
    )
}

pub(super) fn set_arguments(text: &str, arguments: &[String]) -> String {
    let array: toml_edit::Array = arguments.iter().map(String::as_str).collect();
    set_option(text, "arguments", toml_edit::value(array))
}

/// Assign an option in a game file, replacing the line that mentions it
/// wherever it sits, commented out or not, so the value ends up under the
/// comment that explains it rather than at the top of the file.
fn set_option(text: &str, key: &str, item: Item) -> String {
    // Through `toml_edit` for the TOML quoting and escaping, not by hand.
    let mut table = toml_edit::Table::new();
    table[key] = item;
    let assignment = table.to_string();
    let assignment = assignment.trim_end();

    let Some(line_no) = text.lines().position(|line| is_option(line, key)) else {
        // Nothing to replace: the user deleted the block, so add one back.
        return insert_blocks(text, &[assignment]);
    };
    let mut out = String::with_capacity(text.len() + assignment.len());
    for (i, line) in text.lines().enumerate() {
        out.push_str(if i == line_no { assignment } else { line });
        out.push('\n');
    }
    out
}

/// Add an `[env]` table at the end of a game file. The template's commented
/// example stays above it, and `sync_template` counts both as mentioning
/// `env`, so nothing gets added twice.
pub(super) fn append_env(text: &str, env: &BTreeMap<String, String>) -> String {
    let mut table = toml_edit::Table::new();
    for (name, value) in env {
        table[name.as_str()] = toml_edit::value(value.as_str());
    }
    format!("{}\n\n[env]\n{table}", text.trim_end())
}

pub(super) struct Synced {
    text: String,
    added: Vec<String>,
}

/// Append the options an existing config does not mention yet, taking their
/// comments along, so the file keeps documenting every option the launcher
/// knows. Works on text rather than through `toml_edit` because what is carried
/// over are comments, which the document model does not track as data.
///
/// Both files are read as blocks of consecutive non-blank lines; a block
/// belongs to the first option it assigns or the first table it opens,
/// commented out or not. New blocks go in ahead of the first table header so
/// they cannot end up inside a table the user has written.
pub(super) fn sync_template(existing: &str, template: &str) -> Option<Synced> {
    let missing: Vec<&str> = blocks(template)
        .filter(|block| block_option(block).is_some_and(|option| !mentions(existing, option)))
        .collect();
    if missing.is_empty() {
        return None;
    }
    let added = missing
        .iter()
        .filter_map(|block| block_option(block))
        .map(str::to_owned)
        .collect();
    Some(Synced {
        text: insert_blocks(existing, &missing),
        added,
    })
}

/// Add blocks ahead of the first table header, so a top-level key can never end
/// up inside a table the user has written.
fn insert_blocks(existing: &str, blocks: &[&str]) -> String {
    let lines: Vec<&str> = existing.lines().collect();
    let cut = lines
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .unwrap_or(lines.len());

    let mut text = String::with_capacity(existing.len());
    for line in &lines[..cut] {
        text.push_str(line);
        text.push('\n');
    }
    if !text.is_empty() && !text.ends_with("\n\n") {
        text.push('\n');
    }
    for block in blocks {
        text.push_str(block);
        text.push_str("\n\n");
    }
    for line in &lines[cut..] {
        text.push_str(line);
        text.push('\n');
    }
    while text.ends_with("\n\n") {
        text.pop();
    }
    text
}

/// Blocks of consecutive non-blank lines, blank lines dropped.
fn blocks(text: &str) -> impl Iterator<Item = &str> {
    text.split("\n\n")
        .map(|block| block.trim_matches('\n'))
        .filter(|block| !block.is_empty())
}

/// The options a template declares, one per block, in order.
#[cfg(test)]
fn declared_options(template: &str) -> impl Iterator<Item = &str> {
    blocks(template).filter_map(block_option)
}

fn block_option(block: &str) -> Option<&str> {
    block.lines().find_map(option_name)
}

/// Whether `text` mentions the top-level `option`: a line that assigns it or
/// opens a table of that name, commented out or not. The lines after a table
/// header belong to that table, so they do not count.
///
/// After a commented-out header, such as the example rule of a template, that
/// lasts up to the end of its block or the first line that is not commented
/// out, which a commented-out header cannot own. After a real header it lasts
/// up to the end of the file, as in TOML, where every key after a header
/// belongs to a table. A config file's tables are at its end, where the
/// top-up leaves them. A real table early in a file hides the template
/// blocks after it, since TOML would read an option uncommented there as the
/// table's, and the top-up adds those blocks again above it.
pub(super) fn mentions(text: &str, option: &str) -> bool {
    let mut in_table = false;
    let mut in_example_table = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            in_example_table = false;
            continue;
        }
        let commented = line.trim_start().starts_with('#');
        if opens_table(line) {
            if commented {
                in_example_table = true;
            } else {
                in_table = true;
            }
            if is_option(line, option) {
                return true;
            }
            continue;
        }
        if !commented {
            in_example_table = false;
        }
        if !in_table && !in_example_table && is_option(line, option) {
            return true;
        }
    }
    false
}

/// Whether `line` assigns or opens `option`. A key of the rules matches in any
/// case, as the settings layer reads it; any other option as written.
pub(super) fn is_option(line: &str, option: &str) -> bool {
    option_name(line).is_some_and(|name| {
        name == option
            || matches!(
                (Key::from_setting(name), Key::from_setting(option)),
                (Ok(one), Ok(other)) if one == other
            )
    })
}

/// Whether a line is a table header, commented out or not. The header has to
/// be all of the line but a trailing comment, so `[process."x"] # mine` is
/// one and a comment that only starts with a bracket, such as
/// `# ["d3dcompiler_47=n"] to use ...`, is none.
fn opens_table(line: &str) -> bool {
    let text = without_comment(line.trim_start().trim_start_matches('#')).trim();
    text.starts_with('[') && text.ends_with(']') && option_name(line).is_some()
}

/// `text` up to a `#` outside double quotes, which starts a TOML comment.
fn without_comment(text: &str) -> &str {
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &text[..i],
            _ => {}
        }
    }
    text
}

/// The option a line assigns (`key = …`) or opens (`[table]`), whether or not
/// the line is commented out.
fn option_name(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let line = line.strip_prefix('#').unwrap_or(line).trim_start();
    if let Some(rest) = line.strip_prefix('[') {
        let inner = rest.split(']').next()?.trim();
        // Tolerate an array-of-tables `[[name]]` and a dotted/quoted header
        // such as `[process."my-rule"]`, whose option is the first segment.
        let inner = inner.strip_prefix('[').unwrap_or(inner);
        let head = inner.split('.').next()?.trim().trim_matches('"');
        return is_key(head).then_some(head);
    }
    let name = line.split_once('=')?.0.trim();
    is_key(name).then_some(name)
}

fn is_key(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests;
