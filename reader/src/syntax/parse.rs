//! Text to tree, refusing everything it does not model.
//!
//! ⚠ **Refusal is the design, not a gap.** A parser that absorbs an
//! unimplemented construct into literal text satisfies the round-trip law and
//! contradicts nothing: it prints the text back and reads the same wrong tree a
//! second time. Bash's printer does not object either, because it prints words
//! verbatim. So the only place the error can be caught is here, at the moment a
//! character that opens a construct is seen — and the answer has to be a
//! [`Refusal`], never a `Literal`.
//!
//! What that costs is coverage, and coverage is the thing that is supposed to be
//! low at the start and ratchet. `bash-oracle`'s `syntax-report` ranks the
//! refusals so the next construct is chosen by the corpus rather than by taste.
//!
//! Scanning is over bytes. Every character with meaning to the shell is ASCII, so
//! a multi-byte sequence can only ever be interior to a literal run, and slicing
//! at a special character always lands on a character boundary.

use super::ast::{
    Command, Comment, Glob, Item, Pipeline, Script, Segment, SegmentKind, Span, Timed, Word,
};

/// Why a piece of text was not read.
///
/// ⚠ **A closed enum, so the report can rank it.** A free-text reason would make
/// the failure list ungroupable, and the failure list is what picks the next
/// thing to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    /// `$…` or a backtick — a substitution or a parameter expansion.
    Expansion,
    /// `<` or `>` in any of their forms.
    Redirection,
    /// `&&` or `||`.
    AndOr,
    /// A trailing `&`.
    Background,
    /// `(`, `)`, `{` or `}`.
    Grouping,
    /// `[…]` inside a word — a bracket expression, not the `[` builtin.
    BracketExpression,
    /// A `~` opening a word, which expands to a home directory.
    Tilde,
    /// `if`, `for`, `time`, `!` and the rest: grammar, not a command name.
    ReservedWord,
    /// `FOO=bar cmd` — a command prefix, and a binding the tree must model.
    Assignment,
    /// A quote with no partner.
    UnterminatedQuote,
    /// A backslash at end of input.
    DanglingEscape,
}

impl Reason {
    /// A stable label, for grouping in the corpus report.
    pub fn label(self) -> &'static str {
        match self {
            Reason::Expansion => "expansion ($ or backtick)",
            Reason::Redirection => "redirection (< >)",
            Reason::AndOr => "and-or (&& ||)",
            Reason::Background => "background (&)",
            Reason::Grouping => "grouping (( ) { })",
            Reason::BracketExpression => "bracket expression ([…])",
            Reason::Tilde => "tilde (~)",
            Reason::ReservedWord => "reserved word",
            Reason::Assignment => "assignment prefix (FOO=bar)",
            Reason::UnterminatedQuote => "unterminated quote",
            Reason::DanglingEscape => "dangling escape",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub reason: Reason,
    pub span: Span,
}

/// The words bash treats as grammar when they open a command and are unquoted.
///
/// ⚠ **`'time' ./x.sh` runs `/usr/bin/time` and `time ./x.sh` does not run a
/// program at all.** So a reserved word is refused only where it is reserved:
/// first word of a command, no quoting anywhere in it. That distinction is
/// invisible to both gates — bash prints the quotes straight back — which is why
/// it is decided here, where the quoting is still known.
/// ⚠ **`time` is deliberately absent.** At the head of a pipeline it is grammar
/// and [`Parser::pipeline`] consumes it before any word is read; anywhere else
/// bash runs the program of that name — `a | time b` is accepted and executes
/// `/usr/bin/time`. Refusing it as a word would refuse a legal command.
///
/// `!` stays, because it is grammar at the head and a *syntax error* elsewhere:
/// bash rejects `a | ! b`. Refusing it is the right answer in both positions.
const RESERVED: &[&str] = &[
    "!", "[[", "]]", "case", "coproc", "do", "done", "elif", "else", "esac", "fi", "for",
    "function", "if", "in", "select", "then", "until", "while",
];

pub fn parse(text: &str) -> Result<Script, Refusal> {
    Parser {
        bytes: text.as_bytes(),
        text,
        at: 0,
    }
    .script()
}

struct Parser<'t> {
    bytes: &'t [u8],
    text: &'t str,
    at: usize,
}

impl<'t> Parser<'t> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.at + ahead).copied()
    }

    fn refuse<T>(&self, reason: Reason, width: usize) -> Result<T, Refusal> {
        Err(Refusal {
            reason,
            span: Span::new(self.at, self.at + width),
        })
    }

    fn script(&mut self) -> Result<Script, Refusal> {
        let mut items = Vec::new();
        loop {
            self.skip_blanks();
            match self.peek() {
                None => break,
                // A separator with no command in front of it: `;` after a
                // command already ended, or a blank line. Nothing to record —
                // the tree holds sequence, and an empty step is not one.
                Some(b';') | Some(b'\n') => {
                    self.at += 1;
                }
                Some(b'#') => items.push(Item::Comment(self.comment())),
                _ => {
                    let pipeline = self.pipeline()?;
                    if !pipeline.is_empty() {
                        items.push(Item::Pipeline(pipeline));
                    }
                }
            }
        }
        Ok(Script {
            items,
            span: Span::new(0, self.bytes.len()),
        })
    }

    /// Spaces, tabs, and a backslash-newline — which joins two lines and is
    /// therefore whitespace rather than an escape. Newlines are separators and
    /// are left for the caller.
    fn skip_blanks(&mut self) {
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\r') => self.at += 1,
                Some(b'\\') if self.peek_at(1) == Some(b'\n') => self.at += 2,
                _ => return,
            }
        }
    }

    fn comment(&mut self) -> Comment {
        let start = self.at;
        self.at += 1; // the `#`
        let from = self.at;
        while let Some(byte) = self.peek() {
            if byte == b'\n' {
                break;
            }
            self.at += 1;
        }
        Comment {
            text: self.text[from..self.at].to_string(),
            span: Span::new(start, self.at),
        }
    }

    /// `[time [-p]] [!] cmd [| cmd …]`.
    ///
    /// The prefixes are read in a loop because bash accepts them in either
    /// order and normalises to `time` first — `! time a | b` comes back from
    /// `declare -f` as `time ! a | b`. Reading both into the same two fields is
    /// what makes those two texts one tree.
    fn pipeline(&mut self) -> Result<Pipeline, Refusal> {
        let start = self.at;
        let mut time = None;
        let mut negated = false;
        loop {
            self.skip_blanks();
            if self.take_keyword("!") {
                // ⚠ Toggled, not counted: bash prints `! ! a` back as `a`.
                negated = !negated;
            } else if self.take_keyword("time") {
                // ⚠ The blanks between `time` and `-p` have to go first.
                // Without this the option is never seen and every `time -p`
                // silently becomes a plain `time` with `-p` as the command.
                self.skip_blanks();
                time = Some(if self.take_keyword("-p") {
                    Timed::Posix
                } else {
                    Timed::Plain
                });
            } else {
                break;
            }
        }

        let mut commands = Vec::new();
        loop {
            let command = self.command()?;
            if !command.words.is_empty() {
                commands.push(command);
            }
            self.skip_blanks();
            if self.peek() == Some(b'|') && self.peek_at(1) != Some(b'|') {
                self.at += 1;
                continue;
            }
            break;
        }

        Ok(Pipeline {
            time,
            negated,
            commands,
            span: Span::new(start, self.at),
        })
    }

    /// Take an unquoted word equal to `word`, if that is what is next.
    ///
    /// The boundary check is the whole of it: `time` is a keyword and `timeout`
    /// is a program, and without a lookahead the second would lose its first
    /// four characters. A quote makes it a value rather than grammar, and a
    /// quote character here means the word did not start where we are.
    fn take_keyword(&mut self, word: &str) -> bool {
        let end = self.at + word.len();
        if self.text.get(self.at..end) != Some(word) {
            return false;
        }
        let boundary = matches!(
            self.bytes.get(end).copied(),
            None | Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n') | Some(b';') | Some(b'|')
        );
        if boundary {
            self.at = end;
        }
        boundary
    }

    fn command(&mut self) -> Result<Command, Refusal> {
        let start = self.at;
        let mut words: Vec<Word> = Vec::new();
        loop {
            self.skip_blanks();
            match self.peek() {
                None | Some(b';') | Some(b'\n') => break,
                // The pipeline owns `|`; a bare one ends this command. `||` is
                // an and-or list and is still refused, by the word reader.
                Some(b'|') if self.peek_at(1) != Some(b'|') => break,
                // `#` opens a comment only where a word would start; inside a
                // word it is an ordinary character, which is why this is tested
                // here and not in the word reader.
                Some(b'#') => break,
                _ => {}
            }
            let word = self.word(words.is_empty())?;
            words.push(word);
        }
        Ok(Command {
            words,
            span: Span::new(start, self.at),
        })
    }

    /// One word. `first` is whether it opens the command, which is the only
    /// position where a reserved word or an assignment is grammar.
    fn word(&mut self, first: bool) -> Result<Word, Refusal> {
        let start = self.at;
        let mut segments: Vec<Segment> = Vec::new();
        let mut quoted_anywhere = false;

        // ⚠ **At the head of EVERY word, not just the first.** `cd ~/Code` has
        // its tilde in the second, and scoping this to the command name let
        // exactly that shape absorb an expansion into literal text — the error
        // the whole refusal discipline exists to prevent, reintroduced by a
        // stray condition.
        if self.peek() == Some(b'~') {
            return self.refuse(Reason::Tilde, 1);
        }

        while let Some(byte) = self.peek() {
            let at = self.at;
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' | b';' => break,
                b'\\' if self.peek_at(1) == Some(b'\n') => break,
                b'|' => {
                    if self.peek_at(1) == Some(b'|') {
                        return self.refuse(Reason::AndOr, 1);
                    }
                    break;
                }
                b'&' => {
                    let reason = if self.peek_at(1) == Some(b'&') {
                        Reason::AndOr
                    } else {
                        Reason::Background
                    };
                    return self.refuse(reason, 1);
                }
                b'<' | b'>' => return self.refuse(Reason::Redirection, 1),
                b'(' | b')' | b'{' | b'}' => return self.refuse(Reason::Grouping, 1),
                b'$' | b'`' => return self.refuse(Reason::Expansion, 1),
                b'[' if self.closes_bracket() => {
                    return self.refuse(Reason::BracketExpression, 1);
                }
                b'*' | b'?' => {
                    self.at += 1;
                    let glob = if byte == b'*' { Glob::Any } else { Glob::One };
                    segments.push(Segment {
                        kind: SegmentKind::Glob(glob),
                        span: Span::new(at, self.at),
                    });
                }
                b'\'' => {
                    quoted_anywhere = true;
                    segments.push(self.single_quoted()?);
                }
                b'"' => {
                    quoted_anywhere = true;
                    segments.push(self.double_quoted()?);
                }
                _ => segments.push(self.bare()?),
            }
        }

        let word = Word {
            segments: merge_literals(segments),
            span: Span::new(start, self.at),
        };

        if first
            && !quoted_anywhere
            && let Some(text) = word.as_literal()
        {
            if is_reserved(&text) {
                return Err(Refusal {
                    reason: Reason::ReservedWord,
                    span: word.span,
                });
            }
            if is_assignment(&text) {
                return Err(Refusal {
                    reason: Reason::Assignment,
                    span: word.span,
                });
            }
        }
        Ok(word)
    }

    /// Is the `[` under the cursor the opening of a bracket expression — that
    /// is, does a `]` follow it before the word ends?
    ///
    /// Without this, `[ -f x ]` could not be read at all: its `[` is the test
    /// builtin, a whole word with no `]` after it, and refusing every `[` would
    /// throw the commonest conditional in the corpus away for a construct that
    /// is not there.
    fn closes_bracket(&self) -> bool {
        let mut at = self.at + 1;
        while let Some(&byte) = self.bytes.get(at) {
            match byte {
                b']' => return true,
                b' ' | b'\t' | b'\n' | b'\r' | b';' => return false,
                _ => at += 1,
            }
        }
        false
    }

    fn single_quoted(&mut self) -> Result<Segment, Refusal> {
        let start = self.at;
        self.at += 1;
        let from = self.at;
        loop {
            match self.peek() {
                None => {
                    return Err(Refusal {
                        reason: Reason::UnterminatedQuote,
                        span: Span::new(start, self.bytes.len()),
                    });
                }
                Some(b'\'') => break,
                _ => self.at += 1,
            }
        }
        let text = self.text[from..self.at].to_string();
        self.at += 1;
        Ok(Segment {
            kind: SegmentKind::Literal(text),
            span: Span::new(start, self.at),
        })
    }

    /// Double quotes suppress splitting and globbing but not expansion, so the
    /// text inside is literal and a `$` or a backtick inside is refused exactly
    /// as it would be outside.
    fn double_quoted(&mut self) -> Result<Segment, Refusal> {
        let start = self.at;
        self.at += 1;
        let mut text = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(Refusal {
                    reason: Reason::UnterminatedQuote,
                    span: Span::new(start, self.bytes.len()),
                });
            };
            match byte {
                b'"' => break,
                b'$' | b'`' => return self.refuse(Reason::Expansion, 1),
                b'\\' => {
                    // ⚠ Inside double quotes a backslash escapes only the four
                    // characters that mean something there, and is an ordinary
                    // character before anything else: `"\a"` is a backslash and
                    // an `a`, where `\a` unquoted is just an `a`.
                    match self.peek_at(1) {
                        Some(b'\n') => self.at += 2,
                        Some(next @ (b'$' | b'`' | b'"' | b'\\')) => {
                            text.push(next as char);
                            self.at += 2;
                        }
                        Some(_) | None => {
                            text.push('\\');
                            self.at += 1;
                        }
                    }
                }
                _ => {
                    let from = self.at;
                    while let Some(next) = self.peek() {
                        if matches!(next, b'"' | b'\\' | b'$' | b'`') {
                            break;
                        }
                        self.at += 1;
                    }
                    text.push_str(&self.text[from..self.at]);
                }
            }
        }
        self.at += 1;
        Ok(Segment {
            kind: SegmentKind::Literal(text),
            span: Span::new(start, self.at),
        })
    }

    /// An unquoted run, up to the next character that means something.
    fn bare(&mut self) -> Result<Segment, Refusal> {
        let start = self.at;
        let mut text = String::new();
        while let Some(byte) = self.peek() {
            match byte {
                b'\\' => match self.peek_at(1) {
                    // Handled by the caller as a line join, so the word ends.
                    Some(b'\n') | None => break,
                    Some(next) => {
                        text.push(next as char);
                        self.at += 2;
                    }
                },
                b' ' | b'\t' | b'\r' | b'\n' | b';' | b'\'' | b'"' | b'|' | b'&' | b'<' | b'>'
                | b'(' | b')' | b'{' | b'}' | b'$' | b'`' | b'*' | b'?' => break,
                b'[' if self.closes_bracket() => break,
                _ => {
                    let from = self.at;
                    while let Some(next) = self.peek() {
                        if is_bare_stop(next) || (next == b'[' && self.closes_bracket()) {
                            break;
                        }
                        self.at += 1;
                    }
                    text.push_str(&self.text[from..self.at]);
                }
            }
        }
        if self.at == start {
            // Only reachable from a lone trailing backslash: the loop broke
            // before consuming anything and the caller would spin.
            return self.refuse(Reason::DanglingEscape, 1);
        }
        Ok(Segment {
            kind: SegmentKind::Literal(text),
            span: Span::new(start, self.at),
        })
    }
}

fn is_bare_stop(byte: u8) -> bool {
    matches!(
        byte,
        b' ' | b'\t'
            | b'\r'
            | b'\n'
            | b';'
            | b'\''
            | b'"'
            | b'\\'
            | b'|'
            | b'&'
            | b'<'
            | b'>'
            | b'('
            | b')'
            | b'{'
            | b'}'
            | b'$'
            | b'`'
            | b'*'
            | b'?'
    )
}

/// Is this text one of the words bash reads as grammar at the head of a command?
///
/// Shared with the printer, which has to quote such a word to keep it a value —
/// the two must agree or the round-trip law fails on every `time` in the corpus.
pub fn is_reserved(text: &str) -> bool {
    RESERVED.contains(&text)
}

/// `FOO=bar`, the shape bash reads as a binding rather than a command name.
pub fn is_assignment(text: &str) -> bool {
    let Some(equals) = text.find('=') else {
        return false;
    };
    let name = &text[..equals];
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `a"b"c` is one literal, not three.
///
/// Without this the tree would record where the quotes were, which is exactly
/// the distinction the model says is not one: `'a'`, `"a"` and `a` are the same
/// word. Two commands that differ only in quoting must compare equal, and they
/// only do if the segments merge.
fn merge_literals(segments: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::with_capacity(segments.len());
    for segment in segments {
        match (out.last_mut(), &segment.kind) {
            (Some(last), SegmentKind::Literal(text)) => {
                if let SegmentKind::Literal(existing) = &mut last.kind {
                    existing.push_str(text);
                    last.span = Span::new(last.span.start, segment.span.end);
                    continue;
                }
                out.push(segment);
            }
            _ => out.push(segment),
        }
    }
    out
}
