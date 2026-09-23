//! Python's tokens: the layer where indentation, line joining and string
//! boundaries are decided, so the parser sees a stream with none of them left.

use super::parse::{Reason, Refusal};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    Name(String),
    Number(String),
    Str(StrLit),
    /// Punctuation and operators, including `...`.
    Op(&'static str),
    /// A comment, without the `#`. `own_line` when nothing but whitespace came
    /// before it on its line.
    Comment {
        text: String,
        own_line: bool,
    },
    Newline,
    Indent,
    Dedent,
    End,
}

/// A string literal as written: its prefix and the text between the quotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrLit {
    pub raw: bool,
    pub bytes: bool,
    pub formatted: bool,
    /// The text between the quotes, escapes untouched.
    pub body: String,
    /// Which quote closed it, which an f-string's nested literals may not reuse
    /// before 3.12.
    pub quote: char,
}

/// A token and the byte offset it starts at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub tok: Tok,
    pub at: usize,
}

/// Every token in `text`, ending with [`Tok::End`].
pub fn lex(text: &str) -> Result<Vec<Token>, Refusal> {
    Lexer {
        chars: text.char_indices().collect(),
        text,
        pos: 0,
        out: Vec::new(),
        indents: vec![0],
        depth: 0,
        line_start: true,
        held: Vec::new(),
    }
    .run()
}

/// Operators, longest first so that a prefix never wins.
const OPS: &[&str] = &[
    "**=", "//=", ">>=", "<<=", "...", "->", ":=", "**", "//", ">>", "<<", "<=", ">=", "==", "!=",
    "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "@=", "+", "-", "*", "/", "%", "@", "&", "|",
    "^", "~", "<", ">", "(", ")", "[", "]", "{", "}", ",", ":", ".", ";", "=", "!",
];

struct Lexer<'a> {
    chars: Vec<(usize, char)>,
    text: &'a str,
    pos: usize,
    out: Vec<Token>,
    indents: Vec<usize>,
    /// Open brackets: inside any, a newline is only whitespace.
    depth: usize,
    /// Nothing but whitespace yet on this logical line.
    line_start: bool,
    /// Own-line comments, with their column, waiting for the next code line to
    /// say which block they sit in.
    held: Vec<(Token, usize)>,
}

impl Lexer<'_> {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.pos + ahead).map(|(_, c)| *c)
    }

    fn offset(&self) -> usize {
        self.chars
            .get(self.pos)
            .map_or(self.text.len(), |(at, _)| *at)
    }

    fn refuse(&self, reason: Reason) -> Refusal {
        Refusal {
            reason,
            at: self.offset(),
        }
    }

    fn push(&mut self, tok: Tok, at: usize) {
        self.out.push(Token { tok, at });
    }

    fn run(mut self) -> Result<Vec<Token>, Refusal> {
        while self.pos < self.chars.len() {
            if self.line_start && self.depth == 0 {
                self.indentation()?;
                if self.pos >= self.chars.len() {
                    break;
                }
            }
            let at = self.offset();
            let c = self.chars[self.pos].1;
            match c {
                ' ' | '\t' | '\x0c' => self.pos += 1,
                '\r' => self.pos += 1,
                '\n' => {
                    self.pos += 1;
                    if self.depth == 0 && !self.line_start {
                        self.push(Tok::Newline, at);
                    }
                    if self.depth == 0 {
                        self.line_start = true;
                    }
                }
                '\\' if matches!(self.peek(1), Some('\n')) => self.pos += 2,
                '\\' if matches!(self.peek(1), Some('\r'))
                    && matches!(self.peek(2), Some('\n')) =>
                {
                    self.pos += 3;
                }
                '#' => {
                    let own_line = self.line_start && self.depth == 0;
                    let start = self.pos + 1;
                    while self.pos < self.chars.len() && self.chars[self.pos].1 != '\n' {
                        self.pos += 1;
                    }
                    let text: String = self.chars[start..self.pos]
                        .iter()
                        .map(|(_, c)| *c)
                        .collect();
                    let tok = Tok::Comment {
                        text: text.trim_end_matches('\r').to_string(),
                        own_line,
                    };
                    if own_line {
                        let column = at - self.text[..at].rfind('\n').map_or(0, |nl| nl + 1);
                        self.held.push((Token { tok, at }, column));
                    } else {
                        self.push(tok, at);
                    }
                    // An own-line comment ends its line itself; the newline after it is
                    // not the end of a statement.
                    if own_line && self.depth == 0 {
                        if self.pos < self.chars.len() {
                            self.pos += 1;
                        }
                        self.line_start = true;
                    }
                }
                c if c.is_ascii_digit()
                    || (c == '.' && self.peek(1).is_some_and(|d| d.is_ascii_digit())) =>
                {
                    self.line_start = false;
                    let number = self.number();
                    self.push(Tok::Number(number), at);
                }
                c if c.is_alphabetic() || c == '_' => {
                    self.line_start = false;
                    if let Some(lit) = self.string_with_prefix()? {
                        self.push(Tok::Str(lit), at);
                    } else {
                        let start = self.pos;
                        while self
                            .peek(0)
                            .is_some_and(|c| c.is_alphanumeric() || c == '_')
                        {
                            self.pos += 1;
                        }
                        let name: String = self.chars[start..self.pos]
                            .iter()
                            .map(|(_, c)| *c)
                            .collect();
                        self.push(Tok::Name(name), at);
                    }
                }
                '\'' | '"' => {
                    self.line_start = false;
                    let lit = self.string(false, false, false)?;
                    self.push(Tok::Str(lit), at);
                }
                _ => {
                    self.line_start = false;
                    let rest = &self.text[at..];
                    let Some(op) = OPS.iter().find(|op| rest.starts_with(**op)) else {
                        return Err(self.refuse(Reason::Character(c)));
                    };
                    match *op {
                        "(" | "[" | "{" => self.depth += 1,
                        ")" | "]" | "}" => self.depth = self.depth.saturating_sub(1),
                        _ => {}
                    }
                    self.pos += op.chars().count();
                    self.push(Tok::Op(op), at);
                }
            }
        }
        let end = self.text.len();
        if !self.line_start && self.depth == 0 {
            self.push(Tok::Newline, end);
        }
        self.place_held(0, end);
        self.push(Tok::End, end);
        Ok(self.out)
    }

    /// Close every block deeper than `width`. A held comment indented past
    /// `width` belongs to a block being closed, so it goes before the dedents;
    /// the rest go after, at the level the next line opens.
    fn place_held(&mut self, width: usize, at: usize) {
        let held = std::mem::take(&mut self.held);
        let (inside, outside): (Vec<_>, Vec<_>) =
            held.into_iter().partition(|(_, column)| *column > width);
        self.out
            .extend(inside.into_iter().map(|(comment, _)| comment));
        while width < *self.indents.last().unwrap_or(&0) {
            self.indents.pop();
            self.push(Tok::Dedent, at);
        }
        self.out
            .extend(outside.into_iter().map(|(comment, _)| comment));
    }

    /// At the start of a line: measure it, and open or close blocks. A line holding
    /// nothing, or only a comment, changes no block.
    fn indentation(&mut self) -> Result<(), Refusal> {
        let mut width = 0;
        let mut ahead = 0;
        while let Some(c) = self.peek(ahead) {
            match c {
                ' ' => width += 1,
                '\t' => return Err(self.refuse(Reason::Tab)),
                '\x0c' => width = 0,
                _ => break,
            }
            ahead += 1;
        }
        match self.peek(ahead) {
            None | Some('\n' | '\r' | '#') => {
                self.pos += ahead;
                return Ok(());
            }
            _ => {}
        }
        self.pos += ahead;
        let at = self.offset();
        let current = *self.indents.last().unwrap_or(&0);
        if width > current {
            // Comments between a block's header and its first line are in it.
            self.indents.push(width);
            self.push(Tok::Indent, at);
            let held = std::mem::take(&mut self.held);
            self.out
                .extend(held.into_iter().map(|(comment, _)| comment));
        } else {
            self.place_held(width, at);
            if width != *self.indents.last().unwrap_or(&0) {
                return Err(self.refuse(Reason::Indentation));
            }
        }
        self.line_start = false;
        Ok(())
    }

    fn number(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek(0) {
            let exponent_sign = matches!(c, '+' | '-')
                && self.pos > start
                && matches!(self.chars[self.pos - 1].1, 'e' | 'E')
                && !self.text[self.chars[start].0..self.offset()].starts_with("0x");
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' || exponent_sign {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.chars[start..self.pos]
            .iter()
            .map(|(_, c)| *c)
            .collect()
    }

    /// A string whose prefix begins here, or `None` when this is a name.
    fn string_with_prefix(&mut self) -> Result<Option<StrLit>, Refusal> {
        let mut len = 0;
        while self.peek(len).is_some_and(|c| c.is_ascii_alphabetic()) && len < 3 {
            len += 1;
        }
        for take in (1..=len.min(2)).rev() {
            if !matches!(self.peek(take), Some('\'' | '"')) {
                continue;
            }
            let prefix: String = (0..take)
                .filter_map(|i| self.peek(i))
                .collect::<String>()
                .to_ascii_lowercase();
            let valid = matches!(
                prefix.as_str(),
                "r" | "u" | "b" | "f" | "br" | "rb" | "fr" | "rf"
            );
            if !valid {
                return Ok(None);
            }
            self.pos += take;
            let raw = prefix.contains('r');
            let bytes = prefix.contains('b');
            let formatted = prefix.contains('f');
            return self.string(raw, bytes, formatted).map(Some);
        }
        Ok(None)
    }

    /// A quoted literal starting at the opening quote.
    fn string(&mut self, raw: bool, bytes: bool, formatted: bool) -> Result<StrLit, Refusal> {
        let quote = self.chars[self.pos].1;
        let triple = self.peek(1) == Some(quote) && self.peek(2) == Some(quote);
        self.pos += if triple { 3 } else { 1 };
        let start = self.pos;
        // Inside an f-string's replacement field, nested literals and brackets are
        // skipped whole: since 3.12 they may use this string's own quote.
        let mut fields = 0usize;
        loop {
            let Some(c) = self.peek(0) else {
                return Err(self.refuse(Reason::UnterminatedString));
            };
            if formatted && c == '{' && fields == 0 && self.peek(1) == Some('{') {
                self.pos += 2;
                continue;
            }
            if formatted && c == '{' {
                fields += 1;
                self.pos += 1;
                continue;
            }
            if formatted && c == '}' && fields > 0 {
                fields -= 1;
                self.pos += 1;
                continue;
            }
            if fields > 0 && matches!(c, '\'' | '"') {
                self.skip_nested(c)?;
                continue;
            }
            if c == '\\' {
                self.pos += 2;
                continue;
            }
            if c == '\n' && !triple {
                return Err(self.refuse(Reason::UnterminatedString));
            }
            if c == quote
                && (!triple || (self.peek(1) == Some(quote) && self.peek(2) == Some(quote)))
            {
                let body: String = self.chars[start..self.pos]
                    .iter()
                    .map(|(_, c)| *c)
                    .collect();
                self.pos += if triple { 3 } else { 1 };
                let _ = raw;
                return Ok(StrLit {
                    raw,
                    bytes,
                    formatted,
                    body,
                    quote,
                });
            }
            self.pos += 1;
        }
    }

    /// A string literal nested in an f-string's field, skipped whole.
    fn skip_nested(&mut self, quote: char) -> Result<(), Refusal> {
        let triple = self.peek(1) == Some(quote) && self.peek(2) == Some(quote);
        self.pos += if triple { 3 } else { 1 };
        loop {
            let Some(c) = self.peek(0) else {
                return Err(self.refuse(Reason::UnterminatedString));
            };
            if c == '\\' {
                self.pos += 2;
                continue;
            }
            if c == quote
                && (!triple || (self.peek(1) == Some(quote) && self.peek(2) == Some(quote)))
            {
                self.pos += if triple { 3 } else { 1 };
                return Ok(());
            }
            self.pos += 1;
        }
    }
}
