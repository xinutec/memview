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
//! ⚠ **A refusal names what cannot be read, and the survey checks that it is
//! the truth.** Calling a malformed `for` header a `Redirection` was accurate
//! about the character and wrong about the construct — and since redirections
//! ARE modelled, the survey looked for one and found nothing. The invariant
//! caught it; no test would have.
//!
//! Scanning is over bytes. Every character with meaning to the shell is ASCII, so
//! a multi-byte sequence can only ever be interior to a literal run, and slicing
//! at a special character always lands on a character boundary.

use super::ast::{
    Anchor, AndOr, Arith, Arm, ArmEnd, Assignment, BinaryOp, Brace, Case, Command, CommandKind,
    Comment, Conditional, Connector, Direction, ForArith, ForLoop, Function, Glob, Heredoc, Item,
    Link, Parameter, ParameterOp, Pipeline, ProcessSubstitution, Redirect, RedirectOp,
    RedirectTarget, Replace, Script, Segment, SegmentKind, Simple, Span, Step, Subscript,
    Substitution, Tilde, Timed, UnaryOp, WhileLoop, Word,
};

/// Where a word is being read, which decides what expands inside it.
///
/// ⚠ **Not a style choice — measured.** `FOO=*.txt` binds the literal `*.txt`
/// while `cmd *.txt` names files, and `T=a:~/x` expands a tilde that `cmd a:~/x`
/// leaves alone. A single word reader would have to be wrong in one of the two
/// places.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WordKind {
    /// An argument or a redirection target: globs, and a tilde at the head only.
    Argument,
    /// An assignment's value: no pathname expansion, and a tilde after any
    /// unquoted `:` as well as at the head.
    Value,
}

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
    /// `$(cmd)` — a whole script, whose value is its output. The first
    /// construct that needs this parser to recurse into itself.
    CommandSubstitution,
    /// `` `cmd` `` — the same meaning, a different build.
    ///
    /// ⚠ **Split from `$( )` because bash treats the two differently.** It
    /// NORMALISES the interior of `$(a|b)`, printing it back as `$(a | b)`, and
    /// prints `` `a|b` `` verbatim — so the second gate can see inside one and
    /// not the other. The escaping differs too: a backtick's interior needs
    /// `\``, `\$` and `\\` resolved before it is a script at all.
    Backtick,
    /// `$((…))` — arithmetic, which is its own grammar.
    Arithmetic,
    /// `>`, `>>`, `<`, `2>&1`, `&>`, `>|`, `<>`, `{fd}>` — a file or a
    /// descriptor, and nothing that carries a body.
    Redirection,
    /// `<<<` — a value, not a file, and no body.
    HereString,
    /// `<(cmd)` or `>(cmd)`: a whole command, so it needs grouping first.
    ProcessSubstitution,
    /// A `(`, `)`, `{` or `}` where no group can be read — an unmatched one, or
    /// a `}` closing nothing.
    Grouping,
    /// `{a,b}`, `{1..9}` — brace EXPANSION, which is not grouping at all.
    ///
    /// ⚠ **Split off because a refusal must name the construct.** These share a
    /// character with a brace group and nothing else: this one is word-level,
    /// like a glob — `echo {a,b}.txt` is one word that expands to two — while
    /// `{ a; }` is a command list. Counting them together said "how many
    /// commands hold a brace", which is not a number anything can be built
    /// against, and it hid a word construct inside a compound-statement build.
    BraceExpansion,
    /// `[…]` inside a word — a bracket expression, not the `[` builtin.
    BracketExpression,
    /// A `~` opening a word, which expands to a home directory.
    Tilde,
    /// `if … then … elif … else … fi`.
    Conditional,
    /// `for`, `select`, `while`, `until`, and the `do … done` they carry.
    Loop,
    /// `case … esac`, whose arms are a pattern grammar of their own.
    Case,
    /// `[[ … ]]` — a conditional expression, which is its own language and not
    /// the `[` builtin.
    TestExpression,
    /// `function name`. The `name() { … }` spelling is refused as grouping.
    FunctionDefinition,
    /// `coproc`.
    Coproc,
    /// `!` anywhere but a pipeline's head, which bash calls a syntax error too.
    MisplacedNegation,
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
            Reason::CommandSubstitution => "command substitution ($(…))",
            Reason::Backtick => "backtick substitution (`…`)",
            Reason::Arithmetic => "arithmetic ($((…)))",
            Reason::Redirection => "redirection (> >> < 2>&1 &>)",
            Reason::HereString => "here-string (<<<)",
            Reason::ProcessSubstitution => "process substitution (<( >()",
            Reason::Grouping => "grouping (unmatched ( ) { })",
            Reason::BraceExpansion => "brace expansion ({a,b}, {1..9})",
            Reason::BracketExpression => "bracket expression ([…])",
            Reason::Tilde => "tilde (~)",
            Reason::Conditional => "conditional (if then elif else fi)",
            Reason::Loop => "loop (for while until select do done)",
            Reason::Case => "case … esac",
            Reason::TestExpression => "test expression ([[ ]])",
            Reason::FunctionDefinition => "function definition (function name)",
            Reason::Coproc => "coproc",
            Reason::MisplacedNegation => "! outside a pipeline head",
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
/// Which construct does this reserved word belong to, if it is one?
///
/// ⚠ **Grouped by construct, because a reason is a unit of work.** `if` and
/// `case` share nothing but being keywords: one is a conditional, the other a
/// pattern grammar. Counting them together would say how many commands hold a
/// keyword, which is not a number anybody can build against.
///
/// The interior words (`then`, `do`, `esac`) sit with their openers. They are
/// almost never the first word of a command — the opener is refused long before
/// the scan reaches them — but where one is, it belongs to the same build.
pub fn reserved_word(text: &str) -> Option<Reason> {
    Some(match text {
        "if" | "then" | "elif" | "else" | "fi" => Reason::Conditional,
        "for" | "select" | "while" | "until" | "do" | "done" | "in" => Reason::Loop,
        "case" | "esac" => Reason::Case,
        "[[" | "]]" => Reason::TestExpression,
        "function" => Reason::FunctionDefinition,
        "coproc" => Reason::Coproc,
        "!" => Reason::MisplacedNegation,
        _ => return None,
    })
}

pub fn parse(text: &str) -> Result<Script, Refusal> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        text,
        at: 0,
        pending: Vec::new(),
        bodies: Vec::new(),
        parens: 0,
        arm_depth: 0,
        in_pattern: false,
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
    /// How many `$(` are open. A `)` closes a command list only inside one.
    parens: usize,
    /// How many `case` arm bodies are being read. Inside one, `;;`, `;&` and
    /// `;;&` end the list rather than separating commands in it.
    arm_depth: usize,
    /// Is a `case` PATTERN being read? An unquoted `)` ends one, wherever the
    /// enclosing text would otherwise take it for something else.
    in_pattern: bool,
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
        let items = self.items(&[])?;
        // A heredoc opened on a line the text ended without terminating: its
        // body is whatever is left, which at this point is nothing.
        self.read_pending_bodies()?;
        // Nothing consumed the text, so a keyword is sitting where a command
        // should be: `done` with no loop, or a stray `fi`.
        if let Some(reason) = self.keyword_here() {
            return self.refuse(reason, 1);
        }
        Ok(Script {
            items,
            span: Span::new(0, self.bytes.len()),
        })
    }

    /// A run of items, stopping before any of `until` without consuming it.
    ///
    /// ⚠ **The one place a command list is read**, so a loop's body is the same
    /// grammar as a whole script rather than a second, nearly-identical reader.
    /// The terminators are the keywords that close the construct asking for the
    /// list — `do`, `done` — and they are left in place for the caller to take.
    fn items(&mut self, until: &[&str]) -> Result<Vec<Item>, Refusal> {
        let mut items = Vec::new();
        loop {
            self.skip_blanks();
            if until.iter().any(|word| self.at_keyword(word)) {
                break;
            }
            // Inside a substitution the closing paren ends the list, and the
            // caller takes it.
            if self.parens > 0 && self.peek() == Some(b')') {
                break;
            }
            match self.peek() {
                None => break,
                // ⚠ Inside a `case` arm, `;;`, `;&` and `;;&` END the body. Read
                // as separators they would be eaten as two empty steps, and the
                // arm would swallow the rest of the case.
                Some(b';')
                    if self.arm_depth > 0 && matches!(self.peek_at(1), Some(b';' | b'&')) =>
                {
                    break;
                }
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
        Ok(items)
    }

    /// Is an unquoted reserved word sitting under the cursor?
    fn keyword_here(&self) -> Option<Reason> {
        let end = self.bytes[self.at..]
            .iter()
            .position(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b';' | b'|' | b'&'))
            .map_or(self.bytes.len(), |offset| self.at + offset);
        reserved_word(self.text.get(self.at..end)?)
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
        ) || (self.parens > 0 && self.peek() == Some(b')'))
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
            // ⚠ Words are not what makes a command. `FOO=bar` binds and `> out`
            // truncates, each with no word in it, and dropping them would lose a
            // whole statement rather than a detail.
            if !command.is_empty() {
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
    fn at_keyword(&self, word: &str) -> bool {
        let end = self.at + word.len();
        self.text.get(self.at..end) == Some(word)
            && matches!(
                self.bytes.get(end).copied(),
                // `)` closes a substitution, so `done)` ends the keyword too.
                None | Some(b' ' | b'\t' | b'\r' | b'\n' | b';' | b'|' | b'&' | b')')
            )
    }

    fn take_keyword(&mut self, word: &str) -> bool {
        let end = self.at + word.len();
        if self.text.get(self.at..end) != Some(word) {
            return false;
        }
        // ⚠ The same boundary set [`Parser::at_keyword`] uses, `&` included:
        // `done&` backgrounds a loop and `fi&` a conditional, both without a
        // space, and a closing keyword that did not end there would be read as
        // the word `done&`.
        let boundary = matches!(
            self.bytes.get(end).copied(),
            None | Some(b' ' | b'\t' | b'\r' | b'\n' | b';' | b'|' | b'&' | b')')
        );
        if boundary {
            self.at = end;
        }
        boundary
    }

    fn command(&mut self) -> Result<Command, Refusal> {
        let start = self.at;
        self.skip_blanks();
        if let Some(kind) = self.compound()? {
            // ⚠ A compound takes its redirections after the closing keyword,
            // and bash prints them there: `while a; do b; done > out`.
            let mut redirects = Vec::new();
            loop {
                self.skip_blanks();
                if self.at_end_of_command() {
                    break;
                }
                match self.redirect()? {
                    Some(redirect) => redirects.push(redirect),
                    None => break,
                }
            }
            return Ok(Command {
                kind,
                redirects,
                span: Span::new(start, self.at),
            });
        }
        let mut assignments: Vec<Assignment> = Vec::new();
        let mut words: Vec<Word> = Vec::new();
        let mut redirects: Vec<Redirect> = Vec::new();
        loop {
            self.skip_blanks();
            // ⚠ **A prefix, so it is read only while no word has been.** `A=1 cmd
            // B=2` binds `A` and passes `B=2` as an argument — bash prints that
            // back unchanged — and the test is on the raw bytes because whether
            // the NAME is quoted is what decides it. See [`opens_assignment`].
            if words.is_empty() && opens_assignment(self.bytes, self.at) {
                assignments.push(self.assignment()?);
                continue;
            }
            match self.peek() {
                None | Some(b';') | Some(b'\n') => break,
                // `&>` is a redirection, not the list's `&`. Checked first, or
                // every `cmd &> log` would end the command at the ampersand.
                Some(b'&') if self.peek_at(1) == Some(b'>') => {}
                // The pipeline owns a bare `|`; `||`, `&&` and `&` belong to
                // the list above it. All of them end this command.
                Some(b'|') | Some(b'&') => break,
                Some(b')') if self.parens > 0 => break,
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
            let word = self.word(words.is_empty(), WordKind::Argument)?;
            words.push(word);
        }
        Ok(Command {
            kind: CommandKind::Simple(Simple { assignments, words }),
            redirects,
            span: Span::new(start, self.at),
        })
    }

    /// A loop, if one opens here.
    ///
    /// ⚠ **The body is read by [`Parser::items`], the same reader a whole script
    /// uses.** A loop's body is a command list and nothing more, so a second
    /// reader for it would be a second place for the grammar to be wrong.
    fn compound(&mut self) -> Result<Option<CommandKind>, Refusal> {
        // ⚠ `((` is arithmetic, not two subshells — and bash agrees: `((a))`
        // evaluates where `( (a) )` would run a command called `a`. Checked
        // first, because the subshell reader would otherwise take the first
        // paren and leave a tree that means something else entirely.
        if self.peek() == Some(b'(') && self.peek_at(1) == Some(b'(') {
            return Ok(Some(CommandKind::Arithmetic(self.arith_command()?)));
        }
        if self.peek() == Some(b'(') {
            return Ok(Some(CommandKind::Subshell(self.subshell()?)));
        }
        // ⚠ `{` is the keyword only where a word could not start: `{ a; }` is a
        // group and `{a,b}` is one word. Bash decides on the blank, and so does
        // this — `{a,b}` falls through to the word reader, which names it a
        // brace expansion rather than a group.
        if self.peek() == Some(b'{')
            && matches!(self.peek_at(1), Some(b' ' | b'\t' | b'\n' | b'\r'))
        {
            return Ok(Some(CommandKind::Group(self.brace_group()?)));
        }
        if let Some(function) = self.function()? {
            return Ok(Some(CommandKind::Function(function)));
        }
        if self.at_keyword("if") {
            self.at += 2;
            return Ok(Some(CommandKind::If(self.conditional()?)));
        }
        if self.at_keyword("case") {
            self.at += 4;
            return Ok(Some(CommandKind::Case(self.case_command()?)));
        }
        let select = self.at_keyword("select");
        if select || self.at_keyword("for") {
            self.at += if select { 6 } else { 3 };
            self.skip_blanks();
            // `for ((…))` is a different loop with the same keyword.
            if !select && self.peek() == Some(b'(') && self.peek_at(1) == Some(b'(') {
                return Ok(Some(CommandKind::ForArith(self.for_arith()?)));
            }
            return Ok(Some(CommandKind::For(self.for_loop(select)?)));
        }
        let until = self.at_keyword("until");
        if until || self.at_keyword("while") {
            self.at += 5; // both `while` and `until` are five characters
            let condition = self.items(&["do"])?;
            if condition
                .iter()
                .any(|item| matches!(item, Item::Comment(_)))
            {
                return self.refuse(Reason::CommentInList, 1);
            }
            let body = self.loop_body()?;
            return Ok(Some(CommandKind::While(WhileLoop {
                until,
                condition,
                body,
            })));
        }
        Ok(None)
    }

    /// `for NAME [in words]; do body done`.
    fn for_loop(&mut self, select: bool) -> Result<ForLoop, Refusal> {
        self.skip_blanks();
        let from = self.at;
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            self.at += 1;
        }
        if self.at == from {
            return self.refuse(Reason::EmptyOperand, 1);
        }
        let name = self.text[from..self.at].to_string();
        self.skip_blanks();
        let words = if self.take_keyword("in") {
            let mut words = Vec::new();
            loop {
                self.skip_blanks();
                if self.at_end_of_command() {
                    break;
                }
                // ⚠ A redirection in a `for` header is a syntax error to bash —
                // `for f in a 2>/dev/null; do x; done` is refused outright — so
                // what cannot be read here is the LOOP, not a redirection.
                // Naming it `Redirection` sent the survey looking for a
                // construct that is modelled, and it reported nothing.
                //
                // Remapped rather than tested for, because the operator can sit
                // at the head of the word or inside it — `2>/dev/null` starts
                // with a digit — and both mean the same thing about the header.
                words.push(self.word(false, WordKind::Argument).map_err(|refusal| {
                    match refusal.reason {
                        Reason::Redirection => Refusal {
                            reason: Reason::Loop,
                            span: refusal.span,
                        },
                        _ => refusal,
                    }
                })?);
            }
            words
        } else {
            // ⚠ **Desugared, because bash desugars it.** `for f; do …` comes
            // back from `declare -f` as `for f in "$@"; do …`, so a tree that
            // recorded the omission would make one command two trees and the
            // second gate would say so.
            vec![Word {
                segments: vec![Segment {
                    kind: SegmentKind::Parameter(Parameter {
                        name: "@".to_string(),
                        quoted: true,
                        subscript: None,
                        op: None,
                    }),
                    span: Span::new(self.at, self.at),
                }],
                span: Span::new(self.at, self.at),
            }]
        };
        let body = self.loop_body()?;
        Ok(ForLoop {
            name,
            words,
            select,
            body,
        })
    }

    /// `do body done`, with the separator before `do` already allowed for.
    fn loop_body(&mut self) -> Result<Vec<Item>, Refusal> {
        // The `;` or newline between the header and `do` is a separator like any
        // other, and `items` steps over both.
        let skipped = self.items(&["do"])?;
        if !skipped.is_empty() {
            return self.refuse(Reason::Loop, 1);
        }
        if !self.take_keyword("do") {
            return self.refuse(Reason::Loop, 1);
        }
        let body = self.items(&["done"])?;
        if !self.take_keyword("done") {
            return self.refuse(Reason::Loop, 1);
        }
        // ⚠ The printer puts a loop on one line, where a comment would swallow
        // everything after it. Refused rather than dropped — the same answer
        // this tree gives a comment between a list operator and its right-hand
        // side, and for the same reason.
        if body.iter().any(|item| matches!(item, Item::Comment(_))) {
            return self.refuse(Reason::CommentInList, 1);
        }
        Ok(body)
    }

    /// `( list )` — a command list that runs in a subshell.
    ///
    /// ⚠ **The closing paren is found the same way a `$( )`'s is**, by the
    /// depth counter every list reader already consults. Sharing it is what
    /// makes `( echo ")" )` work: the quote reader has stepped over the paren
    /// inside the word before the list reader ever sees it.
    fn subshell(&mut self) -> Result<Vec<Item>, Refusal> {
        self.at += 1; // the `(`
        self.parens += 1;
        let items = self.items(&[]);
        self.parens -= 1;
        let items = items?;
        if self.peek() != Some(b')') {
            return self.refuse(Reason::Grouping, 1);
        }
        self.at += 1;
        if items.is_empty() {
            return self.refuse(Reason::EmptyOperand, 1);
        }
        self.body_without_comments(items)
    }

    /// `{ list; }` — a command list in this shell.
    fn brace_group(&mut self) -> Result<Vec<Item>, Refusal> {
        self.at += 1; // the `{`
        let items = self.items(&["}"])?;
        if !self.take_keyword("}") {
            return self.refuse(Reason::Grouping, 1);
        }
        if items.is_empty() {
            return self.refuse(Reason::EmptyOperand, 1);
        }
        self.body_without_comments(items)
    }

    /// `name() { … }` or `name() ( … )`, if one starts here.
    ///
    /// ⚠ **Both spellings give the same tree**, because bash gives them the
    /// same print: a `( … )` body comes back wrapped in a brace group. See
    /// [`Function`].
    fn function(&mut self) -> Result<Option<Function>, Refusal> {
        // ⚠ **`function NAME` has to be read, because bash WRITES it.**
        // `declare -f` prints every definition that way whichever spelling was
        // used, so a parser that refused the keyword could not read back its own
        // print — 141 commands failed the round-trip law on exactly that, and
        // gate 2 would have failed on the same text for the same reason.
        if self.at_keyword("function") {
            let start = self.at;
            self.at += 8;
            self.skip_blanks();
            let from = self.at;
            while self.peek().is_some_and(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')
            }) {
                self.at += 1;
            }
            if self.at == from {
                self.at = start;
                return self.refuse(Reason::FunctionDefinition, 1);
            }
            let name = self.text[from..self.at].to_string();
            self.skip_blanks();
            // The parens are optional after `function`, and carry nothing.
            if self.peek() == Some(b'(') {
                self.at += 1;
                self.skip_blanks();
                if self.peek() != Some(b')') {
                    return self.refuse(Reason::FunctionDefinition, 1);
                }
                self.at += 1;
            }
            self.skip_blanks_and_newlines()?;
            let body = self.function_body()?;
            return Ok(Some(Function { name, body }));
        }
        let start = self.at;
        let mut at = self.at;
        while self.bytes.get(at).is_some_and(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')
        }) {
            at += 1;
        }
        if at == start {
            return Ok(None);
        }
        // `name ()` is legal too, so the blanks between are stepped over — but
        // only to look; nothing is consumed unless this really is a definition.
        let mut after = at;
        while self
            .bytes
            .get(after)
            .is_some_and(|b| matches!(b, b' ' | b'\t'))
        {
            after += 1;
        }
        if self.bytes.get(after) != Some(&b'(') {
            return Ok(None);
        }
        let mut close = after + 1;
        while self
            .bytes
            .get(close)
            .is_some_and(|b| matches!(b, b' ' | b'\t'))
        {
            close += 1;
        }
        if self.bytes.get(close) != Some(&b')') {
            return Ok(None);
        }
        let name = self.text[start..at].to_string();
        self.at = close + 1;
        self.skip_blanks_and_newlines()?;
        let body = self.function_body()?;
        Ok(Some(Function { name, body }))
    }

    /// The `{ … }` or `( … )` after a definition's name.
    ///
    /// ⚠ A `( … )` body comes back from `declare -f` wrapped in a brace group,
    /// so that is the tree both spellings give — bash's own canonical form, and
    /// the same collapse an `elif` gets.
    fn function_body(&mut self) -> Result<Vec<Item>, Refusal> {
        match self.peek() {
            Some(b'(') => {
                let span = Span::new(self.at, self.at);
                let inner = self.subshell()?;
                Ok(vec![one_command(CommandKind::Subshell(inner), span)])
            }
            Some(b'{') => self.brace_group(),
            _ => self.refuse(Reason::FunctionDefinition, 1),
        }
    }

    /// A compound's body, refused if it carries a comment.
    ///
    /// ⚠ The printer writes every compound on one line, where a comment would
    /// swallow the rest of it — the same answer a loop body and a conditional
    /// arm get, and for the same reason.
    fn body_without_comments(&self, items: Vec<Item>) -> Result<Vec<Item>, Refusal> {
        if items.iter().any(|item| matches!(item, Item::Comment(_))) {
            return self.refuse(Reason::CommentInList, 1);
        }
        Ok(items)
    }

    /// `if cond; then body [elif …] [else body] fi`, with the opening keyword
    /// already taken.
    ///
    /// ⚠ **An `elif` recurses, and the recursion takes the `fi`.** A whole chain
    /// closes with exactly one `fi`, so it belongs to whichever arm ends the
    /// chain — the nested call where there is an `elif`, this one otherwise.
    /// That is the desugaring bash itself performs, and [`Conditional`] says why
    /// the tree has to follow it.
    fn conditional(&mut self) -> Result<Conditional, Refusal> {
        let condition = self.arm(&["then"])?;
        if !self.take_keyword("then") {
            return self.refuse(Reason::Conditional, 1);
        }
        let then = self.arm(&["elif", "else", "fi"])?;
        let otherwise = if self.at_keyword("elif") {
            let start = self.at;
            self.at += 4;
            let nested = self.conditional()?;
            Some(vec![one_command(
                CommandKind::If(nested),
                Span::new(start, self.at),
            )])
        } else if self.take_keyword("else") {
            let body = self.arm(&["fi"])?;
            if !self.take_keyword("fi") {
                return self.refuse(Reason::Conditional, 1);
            }
            Some(body)
        } else {
            if !self.take_keyword("fi") {
                return self.refuse(Reason::Conditional, 1);
            }
            None
        };
        Ok(Conditional {
            condition,
            then,
            otherwise,
        })
    }

    /// One of a conditional's lists — a condition or a branch.
    ///
    /// ⚠ **An empty one is a syntax error, not an empty list.** Bash refuses
    /// `if; then b; fi` and `if a; then fi` outright, so this is a claim about
    /// the input rather than about what is modelled, and `bash -n` adjudicates
    /// it.
    fn arm(&mut self, until: &[&str]) -> Result<Vec<Item>, Refusal> {
        let items = self.items(until)?;
        // ⚠ The printer puts a conditional on one line, where a comment would
        // swallow everything after it. Refused rather than dropped — the same
        // answer a loop body's comment gets, and bash has no opinion either way
        // because it deletes them.
        if items.iter().any(|item| matches!(item, Item::Comment(_))) {
            return self.refuse(Reason::CommentInList, 1);
        }
        if items.is_empty() {
            return self.refuse(Reason::EmptyOperand, 1);
        }
        Ok(items)
    }

    /// `case word in [pattern) body ;;]… esac`, with `case` already taken.
    ///
    /// ⚠ **`esac` right after `in` is a case with NO ARMS**, and legal — bash
    /// accepts `case $x in esac`. It is also why `esac` cannot be a bare
    /// pattern: bash reads the keyword first and calls the `)` a syntax error.
    /// A *quoted* `esac` is an ordinary pattern, which is why the printer has to
    /// quote one.
    fn case_command(&mut self) -> Result<Case, Refusal> {
        self.skip_blanks();
        if self.at_end_of_command() {
            return self.refuse(Reason::EmptyOperand, 1);
        }
        let word = self.word(false, WordKind::Argument)?;
        self.skip_blanks_and_newlines()?;
        if !self.take_keyword("in") {
            return self.refuse(Reason::Case, 1);
        }
        let mut arms = Vec::new();
        loop {
            self.skip_blanks_and_newlines()?;
            if self.take_keyword("esac") {
                return Ok(Case { word, arms });
            }
            // A comment here has nowhere to go in a one-line print, exactly as
            // one in a loop body has.
            if self.peek() == Some(b'#') {
                return self.refuse(Reason::CommentInList, 1);
            }
            if self.peek().is_none() {
                return self.refuse(Reason::Case, 1);
            }
            arms.push(self.case_arm()?);
        }
    }

    /// `[(] pattern [| pattern]… ) body [;;|;&|;;&]`.
    fn case_arm(&mut self) -> Result<Arm, Refusal> {
        // ⚠ **A leading `(` is stepped over and NOT recorded.** Bash prints
        // `(a)` back as `a)`, so a tree holding the paren would make one command
        // two trees and the second gate could never object.
        if self.peek() == Some(b'(') {
            self.at += 1;
        }
        let mut patterns = Vec::new();
        loop {
            self.skip_blanks_and_newlines()?;
            let pattern = self.pattern()?;
            if pattern.segments.is_empty() {
                return self.refuse(Reason::EmptyOperand, 1);
            }
            patterns.push(pattern);
            self.skip_blanks();
            match self.peek() {
                Some(b'|') => self.at += 1,
                Some(b')') => {
                    self.at += 1;
                    break;
                }
                // `a b)` is two words where one pattern belongs, which bash
                // refuses outright.
                _ => return self.refuse(Reason::Case, 1),
            }
        }
        self.arm_depth += 1;
        let body = self.items(&["esac"]);
        self.arm_depth -= 1;
        let body = body?;
        if body.iter().any(|item| matches!(item, Item::Comment(_))) {
            return self.refuse(Reason::CommentInList, 1);
        }
        // ⚠ **Three terminators, and they are three different programs** — `;;`
        // stops, `;&` runs the next arm's body without testing it, `;;&` goes on
        // testing. Measured by running them; see `reader/probes/case.sh`.
        let end = match (self.peek(), self.peek_at(1), self.peek_at(2)) {
            (Some(b';'), Some(b';'), Some(b'&')) => {
                self.at += 3;
                ArmEnd::KeepTesting
            }
            (Some(b';'), Some(b';'), _) => {
                self.at += 2;
                ArmEnd::Stop
            }
            (Some(b';'), Some(b'&'), _) => {
                self.at += 2;
                ArmEnd::FallThrough
            }
            // ⚠ The last arm may leave it out, and bash writes `;;` in when it
            // prints — so the omission is not recorded and the two spellings are
            // one tree. Anything else here is a case that never closes, which
            // the caller names.
            _ => ArmEnd::Stop,
        };
        Ok(Arm {
            patterns,
            body,
            end,
        })
    }

    /// One pattern: a word, read where `)` and `|` end it.
    ///
    /// ⚠ **A word, not a string.** Bash prints a pattern back verbatim, so the
    /// second gate has no opinion about what is in one — the same blind spot it
    /// has about a word, and the same answer. `'*'` is a literal asterisk and
    /// `*` is a glob, which is a difference in what the arm MATCHES, and only
    /// construction can get it right.
    fn pattern(&mut self) -> Result<Word, Refusal> {
        let was = self.in_pattern;
        self.in_pattern = true;
        let word = self.word(false, WordKind::Argument);
        self.in_pattern = was;
        word
    }

    /// `NAME=value` or `NAME+=value`, with the value read as a value.
    fn assignment(&mut self) -> Result<Assignment, Refusal> {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            self.at += 1;
        }
        let name = self.text[start..self.at].to_string();
        let append = self.peek() == Some(b'+');
        if append {
            self.at += 1;
        }
        self.at += 1; // the `=`
        let value = self.word(false, WordKind::Value)?;
        Ok(Assignment {
            name,
            append,
            value,
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
        // ⚠ Always the effective descriptor, never the written one — see
        // `Redirect::fd`. `1> f` and `> f` are one redirection, and bash says so
        // by printing the first as the second. Taken from the WRITTEN operator,
        // before the normalisations below change it: `cat <&-` closes fd 0 and
        // bash prints it `cat 0>&-`, so the direction decides the descriptor
        // even where it does not survive into the operator.
        let effective_fd = fd.or(op.default_fd());
        // ⚠ `>&2` duplicates a descriptor; `>&file` sends BOTH streams to a
        // file. Same two characters, different construct, and the target is
        // what tells them apart — so the operator is settled after reading it.
        let op = match (op, &target) {
            (RedirectOp::DupOut, RedirectTarget::File(_)) => RedirectOp::BothWord,
            // ⚠ **Closing has no direction.** Bash prints `3<&-` back as
            // `3>&-`, and `<&-` as `0>&-` — measured — so a tree that kept the
            // written direction made one operation two trees. Found by the
            // second gate on one command in 129,329, which is the shape of
            // defect only a reader that is not ours can see.
            (RedirectOp::DupIn, RedirectTarget::Close) => RedirectOp::DupOut,
            _ => op,
        };
        Ok(Some(Redirect {
            fd: effective_fd,
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
        Ok(RedirectTarget::File(self.word(false, WordKind::Argument)?))
    }

    /// One word. `first` is whether it opens the command, which is the only
    /// position where a reserved word or an assignment is grammar.
    fn word(&mut self, first: bool, kind: WordKind) -> Result<Word, Refusal> {
        let start = self.at;
        let mut segments: Vec<Segment> = Vec::new();
        let mut quoted_anywhere = false;

        // ⚠ **At the head of EVERY word, not just the first.** `cd ~/Code` has
        // its tilde in the second, and scoping this to the command name once let
        // exactly that shape absorb an expansion into literal text.
        if self.peek() == Some(b'~') {
            segments.push(self.tilde()?);
        }
        // ⚠ In a value only, a tilde after an unquoted `:` expands too — bash
        // binds `T=a:~/x` to `a:/home/…/x`. Measured, and the reason a value
        // cannot share the argument reader.
        let tilde_follows_colon = kind == WordKind::Value;

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
                // ⚠ **The space decides, and there is no other test.**
                // `diff < (a) b` — one blank between them — is a syntax error to
                // bash rather than a redirection to a subshell, so `<`
                // immediately followed by `(` is a process substitution
                // wherever it appears. Measured.
                b'<' | b'>' if self.peek_at(1) == Some(b'(') => {
                    segments.push(self.process_substitution()?);
                }
                b'<' | b'>' => {
                    // ⚠ Three constructs share these characters and they are not
                    // one build. A heredoc's operand is on the following lines;
                    // a here-string's is a value on this one.
                    // A `<<` reaching this reader is glued to the end of a word
                    // (`foo<<EOF`), which bash reads as a word and a redirection
                    // and this parser does not split — so what is unmodelled here
                    // is the gluing, not the heredoc.
                    let reason = if byte == b'<'
                        && self.peek_at(1) == Some(b'<')
                        && self.peek_at(2) == Some(b'<')
                    {
                        Reason::HereString
                    } else {
                        Reason::Redirection
                    };
                    return self.refuse(reason, 1);
                }
                // Inside a substitution a `)` closes it rather than opening a
                // group, and inside a `case` pattern it ends the pattern — so
                // either way it ends the word instead of being refused. A quoted
                // or escaped one never reaches here, which is what makes
                // `'a)b')` a pattern holding a paren.
                b')' if self.parens > 0 || self.in_pattern => break,
                // ⚠ **A brace INSIDE a word is expansion, not grouping.**
                // `echo {a,b}.txt` is one word that expands to two, which is a
                // glob-level construct and a different build from `{ a; }`.
                // Naming it `Grouping` counted a word construct inside a
                // compound-statement build and hid it there.
                b'{' => segments.push(self.brace_or_literal()?),
                // ⚠ A `}` with no expansion open is an ordinary character —
                // `echo a}b` prints `a}b` — so it joins the word rather than
                // being refused.
                b'}' => {
                    self.at += 1;
                    segments.push(Segment {
                        kind: SegmentKind::Literal("}".to_string()),
                        span: Span::new(at, self.at),
                    });
                }
                b'(' | b')' => return self.refuse(Reason::Grouping, 1),
                // ⚠ A `$` that opens nothing is an ordinary character, and
                // bash agrees: `echo $`, `echo a$` and `echo $.` all parse and
                // print back unchanged.
                b'$' | b'`' => match classify_expansion(self.bytes, self.at, false) {
                    Some(Reason::Parameter) => segments.push(self.parameter(false)?),
                    Some(Reason::CommandSubstitution) => segments.push(self.substitution(false)?),
                    Some(Reason::Arithmetic) => segments.push(self.arith_expansion()?),
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
                // ⚠ A `*` is a glob only where pathname expansion happens. In
                // an assignment's value it is an ordinary character — measured,
                // `FOO=*.txt` binds those five characters — so it falls through
                // to `bare`, which reads it as literal text.
                b'*' | b'?' if kind == WordKind::Argument => {
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
                b'~' if tilde_follows_colon
                    && matches!(segments.last().map(|s| &s.kind),
                        Some(SegmentKind::Literal(text)) if text.ends_with(':')) =>
                {
                    segments.push(self.tilde()?);
                }
                _ => segments.push(self.bare(kind)?),
            }
        }

        let mut segments = merge_literals(segments);
        // ⚠ `FOO=` and `FOO=''` bind the same empty value, so they are one tree
        // — and they have to be, or the printer's one spelling of an empty value
        // would fail the round-trip law on whichever it did not choose.
        if kind == WordKind::Value
            && matches!(&segments[..], [Segment { kind: SegmentKind::Literal(text), .. }] if text.is_empty())
        {
            segments.clear();
        }
        let word = Word {
            segments,
            span: Span::new(start, self.at),
        };

        if first
            && !quoted_anywhere
            && let Some(text) = word.as_literal()
            && let Some(reason) = reserved_word(&text)
        {
            return Err(Refusal {
                reason,
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
                    Some(Reason::CommandSubstitution) => {
                        flush!();
                        segments.push(self.substitution(true)?);
                        from = self.at;
                    }
                    Some(Reason::Arithmetic) => {
                        flush!();
                        segments.push(self.arith_expansion()?);
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

    /// `$(cmd)` — a whole script, read by the reader that reads a whole script.
    fn substitution(&mut self, quoted: bool) -> Result<Segment, Refusal> {
        let start = self.at;
        self.at += 2; // `$(`
        let items = self.parenthesised_list()?;
        Ok(Segment {
            kind: SegmentKind::Substitution(Substitution { items, quoted }),
            span: Span::new(start, self.at),
        })
    }

    /// `<(cmd)` or `>(cmd)` — the same recursion, a different thing done with
    /// what it prints.
    ///
    /// ⚠ **It is a segment, so it glues.** `diff x<(a)` is one word and
    /// `x=<(a)` is a binding, both measured — which is why this is reached from
    /// the word reader rather than from the redirection reader, even though a
    /// redirection target is where the corpus mostly writes it.
    fn process_substitution(&mut self) -> Result<Segment, Refusal> {
        let start = self.at;
        let direction = match self.peek() {
            Some(b'<') => Direction::Read,
            _ => Direction::Write,
        };
        self.at += 2; // `<(` or `>(`
        let items = self.parenthesised_list()?;
        Ok(Segment {
            kind: SegmentKind::ProcessSubstitution(ProcessSubstitution { direction, items }),
            span: Span::new(start, self.at),
        })
    }

    /// The command list inside `$( … )` or `<( … )`, from just past the `(` to
    /// just past the `)`, with its own heredoc bodies already paired up.
    ///
    /// ⚠ **A heredoc in here belongs to this list and nothing else.** Both
    /// halves of that matter and both are measured in
    /// `reader/probes/substitution-heredoc.sh`:
    ///
    /// - An opener the ENCLOSING line left waiting may not be handed a body from
    ///   in here. `cat <<A "$(cat <<X⏎i⏎X⏎)"` gives the argument `X`'s body and
    ///   stdin `A`'s, so the pending list is set aside for the duration.
    /// - This list's own bodies are paired up *here*, at the closing paren,
    ///   rather than left for the walk in [`fill_script`]. Bash reads a body when
    ///   the line holding its opener ends, and a line inside a substitution ends
    ///   before the one around it — an order no walk over the finished tree
    ///   reproduces, since it is neither the order the openers were written in
    ///   nor the order the tree holds them in. Draining them at the close makes
    ///   the question local, and leaves the outer walk with exactly the bodies
    ///   the outer line opened.
    fn parenthesised_list(&mut self) -> Result<Vec<Item>, Refusal> {
        let outer = std::mem::take(&mut self.pending);
        // A whole script starts here, so neither the `case` arm nor the pattern
        // around it reaches inside: `$( )` is its own context, and `self.parens`
        // is what ends a word in there.
        let in_pattern = std::mem::replace(&mut self.in_pattern, false);
        let arm_depth = std::mem::replace(&mut self.arm_depth, 0);
        let bodies_before = self.bodies.len();
        self.parens += 1;
        let items = self.items(&[]);
        self.parens -= 1;
        let mut items = items?;
        if self.peek() != Some(b')') {
            return self.refuse(Reason::UnterminatedExpansion, 1);
        }
        // ⚠ **An opener still waiting at the `)` gets an EMPTY body**, because
        // that is what bash makes of it: `$(cat <<X)` warns `command
        // substitution: 1 unterminated here-document`, expands to nothing, and
        // `bash -n` accepts it. Reading on from here instead would take a body
        // out of the text after the substitution, which belongs to nobody.
        for pending in std::mem::take(&mut self.pending) {
            let body = self.finish_body(&pending, String::new())?;
            self.bodies.push(body);
        }
        self.at += 1;
        let mut inner = self.bodies.drain(bodies_before..);
        fill_items(&mut items, &mut inner);
        debug_assert!(
            inner.next().is_none(),
            "a heredoc body was read inside a substitution that no opener in it claimed"
        );
        drop(inner);
        self.pending = outer;
        self.in_pattern = in_pattern;
        self.arm_depth = arm_depth;
        if items.iter().any(|item| matches!(item, Item::Comment(_))) {
            return self.refuse(Reason::CommentInList, 1);
        }
        Ok(items)
    }

    /// `$name`, `${name}`, `$1`, `$@` — with the braces resolved away.
    ///
    /// Only reached when [`classify_expansion`] has already said this is a plain
    /// parameter, so every branch below finds a name.
    fn parameter(&mut self, quoted: bool) -> Result<Segment, Refusal> {
        let start = self.at;
        self.at += 1;
        let name = if self.peek() == Some(b'{') {
            return self.braced_parameter(start, quoted);
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
            kind: SegmentKind::Parameter(Parameter {
                name,
                quoted,
                subscript: None,
                op: None,
            }),
            span: Span::new(start, self.at),
        })
    }

    /// `${…}` — a name, maybe a subscript, maybe an operator on it.
    ///
    /// ⚠ **Nothing here may be read as text.** Bash prints every operator form
    /// back verbatim, so the second gate sees the same characters on both sides
    /// and cannot object to a wrong tree — an operator absorbed into a literal
    /// would satisfy both gates and be silently wrong. Every branch therefore
    /// either spells the operator or refuses by name.
    fn braced_parameter(&mut self, start: usize, quoted: bool) -> Result<Segment, Refusal> {
        self.at += 1; // the `{`
        // `${#x}` and `${!x}` put their operator in front of the name, so they
        // are read before it. `${#}` is the special parameter, not a length.
        let prefix = match (self.peek(), self.peek_at(1)) {
            (Some(b'#'), Some(byte)) if byte != b'}' => {
                self.at += 1;
                Some(ParameterOp::Length)
            }
            (Some(b'!'), Some(byte)) if byte != b'}' => {
                self.at += 1;
                Some(ParameterOp::Indirect)
            }
            _ => None,
        };
        let from = self.at;
        if prefix.is_none()
            && let Some(byte) = self.peek()
            && !byte.is_ascii_alphanumeric()
            && byte != b'_'
        {
            // A special parameter in braces: `${@}`, `${?}`.
            self.at += 1;
        }
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            self.at += 1;
        }
        let name = self.text[from..self.at].to_string();
        if name.is_empty() {
            return self.refuse(Reason::ParameterOperator, 1);
        }
        let subscript = self.subscript()?;
        let op = match prefix {
            Some(op) => {
                if self.peek() != Some(b'}') {
                    return self.refuse(Reason::ParameterOperator, 1);
                }
                Some(op)
            }
            None => self.parameter_op()?,
        };
        if self.peek() != Some(b'}') {
            return self.refuse(Reason::UnterminatedExpansion, 1);
        }
        self.at += 1;
        Ok(Segment {
            kind: SegmentKind::Parameter(Parameter {
                name,
                quoted,
                subscript,
                op,
            }),
            span: Span::new(start, self.at),
        })
    }

    /// `[0]`, `[@]`, `[$i]` — which element, where there is one.
    fn subscript(&mut self) -> Result<Option<Subscript>, Refusal> {
        if self.peek() != Some(b'[') {
            return Ok(None);
        }
        self.at += 1;
        if self.peek_at(1) == Some(b']') {
            let all = match self.peek() {
                Some(b'@') => Some(Subscript::All),
                Some(b'*') => Some(Subscript::Joined),
                _ => None,
            };
            if let Some(subscript) = all {
                self.at += 2;
                return Ok(Some(subscript));
            }
        }
        let index = self.operand(b"]")?;
        if self.peek() != Some(b']') {
            return self.refuse(Reason::UnterminatedExpansion, 1);
        }
        self.at += 1;
        // ⚠ An index that is arithmetic is refused, not stored. `+` inside
        // `${a[i+1]}` is an operator, and keeping it as literal text is the one
        // failure neither gate can see. None occur in the corpus.
        if index
            .as_literal()
            .is_some_and(|text| !text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        {
            return self.refuse(Reason::Arithmetic, 1);
        }
        Ok(Some(Subscript::Index(index)))
    }

    /// The operator after a name, if the text gives one.
    fn parameter_op(&mut self) -> Result<Option<ParameterOp>, Refusal> {
        // ⚠ The colon is a field: `${x-y}` substitutes only for an UNSET `x`,
        // `${x:-y}` also for an empty one. Bash keeps both spellings, so
        // collapsing them would be a wrong tree no gate could report.
        let colon = self.peek() == Some(b':');
        let at = if colon { self.at + 1 } else { self.at };
        let op = match self.bytes.get(at).copied() {
            Some(b'-') => Some(1),
            Some(b'=') => Some(2),
            Some(b'?') => Some(3),
            Some(b'+') => Some(4),
            _ => None,
        };
        if let Some(which) = op {
            self.at = at + 1;
            let word = self.operand(b"}")?;
            return Ok(Some(match which {
                1 => ParameterOp::Default { colon, word },
                2 => ParameterOp::Assign { colon, word },
                3 => ParameterOp::Error { colon, word },
                _ => ParameterOp::Alternate { colon, word },
            }));
        }
        // A `:` that opens none of those is a substring, whose operands are
        // arithmetic — a language of its own, and not this build.
        if colon {
            return self.refuse(Reason::ParameterOperator, 1);
        }
        match self.peek() {
            Some(byte @ (b'#' | b'%')) => {
                self.at += 1;
                let longest = self.peek() == Some(byte);
                if longest {
                    self.at += 1;
                }
                let pattern = self.operand(b"}")?;
                Ok(Some(if byte == b'#' {
                    ParameterOp::StripPrefix { longest, pattern }
                } else {
                    ParameterOp::StripSuffix { longest, pattern }
                }))
            }
            Some(b'/') => Ok(Some(ParameterOp::Replace(self.replace()?))),
            Some(byte @ (b'^' | b',')) => {
                self.at += 1;
                let every = self.peek() == Some(byte);
                if every {
                    self.at += 1;
                }
                Ok(Some(ParameterOp::Case {
                    upper: byte == b'^',
                    every,
                }))
            }
            _ => Ok(None),
        }
    }

    /// `${x/pat/rep}`, and the spellings that narrow which occurrence.
    fn replace(&mut self) -> Result<Replace, Refusal> {
        self.at += 1; // the `/`
        let every = self.peek() == Some(b'/');
        if every {
            self.at += 1;
        }
        // ⚠ `${x/#pat/rep}` anchors at the start, and the `#` is NOT part of the
        // pattern. Only meaningful straight after the slash — a `#` later on is
        // an ordinary character in the pattern.
        let anchor = match self.peek() {
            Some(b'#') => Some(Anchor::Start),
            Some(b'%') => Some(Anchor::End),
            _ => None,
        };
        if anchor.is_some() {
            self.at += 1;
        }
        let pattern = self.operand(b"/}")?;
        // `${x/pat}` deletes the match; with no `/` there is no replacement to
        // read, which is a different text from `${x/pat/}` and the same tree.
        let replacement = if self.peek() == Some(b'/') {
            self.at += 1;
            Some(self.operand(b"}")?)
        } else {
            None
        };
        Ok(Replace {
            every,
            anchor,
            pattern,
            replacement,
        })
    }

    /// A word inside `${…}`, up to one of `stop` at brace depth zero.
    ///
    /// ⚠ **It nests, and it holds spaces.** `${x:-$(date)}` and `${x:-${y}}` are
    /// both legal, so this cannot stop at the first `}`; `${x:-a b}` is ONE word
    /// to bash, so it cannot stop at a space either. Both measured.
    fn operand(&mut self, stop: &[u8]) -> Result<Word, Refusal> {
        let start = self.at;
        let mut segments: Vec<Segment> = Vec::new();
        loop {
            let Some(byte) = self.peek() else {
                return self.refuse(Reason::UnterminatedExpansion, 1);
            };
            if stop.contains(&byte) {
                break;
            }
            let at = self.at;
            match byte {
                b'$' | b'`' => match classify_expansion(self.bytes, self.at, false) {
                    Some(Reason::Parameter) => segments.push(self.parameter(false)?),
                    Some(Reason::CommandSubstitution) => segments.push(self.substitution(false)?),
                    Some(reason) => return self.refuse(reason, 1),
                    None => {
                        self.at += 1;
                        segments.push(Segment {
                            kind: SegmentKind::Literal("$".to_string()),
                            span: Span::new(at, self.at),
                        });
                    }
                },
                b'\'' => segments.push(self.single_quoted()?),
                b'"' => segments.extend(self.double_quoted()?),
                // ⚠ Alternatives nest: `{a,{b,c}}` is one expansion holding
                // another, and the inner one consumes its own `}`.
                b'{' => segments.push(self.brace_or_literal()?),
                b'\\' => match self.peek_at(1) {
                    Some(next) => {
                        self.at += 2;
                        segments.push(Segment {
                            kind: SegmentKind::Literal((next as char).to_string()),
                            span: Span::new(at, self.at),
                        });
                    }
                    None => return self.refuse(Reason::DanglingEscape, 1),
                },
                // ⚠ A glob here is the PATTERN language, which is the same one
                // pathname expansion uses — `${f%%.*}` cuts at the first dot the
                // way `*.txt` matches. So it is a `Glob`, not literal text.
                b'*' | b'?' => {
                    self.at += 1;
                    let glob = if byte == b'*' { Glob::Any } else { Glob::One };
                    segments.push(Segment {
                        kind: SegmentKind::Glob(glob),
                        span: Span::new(at, self.at),
                    });
                }
                _ => {
                    let from = self.at;
                    while let Some(next) = self.peek() {
                        if stop.contains(&next)
                            || matches!(next, b'$' | b'`' | b'\'' | b'"' | b'\\' | b'*' | b'?')
                        {
                            break;
                        }
                        self.at += 1;
                    }
                    segments.push(Segment {
                        kind: SegmentKind::Literal(self.text[from..self.at].to_string()),
                        span: Span::new(from, self.at),
                    });
                }
            }
        }
        Ok(Word {
            segments: merge_literals(segments),
            span: Span::new(start, self.at),
        })
    }

    /// `$((…))` — arithmetic whose value becomes part of a word.
    fn arith_expansion(&mut self) -> Result<Segment, Refusal> {
        let start = self.at;
        self.at += 3; // `$((`
        let value = self.arithmetic(b")")?;
        self.take_arith_close()?;
        match value {
            Some(value) => Ok(Segment {
                kind: SegmentKind::Arithmetic(value),
                span: Span::new(start, self.at),
            }),
            // `$(())` evaluates to 0, but nothing in the corpus writes it and a
            // node with no expression would be a lie about the text.
            None => self.refuse(Reason::Arithmetic, 1),
        }
    }

    /// `((…))` — arithmetic as a command.
    fn arith_command(&mut self) -> Result<Arith, Refusal> {
        self.at += 2; // `((`
        let value = self.arithmetic(b")")?;
        self.take_arith_close()?;
        value.ok_or_else(|| Refusal {
            reason: Reason::Arithmetic,
            span: Span::new(self.at, self.at + 1),
        })
    }

    /// The `))` that closes an arithmetic expansion or command.
    fn take_arith_close(&mut self) -> Result<(), Refusal> {
        self.skip_arith_blanks();
        if self.peek() != Some(b')') || self.peek_at(1) != Some(b')') {
            return self.refuse(Reason::Arithmetic, 1);
        }
        self.at += 2;
        Ok(())
    }

    /// `for ((init; condition; step)); do … done` — the C-style loop, which
    /// shares only its keyword with `for NAME in words`.
    fn for_arith(&mut self) -> Result<ForArith, Refusal> {
        self.at += 2; // `((`
        let init = self.arithmetic(b";")?;
        if self.peek() != Some(b';') {
            return self.refuse(Reason::Arithmetic, 1);
        }
        self.at += 1;
        let condition = self.arithmetic(b";")?;
        if self.peek() != Some(b';') {
            return self.refuse(Reason::Arithmetic, 1);
        }
        self.at += 1;
        let step = self.arithmetic(b")")?;
        self.take_arith_close()?;
        let body = self.loop_body()?;
        Ok(ForArith {
            init,
            condition,
            step,
            body,
        })
    }

    /// An arithmetic expression, up to but not including its terminator.
    ///
    /// ⚠ **Precedence climbing, not a flat scan.** `1+2*3` is one tree and
    /// `(1+2)*3` is another; a reader that kept the text would satisfy the
    /// round-trip law and say nothing true, and bash prints arithmetic verbatim
    /// so the second gate cannot object either. This is the construct where
    /// "nothing is left unparsed" has to be taken literally.
    ///
    /// `None` where there is no expression at all, which `for ((;;))` needs.
    fn arithmetic(&mut self, stop: &[u8]) -> Result<Option<Arith>, Refusal> {
        self.skip_arith_blanks();
        if self.peek().is_none_or(|byte| stop.contains(&byte)) {
            return Ok(None);
        }
        let first = self.arith_binary(0, stop)?;
        // A comma sequence is the lowest precedence of all, and its value is the
        // last part — `for ((i=0, j=1; …))` is where it shows up.
        let mut parts = vec![first];
        loop {
            self.skip_arith_blanks();
            if self.peek() != Some(b',') {
                break;
            }
            self.at += 1;
            parts.push(self.arith_binary(0, stop)?);
        }
        Ok(Some(if parts.len() == 1 {
            parts.pop().expect("one part")
        } else {
            Arith::Sequence(parts)
        }))
    }

    /// Binary operators at `least` precedence or tighter, then the ternary and
    /// assignment forms which bind loosest and associate to the RIGHT.
    fn arith_binary(&mut self, least: u8, stop: &[u8]) -> Result<Arith, Refusal> {
        let mut left = self.arith_unary(stop)?;
        loop {
            self.skip_arith_blanks();
            // ⚠ Assignment is right-associative and takes an lvalue, so it is
            // handled here rather than as another binary operator: `a = b = 1`
            // is `a = (b = 1)`.
            if let Some((op, width)) = self.arith_assign_op() {
                self.at += width;
                let value = self.arith_binary(0, stop)?;
                left = Arith::Assign {
                    target: Box::new(left),
                    op,
                    value: Box::new(value),
                };
                continue;
            }
            if self.peek() == Some(b'?') && least == 0 {
                self.at += 1;
                let then = self.arith_binary(0, b":")?;
                self.skip_arith_blanks();
                if self.peek() != Some(b':') {
                    return self.refuse(Reason::Arithmetic, 1);
                }
                self.at += 1;
                let otherwise = self.arith_binary(0, stop)?;
                left = Arith::Ternary {
                    condition: Box::new(left),
                    then: Box::new(then),
                    otherwise: Box::new(otherwise),
                };
                continue;
            }
            let Some((op, width, precedence)) = self.arith_binary_op(stop) else {
                break;
            };
            if precedence < least {
                break;
            }
            self.at += width;
            // `**` is the one right-associative arithmetic operator.
            let next = if op == BinaryOp::Power {
                precedence
            } else {
                precedence + 1
            };
            let right = self.arith_binary(next, stop)?;
            left = Arith::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn arith_unary(&mut self, stop: &[u8]) -> Result<Arith, Refusal> {
        self.skip_arith_blanks();
        let op = match (self.peek(), self.peek_at(1)) {
            (Some(b'+'), Some(b'+')) => Some((UnaryOp::Step(Step::Increment), 2)),
            (Some(b'-'), Some(b'-')) => Some((UnaryOp::Step(Step::Decrement), 2)),
            (Some(b'-'), _) => Some((UnaryOp::Negate, 1)),
            (Some(b'+'), _) => Some((UnaryOp::Plus, 1)),
            (Some(b'!'), _) => Some((UnaryOp::Not, 1)),
            (Some(b'~'), _) => Some((UnaryOp::BitNot, 1)),
            _ => None,
        };
        if let Some((op, width)) = op {
            self.at += width;
            let operand = self.arith_unary(stop)?;
            return Ok(Arith::Unary {
                op,
                operand: Box::new(operand),
            });
        }
        let operand = self.arith_operand(stop)?;
        // `i++` takes the value before the change, so it wraps what came before.
        self.skip_arith_blanks();
        let step = match (self.peek(), self.peek_at(1)) {
            (Some(b'+'), Some(b'+')) => Some(Step::Increment),
            (Some(b'-'), Some(b'-')) => Some(Step::Decrement),
            _ => None,
        };
        match step {
            Some(op) => {
                self.at += 2;
                Ok(Arith::Postfix {
                    op,
                    operand: Box::new(operand),
                })
            }
            None => Ok(operand),
        }
    }

    fn arith_operand(&mut self, stop: &[u8]) -> Result<Arith, Refusal> {
        self.skip_arith_blanks();
        match self.peek() {
            Some(b'(') => {
                self.at += 1;
                let inner = self.arithmetic(b")")?;
                if self.peek() != Some(b')') {
                    return self.refuse(Reason::Arithmetic, 1);
                }
                self.at += 1;
                inner.ok_or_else(|| Refusal {
                    reason: Reason::Arithmetic,
                    span: Span::new(self.at, self.at + 1),
                })
            }
            // ⚠ An expansion inside arithmetic is still an expansion: `$x` is
            // read by the same reader that reads it anywhere else, so a `$(cmd)`
            // in here recurses into a whole script exactly as it should.
            Some(b'$') | Some(b'`') => match classify_expansion(self.bytes, self.at, false) {
                Some(Reason::Parameter) => Ok(Arith::Expansion(Box::new(self.parameter(false)?))),
                Some(Reason::CommandSubstitution) => {
                    Ok(Arith::Expansion(Box::new(self.substitution(false)?)))
                }
                Some(reason) => self.refuse(reason, 1),
                None => self.refuse(Reason::Arithmetic, 1),
            },
            Some(byte) if byte.is_ascii_digit() => {
                let from = self.at;
                // `0x1f` and `007` are one token; the letters belong to the
                // number, so the run is alphanumeric rather than digits.
                while self.peek().is_some_and(|b| b.is_ascii_alphanumeric()) {
                    self.at += 1;
                }
                let text = self.text[from..self.at].to_string();
                if self.peek() != Some(b'#') {
                    return Ok(Arith::Number(text));
                }
                self.at += 1;
                let digits = match self.peek() {
                    Some(b'$') | Some(b'`') => self.arith_operand(stop)?,
                    Some(b) if b.is_ascii_alphanumeric() => {
                        let from = self.at;
                        while self.peek().is_some_and(|b| b.is_ascii_alphanumeric()) {
                            self.at += 1;
                        }
                        Arith::Number(self.text[from..self.at].to_string())
                    }
                    _ => return self.refuse(Reason::Arithmetic, 1),
                };
                Ok(Arith::Based {
                    base: text,
                    digits: Box::new(digits),
                })
            }
            Some(byte) if byte.is_ascii_alphabetic() || byte == b'_' => {
                let from = self.at;
                while self
                    .peek()
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    self.at += 1;
                }
                Ok(Arith::Variable(self.text[from..self.at].to_string()))
            }
            _ => {
                let _ = stop;
                self.refuse(Reason::Arithmetic, 1)
            }
        }
    }

    /// `=`, `+=`, `<<=` … — an assignment, and what it does first.
    fn arith_assign_op(&mut self) -> Option<(Option<BinaryOp>, usize)> {
        let two = |op: BinaryOp| Some((Some(op), 2));
        match (self.peek(), self.peek_at(1), self.peek_at(2)) {
            (Some(b'<'), Some(b'<'), Some(b'=')) => Some((Some(BinaryOp::ShiftLeft), 3)),
            (Some(b'>'), Some(b'>'), Some(b'=')) => Some((Some(BinaryOp::ShiftRight), 3)),
            (Some(b'*'), Some(b'*'), Some(b'=')) => Some((Some(BinaryOp::Power), 3)),
            (Some(b'+'), Some(b'='), _) => two(BinaryOp::Add),
            (Some(b'-'), Some(b'='), _) => two(BinaryOp::Subtract),
            (Some(b'*'), Some(b'='), _) => two(BinaryOp::Multiply),
            (Some(b'/'), Some(b'='), _) => two(BinaryOp::Divide),
            (Some(b'%'), Some(b'='), _) => two(BinaryOp::Remainder),
            (Some(b'&'), Some(b'='), _) => two(BinaryOp::BitAnd),
            (Some(b'^'), Some(b'='), _) => two(BinaryOp::BitXor),
            (Some(b'|'), Some(b'='), _) => two(BinaryOp::BitOr),
            // A lone `=`, which is not `==`.
            (Some(b'='), next, _) if next != Some(b'=') => Some((None, 1)),
            _ => None,
        }
    }

    /// The binary operator under the cursor: which, how wide, how tightly.
    fn arith_binary_op(&mut self, stop: &[u8]) -> Option<(BinaryOp, usize, u8)> {
        if self.peek().is_some_and(|byte| stop.contains(&byte)) {
            return None;
        }
        let (op, width) = match (self.peek()?, self.peek_at(1)) {
            (b'*', Some(b'*')) => (BinaryOp::Power, 2),
            (b'<', Some(b'<')) => (BinaryOp::ShiftLeft, 2),
            (b'>', Some(b'>')) => (BinaryOp::ShiftRight, 2),
            (b'<', Some(b'=')) => (BinaryOp::LessOrEqual, 2),
            (b'>', Some(b'=')) => (BinaryOp::GreaterOrEqual, 2),
            (b'=', Some(b'=')) => (BinaryOp::Equal, 2),
            (b'!', Some(b'=')) => (BinaryOp::NotEqual, 2),
            (b'&', Some(b'&')) => (BinaryOp::And, 2),
            (b'|', Some(b'|')) => (BinaryOp::Or, 2),
            (b'*', _) => (BinaryOp::Multiply, 1),
            (b'/', _) => (BinaryOp::Divide, 1),
            (b'%', _) => (BinaryOp::Remainder, 1),
            (b'+', _) => (BinaryOp::Add, 1),
            (b'-', _) => (BinaryOp::Subtract, 1),
            (b'<', _) => (BinaryOp::Less, 1),
            (b'>', _) => (BinaryOp::Greater, 1),
            (b'&', _) => (BinaryOp::BitAnd, 1),
            (b'^', _) => (BinaryOp::BitXor, 1),
            (b'|', _) => (BinaryOp::BitOr, 1),
            _ => return None,
        };
        // Bash's own table, tightest last. The numbers matter only relative to
        // each other; they are the reason `1+2*3` is not `(1+2)*3`.
        let precedence = match op {
            BinaryOp::Or => 1,
            BinaryOp::And => 2,
            BinaryOp::BitOr => 3,
            BinaryOp::BitXor => 4,
            BinaryOp::BitAnd => 5,
            BinaryOp::Equal | BinaryOp::NotEqual => 6,
            BinaryOp::Less
            | BinaryOp::LessOrEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterOrEqual => 7,
            BinaryOp::ShiftLeft | BinaryOp::ShiftRight => 8,
            BinaryOp::Add | BinaryOp::Subtract => 9,
            BinaryOp::Multiply | BinaryOp::Divide | BinaryOp::Remainder => 10,
            BinaryOp::Power => 11,
        };
        Some((op, width, precedence))
    }

    /// Blanks and newlines, both of which arithmetic ignores entirely.
    fn skip_arith_blanks(&mut self) {
        while self
            .peek()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.at += 1;
        }
    }

    /// `{a,b}` or `{1..9}` — or the literal characters, where neither is what
    /// this is.
    ///
    /// ⚠ **The fallback is not absorption.** `{a}` holds nothing to expand and
    /// bash prints it back as itself, so a literal `{` is the FAITHFUL reading
    /// rather than a construct being swallowed — which is why the decision is
    /// made by [`brace_expansion`] before any character is consumed, and shared
    /// with the survey so both agree about which braces are which.
    fn brace_or_literal(&mut self) -> Result<Segment, Refusal> {
        let start = self.at;
        let Some(shape) = brace_expansion(self.bytes, self.at) else {
            self.at += 1;
            return Ok(Segment {
                kind: SegmentKind::Literal("{".to_string()),
                span: Span::new(start, self.at),
            });
        };
        if let BraceShape::Range {
            end,
            from,
            to,
            step,
        } = shape
        {
            self.at = end;
            return Ok(Segment {
                kind: SegmentKind::Brace(Brace::Range { from, to, step }),
                span: Span::new(start, self.at),
            });
        }
        self.at += 1; // the `{`
        let mut alternatives = Vec::new();
        loop {
            alternatives.push(self.operand(b",}")?);
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    break;
                }
                // `brace_expansion` found the closing brace, so the only way to
                // arrive here is a bug in one of them agreeing with the other.
                _ => return self.refuse(Reason::BraceExpansion, 1),
            }
        }
        Ok(Segment {
            kind: SegmentKind::Brace(Brace::Alternatives(alternatives)),
            span: Span::new(start, self.at),
        })
    }

    /// An unquoted run, up to the next character that means something.
    fn bare(&mut self, kind: WordKind) -> Result<Segment, Refusal> {
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
                b'*' | b'?' if kind == WordKind::Argument => break,
                // ⚠ End the literal so the word reader sees this `~`: in a value
                // a tilde right after a `:` expands, and it is the only place
                // this reader has to hand a character back.
                b'~' if kind == WordKind::Value && text.ends_with(':') => break,
                b' ' | b'\t' | b'\r' | b'\n' | b';' | b'\'' | b'"' | b'|' | b'&' | b'<' | b'>'
                | b'(' | b')' | b'{' | b'}' | b'$' | b'`' => break,
                b'[' if self.closes_bracket() => break,
                _ => {
                    let from = self.at;
                    while let Some(next) = self.peek() {
                        // In a value `*` and `?` are ordinary characters, so the
                        // run may swallow them.
                        let globs_here = kind == WordKind::Argument || !matches!(next, b'*' | b'?');
                        if (is_bare_stop(next) && globs_here)
                            || (next == b'[' && self.closes_bracket())
                        {
                            break;
                        }
                        self.at += 1;
                        // ⚠ In a value the run ends after every `:`, because a
                        // tilde there expands — `PATH=a:~/bin` — and `~` is not
                        // otherwise a character this reader stops at. The pieces
                        // merge back together where it is not one.
                        if kind == WordKind::Value && next == b':' {
                            break;
                        }
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

/// One compound, wrapped in the list layers between it and an [`Item`].
///
/// Used only for the conditional an `elif` desugars into: bash puts a whole
/// nested `if` in the `else` arm, and an arm is a command list, so the layers
/// have to be there. They carry no grammar of their own — no connector, no
/// pipe, no `&` — which is what makes this a spelling of the same tree the text
/// `else if …; fi` produces rather than a different one.
fn one_command(kind: CommandKind, span: Span) -> Item {
    Item::List(AndOr {
        first: Pipeline {
            time: None,
            negated: false,
            commands: vec![Command {
                kind,
                redirects: Vec::new(),
                span,
            }],
            span,
        },
        rest: Vec::new(),
        background: false,
        span,
    })
}

/// Does a brace EXPANSION start at `at`, and where does it end?
///
/// `None` where the braces hold nothing to expand: `{a}` and `{}` are ordinary
/// text to bash, so they are ordinary text here — measured, not assumed.
///
/// ⚠ **Shared with the survey deliberately**, exactly as [`classify_expansion`]
/// is. Whether a given `{` opens an expansion has one answer; two
/// implementations of it would drift, and the drift would look like a parser
/// bug rather than a disagreement.
pub fn brace_expansion(bytes: &[u8], at: usize) -> Option<BraceShape> {
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut comma = false;
    let mut scan = at;
    while let Some(&byte) = bytes.get(scan) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let interior = &bytes[at + 1..scan];
                    return if comma {
                        Some(BraceShape::Alternatives { end: scan + 1 })
                    } else {
                        range_parts(interior).map(|(from, to, step)| BraceShape::Range {
                            end: scan + 1,
                            from,
                            to,
                            step,
                        })
                    };
                }
            }
            b',' if depth == 1 => comma = true,
            // A quote suspends everything: `{a,'b}'}` closes where the quote
            // says, not where the first `}` is.
            b'\'' | b'"' => {
                let quote = byte;
                scan += 1;
                while bytes.get(scan).is_some_and(|b| *b != quote) {
                    scan += 1;
                }
                // A quote that never closes: there is no expansion here to find.
                bytes.get(scan)?;
            }
            b'\\' => scan += 1,
            _ => {}
        }
        scan += 1;
    }
    None
}

/// What a `{ … }` in a word turns out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BraceShape {
    Alternatives {
        end: usize,
    },
    Range {
        end: usize,
        from: String,
        to: String,
        step: Option<String>,
    },
}

impl BraceShape {
    pub fn end(&self) -> usize {
        match self {
            BraceShape::Alternatives { end } | BraceShape::Range { end, .. } => *end,
        }
    }
}

/// `1..9`, `a..e`, `1..9..2` — the pieces, where the interior spells a sequence.
fn range_parts(interior: &[u8]) -> Option<(String, String, Option<String>)> {
    let text = std::str::from_utf8(interior).ok()?;
    let mut parts = text.split("..");
    let from = parts.next()?.to_string();
    let to = parts.next()?.to_string();
    let step = parts.next().map(str::to_string);
    if parts.next().is_some() || from.is_empty() || to.is_empty() {
        return None;
    }
    // A range runs over digits or over single letters; anything else holding two
    // dots is text bash leaves alone (`{1.5..3}`).
    let number = |part: &str| {
        let digits = part.strip_prefix('-').unwrap_or(part);
        !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
    };
    let letter =
        |part: &str| part.len() == 1 && part.starts_with(|c: char| c.is_ascii_alphabetic());
    if !(number(&from) && number(&to)) && !(letter(&from) && letter(&to)) {
        return None;
    }
    // ⚠ **The step is always a NUMBER, even over letters.** `{a..e..2}` gives
    // `a c e`, and `{x..y..z}` does not expand at all — measured. A check that
    // let a letter through there would have built a Range node for text bash
    // reads literally: a wrong tree that prints and re-reads as itself, and
    // that bash's verbatim printing puts beyond the second gate's reach.
    if step.as_deref().is_some_and(|s| !number(s)) {
        return None;
    }
    Some((from, to, step))
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
        return Some(Reason::Backtick);
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
        Some(b'{') => Some(braced_parameter(bytes, at + 2)),
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
fn braced_parameter(bytes: &[u8], from: usize) -> Reason {
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
    // `${#x}` measures and `${!x}` dereferences — both are operators on a name,
    // and both are modelled.
    let from = match bytes.get(from) {
        Some(b'#' | b'!') => from + 1,
        _ => from,
    };
    let mut at = from;
    while bytes
        .get(at)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        at += 1;
    }
    // `${}` names nothing, so it is not the same node `$name` is.
    if at == from {
        return Reason::ParameterOperator;
    }
    // A subscript is part of naming the value, and what follows it decides the
    // rest exactly as it would without one.
    if bytes.get(at) == Some(&b'[') {
        match bytes[at..].iter().position(|byte| *byte == b']') {
            Some(offset) => at += offset + 1,
            None => return Reason::ParameterOperator,
        }
    }
    match bytes.get(at) {
        Some(b'}') => Reason::Parameter,
        // ⚠ `:` opens four operators and one that is NOT modelled: `${x:1:3}`
        // takes arithmetic on both sides, so it is refused where `${x:-y}` is
        // read. Deciding that needs the character after the colon.
        Some(b':') => match bytes.get(at + 1) {
            Some(b'-' | b'=' | b'?' | b'+') => Reason::Parameter,
            _ => Reason::ParameterOperator,
        },
        Some(b'-' | b'=' | b'?' | b'+' | b'#' | b'%' | b'/' | b'^' | b',') => Reason::Parameter,
        // ⚠ The text ran out before the brace closed, and that is a claim about
        // the INPUT rather than about what is modelled: bash refuses `${x` too.
        // Classifying it as unmodelled would hide it from `bash -n`, which is
        // the check that keeps "we cannot read it" apart from "it is not shell".
        None => Reason::Parameter,
        _ => Reason::ParameterOperator,
    }
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
    fill_items(&mut script.items, bodies);
}

fn fill_and_or(list: &mut AndOr, bodies: &mut impl Iterator<Item = Heredoc>) {
    fill_pipeline(&mut list.first, bodies);
    for link in &mut list.rest {
        fill_pipeline(&mut link.pipeline, bodies);
    }
}

fn fill_pipeline(pipeline: &mut Pipeline, bodies: &mut impl Iterator<Item = Heredoc>) {
    for command in &mut pipeline.commands {
        // ⚠ A compound's interior comes BEFORE its redirections in the text —
        // `while a; do b; done <<EOF` opens its heredoc after the body — and the
        // pairing is positional, so the walk has to visit them in that order.
        match &mut command.kind {
            CommandKind::Simple(_) => {}
            CommandKind::For(loop_) => {
                for word in &mut loop_.words {
                    let _ = word;
                }
                fill_items(&mut loop_.body, bodies);
            }
            CommandKind::While(loop_) => {
                fill_items(&mut loop_.condition, bodies);
                fill_items(&mut loop_.body, bodies);
            }
            CommandKind::If(conditional) => {
                fill_items(&mut conditional.condition, bodies);
                fill_items(&mut conditional.then, bodies);
                if let Some(otherwise) = &mut conditional.otherwise {
                    fill_items(otherwise, bodies);
                }
            }
            // The subject and the patterns are words, which carry no opener the
            // outer walk can reach — one inside a `$( )` there was paired at its
            // own closing paren.
            CommandKind::Case(case) => {
                for arm in &mut case.arms {
                    fill_items(&mut arm.body, bodies);
                }
            }
            CommandKind::Subshell(items) | CommandKind::Group(items) => {
                fill_items(items, bodies);
            }
            CommandKind::Function(function) => fill_items(&mut function.body, bodies),
            CommandKind::ForArith(loop_) => fill_items(&mut loop_.body, bodies),
            // An arithmetic command carries no list, so no heredoc can open in
            // one.
            CommandKind::Arithmetic(_) => {}
        }
        for redirect in &mut command.redirects {
            if let RedirectTarget::Here(here) = &mut redirect.target
                && let Some(body) = bodies.next()
            {
                *here = body;
            }
        }
    }
}

fn fill_items(items: &mut [Item], bodies: &mut impl Iterator<Item = Heredoc>) {
    for item in items {
        if let Item::List(list) = item {
            fill_and_or(list, bodies);
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
    reserved_word(text).is_some()
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
