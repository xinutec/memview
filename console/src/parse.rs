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
    /// The operation, in one or two words, for the chip.
    pub kind: &'static str,
    /// The stable key behind that chip, for styling.
    ///
    /// ⚠ **Separate from `kind` so the wording is free to change.** The chip's
    /// colour selects on this; while the two were one field, improving a label
    /// silently dropped its colour.
    pub key: &'static str,
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
        // ⚠ **The construct, in the words the label was written in.** The flat
        // grammar handed back the TEXT it gave up at, and the sheet's sentence
        // was built around that — "stopped at `>/dev/tcp/…`". This reader knows
        // something better, the construct it was looking at, but `{:?}` on it
        // spells a Rust identifier: "stopped at `Grouping`". `Reason::label` is
        // the phrase a person reads, and the sheet's wording now fits it.
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
        reached: condition(step.reached),
        scope: step.scope.clone(),
        cwd: step.cwd.clone(),
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
    // ⚠ **The words come from `reader::reading::naming`, not from here.** This
    // function decides only what a step SAYS beyond its paths; what the
    // operation is CALLED is one table shared with the viewer, because two
    // exhaustive matches over one enum both compile and can still disagree.
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
        // ⚠ **The script is not repeated as a phrase.** It is already the one
        // file a `Run` projects to, so saying it twice costs two lines of a
        // 412px screen to tell a reader the same absolute path they are looking
        // at. Found by reading the runner's own output for a real command.
        Op::Run { .. } => String::new(),
        // The script itself is not repeated: its commands are the steps below
        // this one, which is a better answer than the text they came from.
        Op::Nested { .. } | Op::Python { .. } | Op::JavaScript { .. } => String::new(),
        // ⚠ **Names the TABLES, not the statements.** The step's subject line is
        // what the command acted on, and for every other verb that is a path;
        // for this one it is a table, which is the only place in this sheet a
        // subject is not a file. Saying "sql" and nothing else would leave the
        // one interesting fact — which table — off the screen.
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
        // The same line for both payload shapes: a reader watching a step wants
        // the machine, and whether the far side had a shell is a fact about how
        // the payload was READ, not about what happened.
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
