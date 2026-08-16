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
    AndOr, Command, Comment, Connector, Glob, Heredoc, Item, Link, Parameter, Pipeline, Redirect,
    RedirectOp, RedirectTarget, Script, Segment, SegmentKind, Span, Tilde, Timed, Word,
};

/// Why a piece of text was not read.
///
/// ⚠ **A closed enum, so the report can rank it.** A free-text reason would make
/// the failure list ungroupable, and the failure list is what picks the next
/// thing to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    /// `$'…'` — ANSI-C quoting.
    ///
    /// ⚠ **Not an expansion at all.** Bash resolves the escapes at parse time
    /// and prints `$'\x41'` back as `'A'`, so this is a *spelling of a literal*
    /// and belongs in a `Literal` segment once it is read. Grouped here only
    /// because a `$` opens it.
    AnsiQuote,
    /// `$"…"` — locale translation, which bash prints back as a plain double
    /// quoted string. A literal too, for the same reason.
    LocaleQuote,
    /// `$name`, `$1`, `$@`, `${name}` — a parameter and nothing else.
    Parameter,
    /// `${name:-default}`, `${#name}`, `${name%%suffix}` — a parameter with an
    /// operator on it, which is a small language of its own and a separate
    /// build from naming one.
    ParameterOperator,
    /// `$(cmd)` or `` `cmd` `` — a whole script, whose value is its output.
    /// The first construct that needs this parser to recurse into itself.
    CommandSubstitution,
    /// `$((…))` — arithmetic, which is its own grammar.
    Arithmetic,
    /// `>`, `>>`, `<`, `2>&1`, `&>`, `>|`, `<>`, `{fd}>` — a file or a
    /// descriptor, and nothing that carries a body.
    Redirection,
    /// `<<<` — a value, not a file, and no body.
    HereString,
    /// `<(cmd)` or `>(cmd)`: a whole command, so it needs grouping first.
    ProcessSubstitution,
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
    /// `${x` — an expansion with no closing brace. Bash refuses it too, so this
    /// is a claim about the input and `bash -n` adjudicates it.
    UnterminatedExpansion,
    /// A backslash at end of input.
    DanglingEscape,
    /// `a |` or `a &&` with nothing after it. Bash calls this a syntax error too.
    EmptyOperand,
    /// A comment between a list operator and its right-hand side. Bash deletes
    /// it, and this tree keeps comments, so there is nowhere to put it.
    CommentInList,
}

impl Reason {
    /// A stable label, for grouping in the corpus report.
    pub fn label(self) -> &'static str {
        match self {
            Reason::AnsiQuote => "ANSI-C quoting ($'…')",
            Reason::LocaleQuote => "locale quoting ($\"…\")",
            Reason::Parameter => "parameter ($name, ${name})",
            Reason::ParameterOperator => "parameter with an operator (${x:-y}, ${#x})",
            Reason::CommandSubstitution => "command substitution ($(…), `…`)",
            Reason::Arithmetic => "arithmetic ($((…)))",
            Reason::Redirection => "redirection (> >> < 2>&1 &>)",
            Reason::HereString => "here-string (<<<)",
            Reason::ProcessSubstitution => "process substitution (<( >()",
            Reason::Grouping => "grouping (( ) { })",
            Reason::BracketExpression => "bracket expression ([…])",
            Reason::Tilde => "tilde (~)",
            Reason::ReservedWord => "reserved word",
            Reason::Assignment => "assignment prefix (FOO=bar)",
            Reason::UnterminatedQuote => "unterminated quote",
            Reason::UnterminatedExpansion => "unterminated expansion (${ with no })",
            Reason::DanglingEscape => "dangling escape",
            Reason::EmptyOperand => "empty operand (an operator with nothing after it)",
            Reason::CommentInList => "comment inside an and-or list",
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
/// A reserved word is refused only where it is reserved: first word of a
/// command, no quoting anywhere in it. `'time' ./x.sh` runs `/usr/bin/time`
/// while `time ./x.sh` runs no program at all, and that distinction is invisible
/// to both gates — bash prints the quotes straight back — which is why it is
/// decided here, where the quoting is still known.
///
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
    let mut parser = Parser {
        bytes: text.as_bytes(),
        text,
        at: 0,
        pending: Vec::new(),
        bodies: Vec::new(),
    };
    let mut script = parser.script()?;
    let mut bodies = parser.bodies.into_iter();
    fill_script(&mut script, &mut bodies);
    debug_assert!(
        bodies.next().is_none(),
        "a heredoc body was read that no opener in the tree claimed"
    );
    Ok(script)
}

struct Parser<'t> {
    bytes: &'t [u8],
    text: &'t str,
    at: usize,
    /// Heredocs opened on the line being read, still waiting for their bodies.
    ///
    /// ⚠ **A heredoc body cannot be found by scanning ahead for a newline.** The
    /// rest of the opener's line may hold a newline that does not end it — inside
    /// a quoted word, or after a backslash — and bash starts the body after the
    /// *logical* line instead. Both are measured in `reader/probes/heredoc.sh`.
    /// Deferring to the point where the parser actually consumes a line ending
    /// gets that right for free, because the quote and escape readers have
    /// already stepped over the newlines that do not count.
    pending: Vec<Pending>,
    /// Bodies, in the order they were read, waiting to be matched to openers.
    bodies: Vec<Heredoc>,
}

/// A heredoc's opener, held until the line it was written on ends.
struct Pending {
    delimiter: String,
    quoted: bool,
    strip_tabs: bool,
    span: Span,
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
                Some(b';') => {
                    self.at += 1;
                }
                Some(b'\n') => self.take_newline()?,
                Some(b'#') => items.push(Item::Comment(self.comment())),
                _ => {
                    let list = self.and_or()?;
                    if !list.is_empty() {
                        items.push(Item::List(list));
                    }
                }
            }
        }
        // A heredoc opened on a line the text ended without terminating: its
        // body is whatever is left, which at this point is nothing.
        self.read_pending_bodies()?;
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

    /// Blanks, and the newlines that a list or pipeline operator makes into
    /// continuations rather than terminators.
    fn skip_blanks_and_newlines(&mut self) -> Result<(), Refusal> {
        loop {
            self.skip_blanks();
            if self.peek() == Some(b'\n') {
                self.take_newline()?;
            } else {
                return Ok(());
            }
        }
    }

    /// Consume the newline under the cursor, and with it the bodies of every
    /// heredoc opened on the line it ends.
    ///
    /// ⚠ **The one place a line ending is consumed.** A heredoc body starts here
    /// and nowhere else, so routing every newline through this function is what
    /// makes "the body follows the line" true rather than approximately true.
    fn take_newline(&mut self) -> Result<(), Refusal> {
        self.at += 1;
        self.read_pending_bodies()
    }

    fn read_pending_bodies(&mut self) -> Result<(), Refusal> {
        for pending in std::mem::take(&mut self.pending) {
            let body = self.heredoc_body(&pending)?;
            self.bodies.push(body);
        }
        Ok(())
    }

    /// The lines from the cursor up to the one holding the delimiter alone.
    /// ⚠ **The end of the input terminates a body, because that is what bash
    /// makes of it** — with `warning: here-document at line N delimited by
    /// end-of-file` on stderr, and the rest of the text as the body. The corpus
    /// is shell history and holds 13 such commands; reading them the way they
    /// ran is the whole point, and refusing them would drop real work for a
    /// shape bash has a definite answer about.
    ///
    /// The printer writes the delimiter back, so `t₂` is terminated where `t₁`
    /// was not. That is a normalisation the law permits, and it is the same
    /// tree.
    fn heredoc_body(&mut self, pending: &Pending) -> Result<Heredoc, Refusal> {
        let mut body = String::new();
        loop {
            if self.at >= self.bytes.len() {
                return self.finish_body(pending, body);
            }
            let from = self.at;
            while self.peek().is_some_and(|byte| byte != b'\n') {
                self.at += 1;
            }
            let line = &self.text[from..self.at];
            // A final line with no newline after it still terminates the body.
            if self.peek() == Some(b'\n') {
                self.at += 1;
            }
            // ⚠ `<<-` strips leading TABS, and only tabs. A line indented with
            // spaces is body text and a terminator preceded by one is not a
            // terminator — both measured, and both the shape a `<<-` in the
            // corpus is most likely to be written wrong in.
            let line = if pending.strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            if line == pending.delimiter {
                return self.finish_body(pending, body);
            }
            body.push_str(line);
            body.push('\n');
        }
    }

    /// Resolve a body once its extent is known, however it ended.
    fn finish_body(&self, pending: &Pending, body: String) -> Result<Heredoc, Refusal> {
        let body = if pending.quoted {
            body
        } else {
            join_continuations(&body)
        };
        // ⚠ Joining can *create* a terminator: a body holding `EO\⏎F` becomes a
        // line reading `EOF`, and printing that back would end the heredoc
        // early. Refused rather than printed, because the printer has no other
        // spelling available to it.
        if body.lines().any(|line| line == pending.delimiter) {
            return Err(Refusal {
                reason: Reason::EmptyOperand,
                span: pending.span,
            });
        }
        Ok(Heredoc {
            delimiter: pending.delimiter.clone(),
            quoted: pending.quoted,
            body,
        })
    }

    /// Is there no command here — only a terminator or the end of the text?
    fn at_end_of_command(&self) -> bool {
        matches!(
            self.peek(),
            None | Some(b';') | Some(b'\n') | Some(b'|') | Some(b'&') | Some(b'#')
        )
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

    /// `pipeline [(&& | ||) pipeline …] [&]`.
    ///
    /// A newline after a connector continues the list — bash's grammar is
    /// `list AND_AND newline_list list` — so `a &&⏎b` is one list, exactly as a
    /// newline after `|` stays inside a pipeline.
    fn and_or(&mut self) -> Result<AndOr, Refusal> {
        let start = self.at;
        let first = self.pipeline()?;
        let mut rest = Vec::new();
        loop {
            self.skip_blanks();
            let connector = match (self.peek(), self.peek_at(1)) {
                (Some(b'&'), Some(b'&')) => Connector::And,
                (Some(b'|'), Some(b'|')) => Connector::Or,
                _ => break,
            };
            self.at += 2;
            self.skip_blanks_and_newlines()?;
            // ⚠ Bash accepts a comment here and DELETES it. This tree keeps
            // comments byte-exact, so accepting one would be destructive —
            // refused rather than silently dropped.
            if self.peek() == Some(b'#') {
                return self.refuse(Reason::CommentInList, 1);
            }
            if self.at_end_of_command() {
                return self.refuse(Reason::EmptyOperand, 1);
            }
            rest.push(Link {
                connector,
                pipeline: self.pipeline()?,
            });
        }

        // A single `&` ends the list and backgrounds it. `&&` was taken above,
        // so anything left here is the async operator.
        self.skip_blanks();
        let background = self.peek() == Some(b'&');
        if background {
            self.at += 1;
        }

        Ok(AndOr {
            first,
            rest,
            background,
            span: Span::new(start, self.at),
        })
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
                // ⚠ **A newline after `|` continues the pipeline.** Bash's
                // grammar is `pipeline '|' newline_list pipeline`, and without
                // this `a |⏎b` read as TWO pipelines — a silent misparse that
                // both gates passed, because the printed form was two lines and
                // read back as two pipelines just as wrongly.
                self.skip_blanks_and_newlines()?;
                if self.at_end_of_command() {
                    return self.refuse(Reason::EmptyOperand, 1);
                }
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
        let mut redirects: Vec<Redirect> = Vec::new();
        loop {
            self.skip_blanks();
            match self.peek() {
                None | Some(b';') | Some(b'\n') => break,
                // `&>` is a redirection, not the list's `&`. Checked first, or
                // every `cmd &> log` would end the command at the ampersand.
                Some(b'&') if self.peek_at(1) == Some(b'>') => {}
                // The pipeline owns a bare `|`; `||`, `&&` and `&` belong to
                // the list above it. All of them end this command.
                Some(b'|') | Some(b'&') => break,
                // `#` opens a comment only where a word would start; inside a
                // word it is an ordinary character, which is why this is tested
                // here and not in the word reader.
                Some(b'#') => break,
                _ => {}
            }
            if let Some(redirect) = self.redirect()? {
                redirects.push(redirect);
                continue;
            }
            let word = self.word(words.is_empty())?;
            words.push(word);
        }
        Ok(Command {
            words,
            redirects,
            span: Span::new(start, self.at),
        })
    }

    /// A redirection, if one starts here.
    ///
    /// ⚠ **The descriptor must touch the operator.** `cat 2>out` redirects fd 2;
    /// `cat 2 > out` passes `2` as an argument and redirects stdout. Only a run
    /// of digits ending exactly at `<` or `>` is a descriptor, and because this
    /// is tried at a word boundary, `file2>out` cannot be read as one either.
    fn redirect(&mut self) -> Result<Option<Redirect>, Refusal> {
        let start = self.at;
        let mut at = self.at;
        while self.bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        let fd: Option<u32> = (at > self.at)
            .then(|| self.text[self.at..at].parse().ok())
            .flatten();
        // A digit run too long to be a descriptor is a word, not a redirection.
        if at > self.at && fd.is_none() {
            return Ok(None);
        }
        let after_fd = self.bytes.get(at).copied();
        let op = match (after_fd, self.bytes.get(at + 1).copied()) {
            // `&>` and `&>>` take no descriptor before them.
            (Some(b'&'), Some(b'>')) if fd.is_none() => {
                if self.bytes.get(at + 2) == Some(&b'>') {
                    at += 3;
                    RedirectOp::BothAppend
                } else {
                    at += 2;
                    RedirectOp::Both
                }
            }
            (Some(b'>'), Some(b'>')) => {
                at += 2;
                RedirectOp::Append
            }
            (Some(b'>'), Some(b'|')) => {
                at += 2;
                RedirectOp::Clobber
            }
            (Some(b'>'), Some(b'&')) => {
                at += 2;
                RedirectOp::DupOut
            }
            (Some(b'<'), Some(b'&')) => {
                at += 2;
                RedirectOp::DupIn
            }
            (Some(b'<'), Some(b'>')) => {
                at += 2;
                RedirectOp::ReadWrite
            }
            // A here-string is refused elsewhere; leave it alone.
            (Some(b'<'), Some(b'<')) if self.bytes.get(at + 2) == Some(&b'<') => return Ok(None),
            (Some(b'<'), Some(b'<')) => {
                at += 2;
                let strip_tabs = self.bytes.get(at) == Some(&b'-');
                if strip_tabs {
                    at += 1;
                }
                self.at = at;
                let start_of_delimiter = self.at;
                let (delimiter, quoted) = self.heredoc_delimiter()?;
                self.pending.push(Pending {
                    delimiter,
                    quoted,
                    strip_tabs,
                    span: Span::new(start_of_delimiter, self.at),
                });
                return Ok(Some(Redirect {
                    fd: fd.or(Some(0)),
                    op: if strip_tabs {
                        RedirectOp::HereDash
                    } else {
                        RedirectOp::Here
                    },
                    // A placeholder until the line ends and the body is read;
                    // `fill_script` puts the real one in.
                    target: RedirectTarget::Here(Heredoc {
                        delimiter: String::new(),
                        quoted: false,
                        body: String::new(),
                    }),
                    span: Span::new(start, self.at),
                }));
            }
            // A process substitution is a command, not a target.
            (Some(b'<'), Some(b'(')) | (Some(b'>'), Some(b'(')) => return Ok(None),
            (Some(b'>'), _) => {
                at += 1;
                RedirectOp::Write
            }
            (Some(b'<'), _) => {
                at += 1;
                RedirectOp::Read
            }
            _ => return Ok(None),
        };
        self.at = at;

        let target = self.redirect_target(op)?;
        // ⚠ `>&2` duplicates a descriptor; `>&file` sends BOTH streams to a
        // file. Same two characters, different construct, and the target is
        // what tells them apart — so the operator is settled after reading it.
        let op = match (op, &target) {
            (RedirectOp::DupOut, RedirectTarget::File(_)) => RedirectOp::BothWord,
            _ => op,
        };
        Ok(Some(Redirect {
            // ⚠ Always the effective descriptor, never the written one — see
            // `Redirect::fd`. `1> f` and `> f` are one redirection, and bash
            // says so by printing the first as the second.
            fd: fd.or(op.default_fd()),
            op,
            target,
            span: Span::new(start, self.at),
        }))
    }

    /// The word after `<<`: what the delimiter says, and whether it was quoted.
    ///
    /// ⚠ **Every quoted spelling is one node.** `<<'EOF'`, `<<"EOF"`, `<<\EOF`
    /// and `<<E"O"F` all print back from `declare -f` as `<<'EOF'`, so bash keeps
    /// the text and one bit and forgets which spelling produced them. Keeping
    /// more would be a distinction the second gate reads as a difference that
    /// is not there.
    ///
    /// Nothing here expands: the delimiter is taken literally, so a `*` in it is
    /// an asterisk rather than a glob. A `$` is refused all the same — bash reads
    /// it literally too, but a body's expansion depends on the quoting and
    /// guessing at what `<<$x` means about it is not worth one command.
    fn heredoc_delimiter(&mut self) -> Result<(String, bool), Refusal> {
        self.skip_blanks();
        let mut text = String::new();
        let mut quoted = false;
        let mut read_anything = false;
        while let Some(byte) = self.peek() {
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' | b';' | b'|' | b'&' | b'<' | b'>' | b')' => break,
                b'$' | b'`' => match classify_expansion(self.bytes, self.at, false) {
                    Some(reason) => return self.refuse(reason, 1),
                    None => {
                        read_anything = true;
                        text.push('$');
                        self.at += 1;
                    }
                },
                b'\'' => {
                    quoted = true;
                    read_anything = true;
                    let Segment {
                        kind: SegmentKind::Literal(inner),
                        ..
                    } = self.single_quoted()?
                    else {
                        unreachable!("a single-quoted run is always one literal")
                    };
                    text.push_str(&inner);
                }
                // ⚠ Read literally, NOT with [`Parser::double_quoted`]. A
                // heredoc delimiter undergoes quote removal and nothing else, so
                // `<<"E$F"` ends at a line reading `E$F` — the `$` names no
                // parameter. Sharing the word reader here would put an expansion
                // in a place the shell does not expand.
                b'"' => {
                    quoted = true;
                    read_anything = true;
                    self.at += 1;
                    loop {
                        match self.peek() {
                            None => {
                                return Err(Refusal {
                                    reason: Reason::UnterminatedQuote,
                                    span: Span::new(self.at, self.bytes.len()),
                                });
                            }
                            Some(b'"') => {
                                self.at += 1;
                                break;
                            }
                            Some(b'\\') if matches!(self.peek_at(1), Some(b'"' | b'\\')) => {
                                text.push(self.bytes[self.at + 1] as char);
                                self.at += 2;
                            }
                            Some(byte) => {
                                text.push(byte as char);
                                self.at += 1;
                            }
                        }
                    }
                }
                b'\\' => match self.peek_at(1) {
                    // A line continuation does not end the delimiter, and the
                    // body still starts after the line it continues onto.
                    Some(b'\n') => self.at += 2,
                    Some(next) => {
                        quoted = true;
                        read_anything = true;
                        text.push(next as char);
                        self.at += 2;
                    }
                    None => return self.refuse(Reason::DanglingEscape, 1),
                },
                _ => {
                    read_anything = true;
                    text.push(byte as char);
                    self.at += 1;
                }
            }
        }
        if !read_anything {
            return self.refuse(Reason::EmptyOperand, 1);
        }
        Ok((text, quoted))
    }

    fn redirect_target(&mut self, op: RedirectOp) -> Result<RedirectTarget, Refusal> {
        self.skip_blanks();
        if matches!(op, RedirectOp::DupOut | RedirectOp::DupIn) {
            if self.peek() == Some(b'-') {
                self.at += 1;
                return Ok(RedirectTarget::Close);
            }
            let from = self.at;
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.at += 1;
            }
            if self.at > from
                && let Ok(fd) = self.text[from..self.at].parse::<u32>()
            {
                return Ok(RedirectTarget::Fd(fd));
            }
            self.at = from;
        }
        if self.at_end_of_command() {
            return self.refuse(Reason::EmptyOperand, 1);
        }
        Ok(RedirectTarget::File(self.word(false)?))
    }

    /// One word. `first` is whether it opens the command, which is the only
    /// position where a reserved word or an assignment is grammar.
    fn word(&mut self, first: bool) -> Result<Word, Refusal> {
        let start = self.at;
        // ⚠ Decided here, from the bytes, and not from the finished word — see
        // [`opens_assignment`]. A word that binds a name is grammar, and no
        // amount of looking at its resolved text can tell you whether the name
        // was quoted.
        if first && opens_assignment(self.bytes, self.at) {
            return self.refuse(Reason::Assignment, 1);
        }
        let mut segments: Vec<Segment> = Vec::new();
        let mut quoted_anywhere = false;

        // ⚠ **At the head of EVERY word, not just the first.** `cd ~/Code` has
        // its tilde in the second, and scoping this to the command name once let
        // exactly that shape absorb an expansion into literal text.
        if self.peek() == Some(b'~') {
            segments.push(self.tilde()?);
        }

        while let Some(byte) = self.peek() {
            let at = self.at;
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' | b';' => break,
                // ⚠ **A backslash-newline JOINS a word, it does not end one.**
                // `"a"\⏎"b"` is one argument to bash, and breaking here split a
                // 337-character perl script into three words. Found by the
                // second gate: the round-trip law never saw it, because the
                // pieces print as separate words and read back as separate
                // words just as wrongly.
                b'\\' if self.peek_at(1) == Some(b'\n') => {
                    self.at += 2;
                }
                // `|`, `||` and `&` all end a word; which of them it is, is the
                // list's business rather than this reader's.
                b'|' | b'&' => break,
                b'<' | b'>' => {
                    // ⚠ Four constructs share these characters and they are not
                    // one build. A heredoc's operand is on the following lines;
                    // a process substitution is a whole command. Naming them
                    // apart is what let the corpus say which to do first.
                    // A `<<` reaching this reader is glued to the end of a word
                    // (`foo<<EOF`), which bash reads as a word and a redirection
                    // and this parser does not split — so what is unmodelled here
                    // is the gluing, not the heredoc.
                    let reason = if self.peek_at(1) == Some(b'(') {
                        Reason::ProcessSubstitution
                    } else if byte == b'<'
                        && self.peek_at(1) == Some(b'<')
                        && self.peek_at(2) == Some(b'<')
                    {
                        Reason::HereString
                    } else {
                        Reason::Redirection
                    };
                    return self.refuse(reason, 1);
                }
                b'(' | b')' | b'{' | b'}' => return self.refuse(Reason::Grouping, 1),
                // ⚠ A `$` that opens nothing is an ordinary character, and
                // bash agrees: `echo $`, `echo a$` and `echo $.` all parse and
                // print back unchanged.
                b'$' | b'`' => match classify_expansion(self.bytes, self.at, false) {
                    Some(Reason::Parameter) => segments.push(self.parameter(false)?),
                    Some(reason) => return self.refuse(reason, 1),
                    None => {
                        self.at += 1;
                        segments.push(Segment {
                            kind: SegmentKind::Literal("$".to_string()),
                            span: Span::new(at, self.at),
                        });
                    }
                },
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
                    segments.extend(self.double_quoted()?);
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
            && is_reserved(&text)
        {
            return Err(Refusal {
                reason: Reason::ReservedWord,
                span: word.span,
            });
        }
        Ok(word)
    }

    /// A tilde prefix, at the head of a word where the shell expands one.
    ///
    /// The prefix runs to the first `/` or to the end of the word — that is
    /// bash's rule, and it is why `~/Code` is a home directory followed by a
    /// path rather than a user called `/Code`.
    fn tilde(&mut self) -> Result<Segment, Refusal> {
        let start = self.at;
        self.at += 1;
        let from = self.at;
        while let Some(byte) = self.peek() {
            if byte == b'/' || is_bare_stop(byte) {
                break;
            }
            self.at += 1;
        }
        // ⚠ A quote inside the prefix turns the whole thing off — bash reads
        // `~"foo"` as the literal `~foo`. Rare, and refused rather than guessed.
        if matches!(self.peek(), Some(b'\'') | Some(b'"')) {
            return self.refuse(Reason::Tilde, 1);
        }
        let name = &self.text[from..self.at];
        let tilde = match name {
            "" => Tilde::Home,
            "+" => Tilde::Pwd,
            "-" => Tilde::OldPwd,
            // `~+2` is a directory-stack entry, which is a different thing and
            // is left for whoever needs it.
            _ if name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) =>
            {
                Tilde::User(name.to_string())
            }
            _ => {
                self.at = start;
                return self.refuse(Reason::Tilde, 1);
            }
        };
        Ok(Segment {
            kind: SegmentKind::Tilde(tilde),
            span: Span::new(start, self.at),
        })
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

    /// A double-quoted run, which may hold more than one segment.
    ///
    /// ⚠ **Quoting suppresses splitting and globbing, not expansion**, so a `$`
    /// in here is a parameter exactly as it is outside — and the segment it
    /// makes carries `quoted: true`, which is the whole difference between
    /// `echo $x` and `echo "$x"`.
    fn double_quoted(&mut self) -> Result<Vec<Segment>, Refusal> {
        let open = self.at;
        self.at += 1;
        let mut segments: Vec<Segment> = Vec::new();
        let mut text = String::new();
        let mut from = self.at;
        // Flush the literal run gathered so far, so a parameter can be pushed
        // after it in the order they were written.
        macro_rules! flush {
            () => {
                if !text.is_empty() {
                    segments.push(Segment {
                        kind: SegmentKind::Literal(std::mem::take(&mut text)),
                        span: Span::new(from, self.at),
                    });
                }
            };
        }
        loop {
            let Some(byte) = self.peek() else {
                return Err(Refusal {
                    reason: Reason::UnterminatedQuote,
                    span: Span::new(open, self.bytes.len()),
                });
            };
            match byte {
                b'"' => break,
                b'$' | b'`' => match classify_expansion(self.bytes, self.at, true) {
                    Some(Reason::Parameter) => {
                        flush!();
                        segments.push(self.parameter(true)?);
                        from = self.at;
                    }
                    Some(reason) => return self.refuse(reason, 1),
                    None => {
                        text.push('$');
                        self.at += 1;
                    }
                },
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
                    let run = self.at;
                    while let Some(next) = self.peek() {
                        if matches!(next, b'"' | b'\\' | b'$' | b'`') {
                            break;
                        }
                        self.at += 1;
                    }
                    text.push_str(&self.text[run..self.at]);
                }
            }
        }
        flush!();
        self.at += 1;
        // An empty pair of quotes is a word with no content, and the word reader
        // needs a segment to say so.
        if segments.is_empty() {
            segments.push(Segment {
                kind: SegmentKind::Literal(String::new()),
                span: Span::new(open, self.at),
            });
        }
        Ok(segments)
    }

    /// `$name`, `${name}`, `$1`, `$@` — with the braces resolved away.
    ///
    /// Only reached when [`classify_expansion`] has already said this is a plain
    /// parameter, so every branch below finds a name.
    fn parameter(&mut self, quoted: bool) -> Result<Segment, Refusal> {
        let start = self.at;
        self.at += 1;
        let name = if self.peek() == Some(b'{') {
            self.at += 1;
            let from = self.at;
            while self.peek().is_some_and(|byte| byte != b'}') {
                self.at += 1;
            }
            if self.peek().is_none() {
                self.at = start;
                return self.refuse(Reason::UnterminatedExpansion, 1);
            }
            let name = self.text[from..self.at].to_string();
            self.at += 1;
            name
        } else if self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            // ⚠ **Exactly one digit.** `$10` is `${1}` followed by a `0` — bash
            // prints both spellings identically, so this was settled by running
            // it rather than by reading the printer.
            self.at += 1;
            self.text[start + 1..self.at].to_string()
        } else if self
            .peek()
            .is_some_and(|byte| !byte.is_ascii_alphanumeric() && byte != b'_')
        {
            // A special parameter: `$@`, `$?`, `$$` and the rest, one character
            // each.
            self.at += 1;
            self.text[start + 1..self.at].to_string()
        } else {
            let from = self.at;
            while self
                .peek()
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                self.at += 1;
            }
            self.text[from..self.at].to_string()
        };
        Ok(Segment {
            kind: SegmentKind::Parameter(Parameter { name, quoted }),
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

/// Which construct does the `$` or backtick at `at` open?
///
/// `None` where it opens nothing: a `$` not followed by something expandable is
/// an ordinary dollar sign, and bash agrees — `echo $`, `echo a$` and `echo $.`
/// all parse and print back unchanged.
///
/// ⚠ **Shared with the survey deliberately.** The survey is otherwise a separate
/// scanner, but a disagreement about *which* expansion this is would show up as
/// survey drift with no way to tell which of the two was wrong. The lexical
/// question has one answer, so it has one implementation.
pub fn classify_expansion(bytes: &[u8], at: usize, in_double_quotes: bool) -> Option<Reason> {
    if bytes.get(at) == Some(&b'`') {
        return Some(Reason::CommandSubstitution);
    }
    if bytes.get(at) != Some(&b'$') {
        return None;
    }
    match bytes.get(at + 1).copied() {
        Some(b'(') if bytes.get(at + 2) == Some(&b'(') => Some(Reason::Arithmetic),
        Some(b'(') => Some(Reason::CommandSubstitution),
        // Inside double quotes a following quote is an ordinary character —
        // `"a$'b'"` holds a dollar, not an ANSI-C string — so the quoting forms
        // are only reachable from unquoted text.
        Some(b'\'') if !in_double_quotes => Some(Reason::AnsiQuote),
        Some(b'"') if !in_double_quotes => Some(Reason::LocaleQuote),
        Some(b'{') => Some(brace_expansion(bytes, at + 2)),
        Some(byte) if byte.is_ascii_alphanumeric() || byte == b'_' => Some(Reason::Parameter),
        // `$@`, `$*`, `$?`, `$$`, `$!`, `$#`, `$-`: the special parameters, each
        // one character and each naming a value the shell already holds.
        Some(b'@' | b'*' | b'?' | b'$' | b'!' | b'#' | b'-') => Some(Reason::Parameter),
        _ => None,
    }
}

/// `${…}`: naming a parameter, or operating on one?
///
/// The split is what the braces hold. A bare name is the same node `$name` is;
/// anything else is an operator, and operators are a language rather than a
/// construct — `${x:-y}` defaults, `${#x}` measures, `${x%%y}` is a rational
/// transduction that would need an automaton.
fn brace_expansion(bytes: &[u8], from: usize) -> Reason {
    // A lone special parameter in braces — `${@}`, `${#}` — is still just a name
    // for a value the shell holds. Checked first, because the loop below stops
    // at the very characters that spell one.
    if matches!(
        bytes.get(from),
        Some(b'@' | b'*' | b'?' | b'$' | b'!' | b'#' | b'-')
    ) && bytes.get(from + 1) == Some(&b'}')
    {
        return Reason::Parameter;
    }
    let mut at = from;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            b'}' => break,
            _ if byte.is_ascii_alphanumeric() || byte == b'_' => at += 1,
            _ => return Reason::ParameterOperator,
        }
    }
    // `${}` names nothing, so it is not the same node `$name` is.
    if at == from {
        return Reason::ParameterOperator;
    }
    Reason::Parameter
}

/// Resolve the backslash-newlines in an unquoted heredoc body, as bash does at
/// parse time.
///
/// ⚠ **The join is not a text replacement.** A backslash escapes the character
/// after it, so an escaped backslash protects the newline that follows: `a\\⏎b`
/// stays two lines while `a\⏎b` becomes one. Measured — a naive
/// `replace("\\\n", "")` gets the first wrong.
fn join_continuations(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut at = 0;
    while at < bytes.len() {
        match (bytes[at], bytes.get(at + 1)) {
            (b'\\', Some(b'\n')) => at += 2,
            // The backslash and whatever it escapes both stay: bash resolves
            // neither until the body is expanded, and `\$` comes back from
            // `declare -f` with the backslash still on it. The escaped character
            // is copied whole rather than by two bytes — it may be multi-byte,
            // and it is the only place in this module where a slice is taken at
            // a position the shell's own grammar did not choose.
            (b'\\', Some(_)) => {
                out.push('\\');
                at += 1;
                let width = body[at..].chars().next().map_or(0, char::len_utf8);
                out.push_str(&body[at..at + width]);
                at += width;
            }
            _ => {
                let from = at;
                while at < bytes.len() && bytes[at] != b'\\' {
                    at += 1;
                }
                out.push_str(&body[from..at]);
            }
        }
    }
    out
}

/// Hand each heredoc opener the body that was read for it.
///
/// ⚠ **This is a positional match, and it is sound because both sequences are in
/// the order the text is written.** Bodies are read when a line ends, in the
/// order their openers appeared on it; the walk below visits redirections in
/// that same order. Nothing else pairs them — a heredoc's opener and its body
/// share no delimiter that is unique (`cat <<A <<A` is legal, and its two bodies
/// differ), so order is the only thing that can.
fn fill_script(script: &mut Script, bodies: &mut impl Iterator<Item = Heredoc>) {
    for item in &mut script.items {
        if let Item::List(list) = item {
            fill_and_or(list, bodies);
        }
    }
}

fn fill_and_or(list: &mut AndOr, bodies: &mut impl Iterator<Item = Heredoc>) {
    fill_pipeline(&mut list.first, bodies);
    for link in &mut list.rest {
        fill_pipeline(&mut link.pipeline, bodies);
    }
}

fn fill_pipeline(pipeline: &mut Pipeline, bodies: &mut impl Iterator<Item = Heredoc>) {
    for command in &mut pipeline.commands {
        for redirect in &mut command.redirects {
            if let RedirectTarget::Here(here) = &mut redirect.target
                && let Some(body) = bodies.next()
            {
                *here = body;
            }
        }
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

/// Does an assignment prefix start at `at`?
///
/// ⚠ **Read from the raw bytes, before any quoting is resolved, because that is
/// where the rule lives.** Bash asks whether the NAME is quoted, not whether the
/// word is: `FOO="bar"` and `FOO='bar'` are bindings, while `'FOO=bar'` and
/// `"FOO"=bar` are commands with an odd name. Measured, all four.
///
/// Testing the finished word instead is what made `FOO="bar" cmd` parse as a
/// command *named* `FOO=bar` — a wrong tree that prints and re-reads as itself,
/// and that bash's printer cannot object to either, since it emits the word
/// verbatim. Only construction catches it.
pub fn opens_assignment(bytes: &[u8], at: usize) -> bool {
    let mut at = match bytes.get(at) {
        Some(byte) if byte.is_ascii_alphabetic() || *byte == b'_' => at + 1,
        _ => return false,
    };
    while bytes
        .get(at)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        at += 1;
    }
    // `FOO+=bar` appends, and is a binding just as much as `FOO=bar` is.
    if bytes.get(at) == Some(&b'+') {
        at += 1;
    }
    bytes.get(at) == Some(&b'=')
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
