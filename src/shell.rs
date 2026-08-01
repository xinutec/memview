//! A parser for the shell Claude has actually written, and nothing more.
//!
//! Much of the fleet's file use never passes through the `Write` or `Edit`
//! tools: the history holds 87,918 `Bash` calls against 36,371 Write and Edit
//! ones, and a `sed -i`, a heredoc or a `cp` changes a file as surely as any of
//! them. Counting only the tools that announce themselves undercounts, and
//! undercounts *unevenly* — an agent that reaches for `sed` loses work an agent
//! that reaches for `Edit` keeps.
//!
//! This module reads the syntax; [`crate::shell_files`] reads what the commands
//! mean, and is where a path is finally attributed to anybody.
//!
//! **This is not a shell and does not aim to become one.** It runs nothing and
//! expands nothing. The grammar (`shell.pest`) starts as the smallest thing that
//! describes a command, and grows only where the corpus proves it must:
//! `cargo run --bin shell-report -- <corpus>` reports what fraction parses and
//! what the failures look like, and every construct added is one that report
//! asked for. Restrictive on purpose — a grammar that accepts everything tells
//! you nothing about what it read.

use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "shell.pest"]
struct ShellParser;

/// One command as written: its words, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Simple {
    /// The command and its arguments, quotes removed, expansions left alone.
    pub argv: Vec<String>,
    /// The subshells enclosing this command, outermost first — `[]` at the top
    /// level, `[1]` inside the first `( … )`, `[1, 2]` inside a group within it.
    ///
    /// The commands come back as a flat list, which loses the one thing a
    /// subshell is *for*: `(cd android && ./gradlew build)` must not move the
    /// script's directory. Without this, every command after that group resolves
    /// against `android/` and names files that were never touched — exactly the
    /// invented path the whole exercise exists to avoid.
    ///
    /// Ids rather than a depth, because depth alone cannot tell
    /// `(cd a && x); (cd b && y)` from one group containing both: the second
    /// group would inherit the first's directory. A brace group gets no id,
    /// because in bash it forks no shell — `{ cd x; }` really does move the
    /// caller.
    pub scope: Vec<usize>,
    /// Files named by `>`, `>>` or `<` on this command. Kept apart from `argv`
    /// because a redirect target is a file the command *uses* without ever being
    /// passed it — and because leaving it in argv makes `> /tmp/log` look like an
    /// argument, which is what the first version did.
    pub redirects: Vec<Redirect>,
}

/// A file named by a redirection, and which way it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub target: String,
    /// `>` and `>>` write; `<` reads.
    pub write: bool,
}

/// Remove heredoc *bodies* before the grammar sees the text.
///
/// A body is data, not shell — commit messages, Python, YAML — and parsing prose
/// as shell finds commands nobody ran. It is stripped here rather than in the
/// grammar because a heredoc is the one construct whose terminator is chosen at
/// runtime, and the body does not even begin until the *line* ends, so it cannot
/// be expressed where it appears.
///
/// Honours the quoted (`<<'EOF'`) and indented (`<<-`) forms, both of which the
/// corpus uses, and leaves `<<<` alone: a here-string has no body.
fn strip_heredocs(script: &str) -> String {
    let mut out = String::with_capacity(script.len());
    let mut lines = script.lines().peekable();
    while let Some(line) = lines.next() {
        out.push_str(line);
        out.push('\n');
        for (delim, indented) in heredoc_delimiters(line) {
            for body in lines.by_ref() {
                let candidate = if indented { body.trim_start() } else { body };
                if let Some(residue) = terminator_residue(candidate, &delim) {
                    // **Keep what FOLLOWS the delimiter, not the delimiter.**
                    // Dropping the whole line broke the corpus's commonest
                    // heredoc shape, `bash -c 'python3 - <<PY … PY'`, where the
                    // closing quote sits on it — the string was left unterminated.
                    // Keeping the whole line instead left the delimiter behind as
                    // a command named `PY` that nobody ran. Only the punctuation
                    // is shell; the word is the heredoc's own bookkeeping.
                    out.push_str(residue);
                    out.push('\n');
                    break;
                }
            }
        }
    }
    out
}

/// Whether this line ends the heredoc, and what shell text it leaves behind.
///
/// Not just `line == delim`. The corpus's commonest heredoc is nested inside a
/// quoted argument — `bash -c 'python3 - <<PY … PY'` — where the *inner* shell
/// sees a bare `PY` but the text on disk reads `PY'`. Requiring an exact match
/// meant the terminator was never found, the rest of the script was eaten as
/// body, and the quote it closed was left open. So a delimiter followed only by
/// the punctuation that closes the construct around it counts, and that
/// punctuation — never the delimiter itself — is what is kept.
fn terminator_residue<'a>(line: &'a str, delim: &str) -> Option<&'a str> {
    let line = line.trim_end();
    let rest = line.strip_prefix(delim)?;
    rest.chars()
        .all(|c| "'\");&| \t".contains(c))
        .then_some(rest)
}

/// The heredoc delimiters opened on one line, in order — a line may open two.
fn heredoc_delimiters(line: &str) -> Vec<(String, bool)> {
    let mut found = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if &bytes[i..i + 2] != b"<<" {
            i += 1;
            continue;
        }
        let mut j = i + 2;
        if bytes.get(j) == Some(&b'<') {
            i = j + 1; // `<<<` is a here-string.
            continue;
        }
        let indented = bytes.get(j) == Some(&b'-');
        if indented {
            j += 1;
        }
        while bytes.get(j) == Some(&b' ') {
            j += 1;
        }
        if matches!(bytes.get(j), Some(b'\'') | Some(b'"')) {
            j += 1;
        }
        let start = j;
        while j < bytes.len()
            && !bytes[j].is_ascii_whitespace()
            && !matches!(bytes[j], b'\'' | b'"' | b';' | b'|' | b'&' | b')')
        {
            j += 1;
        }
        if start < j {
            found.push((line[start..j].to_string(), indented));
        }
        i = j.max(i + 2);
    }
    found
}

/// Every simple command in one script, or why it could not be read.
pub fn parse(script: &str) -> Result<Vec<Simple>, String> {
    let stripped = strip_heredocs(script);
    let mut out = Vec::new();
    let top = ShellParser::parse(Rule::script, &stripped)
        // The position the parser gave up at, with the text from there — the
        // grammar is grown from these, and "expected X" without the offending
        // text names a rule rather than a construct to support.
        .map_err(|e| {
            let at = match e.location {
                pest::error::InputLocation::Pos(p) => p,
                pest::error::InputLocation::Span((p, _)) => p,
            };
            stripped[at..].chars().take(24).collect::<String>()
        })?
        .next()
        .expect("script always yields one pair");
    walk(top, &[], &mut 0, &mut out);
    Ok(out)
}

/// Collect every simple command under a node, innermost first.
///
/// Order is the running order: the commands inside `$( … )`, `<( … )` and a
/// subshell run before the command they belong to, so they are emitted first.
///
/// `scope` is the chain of subshells the node sits in, and `next` hands out the
/// ids, so two sibling groups are never confused for one.
fn walk(
    pair: pest::iterators::Pair<Rule>,
    scope: &[usize],
    next: &mut usize,
    out: &mut Vec<Simple>,
) {
    match pair.as_rule() {
        Rule::command => {
            let mut cmd = Simple {
                argv: Vec::new(),
                scope: scope.to_vec(),
                redirects: Vec::new(),
            };
            for part in pair.into_inner() {
                match part.as_rule() {
                    Rule::word => {
                        // A word may contain substitutions, whose commands run
                        // first and are emitted before this one.
                        for inner in part.clone().into_inner() {
                            if matches!(inner.as_rule(), Rule::subst | Rule::backtick) {
                                nested(inner, scope, next, out);
                            }
                        }
                        cmd.argv.push(unquote(part.as_str()));
                    }
                    Rule::redirect => collect_redirect(part, scope, next, &mut cmd, out),
                    // A group or a function body: its commands are the commands.
                    // `( … )` is a subshell and holds its own directory; `{ … }`
                    // shares the caller's, in bash and so here.
                    _ => {
                        if subshell(&part) {
                            walk(part, &descend(scope, next), next, out);
                        } else {
                            walk(part, scope, next, out);
                        }
                    }
                }
            }
            if !cmd.argv.is_empty() || !cmd.redirects.is_empty() {
                out.push(cmd);
            }
        }
        _ => {
            for inner in pair.into_inner() {
                walk(inner, scope, next, out);
            }
        }
    }
}

/// Whether a group is `( … )` rather than `{ … }` — the grammar matches both
/// with one rule, and only the paren form forks a shell.
fn subshell(pair: &pest::iterators::Pair<Rule>) -> bool {
    pair.as_rule() == Rule::group && pair.as_str().starts_with('(')
}

/// A fresh scope one level inside `outer`.
fn descend(outer: &[usize], next: &mut usize) -> Vec<usize> {
    *next += 1;
    let mut inner = outer.to_vec();
    inner.push(*next);
    inner
}

/// A redirection: a file target, a descriptor form that names none, or a process
/// substitution, which is commands rather than a file.
fn collect_redirect(
    pair: pest::iterators::Pair<Rule>,
    scope: &[usize],
    next: &mut usize,
    cmd: &mut Simple,
    out: &mut Vec<Simple>,
) {
    for part in pair.into_inner() {
        match part.as_rule() {
            Rule::file_redirect => {
                let mut write = true;
                let mut target = None;
                for bit in part.into_inner() {
                    match bit.as_rule() {
                        Rule::read => write = false,
                        Rule::word => target = Some(unquote(bit.as_str())),
                        _ => {}
                    }
                }
                if let Some(target) = target {
                    cmd.redirects.push(Redirect { target, write });
                }
            }
            // `<<'PY'` names a delimiter, `<<<x` a value, `2>&1` a descriptor:
            // none of them is a file, and none belongs in argv either.
            Rule::heredoc | Rule::herestring | Rule::fd_dup => {}
            Rule::procsub => walk(part, &descend(scope, next), next, out),
            _ => {}
        }
    }
}

/// Re-parse the text inside `$( … )` or backticks as the script it is.
///
/// A body that cannot be read contributes nothing rather than failing the
/// command around it — the outer command was still run, and its own files are
/// still worth having.
fn nested(
    pair: pest::iterators::Pair<Rule>,
    scope: &[usize],
    next: &mut usize,
    out: &mut Vec<Simple>,
) {
    let text = pair.as_str();
    let inner = text
        .strip_prefix("$(")
        .and_then(|t| t.strip_suffix(')'))
        .or_else(|| text.strip_prefix('`').and_then(|t| t.strip_suffix('`')))
        .unwrap_or("");
    if let Ok(cmds) = parse(inner) {
        // A substitution is a subshell like any other, so it gets an id — and
        // re-parsing numbered its own groups from scratch, so those are hung
        // below it. Two levels of the same number cannot collide: the prefix is
        // unique even when the suffix is not.
        let own = descend(scope, next);
        out.extend(cmds.into_iter().map(|c| Simple {
            scope: own.iter().copied().chain(c.scope).collect(),
            ..c
        }));
    }
}

/// Strip one layer of quoting from a word.
///
/// A word can be several runs stuck together — `--flag="a b"` — so this walks
/// the text rather than testing the ends.
fn unquote(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let chars: Vec<char> = word.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\'' => {
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    out.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        out.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    out.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}
