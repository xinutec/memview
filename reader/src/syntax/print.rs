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

use super::ast::{
    AndOr, Command, Connector, Glob, Item, Pipeline, Redirect, RedirectOp, RedirectTarget, Script,
    Segment, SegmentKind, Timed, Word,
};
use super::parse::{is_assignment, is_reserved};

pub fn print(script: &Script) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(script.items.len());
    for item in &script.items {
        lines.push(match item {
            Item::Comment(comment) => format!("#{}", comment.text),
            Item::List(list) => print_and_or(list),
        });
    }
    lines.join("\n")
}

/// `a && b || c &` — connectors inline, `&` last.
///
/// One line, unlike `;`-separated lists which become one item each. That is
/// bash's own split: `declare -f` keeps `a && b` together and breaks `a; b`
/// apart, so following it keeps the printed form comparable with bash's.
pub fn print_and_or(list: &AndOr) -> String {
    let mut out = print_pipeline(&list.first);
    for link in &list.rest {
        out.push_str(match link.connector {
            Connector::And => " && ",
            Connector::Or => " || ",
        });
        out.push_str(&print_pipeline(&link.pipeline));
    }
    if list.background {
        out.push_str(" &");
    }
    out
}

/// ⚠ **`time` before `!`, whichever order they were written in.** That is the
/// order bash's own printer emits, and matching it is what lets the second gate
/// compare trees rather than argue about spelling.
pub fn print_pipeline(pipeline: &Pipeline) -> String {
    let mut parts: Vec<String> = Vec::new();
    match pipeline.time {
        Some(Timed::Plain) => parts.push("time".into()),
        Some(Timed::Posix) => parts.push("time -p".into()),
        None => {}
    }
    if pipeline.negated {
        parts.push("!".into());
    }
    let commands: Vec<String> = pipeline
        .commands
        .iter()
        .enumerate()
        .map(|(index, command)| print_command(command, index == 0))
        .collect();
    parts.push(commands.join(" | "));
    parts.retain(|part| !part.is_empty());
    parts.join(" ")
}

/// `head` is whether this command opens the pipeline — the only place `time`
/// and `!` are grammar, and therefore the only place a word spelling one has to
/// be quoted to stay a value.
fn print_command(command: &Command, head: bool) -> String {
    // ⚠ Words first, then redirections — bash's own order. `> out cat f` comes
    // back from `declare -f` as `cat f > out`, so putting them anywhere else
    // would be a spelling bash does not use and the tree does not record.
    let mut parts: Vec<String> = command
        .words
        .iter()
        .enumerate()
        .map(|(index, word)| print_word(word, index == 0 && head))
        .collect();
    parts.extend(command.redirects.iter().map(print_redirect));
    parts.join(" ")
}

fn print_redirect(redirect: &Redirect) -> String {
    // The descriptor is written only when it is not the operator's own default,
    // which is how `>` stays `>` and `2>` stays `2>`.
    let fd = match redirect.fd {
        Some(fd) if Some(fd) != redirect.op.default_fd() => fd.to_string(),
        _ => String::new(),
    };
    let op = match redirect.op {
        RedirectOp::Read => "<",
        RedirectOp::Write => ">",
        RedirectOp::Append => ">>",
        RedirectOp::ReadWrite => "<>",
        RedirectOp::Clobber => ">|",
        RedirectOp::DupOut => ">&",
        RedirectOp::DupIn => "<&",
        RedirectOp::Both => "&>",
        RedirectOp::BothAppend => "&>>",
        RedirectOp::BothWord => ">&",
    };
    match &redirect.target {
        // No space after a dup operator: `2>&1`, not `2>& 1`.
        RedirectTarget::Fd(target) => format!("{fd}{op}{target}"),
        RedirectTarget::Close => format!("{fd}{op}-"),
        RedirectTarget::File(word) => format!("{fd}{op} {}", print_word(word, false)),
    }
}

/// One word. `first` is whether it opens the pipeline's head command, the only
/// position where the shell reads a word as grammar rather than as a value.
pub fn print_word(word: &Word, first: bool) -> String {
    // ⚠ **A word that would read back as grammar is quoted whole.** `time` at
    // the head of a command is a keyword and `FOO=bar` is a binding, so printing
    // either bare turns a value the tree holds into something the parser would
    // refuse — and a refusal on `t₂` is a round-trip failure. Quoting is what
    // says "this really is the name of a program", which is exactly what the
    // shell means by it.
    // ⚠ `time` is checked by name because it is NOT in `RESERVED` — it is
    // grammar only here, at a pipeline's head, and a plain program name after a
    // `|`. Printing this tree's `time` bare would turn a command into a keyword.
    if first
        && let Some(text) = word.as_literal()
        && (is_reserved(&text) || is_assignment(&text) || text == "time")
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
