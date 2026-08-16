//! The second gate: bash printing its own parse, and us reading it back.
//!
//! Wrapping a command in a function and running `declare -f` on it makes bash
//! render its tree as text without running anything. That is the only view of
//! bash's parse available without execution, so it is the one independent check
//! this design has.
//!
//! **What it can and cannot say**, measured by `reader/probes/bash-printer.sh`
//! against bash 5.3:
//!
//! - it is a fixpoint, and it reproduces heredoc bodies verbatim;
//! - it prints *words* exactly as written, so it says nothing about what is
//!   inside one. An unimplemented expansion absorbed into a literal is printed
//!   back and absorbed again, and this gate agrees with the mistake;
//! - it normalises and desugars *structure*, and that is where it is worth
//!   having: `ls |& cat` comes back `ls 2>&1 | cat`, `! time a | b` comes back
//!   `time ! a | b`, and a compound is laid out with `do` on its own line;
//! - it deletes comments, so comments are excluded from the comparison here and
//!   covered by the round-trip law alone.
//!
//! So the comparison is tree against tree, never text against text: bash keeps
//! the spelling this printer normalises away, and comparing the two spellings
//! would report a difference on every quoted word in the corpus.
//!
//! ⚠ **What is fed to bash is this printer's output, never raw corpus text.**
//! That is a safety property, not a convenience. The function wrapper holds only
//! because bash parses a whole definition before running any of it, and a
//! balanced payload defeats that — `echo a; }; rm -rf x; { echo b` closes the
//! function, runs, and reopens a group for the trailing brace to close. Corpus
//! history is full of braces by accident. The printer cannot emit an unquoted
//! `{` or `}` — they are absent from its bare-safe set — so nothing it writes
//! can close the wrapper. **When the grammar grows to accept grouping, that
//! argument lapses and this needs `sandbox-exec` around it.**

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use reader::syntax::ast::{Item, Script};
use reader::syntax::parse::{Refusal, parse};
use reader::syntax::print::print;

/// NUL, which bash cannot put in a word and therefore cannot appear inside
/// `declare -f` output. A printable marker could be produced by a quoted literal
/// in the corpus and would split the stream in the wrong place.
const SEPARATOR: u8 = 0;

/// How many commands go to one bash process. Large enough that process startup
/// stops dominating a corpus run, small enough that a fallback re-run is cheap.
const BATCH: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Bash read our printed form the same way we do.
    Agrees,
    /// Bash refused to parse what the printer wrote. Always a defect here.
    BashRefused,
    /// Bash's print does not read back — the parser cannot read bash's spelling
    /// of a tree it produced itself.
    Unreadable(Refusal),
    /// Two different trees, which is the finding this gate exists for.
    Differs { ours: String, bash: String },
}

impl Verdict {
    pub fn agrees(&self) -> bool {
        matches!(self, Verdict::Agrees)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Agrees => "agrees",
            Verdict::BashRefused => "bash refused our print",
            Verdict::Unreadable(_) => "bash's print does not read back",
            Verdict::Differs { .. } => "DIFFERENT TREE",
        }
    }
}

/// Ask bash to re-read each script's printed form, and compare what it says.
///
/// One verdict per input, in order.
pub fn compare(scripts: &[Script]) -> Result<Vec<Verdict>> {
    let mut verdicts = Vec::with_capacity(scripts.len());
    for chunk in scripts.chunks(BATCH) {
        let printed: Vec<String> = chunk.iter().map(print).collect();
        let rendered = match render(&printed) {
            // A batch is all-or-nothing: bash aborts a script at the first
            // syntax error, so one refusal truncates the stream and every
            // command after it would be misattributed. Re-run the batch one at
            // a time to find out which.
            Ok(blocks) if blocks.len() == printed.len() => blocks,
            _ => printed
                .iter()
                .map(|one| {
                    render(std::slice::from_ref(one))
                        .ok()
                        .and_then(|mut blocks| blocks.pop().flatten())
                })
                .collect(),
        };
        for (script, block) in chunk.iter().zip(rendered) {
            verdicts.push(judge(script, block.as_deref()));
        }
    }
    Ok(verdicts)
}

fn judge(ours: &Script, bash_text: Option<&str>) -> Verdict {
    let Some(bash_text) = bash_text else {
        return Verdict::BashRefused;
    };
    let theirs = match parse(bash_text) {
        Ok(tree) => tree,
        Err(refusal) => return Verdict::Unreadable(refusal),
    };
    // ⚠ Comments are dropped from BOTH sides. Bash deletes them, so keeping ours
    // would report a difference on every commented command and drown the gate in
    // a known limitation.
    let ours = without_comments(ours);
    let theirs = without_comments(&theirs);
    if ours == theirs {
        Verdict::Agrees
    } else {
        Verdict::Differs {
            ours: format!("{ours:?}"),
            bash: format!("{theirs:?}"),
        }
    }
}

fn without_comments(script: &Script) -> Script {
    Script {
        items: script
            .items
            .iter()
            .filter(|item| matches!(item, Item::Pipeline(_)))
            .cloned()
            .collect(),
        span: script.span,
    }
}

/// Run one bash process over a batch, returning its print of each input.
///
/// `None` for an input bash would not parse — reachable only from the one-at-a-
/// time fallback, since a refusal in a batch truncates the whole stream.
fn render(printed: &[String]) -> Result<Vec<Option<String>>> {
    let mut driver = String::new();
    for (index, text) in printed.iter().enumerate() {
        // The body is written on its own lines so that a `#` comment in it
        // cannot swallow the closing brace.
        driver.push_str(&format!("__p{index}__() {{\n{text}\n}}\n"));
        driver.push_str(&format!("declare -f __p{index}__\n"));
        driver.push_str("printf '\\0'\n");
    }

    // ⚠ **The driver goes in a file, not down a pipe.** Writing a batch to
    // bash's stdin while nothing drains its stdout deadlocks as soon as the
    // output passes the pipe buffer — measured at 500 commands, where it hangs
    // rather than failing. A file has no such coupling and costs one write.
    let script = Scratch::write(driver.as_bytes())?;

    let bash = std::env::var("SYNTAX_ORACLE_BASH").unwrap_or_else(|_| "bash".to_string());
    let output = Command::new(&bash)
        .arg(&script.path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .with_context(|| format!("spawning {bash}"))?;

    let mut blocks: Vec<Option<String>> = Vec::with_capacity(printed.len());
    for raw in output.stdout.split(|byte| *byte == SEPARATOR) {
        if blocks.len() == printed.len() {
            break;
        }
        let text = String::from_utf8_lossy(raw);
        match body_of(&text) {
            Some(body) => blocks.push(Some(body)),
            // The trailing piece after the last separator is empty and is not a
            // block; anything else empty means bash printed no function.
            None if text.trim().is_empty() => {}
            None => blocks.push(None),
        }
    }
    Ok(blocks)
}

/// Does bash refuse this text too?
///
/// ⚠ **Only two refusals are claims about the TEXT rather than about us.**
/// `UnterminatedQuote` and `DanglingEscape` say the input is not valid shell;
/// every other reason says only that this parser does not model a construct,
/// which bash has no opinion about. So those two are the ones worth checking,
/// and checking them is what keeps "we cannot read it" and "it does not parse"
/// apart — a distinction `docs/execution-model.md` requires and which silently
/// rots otherwise.
///
/// `bash -n` reads and executes nothing.
pub fn bash_also_refuses(command: &str) -> Result<bool> {
    let bash = std::env::var("SYNTAX_ORACLE_BASH").unwrap_or_else(|_| "bash".to_string());
    let script = Scratch::write(command.as_bytes())?;
    let status = Command::new(&bash)
        .arg("-n")
        .arg(&script.path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("spawning {bash} -n"))?;
    Ok(!status.success())
}

/// A driver file that removes itself.
///
/// Hand-rolled rather than pulled from `tempfile`, because this crate's whole
/// justification is being the one place that touches the outside — a dependency
/// here should buy more than eight lines.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn write(bytes: &[u8]) -> Result<Self> {
        // The pid keeps two concurrent runs apart; the counter keeps one run's
        // batches apart. Neither needs to be unpredictable — nothing else reads
        // this file, and it lives for one `bash` invocation.
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "bash-oracle-{}-{}.sh",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file =
            std::fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
        file.write_all(bytes)?;
        file.flush()?;
        Ok(Self { path })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The function body out of `declare -f` output: drop the `name ()` and `{`
/// lines and the closing `}`.
///
/// Inner indentation is left alone. The parser skips leading blanks, and the
/// indentation of a nested line is bash's statement about structure — reflowing
/// it here would throw away the part of the answer worth having.
fn body_of(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.trim_matches('\n').lines().collect();
    if lines.len() < 3 {
        return None;
    }
    if !lines[0].contains("()") || lines[1].trim() != "{" || lines[lines.len() - 1].trim() != "}" {
        return None;
    }
    Some(lines[2..lines.len() - 1].join("\n"))
}
