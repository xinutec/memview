//! One `Bash` command, shown as the reader saw it.
//!
//! ⚠ **This module renders a walk; it never performs one.** Every figure here
//! comes out of [`reader::shell_files::trace`], which is the same code path the
//! index is built from — so a command that attributes a file in the artefact
//! attributes it here, for the reason shown here. A second walk written for the
//! view would be free to disagree with the first, and would do it silently: the
//! view would look right and the index would be wrong, or the reverse, with
//! nothing to say which.
//!
//! ⚠ **Nothing in this module runs anything.** The text arrives, is parsed, and
//! is described. That is worth stating because everything it describes is a
//! command that *did* run, and the difference is one careless `Command::new`
//! away.
//!
//! The working directory is not the client's to supply — it comes from the
//! session, because a relative path resolves against it and a caller who could
//! choose it could make this view say anything. Where it is not known, it is
//! `None` and only absolute paths survive, exactly as in the miner.

use serde::{Deserialize, Serialize};

use reader::doing::Verdict;
use reader::shell::Reached;
use reader::shell_files::{FileUse, RemoteUse, Step};
use reader::shell_ops::{GitOp, Op};

/// What the client asks about: the command, and how its call turned out.
#[derive(Debug, Deserialize)]
pub struct Asked {
    pub command: String,
    /// The tool result's own verdict, when the call has returned. `None` while
    /// it is still running — which is a real state and not a synonym for
    /// success, so it is carried rather than guessed.
    #[serde(default)]
    pub ok: Option<bool>,
}

/// The parse, flat, in running order.
///
/// Flat with a `depth` on each line rather than a tree of children, because the
/// one client is a phone: a tree costs a level of indentation per nesting and
/// there is no width to spend on it, while a flat list scrolls.
#[derive(Debug, Serialize)]
pub struct Parsed {
    /// Why the grammar could not read it, when it could not. A parse failure is
    /// shown rather than smoothed over — 0.4% of the corpus's calls fail, and a
    /// view that quietly returned no steps for them would be reporting an empty
    /// command instead of an unread one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub steps: Vec<Line>,
    /// Commands whose operation is not in the table, by name and count. Named
    /// here for the same reason the report names them: it is the honest size of
    /// what this cannot read, and on one command it is usually the answer to
    /// "why did nothing come out".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unread: Vec<Unread>,
    /// Commands that exist because a determinate loop was run out. Shown because
    /// a reader counting lines will otherwise find more steps than they wrote.
    #[serde(skip_serializing_if = "is_zero")]
    pub unrolled: usize,
    /// Scripts inside a wrapper that the grammar could not read — a hole in the
    /// middle of a parse that otherwise succeeded.
    #[serde(skip_serializing_if = "is_zero")]
    pub nested_unparsed: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

#[derive(Debug, Serialize)]
pub struct Unread {
    pub name: String,
    pub count: usize,
}

/// One command, with what was decided about it.
#[derive(Debug, Serialize)]
pub struct Line {
    pub depth: usize,
    /// The machine it ran on, when it was not this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// The words as the shell would have run them — see [`Step::argv`].
    pub argv: Vec<String>,
    /// Whether the words shown differ from the words written, so the view can
    /// say so rather than letting a reader wonder why `$f` became `a.ts`.
    pub reached: &'static str,
    /// The subshells enclosing it, so two sibling `( … )` groups can be told
    /// apart — which is the difference between one working directory and two.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The operation in one word, for a label.
    pub kind: &'static str,
    /// What that operation says that its paths do not — the pattern a search
    /// looked for, the program a transform applied, the name of a command
    /// nobody has taught this yet. Empty when the paths are the whole story.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub says: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<Used>,
}

/// One file a command used, and whether that use is a fact.
#[derive(Debug, Serialize)]
pub struct Used {
    pub path: String,
    pub write: bool,
    /// What the *text* said had to hold.
    pub reached: &'static str,
    /// Whether the text's condition and the call's own outcome together make
    /// this certain — [`Verdict::admits`].
    ///
    /// ⚠ **One-sided, and the view must not round it.** `false` means "cannot
    /// say", never "did not happen". It is the only field here that is not a
    /// property of the command alone, and it is the reason the whole view exists:
    /// a command can parse perfectly, classify correctly, name the right path,
    /// and still attribute nothing.
    pub certain: bool,
    /// The machine it is on, for a use that is not local.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

/// Read one command, against the directory the session is in.
pub fn parsed(asked: &Asked, cwd: Option<&str>, home: &str) -> Parsed {
    let verdict = match asked.ok {
        Some(true) => Verdict::Ok,
        Some(false) => Verdict::Failed,
        None => Verdict::Unknown,
    };
    let commands = match reader::project::read(&asked.command) {
        Ok(commands) => commands,
        // The construct that stopped the read, by name. The flat grammar gave
        // back the text it gave up at; this one knows what it was looking at.
        Err(refusal) => {
            let at = format!("{:?}", refusal.reason);
            return Parsed {
                error: Some(at),
                steps: Vec::new(),
                unread: Vec::new(),
                unrolled: 0,
                nested_unparsed: 0,
            };
        }
    };
    let walk = reader::shell_files::trace(&commands, cwd, home);
    Parsed {
        error: None,
        steps: walk.steps.iter().map(|step| line(step, verdict)).collect(),
        unread: walk
            .unhandled
            .iter()
            .map(|(name, count)| Unread {
                name: name.clone(),
                count: *count,
            })
            .collect(),
        unrolled: walk.unrolled,
        nested_unparsed: walk.nested_unparsed.values().sum(),
    }
}

fn line(step: &Step, verdict: Verdict) -> Line {
    let (kind, says) = described(step.op.as_ref());
    let local = step.files.iter().map(|use_| local_use(use_, verdict));
    let away = step.away.iter().map(away_use);
    Line {
        depth: step.depth,
        host: step.host.clone(),
        argv: step.argv.clone(),
        reached: condition(step.reached),
        scope: step.scope.clone(),
        cwd: step.cwd.clone(),
        kind,
        says,
        uses: local.chain(away).collect(),
    }
}

fn local_use(used: &FileUse, verdict: Verdict) -> Used {
    Used {
        path: used.path.clone(),
        write: used.write,
        reached: condition(used.reached),
        certain: verdict.admits(used.reached),
        host: None,
    }
}

/// ⚠ **A remote use is never `certain`.** The verdict belongs to the local call,
/// and what it says about a command that ran on another machine is nothing:
/// `ssh host 'a && b'` reports one status for the whole payload. Marking these
/// uncertain is not caution, it is the same rule that keeps them out of the
/// local index.
fn away_use(used: &RemoteUse) -> Used {
    Used {
        path: used.path.clone(),
        write: used.write,
        reached: "sometimes",
        certain: false,
        host: Some(used.host.clone()),
    }
}

fn condition(reached: Reached) -> &'static str {
    match reached {
        Reached::Always => "always",
        Reached::OnSuccess => "on-success",
        Reached::Sometimes => "sometimes",
    }
}

/// The operation as a label and a phrase.
///
/// The phrase carries what the paths cannot: `grep hsmmDecode src/x.ts` and
/// `cat src/x.ts` project to the same single read, and the difference between
/// them — that one was looking for something — is the whole reason
/// [`reader::shell_ops`] is a typed operation rather than a path table.
fn described(op: Option<&Op>) -> (&'static str, String) {
    let Some(op) = op else {
        return ("redirect", String::new());
    };
    match op {
        Op::Read { .. } => ("read", String::new()),
        Op::Write { .. } => ("write", String::new()),
        Op::Remove { recursive, .. } => (
            "remove",
            if *recursive { "recursive" } else { "" }.to_string(),
        ),
        Op::Copy { .. } => ("copy", String::new()),
        Op::Move { .. } => ("move", String::new()),
        Op::Search { pattern, .. } => ("search", pattern.clone()),
        Op::Transform {
            program, in_place, ..
        } => (
            "transform",
            if *in_place {
                format!("{program} — in place")
            } else {
                program.clone()
            },
        ),
        // ⚠ **The script is not repeated as a phrase.** It is already the one
        // file a `Run` projects to, so saying it twice costs two lines of a
        // 412px screen to tell a reader the same absolute path they are looking
        // at. Found by reading the runner's own output for a real command.
        Op::Run { .. } => ("run", String::new()),
        // The script itself is not repeated: its commands are the steps below
        // this one, which is a better answer than the text they came from.
        Op::Nested { .. } => ("shell", String::new()),
        Op::Python { .. } => ("python", String::new()),
        Op::Remote { host, .. } => ("remote", host.clone()),
        Op::ChangeDir { to } => (
            "cd",
            to.clone()
                .unwrap_or_else(|| "somewhere this reader cannot follow".to_string()),
        ),
        Op::Git(git) => (
            "git",
            match git {
                GitOp::Stage { .. } => "stage — changes no file".to_string(),
                GitOp::Alter { .. } => "alter".to_string(),
                GitOp::Inspect { .. } => "inspect".to_string(),
                GitOp::Other { subcommand } => subcommand.clone(),
            },
        ),
        Op::Nothing => ("nothing", String::new()),
        Op::Unknown { name } => ("unknown", name.clone()),
    }
}
