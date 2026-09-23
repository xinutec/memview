//! What a program replaced in the files it wrote back: the edit an `Edit` call
//! would have carried, when the agent made it in Python instead.
//!
//! **Straight-line code only, evaluated in order.** A name may be rebound — `s =
//! s.replace(…)` is the whole shape — because top-level statements run once each,
//! top to bottom. Anything that could make a statement run twice, once or never
//! refuses the program: an indented line, or a line opening with `if`, `for`,
//! `with`, `try`, `def` and the rest. Nothing is guessed; a program this cannot
//! follow has no rewrites, not approximate ones.
//!
//! Nothing here runs anything. Files are not opened: `open(p).read()` is known as
//! *the text of `p`*, and a rewrite is the list of replacements that text went
//! through before being written back to the same path.

use std::collections::BTreeMap;

use pest::Parser;
use pest::iterators::Pair;

use super::{PythonParser, Rule, text};

/// One `str.replace` a rewrite applied, every occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replaced {
    pub old: String,
    pub new: String,
}

/// A file read, changed by replacements, and written back to the same path.
///
/// The path is unresolved, as a [`crate::program::Use`]'s is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    pub path: String,
    pub replaced: Vec<Replaced>,
}

/// Every rewrite in `source`, in the order written — or none, when anything in it
/// could change which statements run.
pub fn rewrites(source: &str) -> Vec<Rewrite> {
    if super::did_not_run(source).is_some() {
        return Vec::new();
    }
    let Ok(mut parsed) = PythonParser::parse(Rule::program, source) else {
        return Vec::new();
    };
    let Some(program) = parsed.next() else {
        return Vec::new();
    };
    let elements: Vec<Pair<Rule>> = program.into_inner().collect();
    if elements.iter().any(|element| branches(source, element)) {
        return Vec::new();
    }
    let mut run = Run::default();
    for element in elements {
        run.statement(element);
    }
    run.out
}

/// Whether this statement makes the order of the others unknowable.
fn branches(source: &str, element: &Pair<Rule>) -> bool {
    let start = element.as_span().start();
    let line = source[..start].rfind('\n').map_or(0, |at| at + 1);
    let before = &source[line..start];
    // The first statement on an indented line: inside a block of some kind.
    if !before.is_empty() && before.chars().all(|c| c == ' ' || c == '\t') {
        return true;
    }
    match element.as_rule() {
        Rule::funcdef => true,
        Rule::binder => element
            .clone()
            .into_inner()
            .next()
            .is_none_or(|keyword| keyword.as_str() != "import"),
        Rule::expr => element
            .clone()
            .into_inner()
            .next()
            .is_some_and(|head| head.as_rule() == Rule::name && OPENS.contains(&head.as_str())),
        _ => false,
    }
}

/// Words that open a branch, a loop or an exit when they start a statement.
const OPENS: &[&str] = &[
    "if", "elif", "else", "while", "for", "try", "except", "finally", "with", "match", "case",
    "return", "raise", "break", "continue", "exit", "quit", "sys", "os",
];

/// What a name or an expression holds, as far as this follows it.
#[derive(Debug, Clone)]
enum Val {
    Str(String),
    /// `Path('x')`, or a string used where a path is.
    Path(String),
    /// `open(p)` or `open(p, 'w')`.
    File {
        path: String,
        write: bool,
    },
    /// The text of a file, after the replacements listed.
    Text {
        path: String,
        replaced: Vec<Replaced>,
    },
    /// `open`, `Path`.
    Builtin(&'static str),
    /// A method looked up and not yet called: `s.replace`.
    Method(Box<Val>, String),
    Unknown,
}

impl Val {
    fn path(&self) -> Option<&str> {
        match self {
            Val::Str(path) | Val::Path(path) => Some(path),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Run {
    names: BTreeMap<String, Val>,
    out: Vec<Rewrite>,
}

impl Run {
    fn statement(&mut self, element: Pair<Rule>) {
        match element.as_rule() {
            Rule::assign => {
                let mut inner = element.into_inner();
                if let (Some(name), Some(value)) = (inner.next(), inner.next()) {
                    let held = self.value(value);
                    self.names.insert(name.as_str().to_string(), held);
                }
            }
            Rule::augmented => {
                if let Some(name) = element.into_inner().next() {
                    self.names.insert(name.as_str().to_string(), Val::Unknown);
                }
            }
            Rule::expr => {
                self.expr(element);
            }
            _ => {}
        }
    }

    /// One operand and nothing else; anything computed is unknown.
    fn value(&mut self, value: Pair<Rule>) -> Val {
        let mut operands = value.into_inner();
        let (Some(only), None) = (operands.next(), operands.next()) else {
            return Val::Unknown;
        };
        let mut parts = only.into_inner();
        match (parts.next(), parts.next()) {
            (Some(expr), None) if expr.as_rule() == Rule::expr => self.expr(expr),
            _ => Val::Unknown,
        }
    }

    fn expr(&mut self, expr: Pair<Rule>) -> Val {
        let mut parts = expr.into_inner();
        let Some(head) = parts.next() else {
            return Val::Unknown;
        };
        let mut held = match head.as_rule() {
            Rule::string => text(head.as_str()).map_or(Val::Unknown, Val::Str),
            Rule::name => match head.as_str() {
                "open" => Val::Builtin("open"),
                "Path" | "PurePath" => Val::Builtin("Path"),
                "pathlib" => Val::Builtin("pathlib"),
                name => self.names.get(name).cloned().unwrap_or(Val::Unknown),
            },
            Rule::paren => match self.arguments(head).as_slice() {
                [(None, only)] => only.clone(),
                _ => Val::Unknown,
            },
            _ => Val::Unknown,
        };
        for trailer in parts {
            held = match trailer.as_rule() {
                Rule::attr => {
                    let name = trailer
                        .into_inner()
                        .next()
                        .map_or_else(String::new, |n| n.as_str().to_string());
                    match (&held, name.as_str()) {
                        (Val::Builtin("pathlib"), "Path") => Val::Builtin("Path"),
                        _ => Val::Method(Box::new(held), name),
                    }
                }
                Rule::call => {
                    let args = self.arguments(trailer);
                    self.call(held, &args)
                }
                _ => Val::Unknown,
            };
        }
        held
    }

    /// Every argument's keyword and value, read in order.
    fn arguments(&mut self, holder: Pair<Rule>) -> Vec<(Option<String>, Val)> {
        let Some(args) = holder.into_inner().next() else {
            return Vec::new();
        };
        args.into_inner()
            .map(|arg| {
                let mut keyword = None;
                let mut held = Val::Unknown;
                for part in arg.into_inner() {
                    match part.as_rule() {
                        Rule::keyword => {
                            keyword = part.into_inner().next().map(|n| n.as_str().to_string());
                        }
                        Rule::value => held = self.value(part),
                        _ => {}
                    }
                }
                (keyword, held)
            })
            .collect()
    }

    fn call(&mut self, callee: Val, args: &[(Option<String>, Val)]) -> Val {
        let positional: Vec<&Val> = args
            .iter()
            .filter(|(k, _)| k.is_none())
            .map(|(_, v)| v)
            .collect();
        let keyword = |name: &str| {
            args.iter()
                .find(|(k, _)| k.as_deref() == Some(name))
                .map(|(_, v)| v)
        };
        match callee {
            Val::Builtin("Path") => match positional.as_slice() {
                [only] => only
                    .path()
                    .map_or(Val::Unknown, |path| Val::Path(path.to_string())),
                _ => Val::Unknown,
            },
            Val::Builtin("open") => {
                let Some(path) = positional.first().and_then(|v| v.path()) else {
                    return Val::Unknown;
                };
                file(path, positional.get(1).copied().or_else(|| keyword("mode")))
            }
            Val::Method(receiver, name) => self.method(*receiver, &name, &positional, args.len()),
            _ => Val::Unknown,
        }
    }

    fn method(&mut self, receiver: Val, name: &str, positional: &[&Val], given: usize) -> Val {
        match (receiver, name) {
            (Val::Path(path), "read_text") => Val::Text {
                path,
                replaced: Vec::new(),
            },
            (Val::Path(path), "open") => file(&path, positional.first().copied()),
            (Val::File { path, write: false }, "read") if given == 0 => Val::Text {
                path,
                replaced: Vec::new(),
            },
            (Val::Text { path, mut replaced }, "replace") if given == 2 => match positional {
                [Val::Str(old), Val::Str(new)] => {
                    replaced.push(Replaced {
                        old: old.clone(),
                        new: new.clone(),
                    });
                    Val::Text { path, replaced }
                }
                _ => Val::Unknown,
            },
            (Val::Path(path), "write_text") | (Val::File { path, write: true }, "write") => {
                if let [
                    Val::Text {
                        path: read,
                        replaced,
                    },
                ] = positional
                    && *read == path
                    && !replaced.is_empty()
                {
                    self.out.push(Rewrite {
                        path,
                        replaced: replaced.clone(),
                    });
                }
                Val::Unknown
            }
            _ => Val::Unknown,
        }
    }
}

/// `open(path, mode)`: reading unless the mode writes. Appending is neither.
fn file(path: &str, mode: Option<&Val>) -> Val {
    match mode {
        None => Val::File {
            path: path.to_string(),
            write: false,
        },
        Some(Val::Str(mode)) if mode.contains('a') || mode.contains('+') => Val::Unknown,
        Some(Val::Str(mode)) => Val::File {
            path: path.to_string(),
            write: mode.contains('w'),
        },
        Some(_) => Val::Unknown,
    }
}
