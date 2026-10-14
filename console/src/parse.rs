//! One `Bash` command, shown as the reader saw it.
//!
//! Renders [`reader::shell_files::trace`], the path the index is built from, so
//! the two cannot disagree. Runs nothing.

use serde::{Deserialize, Serialize};

use reader::doing::Verdict;
use reader::shell::Reached;
use reader::shell_files::{FileUse, RemoteUse, Step};
use reader::shell_ops::{GitOp, Op};

/// What the client asks about: the command, and how its call turned out.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Asked {
    pub command: String,
    /// The tool result's verdict; `None` while still running.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub ok: Option<bool>,
}

/// The parse, flat, in running order, with a `depth` per line — a phone has no
/// width for a tree.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Parsed {
    /// Why the grammar could not read it, if it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub error: Option<String>,
    pub steps: Vec<Line>,
    /// Commands whose operation is not in the table, by name and count.
    pub unread: Vec<Unread>,
    /// Commands that exist because a determinate loop was run out.
    pub unrolled: usize,
    /// Scripts inside a wrapper that the grammar could not read.
    pub nested_unparsed: usize,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Unread {
    pub name: String,
    pub count: usize,
}

/// Whether a step runs: the reader's [`Reached`], in the sheet's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Reach {
    Always,
    OnSuccess,
    Sometimes,
}

impl From<Reached> for Reach {
    fn from(reached: Reached) -> Self {
        match reached {
            Reached::Always => Self::Always,
            Reached::OnSuccess => Self::OnSuccess,
            Reached::Sometimes => Self::Sometimes,
        }
    }
}

/// One command, with what was decided about it.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Line {
    pub depth: usize,
    /// The machine it ran on, when it was not this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub host: Option<String>,
    /// The words as the shell would have run them — see [`Step::argv`].
    pub argv: Vec<String>,
    /// Whether the words shown differ from the words written.
    pub reached: Reach,
    /// The subshells enclosing it, telling sibling `( … )` groups apart.
    pub scope: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub cwd: Option<String>,
    /// What the command was for, as a sentence, when a lens can say —
    /// [`reader::concept::describe`].
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub concept: Option<String>,
    /// The operation, in one or two words, for the chip.
    pub kind: &'static str,
    /// The stable key behind that chip, for styling.
    pub key: &'static str,
    /// What the operation says beyond its paths, such as a search's pattern.
    pub says: String,
    pub uses: Vec<Used>,
}

/// One file a command used, and whether that use is a fact.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Used {
    pub path: String,
    pub write: bool,
    /// What the *text* said had to hold.
    pub reached: Reach,
    /// Whether the condition and the outcome make this certain — [`Verdict::admits`].
    /// `false` means cannot say, never did not happen.
    pub certain: bool,
    /// The machine it is on, for a use that is not local.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
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
        // The phrase a person reads, not a Rust identifier.
        Err(refusal) => {
            let at = refusal.reason.label().to_string();
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
    let (naming, says) = described(step.op.as_ref());
    let local = step.files.iter().map(|use_| local_use(use_, verdict));
    let away = step.away.iter().map(away_use);
    Line {
        depth: step.depth,
        host: step.host.clone(),
        argv: step.argv.clone(),
        reached: step.reached.into(),
        scope: step.scope.clone(),
        cwd: step.cwd.clone(),
        // The refusal is census material, not for a card.
        concept: reader::concept::lift(step)
            .ok()
            .map(|concept| reader::concept::describe(&concept)),
        kind: naming.chip,
        key: naming.key,
        says,
        uses: local.chain(away).collect(),
    }
}

fn local_use(used: &FileUse, verdict: Verdict) -> Used {
    Used {
        path: used.path.clone(),
        write: used.write,
        reached: used.reached.into(),
        certain: verdict.admits(used.reached),
        host: None,
    }
}

/// A remote use is never `certain`: one status covers the whole payload.
fn away_use(used: &RemoteUse) -> Used {
    Used {
        path: used.path.clone(),
        write: used.write,
        reached: Reach::Sometimes,
        certain: false,
        host: Some(used.host.clone()),
    }
}

/// The operation as a label and a phrase saying what the paths cannot.
fn described(op: Option<&Op>) -> (reader::reading::Naming, String) {
    let Some(op) = op else {
        return (
            reader::reading::Naming {
                key: "redirect",
                chip: "redirect",
                phrase: "redirect",
            },
            String::new(),
        );
    };
    // The words come from `reader::reading::naming`, shared with the viewer.
    let naming = reader::reading::naming(op);
    let says = match op {
        Op::Remove { recursive, .. } => if *recursive { "recursive" } else { "" }.to_string(),
        Op::Search { pattern, .. } => pattern.clone(),
        Op::Transform {
            program, in_place, ..
        } => {
            if *in_place {
                format!("{program} — in place")
            } else {
                program.clone()
            }
        }
        // The script is already its one file.
        Op::Run { .. } => String::new(),
        // What it runs, which is the whole of what is known.
        Op::Opaque { name } => name.clone(),
        // Its commands are the steps below.
        Op::Nested { .. } | Op::Python { .. } | Op::JavaScript { .. } => String::new(),
        // Names the tables.
        Op::Sql { source, .. } => {
            let queried = reader::sql::read(source);
            let mut named: Vec<&str> = queried
                .writes
                .keys()
                .chain(queried.reads.keys())
                .map(String::as_str)
                .collect();
            named.dedup();
            named.join(", ")
        }
        // One line for both payload shapes.
        Op::Remote { host, .. } | Op::RemoteRun { host, .. } => host.clone(),
        Op::ChangeDir { to } => to
            .clone()
            .unwrap_or_else(|| "somewhere this reader cannot follow".to_string()),
        Op::Git(git) => match git {
            GitOp::Stage { .. } => "stage — changes no file".to_string(),
            GitOp::Alter { .. } => "alter".to_string(),
            GitOp::Inspect { .. } => "inspect".to_string(),
            GitOp::Other { subcommand } => subcommand.clone(),
        },
        // Its name is all that is known.
        Op::Unknown { name } => name.clone(),
        Op::Read { .. } | Op::Write { .. } | Op::Copy { .. } | Op::Move { .. } | Op::Nothing => {
            String::new()
        }
    };
    (naming, says)
}
