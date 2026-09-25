//! Programs in another language inside a shell tree, and where each one sits.
//!
//! A separate pass over a finished parse, as `docs/execution-model.md` lays out:
//! improving what it recognises never changes the shell tree of unchanged text.
//! It points at the node holding each program rather than copying it out, so
//! a later step can rewrite the program in place.
//!
//! Which command runs Python, and from where, is [`crate::shell_ops::python_program`]'s
//! answer; this pass maps it onto the tree. A program handed to another shell
//! (`bash -c`, `nix-shell --run`) or another machine is a layer further down
//! and is not descended into yet.

use super::ast::{
    Command, CommandKind, Heredoc, RedirectOp, RedirectTarget, Script, SegmentKind, Word,
};
use super::python;
use crate::shell_ops::{PythonFrom, python_program};

/// One program, and the node its text came from.
#[derive(Debug)]
pub struct Embedded<'t> {
    /// The command that runs it.
    pub command: &'t Command,
    pub site: Site<'t>,
    pub program: Program,
}

/// The node holding a program's text.
#[derive(Debug, Clone, Copy)]
pub enum Site<'t> {
    /// `python3 -c '…'`.
    Argument(&'t Word),
    /// `python3 - <<'PY'`.
    Heredoc(&'t Heredoc),
    /// `python3 <<< '…'`.
    HereString(&'t Word),
}

#[derive(Debug)]
pub enum Program {
    /// The text Python receives, and its tree or why there is none.
    Text {
        source: String,
        tree: Result<python::ast::Module, python::Refusal>,
    },
    /// The text holds a shell expansion, so what Python receives depends on
    /// values the text does not give.
    Expands,
}

/// Every Python program `script` runs directly, outer before inner.
pub fn python(script: &Script) -> Vec<Embedded<'_>> {
    let mut out = Vec::new();
    super::visit::commands(&script.items, &mut |command| {
        if let Some(site) = site(command) {
            let program = match text(site) {
                Some(source) => Program::Text {
                    tree: python::parse(&source),
                    source,
                },
                None => Program::Expands,
            };
            out.push(Embedded {
                command,
                site,
                program,
            });
        }
    });
    out
}

fn site(command: &Command) -> Option<Site<'_>> {
    let CommandKind::Simple(simple) = &command.kind else {
        return None;
    };
    // The same argv the flat chain classifies, so the wrapper tables see the same
    // words; only the positions are needed back.
    let argv: Vec<String> = simple
        .assignments
        .iter()
        .map(|assignment| format!("{}=", assignment.name))
        .chain(simple.words.iter().map(super::print::print_value))
        .collect();
    match python_program(&argv)? {
        PythonFrom::Argument(at) => simple
            .words
            .get(at.checked_sub(simple.assignments.len())?)
            .map(Site::Argument),
        // The last redirection of descriptor 0 is the one standing when it runs.
        PythonFrom::Stdin => {
            let redirect = command.redirects.iter().rfind(|r| r.fd == Some(0))?;
            match (&redirect.op, &redirect.target) {
                (_, RedirectTarget::Here(heredoc)) => Some(Site::Heredoc(heredoc)),
                (RedirectOp::HereString, RedirectTarget::File(word)) => {
                    Some(Site::HereString(word))
                }
                _ => None,
            }
        }
    }
}

/// The text Python receives, or `None` where the shell expands something first.
fn text(site: Site<'_>) -> Option<String> {
    match site {
        Site::Argument(word) | Site::HereString(word) => literal(word),
        Site::Heredoc(heredoc) if heredoc.quoted => Some(heredoc.body.clone()),
        Site::Heredoc(heredoc) => unquoted_body(&heredoc.body),
    }
}

fn literal(word: &Word) -> Option<String> {
    word.segments
        .iter()
        .map(|segment| match &segment.kind {
            SegmentKind::Literal(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// An unquoted body as the shell hands it over: `\$`, `` \` `` and `\\` lose
/// their backslash, any other backslash stays. `None` at an unescaped `$` or
/// backquote, which expands.
fn unquoted_body(body: &str) -> Option<String> {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '$' | '`' => return None,
            '\\' => match chars.peek() {
                Some(&next @ ('$' | '`' | '\\')) => {
                    out.push(next);
                    chars.next();
                }
                _ => out.push('\\'),
            },
            c => out.push(c),
        }
    }
    Some(out)
}
