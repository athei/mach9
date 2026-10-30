//! The wire format of [`ENV_VAR`]: one record per rule, its fields
//! percent-encoded.
//!
//! A record is `key=value` fields joined by `;`. In a value, `%`, `;`, CR,
//! LF and every other control character are written as `%XX`, the hex of
//! the byte; everything else, non-ASCII included, is written as it is, in
//! UTF-8. Reading decodes every `%XX` to its byte, and the bytes of a value
//! must be UTF-8 without a NUL.
//!
//! An `env` field is `env=NAME=value`: the name ends at the first `=`, and
//! an env name cannot contain `=` (see [`EnvName`]). A name that decodes to
//! one with `=`, from a `%3D`, is refused like any other invalid name.

use std::{
    fmt::{self, Write as _},
    str::FromStr,
};

use crate::{
    Argument, DEFAULT_VALUE, DebugChannel, Diagnostic, DllOverride, EnvEntry, EnvName, Exe,
    FieldError, Fingerprint, Key, KeyKind, ParseError, Rule, RulesText, Scalar, Setting, Source,
    Table,
};
#[cfg(doc)]
use crate::{ENV_VAR, FILE_ENV_VAR};

/// Whether `line` is a version header of the old format, `v=` and nothing
/// but ASCII digits, which launchers wrote as the first line of a table. The
/// `v` is a key, so it is matched ASCII-case-insensitively like any other.
///
/// [`Table::parse_lenient`] skips such a first line. It can never be a rule:
/// a rule needs an `exe` field, and this line has only `v`.
fn is_legacy_header(line: &str) -> bool {
    line.split_once('=').is_some_and(|(key, digits)| {
        key.eq_ignore_ascii_case("v")
            && !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

impl Table {
    /// Read the value of [`ENV_VAR`] the way `compatdb.so` does, which must
    /// never abort a process: each rule gets its line as its [`Source`], and
    /// every problem is a [`Diagnostic`] (none when the text is clean).
    ///
    /// - A field with an unknown key, a value its key does not take, no
    ///   `=`, escapes that are not UTF-8 or a NUL is left out, and the rest
    ///   of its line still applies.
    /// - A line without an `exe` is left out as a whole, since it matches no
    ///   process.
    /// - A second field for a matcher or a scalar setting replaces the first
    ///   one.
    /// - A `%` that two hex digits do not follow is kept as it is.
    ///
    /// A first line that is a version header of the old format
    /// (`v=<digits>`) is skipped without a diagnostic, so a launcher that
    /// still writes one keeps working; it still counts as a line.
    #[must_use]
    pub fn parse_lenient(text: &str) -> (Self, Vec<Diagnostic>) {
        Self::parse_text(text, RulesText::Variable)
    }

    /// Read the text of the file [`FILE_ENV_VAR`] names the way
    /// `compatdb.so` does: as [`Table::parse_lenient`] reads the value of
    /// [`ENV_VAR`], except that each rule's [`Source`] is a
    /// [`Source::FileLine`] and each [`Diagnostic`] names the file. Reading
    /// the file is up to the caller.
    #[must_use]
    pub fn parse_file_lenient(text: &str) -> (Self, Vec<Diagnostic>) {
        Self::parse_text(text, RulesText::File)
    }

    /// Read `value`, the whole of the text `text`.
    fn parse_text(value: &str, text: RulesText) -> (Self, Vec<Diagnostic>) {
        let mut lines = value.lines().enumerate().peekable();
        lines.next_if(|(_, line)| is_legacy_header(line));
        let mut table = Self::new();
        let mut diagnostics = Vec::new();
        for (i, line) in lines {
            if line.trim().is_empty() {
                continue;
            }
            let n = i.saturating_add(1);
            match read_record(line, text, n, &mut diagnostics) {
                Some(rule) => table.push_from(rule, text.source(n)),
                None => diagnostics.push(Diagnostic {
                    text,
                    line: n,
                    field: None,
                    error: FieldError::MissingExe,
                }),
            }
        }
        (table, diagnostics)
    }

    /// Read the value of [`ENV_VAR`], refusing any problem that
    /// [`Table::parse_lenient`] would report. A table this gives back writes
    /// itself (see [`Table`]'s `Display`) as `text` with blank lines and a
    /// legacy header dropped and every word in lowercase.
    ///
    /// # Errors
    ///
    /// A [`ParseError`] with every diagnostic, when there is one.
    pub fn parse_strict(text: &str) -> Result<Self, ParseError> {
        let (table, diagnostics) = Self::parse_lenient(text);
        if diagnostics.is_empty() {
            Ok(table)
        } else {
            Err(ParseError(diagnostics))
        }
    }
}

impl FromStr for Table {
    type Err = ParseError;

    /// [`Table::parse_strict`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_strict(s)
    }
}

impl fmt::Display for Table {
    /// The value of [`ENV_VAR`]: one record per rule, one per line, for
    /// every rule that is not built in. The built-in rules are left out:
    /// `compatdb.so` has them compiled in, and the same rules again from the
    /// variable would fold after all of them.
    ///
    /// [`Table::parse_strict`] reads a table this writes back as the same
    /// table when the table's own text is canonical: the table of
    /// [`Table::new`] and [`Table::push`], or of [`Table::parse_strict`] from
    /// text this wrote.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sent = self
            .entries()
            .filter(|(source, _)| *source != Source::BuiltIn)
            .map(|(_, rule)| rule);
        for (i, rule) in sent.enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            f.write_str(&rule_to_record(rule))?;
        }
        Ok(())
    }
}

/// Serialize one rule.
///
/// A library that does not know a key added later ignores that field with
/// a diagnostic and applies the rest of the record. That is safe only for a
/// setting, so the matcher keys (`exe`, `company`, `product`,
/// `original_filename`) are a closed set and a new key is only ever a
/// setting: a library that ignored an unknown matcher would apply the rule
/// to more processes than it names.
fn rule_to_record(rule: &Rule) -> String {
    let mut fields: Vec<String> = vec![field(Key::Exe, rule.exe.as_str())];
    let pins = [
        (Key::Company, &rule.company),
        (Key::Product, &rule.product),
        (Key::OriginalFilename, &rule.original_filename),
    ];
    for (key, pin) in pins {
        if let Some(pin) = pin {
            fields.push(field(key, pin.as_str()));
        }
    }
    for &scalar in Scalar::ALL {
        match rule.scalar(scalar) {
            Setting::Unset => {}
            Setting::Set(value) => fields.push(field(scalar.key(), value.as_str())),
            Setting::Reset => fields.push(field(scalar.key(), DEFAULT_VALUE)),
        }
    }
    for over in &rule.dll_overrides {
        fields.push(field(Key::DllOverrides, over.as_str()));
    }
    match &rule.arguments_override {
        // An override without entries clears the list, so it travels as the
        // key with an empty value.
        Some(texts) if texts.is_empty() => fields.push(field(Key::ArgumentsOverride, "")),
        Some(texts) => fields.extend(
            texts
                .iter()
                .map(|t| field(Key::ArgumentsOverride, t.as_str())),
        ),
        None => {}
    }
    for text in &rule.arguments {
        fields.push(field(Key::Arguments, text.as_str()));
    }
    for channel in &rule.debug_channels {
        fields.push(field(Key::DebugChannels, channel.as_str()));
    }
    for entry in &rule.env {
        // The name has no `=`, so the first `=` of the value ends it.
        let mut value = String::new();
        encode_into(entry.name().as_str(), &mut value);
        value.push('=');
        encode_into(entry.value().unwrap_or_default(), &mut value);
        fields.push(format!("{}={value}", Key::Env));
    }
    fields.join(";")
}

/// Read one record into a rule, or `None` for a record without an `exe`,
/// which matches no process. Every problem with a field goes to
/// `diagnostics`; a field that cannot be read is left out, and the other
/// fields still apply. `text` and `line` say where the record is, for the
/// diagnostics.
///
/// The matcher keys are a closed set (see [`rule_to_record`]), so an unknown
/// key is a setting this library does not know, or a typo. A misspelt
/// matcher is ignored like any other unknown key, with the warning, and the
/// rule then matches more processes than it meant to. The `name` and
/// `enabled` keys of the old format, where an override merged into or
/// dropped a rule by name, are unknown keys too.
fn read_record(
    record: &str,
    text: RulesText,
    line: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Rule> {
    let mut rule = Rule::new(Exe::Any);
    // The rule's `exe`, which every rule needs; an empty one is none.
    let mut exe = None;
    // The keys that hold one value and a field of the record has set.
    let mut seen: Vec<Key> = Vec::new();
    for raw in record.split(';') {
        if raw.is_empty() {
            continue;
        }
        let mut report = |error| {
            diagnostics.push(Diagnostic {
                text,
                line,
                field: Some(raw.to_owned()),
                error,
            });
        };
        let Some((key, value)) = raw.split_once('=') else {
            report(FieldError::MissingEquals);
            continue;
        };
        let key: Key = match key.parse() {
            Ok(key) => key,
            Err(error) => {
                report(error);
                continue;
            }
        };
        // One diagnostic per field: why it was left out, or else that it
        // replaced an earlier one, or else that it kept a `%` as written.
        let mut decoder = Decoder::default();
        if let Err(error) = read_field(&mut rule, &mut exe, key, value, &mut decoder) {
            report(error);
            continue;
        }
        let holds_one = matches!(key.kind(), KeyKind::Matcher | KeyKind::Scalar(_));
        if holds_one && seen.contains(&key) {
            report(FieldError::Duplicate(key));
        } else if decoder.bad_escape {
            report(FieldError::BadEscape);
        }
        if holds_one && !seen.contains(&key) {
            seen.push(key);
        }
    }
    exe.map(|exe| {
        rule.exe = exe;
        rule
    })
}

/// Set the field `key` of `rule`, or of `exe`, from its wire `value`, or
/// say why it cannot be set, leaving both as they were.
fn read_field(
    rule: &mut Rule,
    exe: &mut Option<Exe>,
    key: Key,
    value: &str,
    decoder: &mut Decoder,
) -> Result<(), FieldError> {
    if let Some(scalar) = key.scalar() {
        let text = decoder.decode(value)?;
        // `default`, in any case, resets the setting (see
        // [`Setting::Reset`]); anything else is read as a value of it.
        let setting = if text.eq_ignore_ascii_case(DEFAULT_VALUE) {
            Setting::Reset
        } else {
            Setting::Set(scalar.parse_value(&text)?)
        };
        rule.put_scalar(scalar, setting);
        return Ok(());
    }
    match key {
        Key::Exe => {
            *exe = match Exe::new(decoder.decode(value)?) {
                Ok(name) => Some(name),
                Err(FieldError::MissingExe) => None,
                Err(error) => return Err(error),
            };
        }
        Key::Company => rule.company = Some(Fingerprint::new(decoder.decode(value)?)?),
        Key::Product => rule.product = Some(Fingerprint::new(decoder.decode(value)?)?),
        Key::OriginalFilename => {
            rule.original_filename = Some(Fingerprint::new(decoder.decode(value)?)?);
        }
        Key::DllOverrides => {
            let entry = DllOverride::new(decoder.decode(value)?)?;
            rule.dll_overrides.push(entry);
        }
        Key::Arguments => rule.arguments.push(Argument::new(decoder.decode(value)?)?),
        Key::DebugChannels => {
            let channel = DebugChannel::new(decoder.decode(value)?)?;
            rule.debug_channels.push(channel);
        }
        // Present at all, the override is set; an empty value adds no entry,
        // so `arguments_override=` alone clears the list.
        Key::ArgumentsOverride => {
            let argument = if value.is_empty() {
                None
            } else {
                Some(Argument::new(decoder.decode(value)?)?)
            };
            rule.arguments_override
                .get_or_insert_default()
                .extend(argument);
        }
        // The name ends at the first `=`; a name cannot contain one, so a
        // `%3D` in it is refused. A value without `=` is the variable alone,
        // which it removes.
        Key::Env => {
            let (name, val) = value.split_once('=').unwrap_or((value, ""));
            let name = EnvName::new(decoder.decode(name)?)?;
            rule.env.push(EnvEntry::new(name, decoder.decode(val)?)?);
        }
        // The scalar settings returned above.
        Key::Dxgi
        | Key::D3d9
        | Key::Wined3d
        | Key::OpenGl
        | Key::DpiAware
        | Key::LargeAddressAware
        | Key::Gcmouse
        | Key::ForceNx
        | Key::MetalHud => {}
    }
    Ok(())
}

/// The field `key=value`, with the value escaped.
fn field(key: Key, value: &str) -> String {
    let mut out = format!("{key}=");
    encode_into(value, &mut out);
    out
}

/// Percent-escape the characters that would collide with the wire format
/// (`%`, `;`, and CR/LF) plus any other control character. Everything else,
/// including `=`, `,`, spaces and non-ASCII, passes through.
fn encode_into(raw: &str, out: &mut String) {
    for c in raw.chars() {
        match c {
            '%' => out.push_str("%25"),
            ';' => out.push_str("%3B"),
            c if c.is_ascii_control() => {
                let _ = write!(out, "%{:02X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
}

/// The decoder of the values of one field, which notes a `%` it kept
/// because two hex digits did not follow it.
#[derive(Default)]
struct Decoder {
    /// Whether a value had such a `%`.
    bad_escape: bool,
}

impl Decoder {
    /// Reverse [`encode_into`]: every `%XX` becomes the byte `XX`, in either
    /// case, and the bytes must be UTF-8 without a NUL. A `%` that two hex
    /// digits do not follow stays as it is.
    fn decode(&mut self, encoded: &str) -> Result<String, FieldError> {
        let mut bytes = Vec::with_capacity(encoded.len());
        let mut rest = encoded.as_bytes();
        while let Some((&byte, tail)) = rest.split_first() {
            rest = tail;
            if byte != b'%' {
                bytes.push(byte);
                continue;
            }
            if let Some(escaped) = hex_pair(rest) {
                bytes.push(escaped);
                rest = rest.get(2..).unwrap_or_default();
            } else {
                self.bad_escape = true;
                bytes.push(byte);
            }
        }
        let text = String::from_utf8(bytes).map_err(|_| FieldError::InvalidUtf8)?;
        if text.contains('\0') {
            return Err(FieldError::Nul);
        }
        Ok(text)
    }
}

/// The byte that the two hex digits at the start of `bytes` spell, if they
/// are two hex digits.
fn hex_pair(bytes: &[u8]) -> Option<u8> {
    let pair = bytes.get(..2)?;
    if !pair.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()
}

#[cfg(test)]
mod tests;
