//! A faithful tree for the text the fleet executes, and a printer that puts it
//! back.
//!
//! `docs/execution-model.md` is the design. What it reads today: and-or lists of
//! pipelines, whose commands are either simple — a binding prefix, words and
//! redirections — or a `for`/`while`/`until`/`select` loop carrying a body.
//! Words are typed segments: literal text, globs, tilde prefixes, parameters and
//! `$( )` substitutions, which hold a whole script and are where this parser
//! recurses into itself. Heredocs and comments are nodes. `time` and `!` are
//! fields on the pipeline, not `argv[0]`.
//!
//! **Everything else is refused by name.** The ranked refusals in the
//! `bash-oracle` crate's `syntax-report` are what choose the next construct, and
//! the figures live there rather than here — one `cargo run` away, and moving.
//!
//! ## Why this is not [`crate::shell`]
//!
//! That module is a projection: a flat list of commands with the structure taken
//! out, quoting resolved away, and heredoc bodies removed before parsing. It
//! answers *which commands ran*, well enough to mine a transcript, and it cannot
//! answer what a command would be if written another way, because it cannot
//! write one at all. This module is the tree that one becomes a projection of.
//!
//! ## The three gates
//!
//! - [`law`] — parse, print, parse again: the tree must be equal and the text a
//!   fixpoint. Runs over the whole corpus for free, and lives here because it
//!   needs nothing but this module.
//! - the `bash-oracle` crate — bash prints its own parse of the ORIGINAL
//!   command, and we require the same tree back. Independent of us, and blind in
//!   a different place.
//! - the same crate again — `bash -n` over our print, which asks the question
//!   neither of the others does: is what we emit shell at all?
//!
//! **The last two are a separate crate because they spawn a process**, and this
//! one states that it does not; see the workspace `Cargo.toml`.
//!
//! ⚠ **No gate can see a construct absorbed into a literal**, which is why
//! [`parse`] refuses rather than absorbs. The gates check what the tree says; only
//! the parser can be wrong about what is in it.

pub mod ast;
pub mod law;
pub mod parse;
pub mod print;
pub mod survey;

pub use ast::{
    AndOr, Command, Comment, Connector, Glob, Heredoc, Item, Link, Parameter, Pipeline, Redirect,
    RedirectOp, RedirectTarget, Script, Segment, SegmentKind, Span, Tilde, Timed, Word,
};
pub use law::{Outcome, check};
pub use parse::{Reason, Refusal, parse};
pub use print::print;
pub use survey::survey;
