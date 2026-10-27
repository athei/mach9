//! Command-line words: how a value given to `--config`, or the arguments typed
//! into the New Executable window, are split into words, how a word is quoted
//! so it reads back whole, and how a list of names reads in a message.

/// Split a command line into words the way a shell would: whitespace
/// separates, single quotes are literal, double quotes group while `\"` and
/// `\\` stand for the character they escape, and outside quotes a backslash
/// escapes whatever follows it. Quotes group without contributing anything, so
/// `''` is an empty argument.
pub fn split(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if !escaped && quote.is_none() && ch.is_whitespace() {
            if let Some(word) = word.take() {
                words.push(word);
            }
            continue;
        }
        let word = word.get_or_insert_with(String::new);
        if escaped {
            word.push(ch);
            escaped = false;
        } else if quote == Some('\'') {
            match ch {
                '\'' => quote = None,
                _ => word.push(ch),
            }
        } else if quote == Some('"') {
            match ch {
                '"' => quote = None,
                // Only these two are escapes inside double quotes; anything
                // else keeps the backslash, as a shell would.
                '\\' if matches!(chars.peek(), Some('"' | '\\')) => escaped = true,
                _ => word.push(ch),
            }
        } else {
            match ch {
                '\'' | '"' => quote = Some(ch),
                '\\' => escaped = true,
                _ => word.push(ch),
            }
        }
    }

    if escaped {
        return Err(String::from("ends in a trailing backslash"));
    }
    if let Some(quote) = quote {
        return Err(format!("has an unterminated {quote} quote"));
    }
    if let Some(word) = word {
        words.push(word);
    }
    Ok(words)
}

/// Quote a word so that reading the command line back yields it unchanged,
/// also as part of a `--config` batch, which is split at unquoted semicolons.
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

/// Names in running text, `a`, `b` and `c`, each wrapped in `quote`: a
/// backtick in an error message, nothing in help text.
#[must_use]
pub fn listing(names: &[&str], quote: &str) -> String {
    let quoted: Vec<String> = names
        .iter()
        .map(|name| format!("{quote}{name}{quote}"))
        .collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_and_escapes_group_words() {
        assert_eq!(
            split(r#"'/Games/My Game/Game.exe' "two words" a\ b '' -x"#).unwrap(),
            ["/Games/My Game/Game.exe", "two words", "a b", "", "-x"]
        );
    }

    #[test]
    fn escapes_only_a_quote_or_a_backslash_inside_double_quotes() {
        assert_eq!(
            split(r#""say \"hi\"" "back\\slash" "keep\nas-is""#).unwrap(),
            [r#"say "hi""#, r"back\slash", r"keep\nas-is"]
        );
    }

    #[test]
    fn single_quotes_keep_every_character() {
        assert_eq!(
            split(r#"'\not-an-escape' 'a"b'"#).unwrap(),
            [r"\not-an-escape", r#"a"b"#]
        );
    }

    #[test]
    fn rejects_an_unterminated_quote_or_a_trailing_backslash() {
        assert_eq!(
            split(r#""/Games/Game.exe"#).unwrap_err(),
            r#"has an unterminated " quote"#
        );
        assert_eq!(
            split("'/Games/Game.exe").unwrap_err(),
            "has an unterminated ' quote"
        );
        assert_eq!(
            split(r"/Games/Game.exe \").unwrap_err(),
            "ends in a trailing backslash"
        );
    }

    #[test]
    fn a_quoted_word_reads_back_whole() {
        for word in [
            "",
            "/Games/Game.exe",
            "/Games/My Game/Game.exe",
            r#"od"d"#,
            r"back\slash",
            "semi;colon",
            "tab\there",
        ] {
            assert_eq!(split(&quote(word)).unwrap(), [word], "{word:?}");
        }
    }

    #[test]
    fn leaves_a_word_that_needs_no_quoting_alone() {
        assert_eq!(quote("/Games/Game.exe"), "/Games/Game.exe");
        assert_eq!(quote("+seh,+tid"), "+seh,+tid");
        assert_eq!(quote("a.exe=x;b.exe=y"), "\"a.exe=x;b.exe=y\"");
    }

    #[test]
    fn lists_names_in_running_text() {
        assert_eq!(listing(&[], "`"), "");
        assert_eq!(listing(&["a"], "`"), "`a`");
        assert_eq!(listing(&["a", "b", "c"], "`"), "`a`, `b` and `c`");
        assert_eq!(listing(&["a", "b"], ""), "a and b");
    }
}
