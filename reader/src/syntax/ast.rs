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
    Pipeline(Pipeline),
    Comment(Comment),
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
    pub span: Span,
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
