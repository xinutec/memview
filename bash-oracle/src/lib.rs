//! The second gate: bash printing its own parse, and us reading it back.
//!
//! Wrapping a command in a function and running `declare -f` on it makes bash
//! render its tree as text without running it — the only view of bash's parse
//! available without execution, and so the one check here that is not ours.
//!
//! What that view can and cannot say is measured by
//! `reader/probes/bash-printer.sh` and stated in `docs/execution-model.md`. Two
//! consequences shape this module: bash prints *words* verbatim, so the
//! comparison is tree against tree rather than text against text; and it deletes
//! comments, so comments are excluded from both sides.
//!
//! ⚠ **Bash is shown the ORIGINAL text, never our print of it.** A gate fed its
//! subject's own output can only confirm self-consistency. While it was, it
//! caught nothing and a misparse of `a |\nb` passed both gates.
//!
//! ⚠ **The wrapper is not containment.** A balanced payload closes the function,
//! runs, and reopens a group for the trailing brace — measured, not reasoned
//! about. What keeps such text out is that this runs **only on commands the
//! parser accepted**, and the accepted language refuses `(`, `)`, `{` and `}`.
//! **That lapses the moment grouping is accepted, and then this needs
//! `sandbox-exec` around it.**

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use reader::syntax::ast::{AndOr, CommandKind, Item, Pipeline, Script};
use reader::syntax::parse::{Refusal, parse};

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
    /// Bash would not render this command at all.
    ///
    /// ⚠ **A defect in every case but one**, and the caller has to tell them
    /// apart with [`bash_warns_of_a_runaway_heredoc`]: a heredoc whose delimiter
    /// never appears takes the rest of the input as its body, and that body eats
    /// the closing brace of the wrapper [`compare`] needs. Such a command is
    /// outside this gate, as a comment is — not evidence of a bad tree.
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

/// Ask bash to re-read each command and compare its parse with ours.
///
/// The input is the original text, and every one of them must be a command this
/// parser accepted — that is what makes the wrapper safe, and what makes the
/// comparison mean anything. One verdict per input, in order.
pub fn compare(commands: &[String]) -> Result<Vec<Verdict>> {
    let mut verdicts = Vec::with_capacity(commands.len());
    for chunk in commands.chunks(BATCH) {
        let printed: Vec<String> = chunk.to_vec();
        // Bash aborts a script at the first syntax error, so one command it
        // will not define costs every later block in the batch. Those come back
        // as `None` — addressed, not shifted — and are retried alone.
        let mut rendered = render(&printed).unwrap_or_else(|_| vec![None; printed.len()]);
        for (slot, one) in rendered.iter_mut().zip(&printed) {
            if slot.is_none() {
                *slot = render(std::slice::from_ref(one))
                    .ok()
                    .and_then(|blocks| blocks.into_iter().next().flatten());
            }
        }
        for (command, block) in chunk.iter().zip(rendered) {
            verdicts.push(judge(command, block.as_deref()));
        }
    }
    Ok(verdicts)
}

fn judge(command: &str, bash_text: Option<&str>) -> Verdict {
    let Some(bash_text) = bash_text else {
        return Verdict::BashRefused;
    };
    let ours = match parse(command) {
        Ok(tree) => tree,
        // The caller promised this was accepted; if it was not, say so rather
        // than score it as agreement.
        Err(refusal) => return Verdict::Unreadable(refusal),
    };
    let theirs = match parse(bash_text) {
        Ok(tree) => tree,
        Err(refusal) => return Verdict::Unreadable(refusal),
    };
    // ⚠ Comments are dropped from BOTH sides. Bash deletes them, so keeping ours
    // would report a difference on every commented command and drown the gate in
    // a known limitation.
    let ours = without_comments(&ours);
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
        items: strip(&script.items),
        span: script.span,
    }
}

/// ⚠ **Recursive, because bash deletes a comment wherever it is.** A comment
/// inside a loop body is dropped by `declare -f` just as a top-level one is, so
/// stripping only the outer list would report a difference on every commented
/// body — a known limitation dressed as a finding.
fn strip(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .filter(|item| matches!(item, Item::List(_)))
        .cloned()
        .map(|mut item| {
            if let Item::List(list) = &mut item {
                strip_and_or(list);
            }
            item
        })
        .collect()
}

fn strip_and_or(list: &mut AndOr) {
    strip_pipeline(&mut list.first);
    for link in &mut list.rest {
        strip_pipeline(&mut link.pipeline);
    }
}

fn strip_pipeline(pipeline: &mut Pipeline) {
    for command in &mut pipeline.commands {
        match &mut command.kind {
            CommandKind::Simple(_) => {}
            CommandKind::For(loop_) => loop_.body = strip(&loop_.body),
            CommandKind::While(loop_) => {
                loop_.condition = strip(&loop_.condition);
                loop_.body = strip(&loop_.body);
            }
        }
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

    // ⚠ **Blocks are placed by the INDEX bash prints, never by position.**
    // The stream was read in order once, and a single command whose definition
    // bash declined left no block — shifting every later one, so a command was
    // judged against its neighbour's parse. That surfaced as exactly one
    // DIFFERENT TREE in 81,623 which agreed when re-run alone: the worst shape
    // of bug this gate can have, because it manufactures a disagreement rather
    // than missing one.
    //
    // `declare -f` names the function it prints, and the driver numbered them,
    // so the answer carries its own address and nothing has to line up.
    let mut blocks: Vec<Option<String>> = vec![None; printed.len()];
    for raw in output.stdout.split(|byte| *byte == SEPARATOR) {
        let text = String::from_utf8_lossy(raw);
        if let Some((index, body)) = body_of(&text)
            && index < blocks.len()
        {
            blocks[index] = Some(body);
        }
    }
    Ok(blocks)
}

/// Does bash refuse this text too?
///
/// ⚠ **Only some refusals are claims about the TEXT rather than about us.**
/// `UnterminatedQuote`, `DanglingEscape` and `EmptyOperand` say the input is not
/// valid shell; every other reason says only that this parser does not model a
/// construct, which bash has no opinion about. So those are the ones to check,
/// and checking them is what keeps "we cannot read it" and "it does not parse"
/// apart — a distinction `docs/execution-model.md` requires and which silently
/// rots otherwise.
///
/// `bash -n` reads and executes nothing.
pub fn bash_also_refuses(command: &str) -> Result<bool> {
    Ok(!bash_parse(command)?.0)
}

/// Does bash warn that a heredoc ran off the end of the input?
///
/// ⚠ **This is what makes `BashRefused` legible.** A heredoc whose delimiter
/// never appears takes the rest of the input as its body — bash accepts it,
/// exits zero, and says so only on stderr — and that runaway body swallows the
/// closing brace of the wrapper [`render`] needs, so bash cannot print the
/// command at all. Such a command is outside the second gate, exactly as a
/// comment is. Every *other* refusal of our printed form is a defect, so the
/// warning is what separates the two rather than a blanket exemption.
pub fn bash_warns_of_a_runaway_heredoc(command: &str) -> Result<bool> {
    Ok(bash_parse(command)?.1.contains("delimited by end-of-file"))
}

/// `bash -n` on the text: whether it accepted, and what it said while doing it.
fn bash_parse(command: &str) -> Result<(bool, String)> {
    let bash = std::env::var("SYNTAX_ORACLE_BASH").unwrap_or_else(|_| "bash".to_string());
    let script = Scratch::write(command.as_bytes())?;
    let output = Command::new(&bash)
        .arg("-n")
        .arg(&script.path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .with_context(|| format!("spawning {bash} -n"))?;
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
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

/// The index and body out of one `declare -f` block: the driver's `__pN__`
/// gives the index, and the body is everything between the `{` and `}` lines.
///
/// Inner indentation is left alone. The parser skips leading blanks, and the
/// indentation of a nested line is bash's statement about structure — reflowing
/// it here would throw away the part of the answer worth having.
fn body_of(text: &str) -> Option<(usize, String)> {
    let lines: Vec<&str> = text.trim_matches('\n').lines().collect();
    if lines.len() < 3 {
        return None;
    }
    if !lines[0].contains("()") || lines[1].trim() != "{" || lines[lines.len() - 1].trim() != "}" {
        return None;
    }
    let index = lines[0]
        .trim()
        .strip_prefix("__p")?
        .split("__")
        .next()?
        .parse()
        .ok()?;
    Some((index, lines[2..lines.len() - 1].join("\n")))
}
