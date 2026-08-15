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

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "shell.pest"]
struct ShellParser;

/// What had to hold for a command to run, as far as the text can say.
///
/// A three-point domain over "did this run", ordered by how much it takes on
/// trust. The point of it is that **a file use is only worth recording if the
/// command that made it actually ran**, and the separator before a command is
/// the only thing in the script that says whether it did.
///
/// [`Reached::and`] is the meet: a command inside a group that may not run is
/// itself uncertain, whatever separator precedes it.
/// Serialised one character apiece: this travels on every row of the effects
/// artefact, where the field's name already costs more than its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Reached {
    /// First in its list, or after `;`, a newline, or `&`. Runs whatever
    /// happened before it — which is why `a; b; c` runs all three even when the
    /// call as a whole reports failure.
    #[serde(rename = "a")]
    Always,
    /// After `&&`. Runs only if what precedes it succeeded.
    #[serde(rename = "s")]
    OnSuccess,
    /// After `||`, or under some condition this does not model. Runs sometimes,
    /// and the text cannot say when — the bucket that must never be counted as
    /// certain, whatever the call's exit status turned out to be.
    ///
    /// ⚠ **The `||` half of this is knowable and is thrown away on purpose.
    /// Measured 2026-08-15, memview #101, and the answer was no.** A non-zero
    /// exit on `a || b` proves `b` ran: had `a` succeeded the chain would have
    /// exited 0. The mirror of the `&&` rule in [`crate::doing::Verdict::admits`],
    /// and it is not implemented because the prize is too small to pay for:
    ///
    /// - **4,945** of 132,554 calls failed at all, and only a failure can
    ///   confirm a `||`. Everything else is out of reach by arithmetic.
    /// - **390** of those contain `||` anywhere in their text.
    /// - **113** file uses inside those sit in this bucket — a *ceiling*, since
    ///   it still counts `&&`s demoted by [`forget_discarded_status`] and
    ///   alternatives outside the final segment. Against 19,202 here and 179,477
    ///   confirmed, that is 0.59% of the bucket and 0.06% of the answer.
    ///
    /// ⚠ **And when it is worth doing, it will not need the fourth domain point
    /// the task assumed.** Six hand-written scripts read command by command:
    /// `a || b || c` confirms every link, because a non-zero exit means each in
    /// turn failed; and `a && b || c` confirms `c` too, since that chain can
    /// only exit non-zero through `c`. Both fall out of one rule — *a non-zero
    /// exit proves the last `||` alternative of the final segment ran* — which
    /// needs no `OnFailure` and no change to [`Reached::and`]. What it cannot
    /// confirm is `b`, and no domain point would.
    #[serde(rename = "?")]
    Sometimes,
}

impl Reached {
    /// Both conditions at once: this command's own, and that of whatever
    /// encloses it.
    ///
    /// Two `&&`s meet as one, because a chain of them is still just "everything
    /// before me worked". Anything touching [`Reached::Sometimes`] stays there:
    /// `a && b || c` reaches `c` under a condition made of both, and a domain
    /// this small should say "cannot tell" rather than pick a side.
    pub fn and(self, inner: Reached) -> Reached {
        match (self, inner) {
            (Reached::Always, other) | (other, Reached::Always) => other,
            (Reached::OnSuccess, Reached::OnSuccess) => Reached::OnSuccess,
            _ => Reached::Sometimes,
        }
    }
}

/// One command as written: its words, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Simple {
    /// The command and its arguments, quotes removed, expansions left alone.
    pub argv: Vec<String>,
    /// What had to hold for this command to run at all.
    pub reached: Reached,
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
    /// The bodies of the heredocs this command opened, in order.
    ///
    /// Data rather than shell — a commit message, YAML, a SQL script — and never
    /// parsed as shell. It is kept because some of that data is *itself* a
    /// program that changes files: 4,563 of the corpus's heredocs feed
    /// `python3 -`. Whether a body is worth reading, and in what language, is
    /// [`crate::shell_ops`]'s decision; this layer only refuses to lose it.
    pub heredocs: Vec<String>,
}

/// A file named by a redirection, and which way it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub target: String,
    /// `>` and `>>` write; `<` reads.
    pub write: bool,
}

/// What a hidden heredoc's delimiter is replaced by, in front of its encoded
/// body. Unmistakable on sight in a parse tree, and — the part that matters —
/// recognisable again on a second pass over the same text.
const HEREDOC_MARK: &str = "HEREDOC\u{1}";

/// Move heredoc *bodies* out of the shell text and onto the command that opened
/// them.
///
/// A body is not shell — commit messages, Python, YAML — and parsing prose as
/// shell finds commands nobody ran. It is taken out here rather than in the
/// grammar because a heredoc is the one construct whose terminator is chosen at
/// runtime, and the body does not even begin until the *line* ends, so it cannot
/// be expressed where it appears.
///
/// It is **encoded into the delimiter** rather than discarded, because some of
/// that data is a program in its own right: `python3 - <<'PY'` changes files as
/// surely as `sed -i` does, and 3,547 such writes were invisible while the body
/// went in the bin. Encoded into the text rather than set aside in a table,
/// because the corpus's commonest heredoc is `bash -c 'python3 - <<PY … PY'` —
/// the body belongs to the *inner* command, whose script is re-parsed later from
/// a substring of this very text. A table keyed on this pass would be out of
/// reach by then; a marker travels with the text it belongs to.
///
/// Honours the quoted (`<<'EOF'`) and indented (`<<-`) forms, both of which the
/// corpus uses, and leaves `<<<` alone: a here-string has no body.
fn hide_heredocs(script: &str) -> String {
    let lines: Vec<&str> = script.lines().collect();
    let mut out = String::with_capacity(script.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        let openers = heredoc_openers(line);
        if openers.is_empty() {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let mut rewritten = String::with_capacity(line.len());
        let mut residues = Vec::new();
        let mut cut = 0;
        for opener in &openers {
            // ⚠ **No terminator, no heredoc — and `cut` is left alone, so the
            // text stays exactly as written.** `<<` is two characters, not an
            // operator: an arithmetic shift, a quoted mention of redirection, a
            // grep pattern hunting for one. Consuming until the delimiter turned
            // up meant those swallowed the whole rest of the script as body, with
            // no error, so every command after them vanished from the parse.
            let Some((end, residue)) = find_terminator(&lines, i, opener) else {
                continue;
            };
            let mut body = String::new();
            for text in &lines[i..end] {
                body.push_str(text);
                body.push('\n');
            }
            // **Keep what FOLLOWS the delimiter, not the delimiter.** Dropping
            // the whole line broke the corpus's commonest heredoc shape, `bash -c
            // 'python3 - <<PY … PY'`, where the closing quote sits on it — the
            // string was left unterminated. Keeping the whole line instead left
            // the delimiter behind as a command named `PY` that nobody ran. Only
            // the punctuation is shell; the word is the heredoc's own bookkeeping.
            residues.push(residue);
            i = end + 1;
            rewritten.push_str(&line[cut..opener.at.start]);
            rewritten.push_str("<<");
            rewritten.push_str(HEREDOC_MARK);
            rewritten.push_str(&BASE64.encode(&body));
            cut = opener.at.end;
        }
        rewritten.push_str(&line[cut..]);
        out.push_str(&rewritten);
        out.push('\n');
        for residue in residues {
            out.push_str(&residue);
            out.push('\n');
        }
    }
    out
}

/// Where an opener's body ends and what its terminator line leaves behind, or
/// `None` when the terminator never arrives.
///
/// ⚠ **This is the whole test for whether a `<<` was an operator at all.**
/// Quoting cannot answer it. The corpus's commonest heredoc,
/// `bash -c 'python3 - <<PY … PY'`, has its `<<` inside a quoted argument and is
/// entirely real; `echo 'use << to redirect'` is the same shape and is not. What
/// separates them is that one is terminated and the other never is.
///
/// Looked up rather than consumed, so a `<<` that turns out to be data costs
/// nothing: the lines are still there for the parse that follows.
fn find_terminator(lines: &[&str], from: usize, opener: &Opener) -> Option<(usize, String)> {
    lines.iter().enumerate().skip(from).find_map(|(at, text)| {
        let candidate = if opener.indented {
            text.trim_start()
        } else {
            text
        };
        terminator_residue(candidate, &opener.delim).map(|residue| (at, residue.to_string()))
    })
}

/// The body an already-hidden heredoc carries, from the delimiter the grammar
/// matched. `None` for a delimiter that is not one of ours.
fn heredoc_body(operator: &str) -> Option<String> {
    let encoded = operator
        .trim_start_matches('<')
        .trim_start_matches('-')
        .strip_prefix(HEREDOC_MARK)?;
    String::from_utf8(BASE64.decode(encoded).ok()?).ok()
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

/// A heredoc opened on one line: where its `<<…delimiter` sits, what the
/// delimiter is, and whether the `<<-` form lets the terminator be indented.
struct Opener {
    at: std::ops::Range<usize>,
    delim: String,
    indented: bool,
}

/// The heredocs opened on one line, in order — a line may open two.
///
/// A delimiter already carrying a hidden body is **not** one of them. Without
/// that, re-parsing a nested script — which is a substring of text this has
/// already been over — would take the marker for a fresh opener and swallow the
/// rest of the script hunting a terminator that was consumed on the first pass.
fn heredoc_openers(line: &str) -> Vec<Opener> {
    let mut found = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if &bytes[i..i + 2] != b"<<" {
            i += 1;
            continue;
        }
        let at = i;
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
        let quoted = matches!(bytes.get(j), Some(b'\'') | Some(b'"'));
        if quoted {
            j += 1;
        }
        let start = j;
        while j < bytes.len()
            && !bytes[j].is_ascii_whitespace()
            && !matches!(bytes[j], b'\'' | b'"' | b';' | b'|' | b'&' | b')')
        {
            j += 1;
        }
        let delim = &line[start..j];
        if start < j && !delim.starts_with(HEREDOC_MARK) {
            found.push(Opener {
                // The closing quote is part of the opener, so replacing the
                // range leaves none of it behind: `<<'EOF'` goes whole.
                at: at..j + usize::from(quoted && bytes.get(j) == Some(&bytes[start - 1])),
                delim: delim.to_string(),
                indented,
            });
        }
        i = j.max(i + 2);
    }
    found
}

/// Every simple command in one script, or why it could not be read.
pub fn parse(script: &str) -> Result<Vec<Simple>, String> {
    let stripped = hide_heredocs(script);
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
    walk(top, &[], &mut 0, &mut out, Reached::Always);
    forget_discarded_status(&mut out);
    Ok(out)
}

/// Demote every `&&` whose success the script threw away.
///
/// ⚠ **A call reports one exit status, and `;` discards the one before it.** In
/// `a && b; c`, exit 0 says `c` worked and says *nothing whatever* about `a` —
/// so `b` cannot be confirmed by the status however the call turned out. Only
/// the last `;`-separated segment ends in the status that gets reported, so only
/// its `&&` chain is ever answerable.
///
/// Without this, "the call exited 0, so every `&&` ran" is a plain over-claim,
/// and it is the kind that reads as precision.
pub(crate) fn forget_discarded_status(cmds: &mut [Simple]) {
    // ⚠ **A closing keyword is not a command and has no status of its own.**
    // The grammar surfaces `done`, `fi` and `esac` as ordinary words, so they
    // arrive here looking like unconditional commands sitting *after* the body
    // they close. Left in, the last one anchors the final segment and demotes
    // every `&&` in the whole script — measured on the corpus, that is what a
    // single `for` loop did to everything before it.
    const CLOSERS: [&str; 3] = ["done", "fi", "esac"];
    // The final segment begins at the last unconditional command at the top
    // level; anything nested after it belongs to that segment too.
    let final_segment = cmds
        .iter()
        .rposition(|cmd| {
            cmd.scope.is_empty()
                && cmd.reached == Reached::Always
                && !cmd
                    .argv
                    .first()
                    .is_some_and(|word| CLOSERS.contains(&word.as_str()))
        })
        .unwrap_or(0);
    for cmd in &mut cmds[..final_segment] {
        if cmd.reached == Reached::OnSuccess {
            cmd.reached = Reached::Sometimes;
        }
    }
}

/// Collect every simple command under a node, innermost first.
///
/// Order is the running order: the commands inside `$( … )`, `<( … )` and a
/// subshell run before the command they belong to, so they are emitted first.
///
/// `scope` is the chain of subshells the node sits in, and `next` hands out the
/// ids, so two sibling groups are never confused for one.
///
/// `reached` is the condition on this node from everything enclosing it; the
/// separators between siblings refine it as the walk moves along them.
fn walk(
    pair: pest::iterators::Pair<Rule>,
    scope: &[usize],
    next: &mut usize,
    out: &mut Vec<Simple>,
    reached: Reached,
) {
    match pair.as_rule() {
        Rule::command => {
            let mut cmd = Simple {
                argv: Vec::new(),
                reached,
                scope: scope.to_vec(),
                redirects: Vec::new(),
                heredocs: Vec::new(),
            };
            for part in pair.into_inner() {
                match part.as_rule() {
                    Rule::word => {
                        // A word may contain substitutions, whose commands run
                        // first and are emitted before this one.
                        expansions(&part, scope, next, out, reached);
                        cmd.argv.push(unquote(part.as_str()));
                    }
                    Rule::redirect => collect_redirect(part, scope, next, &mut cmd, out),
                    // A group or a function body: its commands are the commands.
                    // `( … )` is a subshell and holds its own directory; `{ … }`
                    // shares the caller's, in bash and so here.
                    _ => {
                        if subshell(&part) {
                            walk(part, &descend(scope, next), next, out, reached);
                        } else {
                            walk(part, scope, next, out, reached);
                        }
                    }
                }
            }
            if !cmd.argv.is_empty() || !cmd.redirects.is_empty() || !cmd.heredocs.is_empty() {
                out.push(cmd);
            }
        }
        // `case x in a) … ;; b) … ;; esac`. The arms are alternatives, so at most
        // one of them ran and none of them is certain — the same reasoning, and
        // the same [`Reached::Sometimes`], that [`branch`] gives the two halves
        // of an `if`. Recording an arm as `Always` would claim a file use that
        // never happened, which is the one direction of error this reader is
        // built to avoid.
        //
        // ⚠ **The subject is not an arm.** `case $(readlink -f "$p") in` really
        // does run `readlink`, whichever way the match goes, so it keeps the
        // condition standing outside the statement.
        Rule::case_stmt => {
            for inner in pair.into_inner() {
                match inner.as_rule() {
                    Rule::word => expansions(&inner, scope, next, out, reached),
                    Rule::case_arm => {
                        walk(inner, scope, next, out, reached.and(Reached::Sometimes));
                    }
                    // The keywords carry nothing; the patterns are globs, not
                    // files, and are never walked.
                    _ => {}
                }
            }
        }
        // ⚠ **Defining a function runs none of it.** `f() { curl … > out; }`
        // records a write to `out` at the moment the *name* is bound, and the
        // body may never be called at all — the reader claiming a file use that
        // did not happen, which is the one error it is built not to make. It was
        // there from the round that added `func_def` and only surfaced when
        // memview#901 made nine more definitions parse.
        //
        // The body is kept rather than dropped, because when the function IS
        // called its commands are the only place those effects appear — and a
        // call site is an `Unknown` op that names no files. So it lands in
        // [`Reached::Sometimes`], which is precisely "runs sometimes and the text
        // cannot say when".
        Rule::func_def => {
            // The body, and only the body: the name binds and does nothing.
            for inner in pair.into_inner().filter(|p| p.as_rule() == Rule::group) {
                let inside = if subshell(&inner) {
                    descend(scope, next)
                } else {
                    scope.to_vec()
                };
                walk(inner, &inside, next, out, reached.and(Reached::Sometimes));
            }
        }
        _ => {
            // Only the rules that hold a *sequence* carry branch keywords; a
            // pipeline's own children would otherwise re-read the same `then`
            // and a group's would read one that belongs to its caller.
            let sequence = matches!(pair.as_rule(), Rule::script | Rule::body | Rule::case_body);
            // The separators are read as they are passed, so each command is
            // walked under the condition standing at its own position: in
            // `a && b; c`, `b` needs `a` to have worked and `c` needs nothing.
            let mut here = Reached::Always;
            let mut open = Vec::new();
            for inner in pair.into_inner() {
                match inner.as_rule() {
                    Rule::and_if => here = here.and(Reached::OnSuccess),
                    Rule::or_if => here = here.and(Reached::Sometimes),
                    // `;`, a newline or `&` end the chain: what follows runs
                    // however the last one went.
                    Rule::seq => here = Reached::Always,
                    _ => {
                        let arm = if sequence {
                            branch(&mut open, &inner)
                        } else {
                            Reached::Always
                        };
                        walk(inner, scope, next, out, reached.and(here).and(arm));
                    }
                }
            }
        }
    }
}

/// Take one step through a sequence's `if`s, and say whether what stands here
/// runs unconditionally.
///
/// ⚠ **The two arms of an `if` cannot both have run**, so recording both as
/// [`Reached::Always`] claims a file use that never happened — the one direction
/// of error this reader is built to avoid, since every other gap in it records
/// *less* than happened rather than more.
///
/// `if`, `then`, `else` and `fi` are not structure to this grammar: they survive
/// as ordinary words at the front of a command and are stripped later by
/// `unwrap_command`. So the sequence is walked with a stack — one entry per open
/// `if`, holding whether its condition has been passed — and everything inside a
/// branch is [`Reached::Sometimes`], the existing "runs sometimes and the text
/// cannot say when".
///
/// The condition itself stays `Always`: `if grep -q x a.txt` really does run it.
/// An `elif` does not — it is reached only when the test before it failed, which
/// is a condition, so it goes with the branches.
///
/// An `if` left open by the end of the sequence keeps everything after it
/// uncertain. That is the safe direction, and a script that is a fragment of
/// another one has nothing better to say.
fn branch(open: &mut Vec<bool>, pair: &pest::iterators::Pair<Rule>) -> Reached {
    // ⚠ **One command can carry two keywords.** `then if b` opens a nested `if`
    // *and* stands inside the outer one; reading only the first word leaves the
    // inner level unopened, and its `fi` then closes the outer — so everything
    // after the whole statement reads as certain again.
    for keyword in leading_keywords(pair) {
        match keyword {
            "if" => open.push(false),
            "then" | "elif" | "else" => {
                if let Some(innermost) = open.last_mut() {
                    *innermost = true;
                }
            }
            "fi" => {
                open.pop();
            }
            _ => {}
        }
    }
    if open.contains(&true) {
        Reached::Sometimes
    } else {
        Reached::Always
    }
}

/// The run of branch keywords a pipeline opens with, in order.
///
/// Stops at the first word that is not one, which is the command being guarded.
/// A group is not descended into: `( if x; then y; fi )` balances inside itself,
/// and reading its `if` from out here would leave a level open forever.
fn leading_keywords(pair: &pest::iterators::Pair<Rule>) -> Vec<&'static str> {
    const BRANCH: [&str; 5] = ["if", "then", "elif", "else", "fi"];
    let mut out = Vec::new();
    let mut stack = vec![pair.clone()];
    while let Some(part) = stack.pop() {
        match part.as_rule() {
            Rule::word => match BRANCH.iter().find(|kw| **kw == part.as_str()) {
                Some(keyword) => out.push(*keyword),
                None => return out,
            },
            Rule::pipeline | Rule::command => {
                stack.extend(part.into_inner().collect::<Vec<_>>().into_iter().rev());
            }
            _ => return out,
        }
    }
    out
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
            // A heredoc's delimiter is carrying its body — hand it to the
            // command, which is the only thing that knows whether it is a
            // commit message or a program.
            Rule::heredoc => {
                if let Some(body) = heredoc_body(part.as_str()) {
                    cmd.heredocs.push(body);
                }
            }
            // `<<<x` is a value and `2>&1` a descriptor: neither is a file, and
            // neither belongs in argv either.
            Rule::herestring | Rule::fd_dup => {}
            // `diff <(ls a) <(ls b)` — the inner commands run exactly when the
            // command they feed does, so they inherit its condition.
            Rule::procsub => walk(part, &descend(scope, next), next, out, cmd.reached),
            _ => {}
        }
    }
}

/// Every command a word runs before the word is a word: `$( … )` and backticks,
/// at any depth inside it.
///
/// ⚠ **The depth is the point.** A word's expansions used to be read from its
/// immediate children only, which was right for `$(git rev-parse HEAD)` and
/// silently wrong for `"$(git rev-parse HEAD)"` — the quoted form was one opaque
/// token, so the command inside it was never walked and its files were never
/// attributed to anybody. 8,300 distinct commands, 6.5% of the corpus
/// (memview#918). Now `dquoted` yields its expansions and this descends to them.
///
/// It does NOT descend into `squoted`: single quotes expand nothing, so
/// `'$(rm -rf /)'` is text and running it would be inventing a command. That
/// asymmetry is the whole difference between the two quoting rules.
fn expansions(
    word: &pest::iterators::Pair<Rule>,
    scope: &[usize],
    next: &mut usize,
    out: &mut Vec<Simple>,
    reached: Reached,
) {
    for part in word.clone().into_inner() {
        match part.as_rule() {
            Rule::subst | Rule::backtick => nested(part, scope, next, out, reached),
            // A quoted run, or `$(( … ))` whose operands may call out.
            Rule::dquoted | Rule::arith => expansions(&part, scope, next, out, reached),
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
    reached: Reached,
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
            // The substitution runs in order to run the command that holds it,
            // so it is reached exactly as often — and the re-parse starts from
            // `Always`, which is only true relative to the substitution itself.
            reached: reached.and(c.reached),
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
            // ⚠ **Outside quotes a backslash escapes, and dropping it is the
            // point.** `'\''` is how POSIX puts a quote inside a quoted string —
            // close, escaped quote, reopen — and reading the `\'` as two literal
            // characters gave back a word with a backslash where the quote
            // belongs. Every later stage then saw a word nobody wrote. Found by
            // the round-trip probe (memview#833); 274 corpus calls contain a
            // `\'`, most of them `ssh host '…'` payloads whose inner script
            // quotes something, which are exactly the ones naming files
            // elsewhere.
            //
            // A trailing backslash with nothing after it stays as itself: there
            // is no character for it to escape.
            '\\' if i + 1 < chars.len() => {
                out.push(chars[i + 1]);
                i += 2;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}
