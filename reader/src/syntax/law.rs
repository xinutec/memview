//! The round-trip law, as a function of one text.
//!
//! ```text
//! t₁ ──P──→ A₁ ──G──→ t₂ ──P──→ A₂ ──G──→ t₃
//!
//! (1)  A₂ = A₁          the generated text parses to the same tree
//! (2)  t₃ = t₂          the generated form is a fixpoint
//!      t₂ ≠ t₁          permitted: layout and quoting normalise
//! ```
//!
//! Both conditions are checked, not just the first. (2) follows from (1) for a
//! printer that is a pure function of the tree, and checking it separately is
//! what keeps that true: a printer that reached for the source buffer or a span
//! would satisfy (1) and fail here.
//!
//! ⚠ **A refusal is not a failure of the law.** `t₁` that this parser does not
//! read is counted apart and ranked by reason — it is the work queue. What the
//! law is for is the text that *was* read, where a wrong tree would otherwise
//! look exactly like a right one.

use super::ast::Script;
use super::parse::{Refusal, parse};
use super::print::print;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// `t₁` is outside what this parser reads. The work queue, not a defect.
    Refused(Refusal),
    /// `t₂` did not parse. The printer wrote something the parser cannot read,
    /// which is a defect in one of them and always a real one.
    Unreadable(Refusal),
    /// (1) failed: the tree came back different.
    TreeDiffers { first: String, second: String },
    /// (2) failed: printing twice gave two different texts, so the printer is
    /// reading something that is not the tree.
    NotFixpoint { second: String, third: String },
    /// Both conditions hold.
    Holds { printed: String },
}

impl Outcome {
    pub fn holds(&self) -> bool {
        matches!(self, Outcome::Holds { .. })
    }

    /// A stable label, so a corpus run can group outcomes without matching on
    /// the payloads.
    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Refused(_) => "refused",
            Outcome::Unreadable(_) => "t₂ does not parse",
            Outcome::TreeDiffers { .. } => "A₂ ≠ A₁",
            Outcome::NotFixpoint { .. } => "t₃ ≠ t₂",
            Outcome::Holds { .. } => "holds",
        }
    }
}

pub fn check(text: &str) -> Outcome {
    let first: Script = match parse(text) {
        Ok(tree) => tree,
        Err(refusal) => return Outcome::Refused(refusal),
    };
    let second_text = print(&first);
    let second: Script = match parse(&second_text) {
        Ok(tree) => tree,
        Err(refusal) => return Outcome::Unreadable(refusal),
    };
    if second != first {
        return Outcome::TreeDiffers {
            first: format!("{first:?}"),
            second: format!("{second:?}"),
        };
    }
    let third_text = print(&second);
    if third_text != second_text {
        return Outcome::NotFixpoint {
            second: second_text,
            third: third_text,
        };
    }
    Outcome::Holds {
        printed: second_text,
    }
}
