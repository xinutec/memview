//! A faithful tree for the text the fleet executes, and a printer that puts it
//! back.
//!
//! `docs/execution-model.md` is the design; this is the first construct of it.
//! What it reads today is a script of simple commands and comments, with words
//! made of literal text and globs. Everything else is refused by name, and the
//! refusal ranking in the `bash-oracle` crate's `syntax-report` is what chooses what to
//! build next.
//!
//! ## Why this is not [`crate::shell`]
//!
//! That module is a projection: a flat list of commands with the structure taken
//! out, quoting resolved away, and heredoc bodies removed before parsing. It
//! answers *which commands ran*, well enough to mine a transcript, and it cannot
//! answer what a command would be if written another way, because it cannot
//! write one at all. This module is the tree that one becomes a projection of.
//!
//! ## The two gates
//!
//! - [`law`] — parse, print, parse again: the tree must be equal and the text a
//!   fixpoint. Runs over the whole corpus for free, and lives here because it
//!   needs nothing but this module.
//! - the `bash-oracle` crate — bash prints its own parse of our printed form,
//!   and we require the same tree back. Independent of us, and blind in a
//!   different place. **It is a separate crate because it spawns a process**,
//!   and this one states that it does not; see the workspace `Cargo.toml`.
//!
//! ⚠ **Neither gate can see a construct absorbed into a literal**, which is why
//! [`parse`] refuses rather than absorbs. The gates check what the tree says; only
//! the parser can be wrong about what is in it.

pub mod ast;
pub mod law;
pub mod parse;
pub mod print;

pub use ast::{Command, Comment, Glob, Item, Script, Segment, SegmentKind, Span, Word};
pub use law::{Outcome, check};
pub use parse::{Reason, Refusal, parse};
pub use print::print;
