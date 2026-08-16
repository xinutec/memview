//! The tree, and what equality means on it.
//!
//! Every node carries a [`Span`] and no node compares it. That is the round-trip
//! law's first requirement: `A₁` and `A₂` come from different texts and so carry
//! different spans, and a law that could not see past them would be unsatisfiable
//! rather than strict. See `docs/execution-model.md`.

use std::fmt;

/// A byte range in the text a node was read from.
///
/// ⚠ **Two spans always compare equal, whatever they hold.** Equality on the tree
/// has to ignore position, and doing it here rather than in each node's
/// `PartialEq` means a node type added later cannot forget: `#[derive(PartialEq)]`
/// on anything containing a `Span` is automatically position-blind.
///
/// The cost is that `Span` alone is a useless thing to compare, which is why
/// nothing does. Ordering and hashing are deliberately absent for the same
/// reason — a `BTreeMap<Span, _>` would be a bug this type cannot express.
#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// The text this span was cut from, for diagnostics.
    pub fn of<'t>(&self, text: &'t str) -> &'t str {
        text.get(self.start..self.end).unwrap_or("")
    }
}

impl PartialEq for Span {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Span {}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// A whole script: what one `Bash` tool call carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub items: Vec<Item>,
    pub span: Span,
}

/// ⚠ **A comment is an item, not trivia.** It is retained byte-exact so the
/// printer can put it back, and so a later pass can read what it says — a comment
/// naming a file or a machine is evidence about the command beside it.
///
/// The second gate cannot check comments: bash deletes them. They are covered by
/// the round-trip law alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    List(AndOr),
    Comment(Comment),
}

/// `pipeline [(&& | ||) pipeline …] [&]` — bash calls it an and-or list, and it
/// is the unit `;`, a newline and `&` separate.
///
/// ⚠ **`&` belongs to the LIST, not to its last pipeline.** `a && b &`
/// backgrounds the whole list, which `declare -f` prints back as `a && b &`.
/// Hanging the flag on `b` would say something different and wrong.
///
/// The connectors are a flat sequence because bash's are left-associative and
/// equal in precedence: `a && b || c` is `((a && b) || c)`, which a list in
/// order already says. A tree of binary nodes would add a shape the text does
/// not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndOr {
    pub first: Pipeline,
    pub rest: Vec<Link>,
    pub background: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub connector: Connector,
    pub pipeline: Pipeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connector {
    /// `&&`
    And,
    /// `||`
    Or,
}

impl AndOr {
    pub fn is_empty(&self) -> bool {
        self.first.is_empty() && self.rest.is_empty() && !self.background
    }
}

/// `[time [-p]] [!] cmd [| cmd …]`.
///
/// ⚠ **`time` and `!` are fields here, not `argv[0]`.** They are grammar, and
/// scope is what forces it: `time a | b` times the whole pipeline while a
/// wrapper command like `nohup a | b` applies to `a` alone. A reader that puts
/// `time` at `argv[0]` cannot express the difference, which is the misparse the
/// flat reader still carries.
///
/// Both are recognised **only at the head**, which is bash's rule and is
/// observable: `a | ! b` is a syntax error, while `a | time b` runs the program
/// `/usr/bin/time`. So a `time` after a `|` is an ordinary word and this struct
/// says nothing about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    /// Written before or after `!`; bash accepts either and prints this first,
    /// so the tree holds two flags rather than an order.
    pub time: Option<Timed>,
    /// ⚠ A toggle, not a count: bash prints `! ! a` back as `a`.
    pub negated: bool,
    pub commands: Vec<Command>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timed {
    /// `time`
    Plain,
    /// `time -p`, the POSIX output format.
    Posix,
}

impl Pipeline {
    /// Is this pipeline nothing at all — no commands and no grammar?
    ///
    /// `time` on its own is a legal pipeline that bash prints back, so an empty
    /// `commands` is not by itself an empty pipeline.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty() && self.time.is_none() && !self.negated
    }
}

/// The text after `#`, without the `#` and without the newline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub text: String,
    pub span: Span,
}

/// One simple command: a run of words, run in sequence with its neighbours.
///
/// No terminator field. `;` and a newline both mean *sequential* and the tree
/// records the meaning, not the spelling — so `a; b` and `a\nb` are one tree, and
/// the printer picks one spelling. Every separator that means something else
/// (`&&`, `||`, `|`, `&`) is refused rather than flattened to this one, because
/// flattening is the misparse the law cannot see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub kind: CommandKind,
    /// ⚠ **In their own list, because their position among the words means
    /// nothing.** `> out cat f` and `cat f > out` are the same command, and
    /// `declare -f` proves it: bash prints the first back as the second. Order
    /// *within* this list does matter — `cat > out 2>&1` and `cat 2>&1 > out`
    /// send stderr to different places, and bash preserves both as written.
    ///
    /// They sit on the command rather than inside the kind because a compound
    /// takes them the same way a simple command does: `while a; do b; done >
    /// out` redirects the whole loop, and bash prints the redirection after the
    /// `done`.
    pub redirects: Vec<Redirect>,
    pub span: Span,
}

/// A simple command, or one of the compounds that carry a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandKind {
    Simple(Simple),
    /// `for NAME in words; do … done`, and `select` which has the same shape.
    For(ForLoop),
    /// `while cond; do … done`, and `until` which differs only in the sense.
    While(WhileLoop),
}

/// `FOO=bar cmd arg` — a name, its arguments, and the bindings in front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Simple {
    /// `FOO=bar` and friends, in the order written, before the command name.
    ///
    /// ⚠ **A prefix, so only before the first word.** `A=1 cmd B=2` binds `A`
    /// and passes `B=2` as an argument, and bash prints exactly that back.
    /// A command with assignments and no words is a plain binding: `FOO=bar`.
    pub assignments: Vec<Assignment>,
    pub words: Vec<Word>,
}

/// `for NAME in words; do body done`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForLoop {
    pub name: String,
    /// ⚠ **Always explicit, because bash makes it so.** `for f; do …` is printed
    /// back by `declare -f` as `for f in "$@"; do …`, so the tree holds that
    /// same quoted `$@` rather than an absent list. Recording the omission would
    /// make one command two trees and the second gate would say so.
    pub words: Vec<Word>,
    /// `select` rather than `for`: the same shape, a different statement.
    pub select: bool,
    pub body: Vec<Item>,
}

/// `while cond; do body done`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhileLoop {
    /// `until`, which runs the body while the condition FAILS.
    pub until: bool,
    /// A list, not one command: `while read -r a && test x; do` is legal.
    pub condition: Vec<Item>,
    pub body: Vec<Item>,
}

impl Command {
    /// Nothing at all — no binding, no word, no redirection.
    pub fn is_empty(&self) -> bool {
        match &self.kind {
            CommandKind::Simple(simple) => {
                simple.assignments.is_empty()
                    && simple.words.is_empty()
                    && self.redirects.is_empty()
            }
            // A loop is never nothing: it was written.
            _ => false,
        }
    }
}

/// `NAME=value` or `NAME+=value` bound for the length of one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// The name alone, without the `=`.
    pub name: String,
    /// `+=`, which appends rather than replaces.
    pub append: bool,
    /// ⚠ **A value is not an ordinary word, and the difference is semantic.**
    /// Measured: `FOO=*.txt` assigns the four characters `*.txt` — a scalar
    /// assignment does no pathname expansion and no word splitting — while the
    /// same text as an argument names files. So this word is read with globbing
    /// off, and a `*` in it is literal text rather than a [`Glob`].
    ///
    /// Tilde expansion does happen, and in one more place than elsewhere:
    /// `T=a:~/x` expands the tilde after the colon, which no argument would.
    pub value: Word,
    pub span: Span,
}

/// `[n]op target` — `2> err`, `>> log`, `2>&1`, `&> both`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    /// The descriptor being redirected — **always the effective one**, whether
    /// the text spelled it out or not.
    ///
    /// ⚠ **Found by the second gate, on one command in 81,623.** Bash prints
    /// `1>/dev/null` back as `>/dev/null` and `>&2` as `1>&2`: it drops an
    /// explicit default on one operator and supplies it on another. Recording
    /// what was *written* therefore made `1> f` and `> f` two trees for one
    /// thing. Recording what it *means* makes them one, and the printer decides
    /// the spelling.
    ///
    /// `None` only where the operator takes no descriptor at all: `&>` and
    /// `&>>` name both streams and bash rejects a number in front of them.
    pub fd: Option<u32>,
    pub op: RedirectOp,
    pub target: RedirectTarget,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectOp {
    /// `<`
    Read,
    /// `>`
    Write,
    /// `>>`
    Append,
    /// `<>`
    ReadWrite,
    /// `>|`, which writes even under `noclobber`.
    Clobber,
    /// `>&` or `n>&m` — duplicate an output descriptor.
    DupOut,
    /// `<&` or `n<&m` — duplicate an input descriptor.
    DupIn,
    /// `&>` — stdout and stderr to one file.
    Both,
    /// `&>>` — the same, appending.
    BothAppend,
    /// `>&word` where the word is not a descriptor. Kept apart from `&>`
    /// although they mean the same thing, because bash prints them differently
    /// (`&> f` versus `>&f`) and so holds them apart itself.
    BothWord,
    /// `<<` — a body on the following lines.
    Here,
    /// `<<-`, which strips leading tabs from the body and the terminator.
    ///
    /// ⚠ **Kept as an operator although the body is already stripped.** Bash
    /// strips at parse time and prints the `-` back with an unindented body, so
    /// dropping the flag would print a text bash reads the same way but writes
    /// differently. The stripping itself is not re-derivable from the body.
    HereDash,
}

impl RedirectOp {
    /// The descriptor this operator acts on when the text does not say.
    ///
    /// `None` for the `&>` forms, which act on two and admit no number.
    pub fn default_fd(self) -> Option<u32> {
        match self {
            RedirectOp::Read
            | RedirectOp::ReadWrite
            | RedirectOp::DupIn
            | RedirectOp::Here
            | RedirectOp::HereDash => Some(0),
            RedirectOp::Write
            | RedirectOp::Append
            | RedirectOp::Clobber
            | RedirectOp::DupOut
            | RedirectOp::BothWord => Some(1),
            RedirectOp::Both | RedirectOp::BothAppend => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectTarget {
    File(Word),
    Fd(u32),
    /// `>&-`, closing the descriptor.
    Close,
    /// The body of a heredoc, which is the one operand that is not on the line
    /// its operator was written on.
    Here(Heredoc),
}

/// `<<DELIM` and the lines up to the one holding `DELIM` alone.
///
/// ⚠ **The body is not a [`Word`].** A word is a value the shell splits, globs
/// and quotes; a heredoc body is a run of lines handed to a descriptor whole. It
/// is held as a `String` because nothing in it is a segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heredoc {
    /// The delimiter with its quoting removed: `<<'EOF'`, `<<"EOF"`, `<<\EOF`
    /// and `<<E"O"F` all give `EOF`.
    pub delimiter: String,
    /// ⚠ **Was the delimiter quoted at all?** Bash prints every quoted spelling
    /// back as `<<'EOF'`, so it keeps this one bit and forgets the rest — and it
    /// is a bit about the *body*, not about the delimiter: quoting suppresses
    /// expansion inside the body, so `<<'PY'` and `<<PY` are different nodes
    /// rather than two ways of writing one.
    pub quoted: bool,
    /// The body, ending in a newline unless it is empty.
    ///
    /// ⚠ **Not verbatim when `quoted` is false.** Bash joins a backslash-newline
    /// inside an unquoted body at parse time — `a\⏎b` is stored as `ab` — and
    /// leaves it alone inside a quoted one. So the same lines mean two different
    /// strings depending on the delimiter, which is the second reason `quoted`
    /// cannot be recovered from the spelling later.
    ///
    /// A `<<-` body is stored with its leading tabs already gone, which is what
    /// bash prints back.
    pub body: String,
}

/// One word: a sequence of typed segments, with quoting derived at print time.
///
/// `'a'`, `"a"` and `a` are the same word — one `Literal`. `a*b` and `'a*b'` are
/// not: the first has a `Glob` segment where the second has literal text, and
/// they name different files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub segments: Vec<Segment>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub kind: SegmentKind,
    pub span: Span,
}

/// ⚠ **This enum is the refusal boundary.** A construct with no variant here is
/// a parse error, never a `Literal` holding its source text. Absorbing it would
/// satisfy the round-trip law and be wrong, and no gate downstream can see it —
/// see `docs/execution-model.md`, "The law cannot see a systematic misparse".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentKind {
    /// Exact characters, expanded no further by anything.
    Literal(String),
    /// A pathname-expansion operator. Only reachable from unquoted text.
    Glob(Glob),
    /// A tilde prefix. Only reachable from unquoted text at the head of a word,
    /// which is the only place the shell expands one: `~/x` is a home directory
    /// and `"~/x"` is a filename beginning with a tilde.
    Tilde(Tilde),
    /// `$name`, `${name}`, `$1`, `$@` — a parameter's value.
    Parameter(Parameter),
    /// `$(cmd)` — the output of a whole script.
    Substitution(Substitution),
}

/// `$(cmd)`, whose value is what the commands inside it print.
///
/// ⚠ **The interior is a script, and the second gate checks it.** Bash
/// normalises what is inside — `$(a|b)` comes back as `$(a | b)` and
/// `$(ls |& cat)` as `$(ls 2>&1 | cat)` — so unlike a word, this is a real parse
/// on both sides of the comparison and a misparse in here would be caught.
///
/// Backticks are a different node and are not modelled: bash prints their
/// interior verbatim, and their escaping rules differ. Measured at 18 commands
/// against 6,397 for this form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substitution {
    pub items: Vec<Item>,
    /// ⚠ Semantic, exactly as it is on a [`Parameter`]: an unquoted
    /// substitution is split into words and globbed, a quoted one is one word.
    pub quoted: bool,
}

/// A parameter, named and nothing more.
///
/// ⚠ **The braces are not recorded.** `$x` and `${x}` name the same value, so
/// they are one node and the printer puts braces back only where the following
/// character would otherwise extend the name — `${x}y`. Bash keeps the two
/// spellings in its own output, but both sides of the second gate see the same
/// text there, so it has no opinion; this is the same collapse `'a'`, `"a"` and
/// `a` get.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    /// `x`, `1`, `@`, `?` — without the `$` and without any braces.
    ///
    /// ⚠ An unbraced positional takes exactly ONE digit: `$10` is `${1}` then a
    /// `0`, and only `${10}` names the tenth. Measured, since bash's printer
    /// spells both the same.
    pub name: String,
    /// ⚠ **Semantic, unlike a literal's quoting.** An unquoted expansion is
    /// split into words and then globbed; a quoted one is a single word whatever
    /// it holds. `echo $x` and `echo "$x"` are different programs, so this is a
    /// field on the tree rather than a decision the printer gets to make — the
    /// same reason `Glob` is not a `Literal` holding an asterisk.
    pub quoted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tilde {
    /// `~` — the current user's home.
    Home,
    /// `~name`.
    User(String),
    /// `~+`, which is `$PWD`.
    Pwd,
    /// `~-`, which is `$OLDPWD`.
    OldPwd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glob {
    /// `*`
    Any,
    /// `?`
    One,
}

impl Word {
    /// The word's literal text, when it has no expansion in it at all.
    ///
    /// `None` for a word carrying a `Glob`, because such a word names a set and
    /// asking for "the" text of it is a category error.
    pub fn as_literal(&self) -> Option<String> {
        let mut out = String::new();
        for segment in &self.segments {
            match &segment.kind {
                SegmentKind::Literal(text) => out.push_str(text),
                // A glob names a set, a tilde names a directory nobody has told
                // us, and a parameter names a value nobody has told us either;
                // asking for "the" text of any of them is a category error.
                SegmentKind::Glob(_)
                | SegmentKind::Tilde(_)
                | SegmentKind::Parameter(_)
                | SegmentKind::Substitution(_) => {
                    return None;
                }
            }
        }
        Some(out)
    }
}
