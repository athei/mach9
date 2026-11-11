//! A C lexer precise enough to find function calls and definitions.
//!
//! It drops comments and preprocessor directives, keeps string and character
//! literals as single tokens, and records the line of every token. It does no
//! preprocessing: both branches of an `#if` stay in the token stream. It
//! records where each conditional directive is, and the names of the
//! function-like macros the source defines.

use std::ops::Range;

/// What a [`Token`] is.
#[derive(PartialEq, Eq)]
pub enum Kind {
    /// An identifier or keyword.
    Ident,
    /// A number, with any suffix.
    Number,
    /// A string or character literal, quotes included.
    Literal,
    /// One punctuation character, or `->`.
    Punct,
}

/// One token: its kind, its byte range in the source and its line.
pub struct Token {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    /// The 1-based line the token starts on.
    pub line: usize,
}

/// Which part of a conditional a directive is.
pub enum Branch {
    /// `#if`, `#ifdef`, `#ifndef`: the first branch starts.
    If,
    /// `#elif` and its variants, `#else`: another branch starts.
    Else,
    /// `#endif`.
    Endif,
}

/// A conditional directive and where it is.
pub struct Conditional {
    pub branch: Branch,
    /// The index of the first token after the directive.
    pub token: usize,
    pub line: usize,
}

/// A source text and its tokens.
pub struct Lexed<'a> {
    pub src: &'a str,
    pub tokens: Vec<Token>,
    /// The conditional directives, in source order.
    pub conditionals: Vec<Conditional>,
    /// The function-like macros `#define` defines, in source order.
    pub macros: Vec<Macro<'a>>,
}

/// A function-like macro definition.
pub struct Macro<'a> {
    pub name: &'a str,
    /// The byte offset of its name.
    pub offset: usize,
}

impl<'a> Lexed<'a> {
    /// Lex `src`.
    ///
    /// Never fails and never reads past the end: an unterminated comment or
    /// literal ends at the end of the source or line, and bytes outside ASCII
    /// outside literals and comments are skipped.
    pub fn new(src: &'a str) -> Self {
        let lexer = Lexer::new(src.as_bytes()).run();
        Self {
            src,
            tokens: lexer.tokens,
            conditionals: lexer.conditionals,
            macros: lexer
                .macros
                .into_iter()
                .map(|range| Macro {
                    offset: range.start,
                    name: &src[range],
                })
                .collect(),
        }
    }

    /// The text of token `index`, or `""` past the end.
    pub fn text(&self, index: usize) -> &'a str {
        self.tokens
            .get(index)
            .map_or("", |token| &self.src[token.start..token.end])
    }

    /// Whether token `index` is an identifier.
    pub fn is_ident(&self, index: usize) -> bool {
        self.tokens
            .get(index)
            .is_some_and(|token| token.kind == Kind::Ident)
    }

    /// The line of token `index`, or the line of the last token past the end.
    pub fn line(&self, index: usize) -> usize {
        self.tokens
            .get(index)
            .or_else(|| self.tokens.last())
            .map_or(1, |token| token.line)
    }
}

/// The lexer's position in the source and what it found.
struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    tokens: Vec<Token>,
    conditionals: Vec<Conditional>,
    macros: Vec<Range<usize>>,
}

impl<'a> Lexer<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            line: 1,
            tokens: Vec::new(),
            conditionals: Vec::new(),
            macros: Vec::new(),
        }
    }

    fn peek(&self, offset: usize) -> u8 {
        self.bytes.get(self.pos + offset).copied().unwrap_or(0)
    }

    /// Move `count` bytes ahead, but not past the end.
    fn advance(&mut self, count: usize) {
        self.pos = (self.pos + count).min(self.bytes.len());
    }

    fn run(mut self) -> Self {
        let mut line_start = true;
        while self.pos < self.bytes.len() {
            let byte = self.peek(0);
            let next = self.peek(1);
            match byte {
                b'\n' => {
                    self.line += 1;
                    self.advance(1);
                    line_start = true;
                }
                b' ' | b'\t' | b'\r' | 0x0b | 0x0c => self.advance(1),
                b'\\' if next == b'\n' => {
                    self.advance(2);
                    self.line += 1;
                }
                b'/' if next == b'*' => self.block_comment(),
                b'/' if next == b'/' => self.line_comment(),
                b'#' if line_start => self.directive(),
                _ => {
                    line_start = false;
                    self.token(byte, next);
                }
            }
        }
        self
    }

    /// Lex one token starting with `byte`.
    fn token(&mut self, byte: u8, next: u8) {
        let start = self.pos;
        let line = self.line;
        let kind = match byte {
            b'"' | b'\'' => {
                self.literal(byte);
                Kind::Literal
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                self.pos = self.ident_end(self.pos);
                Kind::Ident
            }
            b'0'..=b'9' => {
                self.number();
                Kind::Number
            }
            b'.' if next.is_ascii_digit() => {
                self.number();
                Kind::Number
            }
            b'-' if next == b'>' => {
                self.advance(2);
                Kind::Punct
            }
            _ if byte.is_ascii() => {
                self.advance(1);
                Kind::Punct
            }
            _ => {
                self.advance(1);
                return;
            }
        };
        self.tokens.push(Token {
            kind,
            start,
            end: self.pos,
            line,
        });
    }

    /// The end of the identifier characters starting at `at`.
    fn ident_end(&self, mut at: usize) -> usize {
        while self
            .bytes
            .get(at)
            .is_some_and(|&byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            at += 1;
        }
        at
    }

    fn number(&mut self) {
        self.advance(1);
        loop {
            let byte = self.peek(0);
            let exponent_sign = matches!(byte, b'+' | b'-')
                && matches!(self.bytes[self.pos - 1], b'e' | b'E' | b'p' | b'P');
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.') || exponent_sign {
                self.advance(1);
            } else {
                break;
            }
        }
    }

    /// Skip a literal that opens with `quote`; it ends at its closing quote
    /// or, unterminated, before the end of its line or at the end of the
    /// source.
    fn literal(&mut self, quote: u8) {
        self.advance(1);
        while self.pos < self.bytes.len() {
            match self.peek(0) {
                b'\\' => {
                    if self.peek(1) == b'\n' {
                        self.line += 1;
                    }
                    self.advance(2);
                }
                b'\n' => return,
                byte => {
                    self.advance(1);
                    if byte == quote {
                        return;
                    }
                }
            }
        }
    }

    fn block_comment(&mut self) {
        self.advance(2);
        while self.pos < self.bytes.len() {
            if self.peek(0) == b'*' && self.peek(1) == b'/' {
                self.advance(2);
                return;
            }
            if self.peek(0) == b'\n' {
                self.line += 1;
            }
            self.advance(1);
        }
    }

    /// Skip a `//` comment up to, not including, its newline.
    fn line_comment(&mut self) {
        while self.pos < self.bytes.len() && self.peek(0) != b'\n' {
            self.advance(1);
        }
    }

    /// The first byte at or after `at` that is not a space or a tab.
    fn skip_blanks(&self, mut at: usize) -> usize {
        while matches!(self.bytes.get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
        at
    }

    /// Record what a directive starting at `#` means for the scan: a
    /// conditional, or the name of a function-like macro.
    fn note_directive(&mut self) {
        let word = self.skip_blanks(self.pos + 1);
        let word_end = self.ident_end(word);
        let branch = match &self.bytes[word..word_end] {
            b"if" | b"ifdef" | b"ifndef" => Branch::If,
            b"elif" | b"elifdef" | b"elifndef" | b"else" => Branch::Else,
            b"endif" => Branch::Endif,
            b"define" => {
                let name = self.skip_blanks(word_end);
                let name_end = self.ident_end(name);
                if name_end > name && self.bytes.get(name_end) == Some(&b'(') {
                    self.macros.push(name..name_end);
                }
                return;
            }
            _ => return,
        };
        self.conditionals.push(Conditional {
            branch,
            token: self.tokens.len(),
            line: self.line,
        });
    }

    /// Skip a preprocessor directive up to, not including, the newline that
    /// ends it, across escaped newlines, comments and literals.
    fn directive(&mut self) {
        self.note_directive();
        while self.pos < self.bytes.len() {
            let byte = self.peek(0);
            let next = self.peek(1);
            match byte {
                b'\n' => return,
                b'\\' if next == b'\n' => {
                    self.advance(2);
                    self.line += 1;
                }
                b'/' if next == b'*' => self.block_comment(),
                b'/' if next == b'/' => self.line_comment(),
                b'"' | b'\'' => self.literal(byte),
                _ => self.advance(1),
            }
        }
    }
}
