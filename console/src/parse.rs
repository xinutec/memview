//! One `Bash` command, shown as the reader saw it.
//!
//! This module renders a walk and never performs one: every figure comes out of
//! [`reader::shell_files::trace`], the same path the index is built from, so the
//! view and the index cannot disagree. Nothing here runs anything.
//!
//! The working directory comes from the session, not the client: a caller who
//! could choose it could make this view say anything. Unknown, only absolute
//! paths survive, as in the miner.

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
    /// The tool result's own verdict, when the call has returned. `None` while it is
    /// still running — a real state, not a synonym for success.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub ok: Option<bool>,
}

/// The parse, flat, in running order, with a `depth` per line: the one client is
/// a phone, and a tree costs indentation there is no width for.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Parsed {
    /// Why the grammar could not read it, when it could not. Shown rather than
    /// smoothed over: an empty command and an unread one are different reports.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub error: Option<String>,
    pub steps: Vec<Line>,
    /// Commands whose operation is not in the table, by name and count — on one
    /// command, usually the answer to "why did nothing come out".
    pub unread: Vec<Unread>,
    /// Commands that exist because a determinate loop was run out. Shown because
    /// a reader counting lines will otherwise find more steps than they wrote.
    pub unrolled: usize,
    /// Scripts inside a wrapper that the grammar could not read — a hole in the
    /// middle of a parse that otherwise succeeded.
    pub nested_unparsed: usize,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Unread {
    pub name: String,
    pub count: usize,
}

/// Whether a step runs, as the wire says it: the reader's [`Reached`] under the
/// words the sheet prints, not the one-letter form its index files use.
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
    /// Whether the words shown differ from the words written, so the view can
    /// say so rather than letting a reader wonder why `$f` became `a.ts`.
    pub reached: Reach,
    /// The subshells enclosing it, so two sibling `( … )` groups can be told
    /// apart — which is the difference between one working directory and two.
    pub scope: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub cwd: Option<String>,
    /// What the command was FOR, as a sentence — the L4 concept, when a lens can say
    /// ([`reader::concept::describe`], `docs/concept-model.md`).
    ///
    /// Absent is the honest miss: a command no lens covers stays a counted leaf with
    /// the chip and `says` carrying the L2/L3 reading, rather than a catch-all. Its
    /// unit is the ROW a person approves, not the step; the two rates differ fivefold.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub concept: Option<String>,
    /// The operation, in one or two words, for the chip.
    pub kind: &'static str,
    /// The stable key behind that chip, for styling. Separate from `kind` so the
    /// wording is free to change without dropping the colour.
    pub key: &'static str,
    /// What that operation says that its paths do not — the pattern a search
    /// looked for, the program a transform applied, the name of a command
    /// nobody has taught this yet. Empty when the paths are the whole story.
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
    /// Whether the text's condition and the call's outcome together make this
    /// certain — [`Verdict::admits`]. One-sided: `false` means "cannot say", never
    /// "did not happen". It is the reason the view exists — a command can parse, name
    /// the right path, and still attribute nothing.
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
        // The construct, in the words the label was written in: `{:?}` would spell a
        // Rust identifier, and `Reason::label` is the phrase a person reads.
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
        // `.ok()` and no more: the refusal (`concept::Why`) is census material, and on a
        // card it would explain the reader rather than the command.
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

/// A remote use is never `certain`: `ssh host 'a && b'` reports one status for
/// the whole payload. The same rule keeps them out of the local index.
fn away_use(used: &RemoteUse) -> Used {
    Used {
        path: used.path.clone(),
        write: used.write,
        reached: Reach::Sometimes,
        certain: false,
        host: Some(used.host.clone()),
    }
}

/// The operation as a label and a phrase. The phrase carries what the paths
/// cannot: `grep x f` and `cat f` project to the same read, and that one was
/// looking for something is why [`reader::shell_ops`] is a typed operation.
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
    // The words come from `reader::reading::naming`: one table shared with the
    // viewer, because two exhaustive matches over one enum can still disagree.
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
        // The script is not repeated as a phrase: it is already the one file a `Run`
        // projects to, and a 412px screen has no room to say it twice.
        Op::Run { .. } => String::new(),
        // The script itself is not repeated: its commands are the steps below
        // this one, which is a better answer than the text they came from.
        Op::Nested { .. } | Op::Python { .. } | Op::JavaScript { .. } => String::new(),
        // Names the TABLES: for every other verb the subject is a path, and "sql" alone
        // would leave the one interesting fact off the screen.
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
        // The same line for both payload shapes: whether the far side had a shell is a
        // fact about how the payload was READ, not about what happened.
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
        // The command's own name is the whole of what is known about it, and it
        // is what somebody would teach the table next.
        Op::Unknown { name } => name.clone(),
        Op::Read { .. } | Op::Write { .. } | Op::Copy { .. } | Op::Move { .. } | Op::Nothing => {
            String::new()
        }
    };
    (naming, says)
}
