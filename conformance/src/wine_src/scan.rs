//! Finds the `START_TEST` body of a test unit, the functions the file
//! defines, and the calls in that body.

use std::{collections::BTreeSet, ops::Range};

use super::lex::{Branch, Kind, Lexed};

/// A lexed test file with its bracket pairs.
pub struct Source<'a> {
    pub lexed: Lexed<'a>,
    /// For each opening `(`, `[` or `{`, the index of its closing token.
    pairs: Vec<Option<usize>>,
}

/// Where a `START_TEST(name)` and its body are.
pub struct StartTest {
    /// The unit name in the parentheses.
    pub name: String,
    /// The index of the `START_TEST` token.
    pub token: usize,
    /// The indices of the body's `{` and `}`.
    pub open: usize,
    pub close: usize,
}

/// One call of a named function in a `START_TEST` body.
pub struct Call<'a> {
    pub name: &'a str,
    pub line: usize,
    /// The token ranges of the arguments, split at top-level commas.
    pub args: Vec<Range<usize>>,
    /// Whether the call is a whole expression statement, so its value is
    /// not used and a statement can replace it.
    pub statement: bool,
    /// The index of the call's closing `)`.
    pub end: usize,
}

/// An `#if` whose branches the bracket matching is in.
struct OpenIf {
    line: usize,
    /// The open brackets when the `#if` starts; each branch starts with them.
    start: Vec<usize>,
    /// The open brackets at the end of the first branch, once it ended.
    first_end: Option<Vec<usize>>,
}

impl<'a> Source<'a> {
    /// Lex `src` and match its brackets.
    ///
    /// Each branch of an `#if` starts with the brackets open before it, and
    /// every branch must leave the same brackets open.
    ///
    /// # Errors
    ///
    /// Returns `<line>: <message>` for a closing bracket of the wrong kind
    /// or with nothing open, a bracket never closed, branches of an `#if`
    /// that leave different brackets open (such as two signatures of one
    /// function that each open its body), and an `#else` or `#endif` without
    /// `#if` or an `#if` without `#endif`.
    pub fn new(src: &'a str) -> Result<Self, String> {
        let lexed = Lexed::new(src);
        let pairs = match_brackets(&lexed)?;
        Ok(Self { lexed, pairs })
    }

    fn pair(&self, index: usize) -> Option<usize> {
        self.pairs.get(index).copied().flatten()
    }

    /// Every `START_TEST(name) { ... }` at file scope.
    pub fn start_tests(&self) -> Vec<StartTest> {
        let lexed = &self.lexed;
        let mut found = Vec::new();
        let mut index = 0;
        while index < lexed.tokens.len() {
            if lexed.text(index) == "{" {
                index = self
                    .pair(index)
                    .map_or(lexed.tokens.len(), |close| close + 1);
                continue;
            }
            if lexed.text(index) == "START_TEST"
                && lexed.text(index + 1) == "("
                && lexed.is_ident(index + 2)
                && lexed.text(index + 3) == ")"
                && lexed.text(index + 4) == "{"
                && let Some(close) = self.pair(index + 4)
            {
                found.push(StartTest {
                    name: lexed.text(index + 2).to_owned(),
                    token: index,
                    open: index + 4,
                    close,
                });
                index = close + 1;
                continue;
            }
            index += 1;
        }
        found
    }

    /// The names of the functions the file defines at file scope: an
    /// identifier, a parenthesized parameter list and a `{`.
    pub fn local_functions(&self) -> BTreeSet<&'a str> {
        let lexed = &self.lexed;
        let mut names = BTreeSet::new();
        let mut index = 0;
        while index < lexed.tokens.len() {
            if lexed.text(index) == "{" {
                index = self
                    .pair(index)
                    .map_or(lexed.tokens.len(), |close| close + 1);
                continue;
            }
            if lexed.is_ident(index)
                && lexed.text(index + 1) == "("
                && let Some(close) = self.pair(index + 1)
                && lexed.text(close + 1) == "{"
                && lexed.text(index) != "START_TEST"
            {
                names.insert(lexed.text(index));
            }
            index += 1;
        }
        names
    }

    /// Every call of a named function between the body tokens `open` and
    /// `close`, in source order.
    ///
    /// A call through a member (`a.f()`, `a->f()`) is left out.
    pub fn calls(&self, open: usize, close: usize) -> Vec<Call<'a>> {
        let lexed = &self.lexed;
        let starts = self.statement_starts(open, close);
        let mut calls = Vec::new();
        for index in open + 1..close {
            if !lexed.is_ident(index) || lexed.text(index + 1) != "(" {
                continue;
            }
            if matches!(lexed.text(index - 1), "." | "->") {
                continue;
            }
            let Some(end) = self.pair(index + 1) else {
                continue;
            };
            calls.push(Call {
                name: lexed.text(index),
                line: lexed.line(index),
                args: self.split_args(index + 2, end),
                statement: starts.contains(&index) && lexed.text(end + 1) == ";",
                end,
            });
        }
        calls
    }

    /// The indices of the tokens that start a statement in the body between
    /// `open` and `close`.
    fn statement_starts(&self, open: usize, close: usize) -> BTreeSet<usize> {
        let lexed = &self.lexed;
        let mut starts = BTreeSet::from([open + 1]);
        let mut depth = 0usize;
        for index in open + 1..close {
            let text = lexed.text(index);
            match text {
                "(" => depth += 1,
                ")" => depth = depth.saturating_sub(1),
                ";" if depth == 0 => {
                    starts.insert(index + 1);
                }
                "{" | "}" | "else" | "do" => {
                    starts.insert(index + 1);
                }
                "if" | "while" | "for" | "switch" if lexed.text(index + 1) == "(" => {
                    if let Some(end) = self.pair(index + 1) {
                        starts.insert(end + 1);
                    }
                }
                _ => {}
            }
            if !starts.contains(&index) {
                continue;
            }
            if text == "case" {
                if let Some(colon) = (index + 1..close).find(|&i| lexed.text(i) == ":") {
                    starts.insert(colon + 1);
                }
            } else if lexed.is_ident(index) && lexed.text(index + 1) == ":" {
                starts.insert(index + 2);
            }
        }
        starts
    }

    /// The token ranges between `start` and `end` split at commas outside
    /// nested brackets. No tokens is no arguments.
    fn split_args(&self, start: usize, end: usize) -> Vec<Range<usize>> {
        let lexed = &self.lexed;
        if start >= end {
            return Vec::new();
        }
        let mut args = Vec::new();
        let mut arg_start = start;
        let mut index = start;
        while index < end {
            match lexed.text(index) {
                "(" | "[" | "{" => {
                    index = self.pair(index).map_or(end, |close| close + 1);
                    continue;
                }
                "," => {
                    args.push(arg_start..index);
                    arg_start = index + 1;
                }
                _ => {}
            }
            index += 1;
        }
        args.push(arg_start..end);
        args
    }

    /// The identifier an argument consists of, if it is exactly one.
    pub fn single_ident(&self, arg: &Range<usize>) -> Option<&'a str> {
        (arg.len() == 1 && self.lexed.tokens[arg.start].kind == Kind::Ident)
            .then(|| self.lexed.text(arg.start))
    }

    /// The byte offset of the start of the line token `index` is on.
    pub fn line_start(&self, index: usize) -> usize {
        let offset = self.lexed.tokens[index].start;
        self.lexed.src[..offset]
            .rfind('\n')
            .map_or(0, |newline| newline + 1)
    }
}

/// For each opening bracket of `lexed`, the index of its closing bracket.
fn match_brackets(lexed: &Lexed<'_>) -> Result<Vec<Option<usize>>, String> {
    let count = lexed.tokens.len();
    let mut pairs = vec![None; count];
    let mut open: Vec<usize> = Vec::new();
    let mut ifs: Vec<OpenIf> = Vec::new();
    let mut conditionals = lexed.conditionals.iter().peekable();
    for index in 0..=count {
        while let Some(conditional) = conditionals.next_if(|c| c.token == index) {
            let line = conditional.line;
            match conditional.branch {
                Branch::If => ifs.push(OpenIf {
                    line,
                    start: open.clone(),
                    first_end: None,
                }),
                Branch::Else => {
                    let current = ifs
                        .last_mut()
                        .ok_or_else(|| format!("{line}: #else or #elif without #if"))?;
                    match &current.first_end {
                        None => current.first_end = Some(open.clone()),
                        Some(first) if *first != open => {
                            return Err(branches_differ(current.line));
                        }
                        Some(_) => {}
                    }
                    open.clone_from(&current.start);
                }
                Branch::Endif => {
                    let current = ifs
                        .pop()
                        .ok_or_else(|| format!("{line}: #endif without #if"))?;
                    if current.first_end.is_some_and(|first| first != open) {
                        return Err(branches_differ(current.line));
                    }
                }
            }
        }
        if index == count {
            break;
        }
        let closer = lexed.text(index);
        let opener = match closer {
            "(" | "[" | "{" => {
                open.push(index);
                continue;
            }
            ")" => "(",
            "]" => "[",
            "}" => "{",
            _ => continue,
        };
        let line = lexed.line(index);
        let Some(start) = open.pop() else {
            return Err(format!("{line}: `{closer}` with no bracket open"));
        };
        if lexed.text(start) != opener {
            return Err(format!(
                "{line}: `{closer}` closes the `{}` of line {}",
                lexed.text(start),
                lexed.line(start)
            ));
        }
        pairs[start] = Some(index);
    }
    if let Some(current) = ifs.last() {
        return Err(format!("{}: #if without #endif", current.line));
    }
    if let Some(&start) = open.last() {
        return Err(format!(
            "{}: `{}` is never closed",
            lexed.line(start),
            lexed.text(start)
        ));
    }
    Ok(pairs)
}

fn branches_differ(line: usize) -> String {
    format!(
        "{line}: the branches of this #if leave different brackets open, so the scan cannot \
         follow them"
    )
}
