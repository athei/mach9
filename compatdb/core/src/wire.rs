//! The wire format of [`ENV_VAR`]: one record per rule, its fields
//! percent-encoded.

use std::fmt::Write as _;

use crate::{
    D3d9, DEFAULT_VALUE, Dxgi, ENV_VAR, OpenGl, Rule, SCALAR_KEYS, Source, Table, Wined3d,
};

/// Whether `line` is a version header of the old format, `v=` and nothing
/// but ASCII digits, which launchers wrote as the first line of a table.
///
/// [`Table::parse`] skips such a first line. It can never be a rule: a rule
/// needs an `exe` field, and this line has only `v`.
fn is_legacy_header(line: &str) -> bool {
    line.strip_prefix("v=")
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
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
    for &key in SCALAR_KEYS {
        if let Some(value) = scalar_value(rule, key) {
            fields.push(field(key, value));
        } else if rule.resets.contains(&key) {
            fields.push(field(key, DEFAULT_VALUE));
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

/// The wire spelling of the value `rule` sets for the scalar `key`, or
/// `None` when it sets none.
pub fn scalar_value(rule: &Rule, key: &str) -> Option<&'static str> {
    let boolean = |value: Option<bool>| value.map(|on| if on { "true" } else { "false" });
    match key {
        "dxgi" => rule.dxgi.map(Dxgi::as_str),
        "d3d9" => rule.d3d9.map(D3d9::as_str),
        "wined3d" => rule.wined3d.map(Wined3d::as_str),
        "opengl" => rule.opengl.map(OpenGl::as_str),
        "dpi_aware" => boolean(rule.dpi_aware),
        "large_address_aware" => boolean(rule.large_address_aware),
        "gcmouse" => boolean(rule.gcmouse),
        "force_nx" => boolean(rule.force_nx),
        _ => None,
    }
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
            ignored.push(reason);
        }
    }
    (rule, ignored)
}

/// Set the field `key` of `rule` from its wire `value`, or say why it
/// cannot be set, leaving the rule as it was.
fn set_field(rule: &mut Rule, key: &str, value: &str) -> Result<(), String> {
    let resets = &mut rule.resets;
    match key {
        "exe" => rule.exe = decode(value),
        "company" => rule.company = Some(decode(value)),
        "product" => rule.product = Some(decode(value)),
        "original_filename" => rule.original_filename = Some(decode(value)),
        "dxgi" => set_scalar(&mut rule.dxgi, resets, "dxgi", value, |v| {
            parse_value("dxgi", v, Dxgi::parse)
        })?,
        "d3d9" => set_scalar(&mut rule.d3d9, resets, "d3d9", value, |v| {
            parse_value("d3d9", v, D3d9::parse)
        })?,
        "wined3d" => set_scalar(&mut rule.wined3d, resets, "wined3d", value, |v| {
            parse_value("wined3d", v, Wined3d::parse)
        })?,
        "opengl" => set_scalar(&mut rule.opengl, resets, "opengl", value, |v| {
            parse_value("opengl", v, OpenGl::parse)
        })?,
        "dpi_aware" => set_scalar(&mut rule.dpi_aware, resets, "dpi_aware", value, |v| {
            parse_bool("dpi_aware", v)
        })?,
        "large_address_aware" => set_scalar(
            &mut rule.large_address_aware,
            resets,
            "large_address_aware",
            value,
            |v| parse_bool("large_address_aware", v),
        )?,
        "gcmouse" => set_scalar(&mut rule.gcmouse, resets, "gcmouse", value, |v| {
            parse_bool("gcmouse", v)
        })?,
        "force_nx" => set_scalar(&mut rule.force_nx, resets, "force_nx", value, |v| {
            parse_bool("force_nx", v)
        })?,
        "dll_overrides" => rule.dll_overrides.push(decode(value)),
        "arguments" => rule.arguments.push(decode(value)),
        // Present at all, the override is set; an empty value adds no entry,
        // so `arguments_override=` alone clears the list.
        "arguments_override" => {
            let texts = rule.arguments_override.get_or_insert_default();
            if !value.is_empty() {
                texts.push(decode(value));
            }
        }
        "env" => {
            let (name, val) = value.split_once('=').unwrap_or((value, ""));
            rule.env.push((decode(name), decode(val)));
        }
        other => return Err(format!("unknown key {other:?}")),
    }
    Ok(())
}

/// Set the scalar `slot`, whose key is `key`, from its wire `value`:
/// `default` clears it and marks the key reset (see [`Rule::resets`]),
/// anything else is read by `parse` and takes the key out of `resets`. The
/// last field for a key wins, as for any scalar. A value `parse` rejects
/// leaves both as they were.
fn set_scalar<T>(
    slot: &mut Option<T>,
    resets: &mut Vec<&'static str>,
    key: &'static str,
    value: &str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Result<(), String> {
    if decode(value) == DEFAULT_VALUE {
        *slot = None;
        if !resets.contains(&key) {
            resets.push(key);
        }
    } else {
        *slot = Some(parse(value)?);
        resets.retain(|k| *k != key);
    }
    Ok(())
}

/// A field's value read by `parse`, which knows the values its key takes.
fn parse_value<T>(key: &str, value: &str, parse: fn(&str) -> Option<T>) -> Result<T, String> {
    let decoded = decode(value);
    parse(&decoded).ok_or_else(|| format!("unknown {key} value {decoded:?}"))
}

/// A boolean field's value: exactly `true` or `false`, case-sensitive, so a
/// typo leaves the field out instead of quietly meaning one or the other.
fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    let decoded = decode(value);
    match decoded.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("unknown {key} value {decoded:?}")),
    }
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
