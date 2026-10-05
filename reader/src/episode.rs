//! Recurring sequences of acts across an episode — the key the episode layer
//! is mined with. `--bin episode-report` is the census over it
//! (`docs/concept-model.md`, instrument 5).
//!
//! A single-command concept names the ACT; the author's description names the
//! PURPOSE, and gate 4 found the two agree only where the act is the purpose
//! (`Commit`) and part ways where it is not: `git log -1` is "show the last
//! commit" to the lens and "check the commit landed" to its author. The purpose
//! is in what came before, so the unit here is the EPISODE — the calls one
//! instruction produced, bracketed by the user's turns as `doing.rs` observes
//! them, never inferred from a gap — and the question is which sequences of
//! acts recur across episodes, and whether the description sharpens once the
//! sequence is known.
//!
//! **The key is independent of the descriptions.** They are the witness, and an
//! instrument whose key was built from them would rediscover its own input —
//! the trap `concept-report` was built around (memview#1364). A [`Token`] is
//! the concept a step lifted to, or the shape the census queues it under; a
//! gram is a run of consecutive tokens, within and across calls, with its
//! subjects abstracted to their IDENTITY: the first distinct subject in the
//! gram is `A`, the next `B`, so `Rewrite(f); Page(f)` on two days over two
//! files is one key and `Rewrite(f); Page(g)` is another. That is concept
//! equality up to holes, extended to the RELATION between steps, which is what
//! recurrence detection needs and what a concept with its paths left in would
//! destroy. A call boundary is in the key too: `git log && git status` in one
//! call and the same two in two calls are different shapes, with one
//! description and two, and the key errs toward splitting as every census key
//! here does.
//!
//! What a step becomes is one of four things, each counted: a work token;
//! context (`cd`, `echo`, `sleep` — [`Op::Nothing`] or a `cd`), which is dropped
//! from the sequence so that it does not fragment every gram; a carrier
//! (`bash -c`, `nix-shell --run`), whose work is its children's and which
//! already has their tokens; or a modifier of the act before it. The first run
//! of the census (2026-10-05) put a subjectless `Page` at the head of every
//! table — 117,217 corpus rows end in a pipe to a bare pager — and
//! `Search(A) ; Page` above `Search(A)`: `grep x f | head` is one act, and a
//! pager on a pipe is how its product is shown, not a second thing done. So an
//! act over a STREAM that follows another act in the same call folds into it
//! ([`tokens`]): a pager silently, since fewer of the same lines is the same
//! product, and any other — `| wc -l`, `| grep -v x`, `| sed 's/a//'` — as a
//! `+` suffix, since a count or a filtered subset is a different product the
//! census should keep apart.

use crate::concept::{self, Concept, Subject, Why};
use crate::shell_files::Step;
use crate::shell_ops::Op;

/// One act in an episode: what it was, and what it was over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// A concept's name, or the census's shape for a step no lens lifts.
    pub label: String,
    /// The paths it named, in order, as the identity abstraction reads them.
    /// A subject the text did not determine is not a path and takes no letter.
    pub subjects: Vec<String>,
}

/// What a step became, so a census can balance its count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Became {
    Work(Token),
    /// Touches no file and names no act: `cd`, `echo`, `sleep`.
    Context,
    /// A wrapper whose commands are steps of their own.
    Carrier,
}

/// One `Bash` call: its tokens in order, and what its author said it was for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub tokens: Vec<Token>,
    pub said: Option<String>,
}

/// The paths a lifted concept is over, for identity.
fn named(subjects: &[Subject]) -> Vec<String> {
    subjects
        .iter()
        .filter_map(|subject| match subject {
            Subject::Named(path) => Some(path.clone()),
            Subject::Bounded(_) | Subject::Located(_) | Subject::Hole => None,
        })
        .collect()
}

/// A step's token, or why it has none.
pub fn token(step: &Step) -> Became {
    match concept::lift(step) {
        Ok(concept) => {
            let subjects = match &concept {
                Concept::Rewrite { subjects, .. }
                | Concept::Page { subjects, .. }
                | Concept::Search { subjects, .. }
                | Concept::Measure { subjects, .. }
                | Concept::Stage { subjects, .. } => named(subjects),
                Concept::List { loci, .. } => named(loci),
                Concept::History { paths, .. } | Concept::Status { paths } => named(paths),
                Concept::Commit { .. } => Vec::new(),
            };
            Became::Work(Token {
                label: concept::name(&concept).to_string(),
                subjects,
            })
        }
        Err(Why::Carrier) => Became::Carrier,
        Err(_) => {
            if matches!(step.op, Some(Op::Nothing) | Some(Op::ChangeDir { .. })) {
                return Became::Context;
            }
            // The files this step alone produced, in order, each once.
            let mut subjects: Vec<String> = Vec::new();
            for file in &step.files {
                if !subjects.contains(&file.path) {
                    subjects.push(file.path.clone());
                }
            }
            Became::Work(Token {
                label: concept::shape(step),
                subjects,
            })
        }
    }
}

/// One gram's key: the labels in order, subjects as identity letters, `;`
/// between tokens of one call and `|` where the next call begins.
///
/// `starts_call` marks each token that is the first of its call; the first
/// token's own mark is not written, since a gram does not say where it sits in
/// its call.
pub fn key(gram: &[(&Token, bool)]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = String::new();
    for (i, (token, starts_call)) in gram.iter().enumerate() {
        if i > 0 {
            out.push_str(if *starts_call { " | " } else { " ; " });
        }
        out.push_str(&token.label);
        if !token.subjects.is_empty() {
            let letters: Vec<String> = token
                .subjects
                .iter()
                .map(|path| {
                    let at = match seen.iter().position(|s| s == path) {
                        Some(at) => at,
                        None => {
                            seen.push(path);
                            seen.len() - 1
                        }
                    };
                    letter(at)
                })
                .collect();
            out.push('(');
            out.push_str(&letters.join(","));
            out.push(')');
        }
    }
    out
}

/// `A`, `B`, … `Z`, then `A1`, `B1`, … — one name per distinct subject.
fn letter(at: usize) -> String {
    let c = char::from(b'A' + (at % 26) as u8);
    match at / 26 {
        0 => c.to_string(),
        n => format!("{c}{n}"),
    }
}

/// Every gram of `n` consecutive tokens in an episode, in order, each with the
/// description of the call its LAST token came from — the sentence written
/// closest to the act the gram ends on.
pub fn grams(calls: &[Call], n: usize) -> Vec<(String, Option<&str>)> {
    let flat: Vec<(&Token, bool, usize)> = calls
        .iter()
        .enumerate()
        .flat_map(|(c, call)| {
            call.tokens
                .iter()
                .enumerate()
                .map(move |(i, token)| (token, i == 0, c))
        })
        .collect();
    if n == 0 || flat.len() < n {
        return Vec::new();
    }
    flat.windows(n)
        .map(|window| {
            let gram: Vec<(&Token, bool)> = window.iter().map(|(t, s, _)| (*t, *s)).collect();
            let last = window[n - 1].2;
            (key(&gram), calls[last].said.as_deref())
        })
        .collect()
}

/// Every token of one call, with the count of what each step became.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tokens {
    pub tokens: Vec<Token>,
    pub context: usize,
    pub carriers: usize,
    /// Stream acts folded into the act that fed them.
    pub folded: usize,
}

/// An act over what flows in, with no subject of its own: the tail of a pipe.
fn shapes_a_stream(token: &Token) -> bool {
    token.subjects.is_empty()
        && (matches!(token.label.as_str(), "Page" | "Search" | "Measure")
            || ["read · ", "search · ", "transform · "]
                .iter()
                .any(|phrase| token.label.starts_with(phrase)))
}

/// The tokens of one call's steps, in order, a stream act folded into the act
/// before it — see the module head for why.
pub fn tokens<'a>(steps: impl IntoIterator<Item = &'a Step>) -> Tokens {
    let mut out = Tokens::default();
    // Whether the step just before this one was an act in this call, so a
    // stream act has something to fold into: `cd x && grep y f | head` folds the
    // pager into the grep, and `echo x | head` has nothing to fold into.
    let mut fed = false;
    for step in steps {
        match token(step) {
            Became::Work(token) => {
                if fed && shapes_a_stream(&token) {
                    out.folded += 1;
                    if token.label != "Page"
                        && let Some(last) = out.tokens.last_mut()
                    {
                        last.label.push('+');
                        last.label.push_str(&token.label);
                    }
                } else {
                    out.tokens.push(token);
                }
                fed = true;
            }
            Became::Context => {
                out.context += 1;
                fed = false;
            }
            Became::Carrier => {
                out.carriers += 1;
                fed = false;
            }
        }
    }
    out
}
