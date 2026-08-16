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
//! about. It used to be harmless only because the accepted language refused
//! `(`, `)`, `{` and `}`, which is an argument that expires the moment grouping
//! is built. The bash that renders now runs under [`SANDBOX_PROFILE`], and where
//! there is no sandbox [`renderable`] applies the old guarantee explicitly
//! rather than assuming it still holds.

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
    /// Not shown to bash at all, because this machine has no sandbox and the
    /// text could break out of the wrapper. See [`renderable`].
    NotSandboxed,
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
            Verdict::NotSandboxed => "not shown to bash (no sandbox here)",
        }
    }
}

/// The profile gate 2's bash runs under.
///
/// ⚠ **The wrapper does not contain the command, and never did.** `eval` parses
/// its whole argument before running any of it, so a balanced payload —
/// `echo a; }; touch X; { echo b` — closes the function, runs, and reopens a
/// group for the trailing brace. `reader/probes/bash-printer.sh` demonstrates it
/// rather than arguing it.
///
/// Three denials, each measured against that payload:
///
/// - **`process-fork`** stops anything with a child: `touch X`, `$(…)`, a pipe.
///   `declare -f` needs none, so the gate is unaffected. Denying `process-exec*`
///   instead would stop `sandbox-exec` launching bash at all.
/// - **`file-write*`** stops the redirection forms, which need no child:
///   `: > X` is refused with "Operation not permitted".
/// - **`network*`** stops the one exfiltration route that needs no child either
///   — bash opens a socket with `exec 3<>/dev/tcp/host/port` on its own.
///
/// What a payload can still do is print, which is why blocks are located by the
/// index bash names rather than by position in the stream.
const SANDBOX_PROFILE: &str = concat!(
    "(version 1)(allow default)",
    "(deny process-fork)(deny network*)(deny file-write*)",
    r#"(allow file-write-data (literal "/dev/null") (literal "/dev/stdout") (literal "/dev/stderr"))"#
);

/// Is this machine able to run gate 2 safely?
fn sandbox_available() -> bool {
    cfg!(target_os = "macos") && std::path::Path::new(SANDBOX).exists()
}

const SANDBOX: &str = "/usr/bin/sandbox-exec";

/// May this text be shown to bash?
///
/// ⚠ **The containment argument, made explicit and enforced.** It used to hold
/// implicitly: the gate ran only on accepted commands, and the accepted language
/// refused `(`, `)`, `{` and `}`, so nothing could close the wrapper. Building
/// grouping dissolves that argument, so the check moved here — where a machine
/// with no sandbox falls back to exactly the old guarantee instead of inheriting
/// a safety property that has quietly expired.
///
/// The cost is that a Linux CI runner skips the grouping commands rather than
/// running them unprotected, and says how many it skipped.
pub fn renderable(text: &str) -> bool {
    sandbox_available() || !text.contains(['(', ')', '{', '}'])
}

/// Ask bash to re-read each command and compare its parse with ours.
///
/// The input is the original text, and every one of them must be a command this
/// parser accepted — that is what makes the wrapper safe, and what makes the
/// comparison mean anything. One verdict per input, in order.
pub fn compare(commands: &[String]) -> Result<Vec<Verdict>> {
    let mut verdicts = Vec::with_capacity(commands.len());
    for chunk in commands.chunks(BATCH) {
        // ⚠ Text that could close the wrapper is replaced by a harmless
        // placeholder rather than dropped, so every later block keeps the index
        // bash will name it by. Its verdict is filled in below.
        let printed: Vec<String> = chunk
            .iter()
            .map(|text| {
                if renderable(text) {
                    text.clone()
                } else {
                    ":".to_string()
                }
            })
            .collect();
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
            verdicts.push(if renderable(command) {
                judge(command, block.as_deref())
            } else {
                Verdict::NotSandboxed
            });
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
            CommandKind::If(conditional) => {
                conditional.condition = strip(&conditional.condition);
                conditional.then = strip(&conditional.then);
                conditional.otherwise = conditional.otherwise.as_deref().map(strip);
            }
            CommandKind::Subshell(items) | CommandKind::Group(items) => *items = strip(items),
            CommandKind::Function(function) => function.body = strip(&function.body),
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

    // ⚠ **This is the one call that RUNS corpus text**, so it is the one that
    // has to be contained. See [`SANDBOX_PROFILE`]; `bash -n` elsewhere in this
    // file executes nothing and needs none of it.
    let output = sandboxed(&script.path)?
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .context("spawning bash to render a batch")?;

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

/// Is our printed form shell at all?
///
/// ⚠ **The third gate, and it exists because the first two are blind to this.**
/// Gate 1 re-reads `t₂` with *this* parser, which is more permissive than bash
/// in places; gate 2 is shown the ORIGINAL command by design, so it never looks
/// at what we print. Between them sits a question neither asks: is `t₂` valid?
/// It is not hypothetical — the printer wrote `do b & ; done` for every compound
/// whose body ended in a `&`, which bash refuses, and the loop tests asserted
/// the round-trip law over exactly that text while it held.
///
/// Distinct from gate 2's `BashRefused`, which is about the original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Validity {
    Parses,
    /// Bash refused our print, with what it said about it.
    Refused(String),
}

/// Run `bash -n` over each printed form, in batches.
///
/// ⚠ **Safe to batch and safe to wrap, for one reason: `-n` executes nothing.**
/// Gate 2 must worry about a balanced payload closing its wrapper and running;
/// here the worst a payload can do is parse. That is also why this gate can be
/// applied to text gate 2 must never touch.
///
/// The wrapper is per command so that one text cannot bleed into the next. A
/// runaway heredoc would swallow the closing brace, as it does in [`render`] —
/// but it cannot arise here, because the printer always writes the terminator
/// back. That is a property of `t₂` the original text does not have.
pub fn validity(printed: &[String]) -> Result<Vec<Validity>> {
    let mut out = Vec::with_capacity(printed.len());
    for batch in printed.chunks(BATCH) {
        let mut driver = String::new();
        for (index, text) in batch.iter().enumerate() {
            driver.push_str(&format!("__v{index}__() {{\n{text}\n}}\n"));
        }
        // One process for the whole batch while everything parses, which is the
        // common case by far. A batch that fails says nothing about WHICH member
        // failed, so it is re-asked one at a time — the same shape [`compare`]
        // uses, and for the same reason.
        if bash_parse(&driver)?.0 {
            out.extend(std::iter::repeat_n(Validity::Parses, batch.len()));
            continue;
        }
        for text in batch {
            let wrapped = format!("__v__() {{\n{text}\n}}\n");
            let (ok, said) = bash_parse(&wrapped)?;
            out.push(if ok {
                Validity::Parses
            } else {
                Validity::Refused(said.trim().to_string())
            });
        }
    }
    Ok(out)
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

/// The command that runs `script` under the sandbox, where there is one.
///
/// ⚠ **Not a fallback to "run it anyway".** Where no sandbox exists, this still
/// runs bash — but [`renderable`] has already replaced every text that could
/// escape the wrapper, so what reaches an unsandboxed bash is only what the old
/// containment argument already covered.
fn sandboxed(script: &std::path::Path) -> Result<Command> {
    let bash = std::env::var("SYNTAX_ORACLE_BASH").unwrap_or_else(|_| "bash".to_string());
    if !sandbox_available() {
        let mut command = Command::new(&bash);
        command.arg(script);
        return Ok(command);
    }
    let mut command = Command::new(SANDBOX);
    command
        .arg("-p")
        .arg(SANDBOX_PROFILE)
        .arg(&bash)
        .arg(script);
    Ok(command)
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
