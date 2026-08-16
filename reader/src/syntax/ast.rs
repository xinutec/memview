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
    pub words: Vec<Word>,
    /// ⚠ **In their own list, because their position among the words means
    /// nothing.** `> out cat f` and `cat f > out` are the same command, and
    /// `declare -f` proves it: bash prints the first back as the second. Order
    /// *within* this list does matter — `cat > out 2>&1` and `cat 2>&1 > out`
    /// send stderr to different places, and bash preserves both as written.
    pub redirects: Vec<Redirect>,
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
}

impl RedirectOp {
    /// The descriptor this operator acts on when the text does not say.
    ///
    /// `None` for the `&>` forms, which act on two and admit no number.
    pub fn default_fd(self) -> Option<u32> {
        match self {
            RedirectOp::Read | RedirectOp::ReadWrite | RedirectOp::DupIn => Some(0),
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
                SegmentKind::Glob(_) => return None,
            }
        }
        Some(out)
    }
}
