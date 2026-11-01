//! Command-line words, shared by the launcher, which reads them, and the
//! opener, which writes the `--config` value it opens the app with.

/// Quote a word so that reading the command line back yields it unchanged.
///
/// Also as part of a `--config` batch, which is split at unquoted semicolons.
/// Left alone when it needs no quoting, which is the usual case.
#[must_use]
pub fn quote(word: &str) -> String {
    let needs_quoting = word.is_empty()
        || word
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ';'));
    if !needs_quoting {
        return word.to_owned();
    }
    let mut quoted = String::with_capacity(word.len() + 2);
    quoted.push('"');
    for ch in word.chars() {
        if ch == '"' || ch == '\\' {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}
