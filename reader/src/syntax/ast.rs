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
    /// `if cond; then … [else …] fi`.
    If(Conditional),
    /// `case word in pattern) … ;; esac`.
    Case(Case),
    /// `( list )` — a command list in a subshell, so what it changes it keeps.
    Subshell(Vec<Item>),
    /// `{ list; }` — a command list in THIS shell, grouped for a redirection or
    /// a connector. Kept apart from a subshell because the difference is the
    /// whole point of writing one: `( cd /x )` leaves the cwd alone and
    /// `{ cd /x; }` does not.
    Group(Vec<Item>),
    /// `name() { … }`.
    Function(Function),
    /// `((i++))` — arithmetic as a command, whose exit status is whether the
    /// value was non-zero.
    Arithmetic(Arith),
    /// `for ((i=0; i<3; i++)); do … done`.
    ForArith(ForArith),
}

/// `case word in pattern) body ;; esac` — one arm at most, chosen by a pattern.
///
/// ⚠ **The first construct whose interior is not a command list.** An arm is a
/// list of *patterns*, and a pattern is a word read for matching rather than for
/// naming — so it is a [`Word`], with the same quoting collapse, and `'*'` is a
/// literal asterisk where `*` is a [`Glob`]. Bash prints a pattern back verbatim
/// (measured in `reader/probes/case.sh`), so the second gate has no opinion at
/// all about what is inside one, exactly as it has none about a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    /// The subject, which is an ordinary word — `case "$x" in`, `case $(f) in`.
    pub word: Word,
    /// ⚠ **May be empty.** `case $x in esac` is legal and matches nothing.
    pub arms: Vec<Arm>,
}

/// One arm: the patterns that select it, what it runs, and how it ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arm {
    /// ⚠ **At least one, and a leading `(` is NOT recorded.** Bash prints `(a)`
    /// back as `a)`, so keeping the paren would be a distinction bash collapsed
    /// and the second gate could never object to.
    pub patterns: Vec<Word>,
    /// ⚠ **May be empty**, which the corpus writes for "match this and do
    /// nothing". Bash renders it as a blank line and reads it back the same.
    pub body: Vec<Item>,
    pub end: ArmEnd,
}

/// What the shell does after an arm's body — three different programs.
///
/// ⚠ **Recorded because they are not the same command.** Measured by running
/// them: on the subject `ab` against arms `a*` then `*b`, `;;` prints one thing
/// and the other two print both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmEnd {
    /// `;;` — the case is done.
    ///
    /// ⚠ Also what a missing terminator on the last arm becomes: bash supplies
    /// it, so `case $x in a) esac` and `case $x in a) ;; esac` are one tree.
    Stop,
    /// `;&` — run the next arm's body without testing its pattern.
    FallThrough,
    /// `;;&` — go on testing, starting at the next arm's pattern.
    KeepTesting,
}

/// `name() { body }` — a definition, which runs none of its body.
///
/// ⚠ **The spelling is not recorded, because bash does not keep it.**
/// `declare -f` prints `f() { a; }` back as `function f () { a; }`, and a
/// `( … )` body comes back wrapped in a brace group — so `f() ( a )` and
/// `f() { ( a ); }` are one tree, which is bash's own canonical form. Recording
/// which was written would make one command two trees, exactly as it would for
/// an `elif`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub body: Vec<Item>,
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

/// `if cond; then body [else body] fi`.
///
/// ⚠ **There is no `elif` here, because bash does not keep one.**
/// `if a; then b; elif c; then d; fi` comes back from `declare -f` as
/// `if a; then b; else if c; then d; fi; fi` — an `elif` is sugar for an `else`
/// holding one nested conditional, and bash unfolds it at parse time. A tree
/// with a list of arms would make those two texts two trees, and the second gate
/// would say so. Same desugaring the `words` of a bare [`ForLoop`] get, and for
/// the same reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conditional {
    /// A list, whose LAST command's status decides the branch: `if a; b; then`
    /// runs both and tests `b`.
    pub condition: Vec<Item>,
    /// Never empty — `if a; then fi` is a syntax error, and bash refuses it.
    pub then: Vec<Item>,
    /// The `else` arm, absent where none was written. Never an empty list, for
    /// the same reason `then` is not.
    pub otherwise: Option<Vec<Item>>,
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
    /// `<(cmd)`, `>(cmd)` — a path the shell invents, wired to a whole script.
    ProcessSubstitution(ProcessSubstitution),
    /// `{a,b}`, `{1..9}` — one word that becomes several.
    Brace(Brace),
    /// `$((1+2))` — a number, not a string.
    Arithmetic(Arith),
}

/// Shell arithmetic: a real expression, not a span of text.
///
/// ⚠ **Held as a tree because the alternative is absorption.** Keeping the
/// source between the parens would satisfy the round-trip law — it prints back
/// and re-reads identically — and bash prints arithmetic VERBATIM, whitespace
/// included, so the second gate has no opinion either. An unparsed string here
/// would be exactly the failure `docs/execution-model.md` calls the one no gate
/// can see.
///
/// Spacing is not recorded: `$((1+2))` and `$(( 1 + 2 ))` are one tree, and the
/// printer picks a spelling. `t₂ ≠ t₁` is permitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arith {
    /// `42`, `0x1f`, `007` — as written, because the base is part of what the
    /// text says and `010` is eight.
    Number(String),
    /// `i` — a name, which arithmetic reads without a `$`.
    Variable(String),
    /// `16#ff`, `10#$m` — digits in an explicit base.
    ///
    /// ⚠ **The base is a wrapper, not part of the number.** `10#08` is eight
    /// where `08` alone is an invalid octal, which is exactly why the corpus
    /// writes `$((10#$m % 10))` for a zero-padded minute. The digits may
    /// themselves be an expansion, because the base prefix is applied after the
    /// expansion happens.
    Based {
        base: String,
        digits: Box<Arith>,
    },
    /// `$x`, `${x}`, `$(cmd)` — an expansion whose VALUE is then arithmetic.
    /// Kept as the segment it is rather than flattened to a name: `$x` and `x`
    /// are different texts and, for an unset-versus-empty variable, different
    /// programs.
    Expansion(Box<Segment>),
    Unary {
        op: UnaryOp,
        operand: Box<Arith>,
    },
    /// `i++`, `i--` — where the value is taken before the change.
    Postfix {
        op: Step,
        operand: Box<Arith>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Arith>,
        right: Box<Arith>,
    },
    Ternary {
        condition: Box<Arith>,
        then: Box<Arith>,
        otherwise: Box<Arith>,
    },
    /// `i = 1`, `i += 2` — assignment, which is an expression in arithmetic.
    Assign {
        target: Box<Arith>,
        op: Option<BinaryOp>,
        value: Box<Arith>,
    },
    /// `a, b` — every part evaluated, the last one's value taken.
    Sequence(Vec<Arith>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `-x`
    Negate,
    /// `+x`
    Plus,
    /// `!x`
    Not,
    /// `~x`
    BitNot,
    /// `++x` / `--x`, where the change happens first.
    Step(Step),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Increment,
    Decrement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Power,
    Multiply,
    Divide,
    Remainder,
    Add,
    Subtract,
    ShiftLeft,
    ShiftRight,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
    NotEqual,
    BitAnd,
    BitXor,
    BitOr,
    And,
    Or,
}

impl BinaryOp {
    /// How the operator is written, which is also how the printer writes it.
    pub fn spelling(self) -> &'static str {
        match self {
            BinaryOp::Power => "**",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "/",
            BinaryOp::Remainder => "%",
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::ShiftLeft => "<<",
            BinaryOp::ShiftRight => ">>",
            BinaryOp::Less => "<",
            BinaryOp::LessOrEqual => "<=",
            BinaryOp::Greater => ">",
            BinaryOp::GreaterOrEqual => ">=",
            BinaryOp::Equal => "==",
            BinaryOp::NotEqual => "!=",
            BinaryOp::BitAnd => "&",
            BinaryOp::BitXor => "^",
            BinaryOp::BitOr => "|",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
        }
    }
}

/// `for ((init; condition; step))` — the C-style loop, which shares nothing
/// with `for NAME in words` but its keyword.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForArith {
    /// Each is absent where the text left it out: `for ((;;))` is legal and
    /// loops forever, which an empty expression could not say.
    pub init: Option<Arith>,
    pub condition: Option<Arith>,
    pub step: Option<Arith>,
    pub body: Vec<Item>,
}

/// Brace expansion: the one word-level construct that changes how MANY words
/// there are.
///
/// ⚠ **Not grouping, though it shares the character.** `{ a; }` is a command
/// list; `a{b,c}d` is a single word that expands to `abd acd`. It sits beside
/// [`Glob`] rather than beside a compound statement — both name a set the text
/// does not enumerate.
///
/// ⚠ **A brace with nothing to expand is ordinary text.** `{a}` and `{}` come
/// out of bash as themselves — measured in `reader/probes/brace.sh` — so reading
/// them as literal characters is what bash does, not an absorption of something
/// unmodelled. The test is whether a top-level comma or a range is in there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Brace {
    /// `{a,b,c}` — the alternatives, which nest and may be empty (`{a,}`).
    Alternatives(Vec<Word>),
    /// `{1..9}`, `{a..e}`, `{1..9..2}` — a sequence, descending where `from`
    /// exceeds `to`.
    ///
    /// Held as written rather than enumerated: the tree says what the text says,
    /// and expanding it is the reader's job one layer up.
    Range {
        from: String,
        to: String,
        step: Option<String>,
    },
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

/// `<(cmd)` or `>(cmd)`, whose value is a path the shell invents.
///
/// ⚠ **A word segment, not a command.** It glues: `diff x<(a)` is ONE word — the
/// invented path concatenated onto `x` — and `x=<(a)` is an assignment whose
/// value is one. Both measured. So it sits beside a parameter in a word, and the
/// same node is reachable from a redirection's target, which is where the corpus
/// mostly writes it: `while read -r l; do …; done < <(ls)`.
///
/// ⚠ **The interior is normalised by bash, so the second gate checks it**, just
/// as it does a `$( )`: `<(a|b)` comes back as `<(a | b)`. Measured in
/// `reader/probes/process-substitution.sh`.
///
/// There is no `quoted` bit, because there is nothing to quote: `"<(a)"` is the
/// four literal characters, not a substitution, so this segment only ever
/// reaches the tree from unquoted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSubstitution {
    /// Which direction the invented path carries: `<(cmd)` is one to READ, and
    /// `>(cmd)` one to WRITE. Named for what the enclosing command does with it,
    /// the way [`RedirectOp`] is.
    pub direction: Direction,
    pub items: Vec<Item>,
}

/// Which way a process substitution's invented path is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `<(cmd)` — the command's output, as a file to read.
    Read,
    /// `>(cmd)` — the command's input, as a file to write.
    Write,
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
    /// `${a[0]}`, `${a[@]}` — which element, where the parameter is an array.
    ///
    /// ⚠ **A field rather than part of the name**, because it selects: `${a[0]}`
    /// and `${a[1]}` name the same parameter and different values, and a reader
    /// asking "which variable is this" must not have to unpick a string. The
    /// commonest by far is `${PIPESTATUS[0]}`.
    pub subscript: Option<Subscript>,
    /// `${x:-y}`, `${x%%.*}`, `${#x}` — what is done to the value.
    ///
    /// ⚠ **Neither gate can check what is in here.** Bash prints every operator
    /// form back verbatim — measured in `reader/probes/parameter-op.sh` — so the
    /// second gate compares two identical texts and has no opinion, exactly as
    /// it has none about the inside of a word. That leaves the round-trip law
    /// and construction, which is why an operator this enum cannot spell is a
    /// refusal rather than literal text.
    pub op: Option<ParameterOp>,
    /// ⚠ **Semantic, unlike a literal's quoting.** An unquoted expansion is
    /// split into words and then globbed; a quoted one is a single word whatever
    /// it holds. `echo $x` and `echo "$x"` are different programs, so this is a
    /// field on the tree rather than a decision the printer gets to make — the
    /// same reason `Glob` is not a `Literal` holding an asterisk.
    pub quoted: bool,
}

/// Which element of an array a `${a[…]}` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subscript {
    /// `[@]` — every element, each its own word.
    All,
    /// `[*]` — every element, joined into one.
    ///
    /// Kept apart from [`Subscript::All`] because the difference is the same one
    /// `"$@"` and `"$*"` have, and it decides how many arguments a command gets.
    Joined,
    /// `[0]`, `[i]`, `[$n]` — an index.
    ///
    /// ⚠ **Held as a word, not a number.** The corpus writes `${a[$g]}` as well
    /// as `${a[0]}`, so the index expands. An index that is *arithmetic*
    /// (`${a[i+1]}`) is refused rather than stored: `+` there is an operator,
    /// and keeping it as literal text would be the absorption this tree exists
    /// to prevent. None occur in 131,246 commands.
    Index(Word),
}

/// What a `${…}` does to the value it names.
///
/// ⚠ **The `:` is a field, not a spelling.** `${x-y}` substitutes only when `x`
/// is *unset*; `${x:-y}` also substitutes when it is set and empty. Bash prints
/// both back as written, so nothing downstream would catch the two being
/// collapsed — measured, and the reason every branch below carries the flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterOp {
    /// `${x:-y}` — use `y` instead, leaving `x` alone.
    Default { colon: bool, word: Word },
    /// `${x:=y}` — use `y` and assign it.
    Assign { colon: bool, word: Word },
    /// `${x:?y}` — fail with `y` as the message.
    Error { colon: bool, word: Word },
    /// `${x:+y}` — use `y` only when `x` IS set. The sense is reversed from the
    /// three above, which is why it is its own variant rather than a flag.
    Alternate { colon: bool, word: Word },
    /// `${x#pat}` / `${x##pat}` — cut a matching prefix.
    StripPrefix { longest: bool, pattern: Word },
    /// `${x%pat}` / `${x%%pat}` — cut a matching suffix.
    StripSuffix { longest: bool, pattern: Word },
    /// `${x/pat/rep}` and its anchored spellings.
    Replace(Replace),
    /// `${x^}`, `${x^^}`, `${x,}`, `${x,,}` — change case.
    Case { upper: bool, every: bool },
    /// `${#x}` — how long the value is, or how many elements an array has.
    Length,
    /// `${!x}` — the value of the parameter *named* by `x`.
    Indirect,
}

/// `${x/pat/rep}`: which occurrences, and what replaces them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replace {
    /// `//` — every occurrence rather than the first.
    pub every: bool,
    /// `/#` and `/%` — anchored at the start or the end.
    pub anchor: Option<Anchor>,
    pub pattern: Word,
    /// Absent where the text gave none: `${x/pat}` deletes the match, and
    /// `${x/pat/}` says the same thing. One tree, so the printer picks a
    /// spelling.
    pub replacement: Option<Word>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// `${x/#pat/rep}` — only at the start.
    Start,
    /// `${x/%pat/rep}` — only at the end.
    End,
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
                | SegmentKind::Substitution(_)
                // A process substitution names a path the shell has not invented
                // yet — `/dev/fd/63`, and `/dev/fd/62` for the next one in the
                // same command.
                | SegmentKind::ProcessSubstitution(_)
                // A brace expansion names several words, so "the" text of the
                // word it sits in is a category error twice over.
                | SegmentKind::Brace(_)
                // Arithmetic names a NUMBER nobody has computed yet.
                | SegmentKind::Arithmetic(_) => {
                    return None;
                }
            }
        }
        Some(out)
    }
}
