//! The tree for the Python inside the shell, and a printer that puts it back —
//! the same three gates as the shell's tree ([`super`]), for the language the
//! corpus measured: scripts. `docs/execution-model.md` is the design.
//!
//! Classes, `async`, generators, annotations and `match` are refused by name:
//! the corpus holds almost none of them, and a refusal is ranked, not guessed at.

pub mod ast;
pub mod law;
pub mod lex;
pub mod parse;
pub mod print;

pub use law::{Outcome, check};
pub use parse::{Reason, Refusal, parse};
pub use print::print;
