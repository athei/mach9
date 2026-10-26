//! The wire format of [`ENV_VAR`]: one record per rule, its fields
//! percent-encoded.

use std::fmt::Write as _;

use crate::{DEFAULT_VALUE, ENV_VAR, FieldError, Key, Rule, Scalar, Setting, Source, Table};

/// Whether `line` is a version header of the old format, `v=` and nothing
/// but ASCII digits, which launchers wrote as the first line of a table. The
/// `v` is a key, so it is matched ASCII-case-insensitively like any other.
///
/// [`Table::parse`] skips such a first line. It can never be a rule: a rule
/// needs an `exe` field, and this line has only `v`.
fn is_legacy_header(line: &str) -> bool {
    line.split_once('=').is_some_and(|(key, digits)| {
        key.eq_ignore_ascii_case("v")
            && !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

impl Table {
    /// Serialize the table into the value of [`ENV_VAR`]: one record per
    /// rule, one per line.
    #[must_use]
    pub fn to_env_value(&self) -> String {
        self.rules
            .iter()
            .map(rule_to_record)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Parse the value of [`ENV_VAR`], each rule with its line as its
    /// [`Source`]. Lenient, since the `.so` must never abort a process: a
    /// field with an unknown key or a value its key does not take is ignored
    /// with a diagnostic naming the line and the field, and the rest of its
    /// line still applies. Only a line without an `exe` is ignored as a
    /// whole, since it matches no process. A first line that is a version
    /// header of the old format (`v=<digits>`) is skipped too, so a launcher
    /// that still writes one keeps working. Returns the table and the
    /// diagnostics (empty when clean).
    #[must_use]
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut lines = text.lines().enumerate().peekable();
        lines.next_if(|(_, line)| is_legacy_header(line));
        let mut diagnostics = Vec::new();
        let mut rules = Vec::new();
        for (i, line) in lines {
            if line.trim().is_empty() {
                continue;
            }
            let n = i.saturating_add(1);
            let (rule, ignored) = record_to_rule(line, n);
            diagnostics.extend(
                ignored
                    .into_iter()
                    .map(|field| format!("{ENV_VAR} line {n}: ignoring {field}")),
            );
            if rule.exe.is_empty() {
                diagnostics.push(format!(
                    "{ENV_VAR} line {n}: ignoring the line, it has no exe"
                ));
            } else {
                rules.push(rule);
            }
        }
        (Self { rules }, diagnostics)
    }
}

/// Serialize one rule. `debug_channels` is deliberately left out: whoever
/// starts wine applies those through `WINEDEBUG`, and the library would see
/// them too late. So is `source`, which [`Table::parse`] sets from the line.
///
/// A library that does not know a key added later ignores that field with
/// a diagnostic and applies the rest of the record. That is safe only for a
/// setting, so the matcher keys (`exe`, `company`, `product`,
/// `original_filename`) are a closed set and a new key is only ever a
/// setting: a library that ignored an unknown matcher would apply the rule
/// to more processes than it names.
fn rule_to_record(rule: &Rule) -> String {
    let mut fields: Vec<String> = vec![field("exe", &rule.exe)];
    if let Some(v) = &rule.company {
        fields.push(field("company", v));
    }
    if let Some(v) = &rule.product {
        fields.push(field("product", v));
    }
    if let Some(v) = &rule.original_filename {
        fields.push(field("original_filename", v));
    }
    for &scalar in Scalar::ALL {
        match rule.scalar(scalar) {
            Setting::Unset => {}
            Setting::Set(value) => fields.push(field(scalar.as_str(), value.as_str())),
            Setting::Reset => fields.push(field(scalar.as_str(), DEFAULT_VALUE)),
        }
    }
    for over in &rule.dll_overrides {
        fields.push(field("dll_overrides", over));
    }
    match &rule.arguments_override {
        // An override without entries clears the list, so it travels as the
        // key with an empty value.
        Some(texts) if texts.is_empty() => fields.push(String::from("arguments_override=")),
        Some(texts) => fields.extend(texts.iter().map(|t| field("arguments_override", t))),
        None => {}
    }
    for text in &rule.arguments {
        fields.push(field("arguments", text));
    }
    for (name, value) in &rule.env {
        fields.push(format!("env={}={}", encode(name), encode(value)));
    }
    fields.join(";")
}

/// Read one record into a rule. A field that cannot be read (an unknown key,
/// a value its key does not take, no `=`) is left out, and what was wrong
/// with it is returned next to the rule for the caller to report; the other
/// fields still apply. `line` is the record's line, which becomes the rule's
/// [`Source`].
///
/// The matcher keys are a closed set (see [`rule_to_record`]), so an unknown
/// key is a setting this library does not know, or a typo. A misspelt
/// matcher is ignored like any other unknown key, with the warning, and the
/// rule then matches more processes than it meant to. The `name` and
/// `enabled` keys of the old format, where an override merged into or
/// dropped a rule by name, are unknown keys too.
fn record_to_rule(record: &str, line: usize) -> (Rule, Vec<String>) {
    let mut rule = Rule {
        source: Source::Line(line),
        ..Rule::default()
    };
    let mut ignored = Vec::new();
    for raw in record.split(';') {
        if raw.is_empty() {
            continue;
        }
        let Some((key, value)) = raw.split_once('=') else {
            ignored.push(format!("field {raw:?} without '='"));
            continue;
        };
        if let Err(reason) = set_field(&mut rule, key, value) {
            ignored.push(reason.to_string());
        }
    }
    (rule, ignored)
}

/// Set the field `key` of `rule` from its wire `value`, or say why it
/// cannot be set, leaving the rule as it was. The key is matched
/// ASCII-case-insensitively.
fn set_field(rule: &mut Rule, key: &str, value: &str) -> Result<(), FieldError> {
    let key: Key = key.parse()?;
    if let Some(scalar) = key.scalar() {
        return set_scalar(rule, scalar, value);
    }
    match key {
        Key::Exe => rule.exe = decode(value),
        Key::Company => rule.company = Some(decode(value)),
        Key::Product => rule.product = Some(decode(value)),
        Key::OriginalFilename => rule.original_filename = Some(decode(value)),
        Key::DllOverrides => rule.dll_overrides.push(decode(value)),
        Key::Arguments => rule.arguments.push(decode(value)),
        // Present at all, the override is set; an empty value adds no entry,
        // so `arguments_override=` alone clears the list.
        Key::ArgumentsOverride => {
            let texts = rule.arguments_override.get_or_insert_default();
            if !value.is_empty() {
                texts.push(decode(value));
            }
        }
        Key::Env => {
            let (name, val) = value.split_once('=').unwrap_or((value, ""));
            rule.env.push((decode(name), decode(val)));
        }
        // The scalar settings returned above.
        Key::Dxgi
        | Key::D3d9
        | Key::Wined3d
        | Key::OpenGl
        | Key::DpiAware
        | Key::LargeAddressAware
        | Key::Gcmouse
        | Key::ForceNx => {}
    }
    Ok(())
}

/// Set the scalar setting `scalar` of `rule` from its wire `value`:
/// `default`, in any case, resets it (see [`Setting::Reset`]), and anything
/// else is read as a value of the setting. The last field for a key wins, as
/// for any scalar. A value the setting does not take leaves the rule as it
/// was.
fn set_scalar(rule: &mut Rule, scalar: Scalar, value: &str) -> Result<(), FieldError> {
    let decoded = decode(value);
    let setting = if decoded.eq_ignore_ascii_case(DEFAULT_VALUE) {
        Setting::Reset
    } else {
        Setting::Set(scalar.parse_value(&decoded)?)
    };
    rule.put_scalar(scalar, setting);
    Ok(())
}

fn field(key: &str, value: &str) -> String {
    format!("{key}={}", encode(value))
}

/// Percent-escape the characters that would collide with the wire format
/// (`%`, `;`, and CR/LF) plus any other control character. Everything else,
/// including `=`, `,`, spaces and non-ASCII, passes through.
fn encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    encode_into(raw, &mut out);
    out
}

fn encode_into(raw: &str, out: &mut String) {
    for c in raw.chars() {
        match c {
            '%' => out.push_str("%25"),
            ';' => out.push_str("%3B"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "%{:02X}", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// Reverse [`encode_into`]. Lenient: a `%` not followed by two hex digits is
/// kept literally, so decoding can never fail.
fn decode(encoded: &str) -> String {
    let mut out = String::with_capacity(encoded.len());
    let mut chars = encoded.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let rest = chars.as_str();
        match rest
            .get(..2)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            Some(byte) => {
                out.push(byte as char);
                let _ = chars.next();
                let _ = chars.next();
            }
            None => out.push('%'),
        }
    }
    out
}

#[cfg(test)]
mod tests;
