//! The round-trip law for Python: parse, print, parse again. The tree must come
//! back equal and the print must be a fixpoint — see [`crate::syntax::law`], of
//! which this is the same law over another language.

use super::parse::{Refusal, parse};
use super::print::print;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Outside what the parser reads: the work queue.
    Refused(Refusal),
    /// The print did not parse — a defect in the printer or the parser.
    Unreadable {
        printed: String,
        refusal: Refusal,
    },
    /// (1) failed: the tree came back different.
    TreeDiffers {
        printed: String,
    },
    /// (2) failed: printing twice gave two texts.
    NotFixpoint {
        second: String,
        third: String,
    },
    Holds {
        printed: String,
    },
}

impl Outcome {
    pub fn holds(&self) -> bool {
        matches!(self, Outcome::Holds { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Refused(_) => "refused",
            Outcome::Unreadable { .. } => "print unreadable",
            Outcome::TreeDiffers { .. } => "tree differs",
            Outcome::NotFixpoint { .. } => "not a fixpoint",
            Outcome::Holds { .. } => "holds",
        }
    }
}

/// The law, applied to one program.
pub fn check(text: &str) -> Outcome {
    let first = match parse(text) {
        Ok(tree) => tree,
        Err(refusal) => return Outcome::Refused(refusal),
    };
    let second_text = print(&first);
    let second = match parse(&second_text) {
        Ok(tree) => tree,
        Err(refusal) => {
            return Outcome::Unreadable {
                printed: second_text,
                refusal,
            };
        }
    };
    if second != first {
        return Outcome::TreeDiffers {
            printed: second_text,
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
