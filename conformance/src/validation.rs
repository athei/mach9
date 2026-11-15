//! Khronos' Vulkan validation layers over the test processes, for `run
//! --validation` and `deqp-vk run --validation`.
//!
//! The layers are those `bin/build-vulkan-validation-layers.sh` staged at
//! the pin of `pins/redist.env` ([`Layers`]). Each test process gets
//! [`env()`]: the loader inserts the layer into every instance and logs each
//! instance it creates, so a process whose instances ran without the layer
//! is found ([`Scan::without_layer`]). The layer prints each message on
//! the standard output in the format of its default callback
//! (`CreateDefaultCallbackMessage` in its `layers/error_message/logging.cpp`):
//!
//! ```text
//! Validation Error: [ VUID-vkCmdBeginQuery-queryPool-01922 ] | MessageID = 0x8c4a2b9c
//! vkCmdBeginQuery(): queryPool ... The Vulkan spec states: ...
//! Objects: 1
//!     [0] VkCommandBuffer 0x...
//! ```
//!
//! [`scan`] reads them from a process's output and attributes each to the
//! deqp-vk case that was running; a Wine test process runs one key, which
//! the caller names. Errors and warnings count alike: a message gates
//! unless the committed list of known messages ([`Known`]) names its test
//! and message ID, as `triage.txt` names known failures.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};

use crate::deqp::groups::glob;

/// The name of the layer.
pub const LAYER: &str = "VK_LAYER_KHRONOS_validation";

/// The name of the list of known messages beside the triage file.
pub const KNOWN_FILE: &str = "validation.txt";

/// The file in a run's log directory that lists each message of the run by
/// test ([`Report::detail`]).
pub const MESSAGES_FILE: &str = "validation-messages.txt";

/// The variable of `pins/redist.env` that pins the layers' commit.
const PIN: &str = "VULKAN_VALIDATION_LAYERS_GIT_COMMIT";

/// What the loader logs, with its layer messages on, for every
/// `vkCreateInstance`, with or without layers (`loader_create_instance_chain`
/// in its `loader/loader.c`).
const INSTANCE_LINE: &str = "vkCreateInstance layer callstack setup to:";

/// What it logs for each layer it inserts into an instance, followed by the
/// quoted layer name.
const INSERT_LINE: &str = "Insert instance layer \"";

/// The headings of a message, by the severity they report.
const HEADINGS: [(&str, Severity); 3] = [
    ("Validation Error: [ ", Severity::Error),
    ("Validation Warning: [ ", Severity::Warning),
    ("Validation Performance Warning: [ ", Severity::Warning),
];

/// The staged layers.
pub struct Layers {
    pub dir: PathBuf,
    /// The directory of the layer's manifest, `VK_LAYER_PATH`.
    pub manifest_dir: PathBuf,
    /// The commit they were built from, `COMMIT`.
    pub commit: String,
}

impl Layers {
    /// The layers `bin/build-vulkan-validation-layers.sh` staged in `dir`,
    /// made absolute, built from `pinned`.
    ///
    /// # Errors
    ///
    /// Returns a message when the library, the manifest or `COMMIT` is
    /// missing, or `COMMIT` is not `pinned`.
    pub fn open(dir: &Path, pinned: &str) -> Result<Self, String> {
        let dir = std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let manifest_dir = dir.join("share").join("vulkan").join("explicit_layer.d");
        let build = "bin/build-vulkan-validation-layers.sh builds them";
        for path in [
            dir.join("lib").join("libVkLayer_khronos_validation.dylib"),
            manifest_dir.join("VkLayer_khronos_validation.json"),
        ] {
            if !path.is_file() {
                return Err(format!("{} is missing; {build}", path.display()));
            }
        }
        let path = dir.join("COMMIT");
        let commit = fs::read_to_string(&path)
            .map_err(|e| format!("{}: {e}; {build}", path.display()))?
            .trim()
            .to_owned();
        if commit != pinned {
            return Err(format!(
                "{} holds the validation layers of {commit}, not of the pinned {pinned}; {build}",
                dir.display()
            ));
        }
        Ok(Self {
            dir,
            manifest_dir,
            commit,
        })
    }
}

/// The commit `redist`, the text of `pins/redist.env`, pins the layers at.
#[must_use]
pub fn pinned_commit(redist: &str) -> Option<&str> {
    redist.lines().find_map(|line| {
        let value = line.trim().strip_prefix(PIN)?.strip_prefix('=')?.trim();
        (!value.is_empty()).then_some(value)
    })
}

/// The pinned commit of the layers, from this checkout's `pins/redist.env`.
///
/// # Errors
///
/// Returns a message when the file cannot be read or has no pin.
pub fn pin() -> Result<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("pins")
        .join("redist.env");
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    pinned_commit(&text)
        .map(str::to_owned)
        .ok_or_else(|| format!("{} has no {PIN}", path.display()))
}

/// The settings file the layers read: their defaults, without the message
/// limit.
///
/// The defaults print errors and warnings on the standard output. With the
/// limit, the layer reports a message ID ten times per instance and then no
/// more, so a later case of a `deqp-vk` process that sends it, which a
/// known message may not name, would go unseen.
pub const SETTINGS: &str = "khronos_validation.enable_message_limit = false\n";

/// Write [`SETTINGS`] into `dir`, a run's log directory, and return [`env()`]
/// with it; print that `what` runs under `layers`. With the file, no
/// `vk_layer_settings.txt` in a working directory can change the settings.
///
/// # Errors
///
/// Returns a message when the file cannot be written.
pub fn start(layers: &Layers, dir: &Path, what: &str) -> Result<Vec<(OsString, OsString)>, String> {
    let settings = dir.join("vk_layer_settings.txt");
    fs::write(&settings, SETTINGS).map_err(|e| format!("{}: {e}", settings.display()))?;
    println!(
        "conformance: {what} run under the validation layers of {} in {}",
        layers.commit,
        layers.dir.display()
    );
    Ok(env(layers, &settings))
}

/// The variables a test process gets on top of its environment, which has
/// none of them:
///
/// - `VK_LAYER_PATH` the layers' manifest directory, the only place the
///   loader looks for explicit layers;
/// - `VK_INSTANCE_LAYERS` the layer, which the loader then inserts into
///   every instance;
/// - `VK_LOADER_LAYERS_ALLOW` the layer, without which the
///   `VK_LOADER_LAYERS_DISABLE=~all~` of the deqp-vk processes disables it
///   in spite of `VK_INSTANCE_LAYERS` (`loader_layer_is_available` in the
///   loader's `loader/loader.c`);
/// - `VK_LAYER_SETTINGS_PATH` the [`SETTINGS`] file of [`start`];
/// - `VK_LOADER_DEBUG=layer`, so the loader logs each instance and the
///   layers it inserts ([`scan`]).
#[must_use]
pub fn env(layers: &Layers, settings: &Path) -> Vec<(OsString, OsString)> {
    vec![
        (
            OsString::from("VK_LAYER_PATH"),
            layers.manifest_dir.clone().into_os_string(),
        ),
        (OsString::from("VK_INSTANCE_LAYERS"), OsString::from(LAYER)),
        (
            OsString::from("VK_LOADER_LAYERS_ALLOW"),
            OsString::from(LAYER),
        ),
        (
            OsString::from("VK_LAYER_SETTINGS_PATH"),
            settings.as_os_str().to_owned(),
        ),
        (OsString::from("VK_LOADER_DEBUG"), OsString::from("layer")),
    ]
}

/// How bad a message is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

/// One message of the layer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Message {
    pub severity: Severity,
    /// The message ID, a VUID or one of the layer's own names.
    pub id: String,
    /// The first line of its text.
    pub text: String,
    /// The deqp-vk case that was running, the last one whose `Test case`
    /// line came before it; `None` before the first.
    pub case: Option<String>,
}

/// What one process's output says about the layer.
#[derive(Default, PartialEq, Eq, Debug)]
pub struct Scan {
    /// The instances the loader created.
    pub instances: usize,
    /// The times it inserted the layer into one.
    pub with_layer: usize,
    pub messages: Vec<Message>,
}

impl Scan {
    /// The instances that ran without the layer.
    #[must_use]
    pub const fn without_layer(&self) -> usize {
        self.instances.saturating_sub(self.with_layer)
    }
}

/// Read the output `text` of a process that ran with [`env()`].
///
/// A message is its heading line and every line up to the next empty one.
/// A line outside a message that names a VUID counts as an error with that
/// VUID, so a heading a later release of the layers changes cannot hide
/// one.
#[must_use]
pub fn scan(text: &str) -> Scan {
    let mut scan = Scan::default();
    let mut case: Option<String> = None;
    // Whether the lines belong to a message, and whether it has its text.
    let mut open: Option<bool> = None;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(name) = line
            .strip_prefix("Test case '")
            .and_then(|rest| rest.strip_suffix("'.."))
        {
            case = Some(name.to_owned());
            open = None;
            continue;
        }
        if line.contains(INSTANCE_LINE) {
            scan.instances += 1;
        }
        if let Some(at) = line.find(INSERT_LINE)
            && line[at + INSERT_LINE.len()..].starts_with(&format!("{LAYER}\""))
        {
            scan.with_layer += 1;
        }
        if let Some((severity, id)) = heading(line) {
            scan.messages.push(Message {
                severity,
                id,
                text: String::new(),
                case: case.clone(),
            });
            open = Some(false);
            continue;
        }
        match open {
            Some(_) if line.is_empty() => open = None,
            Some(false) => {
                if let Some(message) = scan.messages.last_mut() {
                    line.clone_into(&mut message.text);
                }
                open = Some(true);
            }
            Some(true) => {}
            None => {
                if let Some(id) = vuid(line) {
                    scan.messages.push(Message {
                        severity: Severity::Error,
                        id,
                        text: line.to_owned(),
                        case: case.clone(),
                    });
                }
            }
        }
    }
    scan
}

/// The severity and message ID of a message's heading in `line`.
///
/// The heading may start anywhere in the line: a Wine test prints a line in
/// several writes to an unbuffered stdout, and the layer prints a whole
/// message at once, often on another thread, so a message can begin inside
/// a test's line.
fn heading(line: &str) -> Option<(Severity, String)> {
    HEADINGS.iter().find_map(|(prefix, severity)| {
        let at = line.find(prefix)?;
        let (id, _) = line[at + prefix.len()..].split_once(" ]")?;
        (!id.is_empty()).then(|| (*severity, id.to_owned()))
    })
}

/// The first VUID `line` names.
fn vuid(line: &str) -> Option<String> {
    let at = line.find("VUID-")?;
    let id: String = line[at..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    Some(id)
}

/// One line of the list of known messages.
#[derive(PartialEq, Eq, Debug)]
pub struct Entry {
    /// A pattern over test names ([`glob`]).
    pub test: String,
    /// A pattern over message IDs.
    pub id: String,
    /// The issue that tracks it.
    pub issue: Option<u32>,
    /// Its 1-based line in the file.
    pub line: usize,
}

/// The human-owned list of known messages, `validation.txt` beside the
/// triage file: a message whose test and ID an entry matches does not gate.
///
/// A line is a pattern over test names, one over message IDs (`*` matches
/// any run of characters in both) and optionally `issue=<n>`; `#` starts a
/// comment, a whole line or the rest of one:
///
/// ```text
/// opengl32/opengl/* Undefined-Value-ShaderOutputNotConsumed-DynamicRendering  # Zink's blits
/// dEQP-VK.draw.* VUID-vkCmdDraw-None-08600 issue=<n>
/// ```
///
/// A Wine test is named `<exe>/<unit>/<key>` (`<exe>/<unit>/listing` for
/// the listing of a unit's keys), a deqp-vk case by its name, and what a
/// `deqp-vk` process reports before its first case `startup`.
#[derive(Default)]
pub struct Known {
    pub entries: Vec<Entry>,
}

impl Known {
    /// Read the list's text; `file` names it in errors.
    ///
    /// # Errors
    ///
    /// Returns `<file>:<line>: <message>` for a line without a message ID
    /// pattern, a pair of patterns given twice, a token other than
    /// `issue=<n>`, and two issues on one line.
    pub fn parse(text: &str, file: &str) -> Result<Self, String> {
        let mut entries: Vec<Entry> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let number = index + 1;
            let at = |e: String| format!("{file}:{number}: {e}");
            let content = line.split_once('#').map_or(line, |(before, _)| before);
            let mut tokens = content.split_whitespace();
            let Some(tests) = tokens.next() else {
                continue;
            };
            let id = tokens
                .next()
                .ok_or_else(|| at(format!("{tests} has no message ID pattern")))?;
            if let Some(first) = entries
                .iter()
                .find(|entry| entry.test == tests && entry.id == id)
            {
                return Err(at(format!(
                    "{tests} {id} again, first on line {}",
                    first.line
                )));
            }
            let mut issue = None;
            for token in tokens {
                let number = token
                    .strip_prefix("issue=")
                    .ok_or_else(|| {
                        at(format!(
                            "unknown token {token:?}; a known message takes `issue=<n>`"
                        ))
                    })?
                    .parse::<u32>()
                    .ok()
                    .filter(|&n| n > 0)
                    .ok_or_else(|| at(format!("{token:?} is not an issue number")))?;
                if issue.replace(number).is_some() {
                    return Err(at(String::from("two issues")));
                }
            }
            entries.push(Entry {
                test: tests.to_owned(),
                id: id.to_owned(),
                issue,
                line: number,
            });
        }
        Ok(Self { entries })
    }

    /// Read the list at `path`; no file is an empty list.
    ///
    /// # Errors
    ///
    /// Returns a message when it cannot be read or does not parse.
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, &path.display().to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// The index of the first entry that matches the message `id` of
    /// `test`.
    #[must_use]
    pub fn find(&self, test: &str, id: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| glob(&entry.test, test) && glob(&entry.id, id))
    }
}

/// A message of a test, with where it was seen.
#[derive(Clone, Debug)]
pub struct Occurrence {
    /// The name the list of known messages matches ([`Known`]).
    pub test: String,
    /// The test as the report shows it, such as with its leg.
    pub label: String,
    /// The raw log that holds it.
    pub log: PathBuf,
    pub message: Message,
}

/// What [`report`] concludes.
pub struct Report {
    /// The report's lines, one per message ID of each class.
    pub summary: String,
    /// One line per test and message ID, for the run's log directory.
    pub detail: String,
    /// The pairs of a test and a message ID that no entry matches.
    pub new: usize,
}

/// The pairs of a test and message ID, one occurrence each, with the
/// number of times it was seen.
fn pairs(occurrences: &[Occurrence]) -> BTreeMap<(String, String), (&Occurrence, usize)> {
    let mut pairs: BTreeMap<(String, String), (&Occurrence, usize)> = BTreeMap::new();
    for occurrence in occurrences {
        pairs
            .entry((occurrence.test.clone(), occurrence.message.id.clone()))
            .and_modify(|(_, count)| *count += 1)
            .or_insert((occurrence, 1));
    }
    pairs
}

/// Compare the `occurrences` of a run with the `known` messages. `ran`
/// names every test the run ran, so an entry none of whose messages came
/// although it matches one of them is listed as not seen.
#[must_use]
pub fn report(occurrences: &[Occurrence], known: &Known, ran: &[String]) -> Report {
    let pairs = pairs(occurrences);
    // Per class (new, or the index of its entry) and message ID: the tests,
    // the times it was seen, and the first occurrence.
    let mut ids: BTreeMap<(Option<usize>, String), (usize, usize, &Occurrence)> = BTreeMap::new();
    let mut detail = String::new();
    let mut new = 0;
    let mut seen = vec![false; known.entries.len()];
    for ((test, id), (occurrence, count)) in &pairs {
        let class = known.find(test, id);
        match class {
            Some(index) => seen[index] = true,
            None => new += 1,
        }
        ids.entry((class, id.clone()))
            .and_modify(|(tests, times, _)| {
                *tests += 1;
                *times += count;
            })
            .or_insert((1, *count, occurrence));
        let _ = writeln!(
            detail,
            "{} {} {id} {}: {count} times, log {}: {}",
            class_text(known, class),
            occurrence.message.severity.as_str(),
            occurrence.label,
            occurrence.log.display(),
            occurrence.message.text
        );
    }
    let mut summary = String::new();
    for ((class, id), (tests, times, first)) in &ids {
        let _ = writeln!(
            summary,
            "validation: {} {} {id}: {tests} tests, {times} times; first {}: {}\n  log: {}",
            class_text(known, *class),
            first.message.severity.as_str(),
            first.label,
            shorten(&first.message.text),
            first.log.display()
        );
    }
    for (entry, seen) in known.entries.iter().zip(&seen) {
        if !seen && ran.iter().any(|test| glob(&entry.test, test)) {
            let _ = writeln!(
                summary,
                "validation: known {} {} (line {}) was not seen",
                entry.test, entry.id, entry.line
            );
        }
    }
    let _ = writeln!(
        summary,
        "validation: {} messages; {new} new and {} known pairs of a test and a message ID",
        occurrences.len(),
        pairs.len() - new
    );
    Report {
        summary,
        detail,
        new,
    }
}

/// `new`, or `known` with the issue of the entry at `class`.
fn class_text(known: &Known, class: Option<usize>) -> String {
    match class.and_then(|index| known.entries.get(index)) {
        None => String::from("new"),
        Some(Entry {
            issue: Some(issue), ..
        }) => format!("known issue={issue}"),
        Some(_) => String::from("known"),
    }
}

/// `text` cut to a length a summary line can hold.
fn shorten(text: &str) -> String {
    const MAX: usize = 240;
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(MAX).collect();
    short.push_str("...");
    short
}

#[cfg(test)]
mod tests;
