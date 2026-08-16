//! Tree to text, reading nothing but the tree.
//!
//! ⚠ **The printer may not look at the source.** Condition (2) of the round-trip
//! law — that the generated form is a fixpoint — follows from condition (1) only
//! when `G` is a pure function of the tree, and the usual way to break it is a
//! printer that reaches back for the original spelling of a token. There is no
//! `&str` of source in this module's signatures, which is the enforcement.
//!
//! The form is canonical: one command per line, one space between words, and the
//! least quoting that reads back as the same tree. Two commands that differ only
//! in layout or quoting print identically, which is what makes the printed form
//! usable as an equivalence test.

use super::ast::{Glob, Item, Script, Segment, SegmentKind, Word};
use super::parse::{is_assignment, is_reserved};

pub fn print(script: &Script) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(script.items.len());
    for item in &script.items {
        lines.push(match item {
            Item::Comment(comment) => format!("#{}", comment.text),
            Item::Command(command) => command
                .words
                .iter()
                .enumerate()
                .map(|(index, word)| print_word(word, index == 0))
                .collect::<Vec<_>>()
                .join(" "),
        });
    }
    lines.join("\n")
}

/// One word. `first` is whether it opens a command, the only position where the
/// shell reads a word as grammar rather than as a value.
pub fn print_word(word: &Word, first: bool) -> String {
    // ⚠ **A word that would read back as grammar is quoted whole.** `time` at
    // the head of a command is a keyword and `FOO=bar` is a binding, so printing
    // either bare turns a value the tree holds into something the parser would
    // refuse — and a refusal on `t₂` is a round-trip failure. Quoting is what
    // says "this really is the name of a program", which is exactly what the
    // shell means by it.
    if first
        && let Some(text) = word.as_literal()
        && (is_reserved(&text) || is_assignment(&text))
    {
        return quote(&text);
    }
    if word.segments.is_empty() {
        return "''".to_string();
    }
    word.segments.iter().map(print_segment).collect()
}

fn print_segment(segment: &Segment) -> String {
    match &segment.kind {
        SegmentKind::Glob(Glob::Any) => "*".to_string(),
        SegmentKind::Glob(Glob::One) => "?".to_string(),
        SegmentKind::Literal(text) => {
            if needs_quoting(text) {
                quote(text)
            } else {
                text.clone()
            }
        }
    }
}

/// Single quotes, because inside them the shell expands nothing at all. An
/// `argv` element is a value, and any spelling that could expand would be a
/// different claim about it.
fn quote(text: &str) -> String {
    if text.is_empty() {
        return "''".to_string();
    }
    // The one character single quotes cannot hold: close, escape it, reopen.
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn needs_quoting(text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    if text.chars().any(|c| !is_bare_safe(c)) {
        return true;
    }
    // `[` and `]` are ordinary characters until they are both there, and then
    // they are a bracket expression. Keeping them bare in isolation is what lets
    // `[ -f x ]` — the corpus's commonest conditional — print as it was written.
    match text.find('[') {
        Some(open) => text[open..].contains(']'),
        None => false,
    }
}

/// Characters a bare word may hold without the shell reading anything into them.
///
/// Deliberately narrow. `~` expands at the head of a word, `#` opens a comment
/// there, `!` is history, and each is cheaper to quote everywhere than to reason
/// about by position — over-quoting costs a character and reads back the same,
/// while under-quoting is a wrong tree.
fn is_bare_safe(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '_' | '-' | '.' | '/' | ':' | ',' | '+' | '@' | '%' | '=' | '[' | ']'
        )
}
