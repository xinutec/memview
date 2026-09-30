//! What a command will leave in the files it writes, predicted from its text and
//! the state it is given — see `docs/execution-model.md`, "Two settings, one
//! evaluator".
//!
//! A pure function. It opens nothing and runs nothing: the text of each file it
//! depends on is asked of a [`Sight`] when the command that needs it is reached.
//! From history nothing is shown, and only what the text alone determines is
//! predicted; before a live call the console answers from disk and keeps what
//! it showed, so the prediction can be made again. [`needs`] lists what a run
//! asked for and was not shown.
//!
//! **Straight-line shell, in order.** Top-level commands run one after another,
//! and a write replaces or extends what the file held at that point. A `for`
//! over words the text spells out runs its body once per word; a brace group is
//! its commands; a subshell is the same with its `cd` and its variables kept
//! inside; each member of a pipeline is a subshell of its own, reading the pipe
//! as its stdin. A variable is known while the text bound it to a literal and
//! nothing since could have changed it, `$HOME` and `$PWD` are known, and a
//! prefix or suffix strip with a spelled-out pattern is computed. What this
//! cannot follow — a program whose output is not modelled, a branch,
//! a word with an expansion in it — yields no prediction for the files it writes,
//! and an [`Unfollowed`] that names why. A file it cannot follow is forgotten from
//! then on, so a later append to it is not guessed at either. A program that
//! writes files itself — `sed -i`, `cp`, `rm` — has them named by the shell
//! tables and refused, so no write it knows of goes unmentioned. One the tables
//! do not know may have written anything, and nothing known survives it — unless
//! it is assumed not to have: a second run makes that assumption, and each file
//! only it predicts is a [`Conditional`] naming the programs assumed. A
//! script handed to another shell — `bash -c`, `nix-shell --run` — is followed
//! in place.
//!
//! **The prediction assumes each command succeeds.** A write after `||` is only
//! sometimes made, and is not followed. Whether the call really went that way is
//! for the check after it to say. A test is a question, not a command that
//! might fail: one sight answers steers `&&`, `||` and `if` exactly, and after
//! one it cannot, what follows is only sometimes run. What a word runs as it
//! expands (`$( )`) runs before its command, followed as a subshell.

use std::collections::{BTreeMap, BTreeSet};

mod python;
mod python_re;
pub mod sed;

use crate::shell::Reached;
use crate::shell_files::files_of;
use crate::shell_ops::{GitOp, Op, basename, classify, innermost, looks_like_path, resolve};
use crate::syntax::ast::{
    AndOr, BinaryTest, Command, CommandKind, Connector, Glob, Item, Parameter, ParameterOp,
    Pipeline, Redirect, RedirectOp, RedirectTarget, Script, Segment, SegmentKind, Simple, TestExpr,
    Tilde, UnaryTest, Word,
};
use crate::syntax::embed::{Program, python_of};
use crate::syntax::print::print_value;

/// What is known of the files a prediction may depend on, by absolute path:
/// `Some(text)`, or `None` for a file that does not exist. A path with no entry is
/// not known at all.
pub type Files = BTreeMap<String, Option<String>>;

/// What is known of directories, by absolute path: the names in one, in no
/// order, or `None` for one that does not exist.
pub type Dirs = BTreeMap<String, Option<Vec<String>>>;

/// What the evaluator may be shown, asked for when a command needs it. It reads
/// and never runs — `docs/execution-model.md`, "Sight". This crate has no
/// implementation that touches a disk: a [`Files`] answers from what it holds,
/// which is how history (nothing) and a test (a fixture) are shown.
pub trait Sight {
    /// `Some(Some(text))`, `Some(None)` for a file that does not exist, or `None`
    /// when the file cannot be shown.
    fn file(&self, path: &str) -> Option<Option<String>>;

    /// The names in a directory, in no order: `Some(None)` for one that does not
    /// exist, or `None` when it cannot be shown.
    fn dir(&self, _path: &str) -> Option<Option<Vec<String>>> {
        None
    }
}

impl Sight for Files {
    fn file(&self, path: &str) -> Option<Option<String>> {
        self.get(path).cloned()
    }
}

/// Files and directories as they were shown: a replay's inputs, or a test's.
#[derive(Debug, Default, Clone)]
pub struct Shown {
    pub files: Files,
    pub dirs: Dirs,
}

impl Sight for Shown {
    fn file(&self, path: &str) -> Option<Option<String>> {
        self.files.get(path).cloned()
    }

    fn dir(&self, path: &str) -> Option<Option<Vec<String>>> {
        self.dirs.get(path).cloned()
    }
}

/// A file as it will be after the command: its text, or `None` for a file the
/// command removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub path: String,
    pub text: Option<String>,
}

/// A write this could not follow, and why. `path` is absent when the target
/// itself could not be named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unfollowed {
    pub path: Option<String>,
    pub why: Why,
}

/// Why a write was not followed. **Each names what would have to be built** — the
/// census over these is the order the evaluator grows in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Why {
    /// A program whose output is not modelled, by name.
    Program(String),
    /// An option of a modelled program that is not: `echo -e`, `printf '%s'`.
    Option(String),
    /// A word or heredoc body whose value is not in the text.
    Expansion,
    /// A write that depends on a file's text, which was not given.
    NotRead,
    /// A file read that does not exist.
    Missing,
    /// A pipeline member reading the pipe, or a path two members both change.
    Pipeline,
    /// A write inside a branch, a `while`, a function, or a loop over words the
    /// text does not spell out.
    Compound,
    /// Only run when something before it failed: after `||`.
    Sometimes,
    /// Run in the background, so not in order.
    Background,
    /// A descriptor other than stdout into the file, whose text is not modelled.
    Descriptor,
    /// A relative path after a `cd` this could not follow.
    Directory,
    /// A Python construct the evaluator does not follow, by name.
    Python(String),
    /// A sed flag, command or pattern it does not follow, by name.
    Sed(String),
    /// A file an undecided `if` left one of several texts, read for its text.
    Branches,
}

impl Why {
    /// The reason as the census names it; a program keeps its name, since which
    /// one is the worklist.
    pub fn census_name(&self) -> String {
        match self {
            Why::Program(program) => format!("program {program}"),
            Why::Option(option) => format!("option {option}"),
            Why::Python(construct) => format!("python {construct}"),
            Why::Sed(construct) => format!("sed {construct}"),
            Why::Branches => "one of several texts".to_string(),
            other => format!("{other:?}").to_lowercase(),
        }
    }
}

/// The unknown programs a conditional prediction assumes left its file alone,
/// by name, each once: those run before the file's last write, which may have
/// changed what that write read, and those run after it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Assumed {
    pub before: Vec<String>,
    pub after: Vec<String>,
}

/// A file as it will be after the command, if the programs assumed did not
/// touch it. The after-look says whether they did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conditional {
    pub written: Written,
    pub assumed: Assumed,
}

/// A file that will be one of these, each a text or `None` for absent: an
/// `if` this could not decide left it differently in each arm. Exact as a
/// set — every member comes from an arm the text has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alternatives {
    pub path: String,
    pub texts: Vec<Option<String>>,
}

/// What a command will write, and what it writes that could not be followed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Prediction {
    /// In the order each file was first written.
    pub written: Vec<Written>,
    pub unfollowed: Vec<Unfollowed>,
    /// Files refused only because an unknown program ran, predicted as if it
    /// had left them alone.
    pub conditional: Vec<Conditional>,
    /// Files left one of several texts by an `if` this could not decide.
    pub alternatives: Vec<Alternatives>,
}

/// The files `predict` asked for and was shown nothing of, in the order it asked.
/// Under the assumption, so files read after an unknown program are asked for.
pub fn needs(script: &Script, cwd: &str, home: &str) -> Vec<String> {
    let nothing = Files::new();
    let mut run = Run::new(cwd, home, &nothing, true);
    run.items(&script.items);
    run.asked
}

/// What `script`, run in `cwd`, will leave in the files it writes.
pub fn predict(script: &Script, cwd: &str, home: &str, sight: &dyn Sight) -> Prediction {
    let mut run = Run::new(cwd, home, sight, false);
    run.items(&script.items);
    let written = run.written();
    let alternatives = run.alternatives();
    if run.assumed.is_empty() {
        return Prediction {
            written,
            unfollowed: run.unfollowed,
            conditional: Vec::new(),
            alternatives,
        };
    }
    let mut assuming = Run::new(cwd, home, sight, true);
    assuming.items(&script.items);
    let conditional = assuming
        .written()
        .into_iter()
        .filter(|file| !written.iter().any(|sure| sure.path == file.path))
        .map(|file| {
            let at = assuming.written_at[&file.path];
            let named = |keep: &dyn Fn(usize) -> bool| {
                let mut names: Vec<String> = Vec::new();
                for (when, program) in &assuming.assumed {
                    if keep(*when) && !names.contains(program) {
                        names.push(program.clone());
                    }
                }
                names
            };
            Conditional {
                assumed: Assumed {
                    before: named(&|when| when < at),
                    after: named(&|when| when > at),
                },
                written: file,
            }
        })
        .collect();
    Prediction {
        written,
        unfollowed: run.unfollowed,
        conditional,
        alternatives,
    }
}

/// A file that is one of `members`: itself when they agree, unknown when one
/// is, and each distinct text or absence once otherwise.
fn one_of(members: Vec<Held>) -> Held {
    let mut distinct: Vec<Held> = Vec::new();
    for member in members {
        let flat = match member {
            Held::OneOf(inner) => inner,
            Held::Unknown => return Held::Unknown,
            held => vec![held],
        };
        for held in flat {
            if !distinct.contains(&held) {
                distinct.push(held);
            }
        }
    }
    match distinct.len() {
        1 => distinct.remove(0),
        _ => Held::OneOf(distinct),
    }
}

/// What a command's exit status is known to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Decided: a test sight could answer, `true`, `false`.
    Known(bool),
    /// A command assumed to succeed, as the prediction assumes of every one.
    Assumed,
    /// A test this could not decide, or a list it could not follow.
    Unknown,
}

/// A file as the run has left it so far.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Held {
    Text(String),
    Absent,
    Unknown,
    /// One of these, each a text or absent: an `if` this could not decide
    /// left it differently in each branch.
    OneOf(Vec<Held>),
}

#[derive(Clone)]
struct Run<'a> {
    cwd: Option<String>,
    home: &'a str,
    sight: &'a dyn Sight,
    /// Variables the text bound to a literal, still known to hold it.
    vars: BTreeMap<String, String>,
    /// Inside a pipeline member after the first: stdin is the pipe.
    piped: bool,
    /// An unconditional `exit` or `return` was reached: nothing after it runs.
    stopped: bool,
    /// Files this run has written or found out about, by path.
    now: BTreeMap<String, Held>,
    /// Paths written, in first-write order.
    order: Vec<String>,
    /// Paths whose given text was asked for and not there.
    asked: Vec<String>,
    /// Directories whose contents this run changed wholesale, oldest first: to
    /// unknown (a formatter's directory) or to absent (`rm -r`). The newest one
    /// over a path decides what it holds.
    trees: Vec<(String, Held)>,
    unfollowed: Vec<Unfollowed>,
    /// Where a command's stdout goes when it redirects none of its own: the
    /// files a redirected group, subshell or loop around it opened. `None`
    /// outside one.
    sink: Option<Vec<String>>,
    /// Whether an unknown program is assumed to leave every file alone.
    assume: bool,
    /// Counts writes and unknown programs, to put them in order.
    clock: usize,
    /// When each path was last written or removed.
    written_at: BTreeMap<String, usize>,
    /// Each unknown program run, and when.
    assumed: Vec<(usize, String)>,
    /// A directory was made or removed in a way the run does not track
    /// (`mkdir`, `os.makedirs`): no listing is known after it.
    listings_unknown: bool,
    /// An `exit` or `return` in a region not followed, which may have ended
    /// the shell there with success: what follows runs only sometimes.
    maybe_stopped: bool,
}

impl<'a> Run<'a> {
    fn new(cwd: &str, home: &'a str, sight: &'a dyn Sight, assume: bool) -> Self {
        Self {
            cwd: Some(cwd.to_string()),
            home,
            sight,
            vars: BTreeMap::new(),
            piped: false,
            stopped: false,
            now: BTreeMap::new(),
            order: Vec::new(),
            asked: Vec::new(),
            trees: Vec::new(),
            unfollowed: Vec::new(),
            sink: None,
            assume,
            clock: 0,
            written_at: BTreeMap::new(),
            assumed: Vec::new(),
            listings_unknown: false,
            maybe_stopped: false,
        }
    }

    /// The names in `dir` as this run has left it: what sight showed, with the
    /// files the run wrote there added and those it removed taken away.
    /// `Ok(None)` for a directory that does not exist; `Err` when the listing
    /// is not known — not shown, or changed in a way the run does not track.
    pub(super) fn listing(&mut self, dir: &str) -> Result<Option<Vec<String>>, Why> {
        let covered = self.trees.iter().any(|(tree, _)| {
            dir == tree || (dir.starts_with(tree.as_str()) && dir[tree.len()..].starts_with('/'))
        });
        if self.listings_unknown || covered {
            return Err(Why::Python("listing".to_string()));
        }
        let Some(shown) = self.sight.dir(dir) else {
            return Err(Why::NotRead);
        };
        let under = format!("{dir}/");
        let mut names = shown;
        for (path, held) in &self.now {
            let Some(name) = path.strip_prefix(&under).filter(|name| !name.contains('/')) else {
                continue;
            };
            let listed = names.get_or_insert_with(Vec::new);
            match held {
                Held::Text(_) => {
                    if !listed.iter().any(|seen| seen == name) {
                        listed.push(name.to_string());
                    }
                }
                Held::Absent => listed.retain(|seen| seen != name),
                // There in every branch, whatever it holds.
                Held::OneOf(members) if members.iter().all(|m| matches!(m, Held::Text(_))) => {
                    if !listed.iter().any(|seen| seen == name) {
                        listed.push(name.to_string());
                    }
                }
                Held::Unknown | Held::OneOf(_) => return Err(Why::Python("listing".to_string())),
            }
        }
        Ok(names)
    }

    /// A directory made or removed without a file in it being followed.
    pub(super) fn listings_changed(&mut self) {
        self.listings_unknown = true;
    }

    /// The files written, as the run left them, in first-write order.
    fn written(&self) -> Vec<Written> {
        self.order
            .iter()
            .filter_map(|path| match self.now.get(path) {
                Some(Held::Text(text)) => Some(Written {
                    path: path.clone(),
                    text: Some(text.clone()),
                }),
                Some(Held::Absent) => Some(Written {
                    path: path.clone(),
                    text: None,
                }),
                _ => None,
            })
            .collect()
    }

    /// The files left one of several texts, in first-write order.
    fn alternatives(&self) -> Vec<Alternatives> {
        self.order
            .iter()
            .filter_map(|path| match self.now.get(path) {
                Some(Held::OneOf(members)) => Some(Alternatives {
                    path: path.clone(),
                    texts: members
                        .iter()
                        .map(|member| match member {
                            Held::Text(text) => Some(text.clone()),
                            _ => None,
                        })
                        .collect(),
                }),
                _ => None,
            })
            .collect()
    }

    /// Stamps a write or removal of `path` with the time.
    fn touched(&mut self, path: &str) {
        self.clock += 1;
        self.written_at.insert(path.to_string(), self.clock);
    }

    /// A program the tables do not know, which may have written any file. Under
    /// the assumption it wrote none it was not told of, and only its name is
    /// kept: a path in its words or its environment (`OUT=/tmp/rows`), and
    /// everything under one, is refused either way. Found live: a test told its
    /// output file in an assignment was assumed to leave that file alone.
    pub(super) fn unknown_program(&mut self, program: String, told: &[String]) {
        self.clock += 1;
        self.assumed.push((self.clock, program.clone()));
        if self.assume {
            // Its bindings are no more known under the assumption.
            self.vars.clear();
            for path in told {
                self.forget_tree(path, Why::Program(program.clone()));
            }
        } else {
            self.forget_everything(Why::Program(program));
        }
    }

    fn items(&mut self, items: &[Item]) {
        for item in items {
            if self.stopped {
                break;
            }
            if let Item::List(list) = item {
                if self.maybe_stopped {
                    self.forget_list(list, Why::Sometimes);
                } else {
                    self.list(list);
                }
            }
        }
    }

    /// A list, each link run or skipped by the status before it: a test this
    /// can decide steers `&&` and `||` exactly; any other command is assumed
    /// to succeed, so what follows `||` after it is only sometimes run; and
    /// after a test it cannot decide, so is what follows either.
    fn list(&mut self, list: &AndOr) -> Status {
        // A background job is a child: its `cd` stays with it.
        if list.background {
            let cwd = self.cwd.clone();
            self.forget_list(list, Why::Background);
            self.cwd = cwd;
            return Status::Unknown;
        }
        let mut status = self.pipeline(&list.first);
        for link in &list.rest {
            let runs = match (link.connector, status) {
                (Connector::And, Status::Known(true) | Status::Assumed) => Some(true),
                (Connector::And, Status::Known(false)) | (Connector::Or, Status::Known(true)) => {
                    Some(false)
                }
                (Connector::Or, Status::Known(false)) => Some(true),
                (Connector::Or, Status::Assumed) | (_, Status::Unknown) => None,
            };
            let runs = if self.maybe_stopped { None } else { runs };
            match runs {
                Some(true) => status = self.pipeline(&link.pipeline),
                // Skipped: the status stands.
                Some(false) => {}
                None => {
                    // A `cd` that only sometimes ran leaves the shell somewhere unknown.
                    let cwd = self.cwd.clone();
                    self.forget_pipeline(&link.pipeline, Why::Sometimes);
                    if self.cwd != cwd {
                        self.cwd = None;
                    }
                    status = Status::Unknown;
                }
            }
        }
        status
    }

    /// Both arms of an `if` this cannot decide, each run from the state before
    /// it, and joined: a file they leave alike is that, one they leave
    /// differently is one of the texts. `false`, with nothing done, when the
    /// arms end the shell differently, which a join cannot hold.
    fn both(&mut self, then: &[Item], otherwise: Option<&[Item]>) -> bool {
        let before = self.clone();
        self.items(then);
        let taken = std::mem::replace(self, before.clone());
        if let Some(otherwise) = otherwise {
            self.items(otherwise);
        }
        if taken.stopped != self.stopped || taken.sink != self.sink {
            *self = before;
            return false;
        }
        self.join(taken, &before);
        true
    }

    /// Joins the run `other` took into this one, both from `before`.
    fn join(&mut self, other: Run<'a>, before: &Run<'a>) {
        if self.cwd != other.cwd {
            self.cwd = None;
        }
        self.vars
            .retain(|name, value| other.vars.get(name) == Some(value));
        self.piped |= other.piped;
        let paths: BTreeSet<String> = self.now.keys().chain(other.now.keys()).cloned().collect();
        let mut before = before.clone();
        for path in paths {
            // A path one arm left alone holds what it held before the `if`.
            let mut side = |run: &Run<'a>| match run.now.get(&path) {
                Some(held) => held.clone(),
                None => before.read(&path),
            };
            let (mine, theirs) = (side(self), side(&other));
            let held = one_of(vec![mine, theirs]);
            if held == Held::Unknown
                && self.now.get(&path) != Some(&Held::Unknown)
                && other.now.get(&path) != Some(&Held::Unknown)
            {
                // Written in one arm, and not known before: neither text can
                // be named.
                self.unfollowed.push(Unfollowed {
                    path: Some(path.clone()),
                    why: Why::Compound,
                });
            }
            self.now.insert(path, held);
        }
        for path in other.order {
            if !self.order.contains(&path) {
                self.order.push(path);
            }
        }
        for tree in other.trees {
            if !self.trees.contains(&tree) {
                self.trees.push(tree);
            }
        }
        for asked in other.asked {
            if !self.asked.contains(&asked) {
                self.asked.push(asked);
            }
        }
        for unfollowed in other.unfollowed {
            if !self.unfollowed.contains(&unfollowed) {
                self.unfollowed.push(unfollowed);
            }
        }
        self.clock = self.clock.max(other.clock);
        for (path, at) in other.written_at {
            let mine = self.written_at.entry(path).or_insert(at);
            *mine = (*mine).max(at);
        }
        for assumed in other.assumed {
            if !self.assumed.contains(&assumed) {
                self.assumed.push(assumed);
            }
        }
        self.listings_unknown |= other.listings_unknown;
    }

    /// The status of the last list in `items`, each run in turn.
    fn condition(&mut self, items: &[Item]) -> Status {
        let mut status = Status::Known(true);
        for item in items {
            if self.stopped {
                break;
            }
            if let Item::List(list) = item {
                status = self.list(list);
            }
        }
        status
    }

    /// What a command's status is known to be: a test decided by sight, or
    /// `true` and `false`; any other command is assumed to succeed.
    fn status(&mut self, command: &Command) -> Status {
        match &command.kind {
            CommandKind::Test(expr) => {
                self.expansions(test_operands(expr), None);
                self.test_expr(expr).map_or(Status::Unknown, Status::Known)
            }
            CommandKind::Simple(simple) if !simple.words.is_empty() => {
                let argv: Option<Vec<String>> =
                    simple.words.iter().map(|w| self.literal(w)).collect();
                let Some(argv) = argv else {
                    return match simple
                        .words
                        .first()
                        .and_then(|w| self.literal(w))
                        .as_deref()
                    {
                        Some("[" | "test") => Status::Unknown,
                        _ => Status::Assumed,
                    };
                };
                match argv[0].as_str() {
                    "true" | ":" => Status::Known(true),
                    "false" => Status::Known(false),
                    "[" if argv.last().map(String::as_str) == Some("]") => self
                        .test_words(&argv[1..argv.len() - 1])
                        .map_or(Status::Unknown, Status::Known),
                    "test" => self
                        .test_words(&argv[1..])
                        .map_or(Status::Unknown, Status::Known),
                    "[" => Status::Unknown,
                    _ => Status::Assumed,
                }
            }
            _ => Status::Assumed,
        }
    }

    /// `test` or `[ … ]` over its words: `None` when this cannot decide it.
    fn test_words(&mut self, words: &[String]) -> Option<bool> {
        match words {
            [] => Some(false),
            [bang, rest @ ..] if bang == "!" => self.test_words(rest).map(|b| !b),
            [one] => Some(!one.is_empty()),
            [op, operand] => match op.as_str() {
                "-n" => Some(!operand.is_empty()),
                "-z" => Some(operand.is_empty()),
                file if file.len() == 2 && file.starts_with('-') => {
                    self.file_test(file.chars().nth(1)?, operand)
                }
                _ => None,
            },
            [left, op, right] => match op.as_str() {
                "=" | "==" => Some(left == right),
                "!=" => Some(left != right),
                "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
                    let (a, b) = (
                        left.trim().parse::<i64>().ok()?,
                        right.trim().parse::<i64>().ok()?,
                    );
                    Some(match op.as_str() {
                        "-eq" => a == b,
                        "-ne" => a != b,
                        "-lt" => a < b,
                        "-le" => a <= b,
                        "-gt" => a > b,
                        _ => a >= b,
                    })
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// `[[ … ]]`: `None` when this cannot decide it.
    fn test_expr(&mut self, expr: &TestExpr) -> Option<bool> {
        match expr {
            TestExpr::Not(inner) => self.test_expr(inner).map(|b| !b),
            // `&&` and `||` short-circuit: an undecided side that is not reached
            // does not matter.
            TestExpr::And(a, b) => match self.test_expr(a)? {
                false => Some(false),
                true => self.test_expr(b),
            },
            TestExpr::Or(a, b) => match self.test_expr(a)? {
                true => Some(true),
                false => self.test_expr(b),
            },
            TestExpr::Unary { op, operand } => {
                let operand = self.literal(operand)?;
                match op {
                    UnaryTest::NonEmpty => Some(!operand.is_empty()),
                    UnaryTest::Empty => Some(operand.is_empty()),
                    UnaryTest::File(letter) => self.file_test(*letter, &operand),
                    _ => None,
                }
            }
            // Only where the right side is literal text: an unquoted glob there
            // is a pattern, and such a word is not literal.
            TestExpr::Binary { op, left, right } => {
                let (left, right) = (self.literal(left)?, self.literal(right)?);
                match op {
                    BinaryTest::Equal => Some(left == right),
                    BinaryTest::NotEqual => Some(left != right),
                    _ => None,
                }
            }
        }
    }

    /// A file test as sight and this run show it: `-e`, `-f`, `-s`, `-d`. A
    /// permission, a link, or a file sight cannot show is not decided.
    fn file_test(&mut self, letter: char, operand: &str) -> Option<bool> {
        let path = self.resolve(operand)?;
        match letter {
            'f' | 'e' | 's' => match self.read(&path) {
                Held::Text(text) => Some(letter != 's' || !text.is_empty()),
                // Shown absent: nothing is there, a directory neither — sight
                // cannot show a directory as a file, and says so.
                Held::Absent => Some(false),
                // Decided only where every branch answers the same.
                Held::OneOf(members) => {
                    let answers: Vec<bool> = members
                        .iter()
                        .map(|m| match m {
                            Held::Text(text) => letter != 's' || !text.is_empty(),
                            _ => false,
                        })
                        .collect();
                    answers
                        .windows(2)
                        .all(|pair| pair[0] == pair[1])
                        .then(|| answers[0])
                }
                Held::Unknown => None,
            },
            'd' => match self.listing(&path) {
                Ok(listed) => Some(listed.is_some()),
                Err(_) => None,
            },
            _ => None,
        }
    }

    /// Each member of a longer pipeline runs in a subshell of its own, all at
    /// once: it keeps its `cd` and bindings inside, sees the files as they
    /// were before the pipeline, and reads the pipe as its stdin. A path two
    /// members both change is left unknown, since their order is not. Found
    /// live: a heredoc edit followed by `2>&1 | grep` was refused whole.
    fn pipeline(&mut self, pipeline: &Pipeline) -> Status {
        let status = self.members(pipeline);
        match (pipeline.negated, status) {
            (false, status) => status,
            (true, Status::Known(b)) => Status::Known(!b),
            // A command assumed to succeed is not assumed to fail.
            (true, _) => Status::Unknown,
        }
    }

    fn members(&mut self, pipeline: &Pipeline) -> Status {
        let members = pipeline.commands.as_slice();
        if let [only] = members {
            self.command(only);
            return self.status(only);
        }
        let (cwd, vars, outer, stopped) = (
            self.cwd.clone(),
            self.vars.clone(),
            self.piped,
            self.stopped,
        );
        let (base_now, base_trees) = (self.now.clone(), self.trees.clone());
        // Only the last member's stdout leaves the pipeline.
        let sink = self.sink.take();
        let mut merged = base_now.clone();
        let mut merged_trees = base_trees.clone();
        let mut changed_by: BTreeMap<String, usize> = BTreeMap::new();
        for (at, member) in members.iter().enumerate() {
            self.now = base_now.clone();
            self.trees = base_trees.clone();
            self.piped = outer || at > 0;
            self.stopped = stopped;
            self.sink = if at + 1 == members.len() {
                sink.clone()
            } else {
                None
            };
            self.command(member);
            self.cwd = cwd.clone();
            self.vars = vars.clone();
            for (path, held) in &self.now {
                if base_now.get(path) == Some(held) {
                    continue;
                }
                *changed_by.entry(path.clone()).or_insert(0) += 1;
                merged.insert(path.clone(), held.clone());
            }
            merged_trees.extend(self.trees.iter().skip(base_trees.len()).cloned());
        }
        self.piped = outer;
        self.stopped = stopped;
        self.sink = sink;
        self.now = merged;
        self.trees = merged_trees;
        for (path, members) in changed_by {
            if members > 1 {
                self.now.insert(path.clone(), Held::Unknown);
                self.unfollowed.push(Unfollowed {
                    path: Some(path),
                    why: Why::Pipeline,
                });
            }
        }
        // The last member's status, which is assumed.
        Status::Assumed
    }

    fn command(&mut self, command: &Command) {
        let compound = matches!(
            command.kind,
            CommandKind::Group(_) | CommandKind::Subshell(_) | CommandKind::For(_)
        );
        if compound && !command.redirects.is_empty() && self.redirected(command) {
            return;
        }
        match &command.kind {
            CommandKind::Simple(simple) => self.simple(command, simple),
            // An `if` whose condition this can decide runs that branch. Its
            // condition is not assumed to succeed: that is what it asks.
            CommandKind::If(branch) if command.redirects.is_empty() => {
                match self.condition(&branch.condition) {
                    Status::Known(true) => self.items(&branch.then),
                    Status::Known(false) => {
                        if let Some(otherwise) = &branch.otherwise {
                            self.items(otherwise);
                        }
                    }
                    Status::Assumed | Status::Unknown
                        if self.both(&branch.then, branch.otherwise.as_deref()) => {}
                    Status::Assumed | Status::Unknown => {
                        let cwd = self.cwd.clone();
                        let arms = std::iter::once(&branch.then).chain(branch.otherwise.as_ref());
                        for arm in arms {
                            for item in arm {
                                if let Item::List(list) = item {
                                    self.forget_list(list, Why::Compound);
                                }
                            }
                        }
                        if self.cwd != cwd {
                            self.cwd = None;
                        }
                    }
                }
            }
            // A group is its commands; a subshell the same, with its `cd` and
            // its bindings kept inside.
            CommandKind::Group(items) if command.redirects.is_empty() => self.items(items),
            CommandKind::Subshell(items) if command.redirects.is_empty() => {
                let (cwd, vars, stopped) = (self.cwd.clone(), self.vars.clone(), self.stopped);
                self.items(items);
                self.cwd = cwd;
                self.vars = vars;
                self.stopped = stopped;
            }
            // A loop over words the text spells out runs its body once per word,
            // the variable bound to each; it stays bound to the last, as bash
            // leaves it.
            CommandKind::For(it) if !it.select && command.redirects.is_empty() => {
                self.expansions(&it.words, None);
                let values: Option<Vec<String>> =
                    it.words.iter().map(|w| self.literal(w)).collect();
                match values {
                    Some(values) if !controls_flow(&it.body) => {
                        for value in values {
                            self.vars.insert(it.name.clone(), value);
                            self.items(&it.body);
                        }
                    }
                    _ => {
                        self.forget_command(command, Why::Compound);
                        if moves(command) {
                            self.cwd = None;
                        }
                    }
                }
            }
            _ => {
                let cwd = self.cwd.clone();
                self.forget_command(command, Why::Compound);
                // A subshell keeps its `cd`; a compound in this shell moves it
                // somewhere this did not follow.
                if matches!(command.kind, CommandKind::Subshell(_)) {
                    self.cwd = cwd;
                } else if moves(command) {
                    self.cwd = None;
                }
            }
        }
    }

    /// `{ …; } > f`, `( … ) >> f`, `for …; done > f`: the file is opened once,
    /// and what each command inside prints without a redirect of its own is
    /// appended to it. `false` when a redirect is not an output this follows —
    /// input into the group — and nothing was done.
    fn redirected(&mut self, command: &Command) -> bool {
        let outputs_only = command.redirects.iter().all(|redirect| {
            !matches!(
                redirect.op,
                RedirectOp::Read
                    | RedirectOp::ReadWrite
                    | RedirectOp::DupIn
                    | RedirectOp::Here
                    | RedirectOp::HereDash
                    | RedirectOp::HereString
            )
        });
        if !outputs_only {
            return false;
        }
        self.expansions(redirect_words(&command.redirects), None);
        let mut opened = Vec::new();
        for target in self.outputs(&command.redirects).unwrap_or_default() {
            match target {
                Target::File { path, append } => {
                    self.write(&path, append, Ok(String::new()));
                    opened.push(path);
                }
                Target::Unnamed(why) => self.unfollowed.push(Unfollowed { path: None, why }),
                // What goes to stderr is not modelled.
                Target::Other(path) => self.write(&path, false, Err(Why::Descriptor)),
            }
        }
        // A stdout redirect replaces where the body prints, even to a file this
        // does not keep (`> /dev/null`); a stderr one leaves it.
        let sink = if prints_to_redirect(&command.redirects) {
            Some(opened)
        } else {
            self.sink.clone()
        };
        let outer = std::mem::replace(&mut self.sink, sink);
        let bare = Command {
            redirects: Vec::new(),
            ..command.clone()
        };
        self.command(&bare);
        self.sink = outer;
        true
    }

    /// What expanding `words` runs, left to right, before the command itself:
    /// a `$( )` written plainly runs in a subshell, followed here (its output
    /// is the word's, not a file's) or, with `why`, forgotten. One inside a
    /// parameter's operand runs only when the parameter says, and `<( )` runs
    /// beside the command: what either runs is forgotten.
    fn expansions<'w>(&mut self, words: impl IntoIterator<Item = &'w Word>, why: Option<&Why>) {
        for word in words {
            for segment in &word.segments {
                match &segment.kind {
                    SegmentKind::Substitution(substitution) => {
                        self.subshell(&substitution.items, why);
                    }
                    SegmentKind::ProcessSubstitution(_) => {
                        self.forget_within(segment, why.cloned().unwrap_or(Why::Background));
                    }
                    _ => self.forget_within(segment, why.cloned().unwrap_or(Why::Sometimes)),
                }
            }
        }
    }

    /// Items run in a subshell: followed, or with `why` forgotten; its `cd`,
    /// bindings and `exit` stay inside, and what it prints is not a file's.
    fn subshell(&mut self, items: &[Item], why: Option<&Why>) {
        let (cwd, vars, stopped) = (self.cwd.clone(), self.vars.clone(), self.stopped);
        let sink = self.sink.take();
        match why {
            None => self.items(items),
            Some(why) => {
                for item in items {
                    if let Item::List(list) = item {
                        self.forget_list(list, why.clone());
                    }
                }
            }
        }
        self.cwd = cwd;
        self.vars = vars;
        self.stopped = stopped;
        self.sink = sink;
    }

    /// Every command held inside one part of a word, forgotten for `why`.
    fn forget_within(&mut self, segment: &Segment, why: Why) {
        let mut inside = Vec::new();
        crate::syntax::visit::segment_commands(segment, &mut |command| inside.push(command));
        if inside.is_empty() {
            return;
        }
        let (cwd, vars) = (self.cwd.clone(), self.vars.clone());
        let sink = self.sink.take();
        // The walk reaches every command, inner ones too: each is forgotten
        // once, without its bodies.
        for command in inside {
            self.forget_command_with(command, why.clone(), false);
        }
        self.cwd = cwd;
        self.vars = vars;
        self.sink = sink;
    }

    /// Appends what a command prints to the files a redirect around it opened.
    fn print_to_sink(
        &mut self,
        name: Option<&str>,
        argv: &[Option<String>],
        redirects: &[Redirect],
    ) {
        let Some(sink) = self.sink.clone() else {
            return;
        };
        if prints_to_redirect(redirects) {
            return;
        }
        let text = if quiet(name, argv) {
            Ok(String::new())
        } else {
            self.stdout(name, argv, redirects)
        };
        for path in sink {
            self.write(&path, true, text.clone());
        }
    }

    fn simple(&mut self, command: &Command, simple: &Simple) {
        let redirects = command.redirects.as_slice();
        // Its words expand before it runs, and what they run runs first.
        let words = simple
            .assignments
            .iter()
            .map(|a| &a.value)
            .chain(&simple.words);
        self.expansions(words.chain(redirect_words(redirects)), None);
        if heredoc_runs(redirects) {
            self.unknown_program("heredoc substitution".to_string(), &[]);
        }
        let argv: Vec<Option<String>> = simple.words.iter().map(|w| self.literal(w)).collect();
        let name = argv.first().cloned().flatten();
        if self.change_dir(&argv) {
            return;
        }
        // The shell ends here, and what came before stands.
        if matches!(name.as_deref(), Some("exit" | "return")) {
            self.stopped = true;
            return;
        }
        // A bare assignment binds; one whose value the text does not spell out,
        // or that appends, leaves the name unknown. A prefix (`A=1 cmd`) binds
        // for that command alone and is not recorded.
        if simple.words.is_empty() {
            for assignment in &simple.assignments {
                match self.literal(&assignment.value) {
                    Some(value) if !assignment.append => {
                        self.vars.insert(assignment.name.clone(), value);
                    }
                    _ => {
                        self.vars.remove(&assignment.name);
                    }
                }
            }
        }
        // A builtin that binds names of its own choosing leaves none known.
        if name.as_deref().is_some_and(|name| REBINDS.contains(&name))
            || (name.as_deref() == Some("printf")
                && argv.iter().skip(1).any(|arg| arg.as_deref() == Some("-v")))
        {
            self.vars.clear();
        }
        // `tee` writes its input to the files it names, as well as to stdout.
        if name.as_deref() == Some("tee") {
            self.tee(&argv, redirects);
        } else if let Some(embedded) = python_of(command) {
            self.python(embedded.program, None);
        } else {
            self.forget_program_writes(simple, redirects, None);
        }
        if !simple.words.is_empty() {
            self.print_to_sink(name.as_deref(), &argv, redirects);
        }
        let Some(out) = self.outputs(redirects) else {
            return;
        };
        let written = self.stdout(name.as_deref(), &argv, redirects);
        for target in out {
            match target {
                Target::File { path, append } => self.write(&path, append, written.clone()),
                Target::Unnamed(why) => self.unfollowed.push(Unfollowed { path: None, why }),
                Target::Other(path) => self.write(&path, false, Err(Why::Descriptor)),
            }
        }
    }

    /// Where this command's output goes, when any of it goes to a file. `None`
    /// when nothing is written.
    fn outputs(&mut self, redirects: &[Redirect]) -> Option<Vec<Target>> {
        let mut out = Vec::new();
        for redirect in redirects {
            let RedirectTarget::File(word) = &redirect.target else {
                continue;
            };
            let (stdout, append) = match (redirect.op, redirect.fd) {
                (RedirectOp::Write | RedirectOp::Clobber, Some(1)) => (true, false),
                (RedirectOp::Append, Some(1)) => (true, true),
                (RedirectOp::Both | RedirectOp::BothWord, _) => (true, false),
                (RedirectOp::BothAppend, _) => (true, true),
                (RedirectOp::Write | RedirectOp::Clobber | RedirectOp::Append, Some(_)) => {
                    (false, false)
                }
                // Opened for reading and writing: what the program writes
                // through it is not modelled.
                (RedirectOp::ReadWrite, _) => (false, false),
                _ => continue,
            };
            let Some(literal) = self.literal(word) else {
                out.push(Target::Unnamed(Why::Expansion));
                continue;
            };
            let Some(path) = self.resolve(&literal) else {
                // `/dev/null`, or a relative path after an unfollowed `cd`.
                if self.cwd.is_none() && !literal.starts_with('/') {
                    out.push(Target::Unnamed(Why::Directory));
                }
                continue;
            };
            out.push(if stdout {
                Target::File { path, append }
            } else {
                Target::Other(path)
            });
        }
        (!out.is_empty()).then_some(out)
    }

    /// What the command prints, when it is a program this models.
    fn stdout(
        &mut self,
        name: Option<&str>,
        argv: &[Option<String>],
        redirects: &[Redirect],
    ) -> Result<String, Why> {
        let args: Vec<String> = argv
            .iter()
            .skip(1)
            .cloned()
            .collect::<Option<_>>()
            .ok_or(Why::Expansion)?;
        match name {
            Some("echo") => echo(&args),
            Some("printf") => printf(&args),
            Some("true" | ":") => Ok(String::new()),
            Some("cat") => self.cat(&args, redirects),
            Some("tee") => self.stdin(redirects),
            // Named for the program that prints, behind any carrier.
            Some(other) => {
                let words: Vec<String> = std::iter::once(other.to_string()).chain(args).collect();
                let program = innermost(&words)
                    .first()
                    .map_or(other, |head| basename(head));
                Err(Why::Program(program.to_string()))
            }
            None => Err(Why::Expansion),
        }
    }

    fn cat(&mut self, args: &[String], redirects: &[Redirect]) -> Result<String, Why> {
        if args.is_empty() {
            return self.stdin(redirects);
        }
        if let Some(flag) = args.iter().find(|arg| arg.starts_with('-')) {
            return Err(Why::Option(format!("cat {flag}")));
        }
        // Every file read before any is judged, so `needs` asks for all of them.
        let held: Vec<Option<Held>> = args
            .iter()
            .map(|arg| self.resolve(arg).map(|path| self.read(&path)))
            .collect();
        let mut out = String::new();
        for file in held {
            match file {
                Some(Held::Text(text)) => out.push_str(&text),
                Some(Held::Absent) => return Err(Why::Missing),
                Some(Held::Unknown) => return Err(Why::NotRead),
                Some(Held::OneOf(_)) => return Err(Why::Branches),
                None => return Err(Why::Directory),
            }
        }
        Ok(out)
    }

    /// What arrives on stdin: a heredoc, a here-string, or a file.
    fn stdin(&mut self, redirects: &[Redirect]) -> Result<String, Why> {
        let mut input = None;
        for redirect in redirects {
            match (&redirect.op, &redirect.target) {
                (RedirectOp::Here | RedirectOp::HereDash, RedirectTarget::Here(heredoc)) => {
                    let literal = heredoc.quoted || !heredoc.body.contains(['$', '`', '\\']);
                    input = Some(if literal {
                        Ok(heredoc.body.clone())
                    } else {
                        Err(Why::Expansion)
                    });
                }
                (RedirectOp::HereString, RedirectTarget::File(word)) => {
                    input = Some(self.literal(word).map(|w| w + "\n").ok_or(Why::Expansion));
                }
                (RedirectOp::Read, RedirectTarget::File(word)) => {
                    let path = self.literal(word).and_then(|w| self.resolve(&w));
                    input = Some(match path.map(|p| self.read(&p)) {
                        Some(Held::Text(text)) => Ok(text),
                        Some(Held::Absent) => Err(Why::Missing),
                        Some(Held::Unknown) => Err(Why::NotRead),
                        Some(Held::OneOf(_)) => Err(Why::Branches),
                        None => Err(Why::Expansion),
                    });
                }
                _ => {}
            }
        }
        // Nothing redirected in: the pipe, or the terminal.
        input.unwrap_or(if self.piped {
            Err(Why::Pipeline)
        } else {
            Err(Why::Program("stdin".to_string()))
        })
    }

    fn tee(&mut self, argv: &[Option<String>], redirects: &[Redirect]) {
        let mut append = false;
        let mut files = Vec::new();
        for arg in argv.iter().skip(1) {
            match arg.as_deref() {
                Some("-a" | "--append") => append = true,
                Some(flag) if flag.starts_with('-') => {
                    self.unfollowed.push(Unfollowed {
                        path: None,
                        why: Why::Option(format!("tee {flag}")),
                    });
                    return;
                }
                Some(file) => files.push(file.to_string()),
                None => self.unfollowed.push(Unfollowed {
                    path: None,
                    why: Why::Expansion,
                }),
            }
        }
        let input = self.stdin(redirects);
        for file in files {
            match self.resolve(&file) {
                Some(path) => self.write(&path, append, input.clone()),
                None => self.unfollowed.push(Unfollowed {
                    path: None,
                    why: Why::Directory,
                }),
            }
        }
    }

    /// Record a write of `text` to `path`, or forget the file when it cannot be followed.
    fn write(&mut self, path: &str, append: bool, text: Result<String, Why>) {
        if !self.order.iter().any(|seen| seen == path) {
            self.order.push(path.to_string());
        }
        self.touched(path);
        // Appending to a file this run already refused leaves it refused, for
        // the reason it first was: `cmd > log; echo "rc=$?" >> log` is unknown
        // because of `cmd`, and a second reason would rank what did not stop it.
        if append && matches!(self.now.get(path), Some(Held::Unknown)) {
            return;
        }
        let now = match (text, append) {
            (Ok(text), false) => Ok(Held::Text(text)),
            (Ok(text), true) => match self.read(path) {
                Held::Text(before) => Ok(Held::Text(before + &text)),
                Held::Absent => Ok(Held::Text(text)),
                Held::Unknown => Err(Why::NotRead),
                // Appended in every branch alike.
                Held::OneOf(members) => Ok(one_of(
                    members
                        .into_iter()
                        .map(|member| match member {
                            Held::Text(before) => Held::Text(before + &text),
                            _ => Held::Text(text.clone()),
                        })
                        .collect(),
                )),
            },
            (Err(why), _) => Err(why),
        };
        match now {
            Ok(held) => {
                self.now.insert(path.to_string(), held);
            }
            Err(why) => {
                self.now.insert(path.to_string(), Held::Unknown);
                self.unfollowed.push(Unfollowed {
                    path: Some(path.to_string()),
                    why,
                });
            }
        }
    }

    /// A write this does not follow to `path` and to everything under it — a
    /// formatter given a directory, `rm -r`.
    fn forget_tree(&mut self, path: &str, why: Why) {
        let under = format!("{path}/");
        let inside: Vec<String> = self
            .now
            .keys()
            .filter(|known| known.starts_with(&under))
            .cloned()
            .collect();
        for known in inside {
            self.now.insert(known.clone(), Held::Unknown);
            self.unfollowed.push(Unfollowed {
                path: Some(known),
                why: why.clone(),
            });
        }
        self.trees.push((path.to_string(), Held::Unknown));
        self.write(path, false, Err(why));
    }

    /// A program that may have written any file: nothing known survives it.
    fn forget_everything(&mut self, why: Why) {
        // A function the text defines could have rebound anything, and calling
        // one is a program this does not know.
        self.vars.clear();
        for (path, held) in &mut self.now {
            if !matches!(held, Held::Unknown) {
                *held = Held::Unknown;
                self.unfollowed.push(Unfollowed {
                    path: Some(path.clone()),
                    why: why.clone(),
                });
            }
        }
        // The root: every path lies under it.
        self.trees.push((String::new(), Held::Unknown));
    }

    /// `rm`: the path, and with `-r` everything under it, is gone. What the run
    /// wrote there is reported removed; anything under it read later is absent.
    fn remove(&mut self, path: &str, recursive: bool) {
        if recursive {
            let under = format!("{path}/");
            let inside: Vec<String> = self
                .now
                .keys()
                .filter(|known| known.starts_with(&under))
                .cloned()
                .collect();
            for known in inside {
                self.now.insert(known, Held::Absent);
            }
            self.trees.push((path.to_string(), Held::Absent));
        }
        if !self.order.iter().any(|seen| seen == path) {
            self.order.push(path.to_string());
        }
        self.touched(path);
        self.now.insert(path.to_string(), Held::Absent);
    }

    /// `cp`: one file to one file, as the text of the source. A flag other than
    /// `-f`, `-p` or `-v` (a tree, no-clobber, a prompt), or several sources,
    /// is refused by name; a destination sight cannot show may be a directory,
    /// under which the written file would have another name, so it is refused
    /// as not read; a source that does not exist fails the copy.
    fn copy(&mut self, argv: &[String], from: &[String], to: &str) {
        let (flags, operands): (Vec<&String>, Vec<&String>) =
            argv.iter().skip(1).partition(|word| word.starts_with('-'));
        if let Some(flag) = flags
            .iter()
            .find(|flag| flag.len() < 2 || !flag[1..].chars().all(|c| "fpv".contains(c)))
        {
            self.write(to, false, Err(Why::Option(format!("cp {flag}"))));
            return;
        }
        if operands.len() != 2 || from.len() != 1 {
            self.write(
                to,
                false,
                Err(Why::Option("cp into a directory".to_string())),
            );
            return;
        }
        let text = match self.read(&from[0]) {
            Held::Text(text) => Ok(text),
            Held::Absent => Err(Why::Missing),
            Held::Unknown => Err(Why::NotRead),
            Held::OneOf(_) => Err(Why::Branches),
        };
        let text = text.and_then(|text| match self.read(to) {
            Held::Unknown => Err(Why::NotRead),
            _ => Ok(text),
        });
        self.write(to, false, text);
    }

    /// `sed -i`: each file rewritten by the script, and its old text kept under
    /// the backup suffix when one was given. What the script or the flags say
    /// that [`sed`] does not follow is refused by name, once per file.
    fn sed(&mut self, argv: &[String], paths: &[String]) {
        let invocation = sed::invocation(argv);
        for path in paths {
            let text = match self.read(path) {
                Held::Text(text) => Ok(text),
                Held::Absent => Err(Why::Missing),
                Held::Unknown => Err(Why::NotRead),
                Held::OneOf(_) => Err(Why::Branches),
            };
            let (after, suffix) = match (&invocation, text) {
                (Err(refused), _) => (Err(Why::Sed(refused.clone())), None),
                (Ok(_), Err(why)) => (Err(why), None),
                (Ok(invocation), Ok(before)) => {
                    let scripts: Vec<&str> =
                        invocation.scripts.iter().map(String::as_str).collect();
                    let after =
                        sed::apply(&scripts, invocation.extended, &before).map_err(Why::Sed);
                    let backup = invocation
                        .suffix
                        .as_ref()
                        .map(|suffix| (format!("{path}{suffix}"), before));
                    (after, backup)
                }
            };
            if let Some((backup, before)) = suffix {
                self.write(&backup, false, Ok(before));
            }
            self.write(path, false, after);
        }
    }

    /// `mv`: one file to one file, as `cp` and then the source removed. A flag
    /// other than `-f` or `-v` (no-clobber, a prompt) and several sources are
    /// refused by name; a destination sight cannot show may be a directory.
    fn rename(&mut self, argv: &[String], from: &[String], to: &str) {
        let (flags, operands): (Vec<&String>, Vec<&String>) = argv
            .iter()
            .skip(1)
            .filter(|word| word.as_str() != "--")
            .partition(|word| word.starts_with('-'));
        if let Some(flag) = flags
            .iter()
            .find(|flag| flag.len() < 2 || !flag[1..].chars().all(|c| "fv".contains(c)))
        {
            self.write(to, false, Err(Why::Option(format!("mv {flag}"))));
            return;
        }
        if operands.len() != 2 || from.len() != 1 {
            self.write(
                to,
                false,
                Err(Why::Option("mv into a directory".to_string())),
            );
            return;
        }
        let text = match self.read(&from[0]) {
            Held::Text(text) => Ok(text),
            Held::Absent => Err(Why::Missing),
            Held::Unknown => Err(Why::NotRead),
            Held::OneOf(_) => Err(Why::Branches),
        };
        let text = text.and_then(|text| match self.read(to) {
            Held::Unknown => Err(Why::NotRead),
            _ => Ok(text),
        });
        let moved = text.is_ok();
        self.write(to, false, text);
        if moved {
            self.remove(&from[0], false);
        }
    }

    /// A file's text as this run has left it, or as it was given.
    fn read(&mut self, path: &str) -> Held {
        if let Some(held) = self.now.get(path) {
            return held.clone();
        }
        if let Some((_, held)) = self.trees.iter().rev().find(|(tree, _)| {
            path.starts_with(tree.as_str()) && path[tree.len()..].starts_with('/')
        }) {
            return held.clone();
        }
        match self.sight.file(path) {
            Some(Some(text)) => Held::Text(text),
            Some(None) => Held::Absent,
            None => {
                if !self.asked.iter().any(|seen| seen == path) {
                    self.asked.push(path.to_string());
                }
                Held::Unknown
            }
        }
    }

    /// Every file a list writes becomes unknown, each for `why`.
    fn forget_list(&mut self, list: &AndOr, why: Why) {
        self.forget_pipeline(&list.first, why.clone());
        for link in &list.rest {
            self.forget_pipeline(&link.pipeline, why.clone());
        }
    }

    /// Each command of a longer pipeline runs in a subshell, so a `cd` in one
    /// reaches no other.
    fn forget_pipeline(&mut self, pipeline: &Pipeline, why: Why) {
        let alone = pipeline.commands.len() == 1;
        for command in &pipeline.commands {
            let cwd = self.cwd.clone();
            self.forget_command(command, why.clone());
            if !alone {
                self.cwd = cwd;
            }
        }
    }

    /// The files a command writes by running rather than through a redirect —
    /// `sed -i`, `cp`, `rm` — as the shell tables read it. None is followed yet, so
    /// each is refused, for `why` or else by the program's name.
    fn forget_program_writes(&mut self, simple: &Simple, redirects: &[Redirect], why: Option<Why>) {
        let literal: Vec<Option<String>> = simple.words.iter().map(|w| self.literal(w)).collect();
        // A directory made or removed changes a listing without a file in it.
        if let Some(Some(head)) = literal.first()
            && matches!(basename(head), "mkdir" | "rmdir")
        {
            self.listings_changed();
        }
        let argv: Vec<String> = simple
            .words
            .iter()
            .zip(&literal)
            .map(|(word, literal)| literal.clone().unwrap_or_else(|| print_value(word)))
            .collect();
        let heredocs: Vec<String> = redirects
            .iter()
            .filter_map(|redirect| match &redirect.target {
                RedirectTarget::Here(heredoc) => Some(heredoc.body.clone()),
                _ => None,
            })
            .collect();
        let op = classify(&argv, &heredocs, self.cwd.as_deref(), self.home);
        if let Op::Nested { script } = &op {
            let literal = literal.iter().all(Option::is_some);
            self.nested(script, &argv, literal, why);
            return;
        }
        // Removed paths the text names exactly are known to be gone.
        if let Op::Remove { paths, recursive } = &op
            && literal.iter().all(Option::is_some)
            && paths.iter().all(|path| glob_root(path).is_none())
            && why.is_none()
        {
            for path in paths {
                self.remove(path, *recursive);
            }
            return;
        }
        // `sed -i` over files sight has shown, where its regex and Rust's agree.
        if let Op::Transform {
            program_file: None,
            paths,
            in_place: true,
            ..
        } = &op
            && literal.iter().all(Option::is_some)
            && why.is_none()
            && argv.first().is_some_and(|head| basename(head) == "sed")
        {
            self.sed(&argv, paths);
            return;
        }
        // A move of one file to another: the destination holds the source's
        // text and the source is gone.
        if let Op::Move { from, to } = &op
            && literal.iter().all(Option::is_some)
            && why.is_none()
            && argv.first().is_some_and(|head| basename(head) == "mv")
        {
            self.rename(&argv, from, to);
            return;
        }
        // A copy of one file to another holds the source's text.
        if let Op::Copy { from, to } = &op
            && literal.iter().all(Option::is_some)
            && why.is_none()
            && argv.first().is_some_and(|head| basename(head) == "cp")
        {
            self.copy(&argv, from, to);
            return;
        }
        if let Some(program) = writes_anything(&op, &argv) {
            // Named for the program whatever holds it: a pipe or a loop is not why
            // what came before is unknown.
            let told = self.told(simple, &literal);
            self.unknown_program(program, &told);
            return;
        }
        let written: Vec<String> = files_of(&op, Reached::Always)
            .into_iter()
            .filter(|file| file.write)
            .map(|file| file.path)
            .collect();
        if written.is_empty() {
            return;
        }
        let why = why.unwrap_or_else(|| {
            let program = innermost(&argv).first().map_or("", |head| basename(head));
            Why::Program(program.to_string())
        });
        // With every word literal the text determines every path, implied ones
        // too. Otherwise a path not among the words may have come out of an
        // expansion and could be any file, so it is refused without one.
        let determined = literal.iter().all(Option::is_some);
        let named: Vec<String> = literal
            .iter()
            .flatten()
            .filter_map(|word| self.resolve(word))
            .collect();
        for path in written {
            if determined || named.contains(&path) {
                // A pattern the program expands itself stands for the files
                // under its fixed part.
                self.forget_tree(glob_root(&path).unwrap_or(&path), why.clone());
            } else if let Some(root) = glob_root(&path).filter(|_| !path.contains(['$', '`'])) {
                // A glob and nothing else: its fixed part is in the text.
                self.forget_tree(root, why.clone());
            } else {
                // Could be any file, so nothing known survives it.
                self.forget_everything(Why::Expansion);
                self.unfollowed.push(Unfollowed {
                    path: None,
                    why: Why::Expansion,
                });
            }
        }
    }

    /// A Python program, followed where it runs in order, and otherwise every file
    /// it writes refused for `why`. One the Python tree cannot read has its files
    /// named by the flat Python reader, so that none goes unmentioned.
    fn python(&mut self, program: Program, why: Option<Why>) {
        match (program, why) {
            (
                Program::Text {
                    tree: Ok(module), ..
                },
                None,
            ) => python::run(self, &module),
            (
                Program::Text {
                    tree: Ok(module), ..
                },
                Some(why),
            ) => {
                python::forget(self, &module, &why);
            }
            (
                Program::Text {
                    source,
                    tree: Err(_),
                },
                why,
            ) => {
                self.forget_flat_python(&source, why.unwrap_or(Why::Python("unread".to_string())));
            }
            (Program::Expands { written }, why) => {
                self.forget_flat_python(&written, why.unwrap_or(Why::Expansion));
            }
        }
    }

    fn forget_flat_python(&mut self, source: &str, why: Why) {
        for used in crate::python::read(source)
            .uses
            .into_iter()
            .filter(|u| u.write)
        {
            let path = (!used.path.starts_with('~'))
                .then(|| self.resolve(&used.path))
                .flatten();
            match path {
                Some(path) => self.write(&path, false, Err(why.clone())),
                None => self.unfollowed.push(Unfollowed {
                    path: None,
                    why: why.clone(),
                }),
            }
        }
    }

    /// A script handed to another shell: `bash -c`, `nix-shell --run`. The same
    /// language against the same files, so it is followed in place, in a child
    /// whose `cd` does not reach back out. Text the outer shell expands first is
    /// not known, and what it writes is refused.
    fn nested(&mut self, script: &str, argv: &[String], literal: bool, why: Option<Why>) {
        let why = why.or_else(|| (!literal).then_some(Why::Expansion));
        if !self.child(script, self.cwd.clone(), why.clone()) {
            let program = innermost(argv).first().map_or("", |head| basename(head));
            self.unfollowed.push(Unfollowed {
                path: None,
                why: why.unwrap_or_else(|| Why::Program(program.to_string())),
            });
        }
    }

    /// Shell text run as a child process in `cwd`: followed, or with `why` every
    /// file it writes refused. Its `cd` does not reach back out. `false` when the
    /// text does not parse, and nothing was done.
    fn child(&mut self, script: &str, cwd: Option<String>, why: Option<Why>) -> bool {
        let Ok(tree) = crate::syntax::parse(script) else {
            return false;
        };
        let parent = std::mem::replace(&mut self.cwd, cwd);
        // A child sees only what was exported, which this does not track, and
        // its `exit` ends only itself.
        let (vars, stopped) = (std::mem::take(&mut self.vars), self.stopped);
        match why {
            None => self.items(&tree.items),
            Some(why) => {
                for item in &tree.items {
                    if let Item::List(list) = item {
                        self.forget_list(list, why.clone());
                    }
                }
            }
        }
        self.cwd = parent;
        self.vars = vars;
        self.stopped = stopped;
        true
    }

    /// Follows a `cd`, which says where every path after it resolves. `false` for
    /// any other command.
    fn change_dir(&mut self, argv: &[Option<String>]) -> bool {
        if argv.first().and_then(Option::as_deref) != Some("cd") {
            return false;
        }
        self.cwd = match argv.get(1) {
            Some(Some(to)) => self.resolve(to),
            None => Some(self.home.to_string()),
            Some(None) => None,
        };
        true
    }

    fn forget_command(&mut self, command: &Command, why: Why) {
        self.forget_command_with(command, why, true);
    }

    /// `forget_command`, and with `deep` its bodies too; without, for a command
    /// whose inner commands are each forgotten on their own.
    fn forget_command_with(&mut self, command: &Command, why: Why, deep: bool) {
        // What its words run would run too.
        let words: Vec<&Word> = match &command.kind {
            CommandKind::Simple(simple) => simple
                .assignments
                .iter()
                .map(|a| &a.value)
                .chain(&simple.words)
                .collect(),
            CommandKind::For(it) => it.words.iter().collect(),
            CommandKind::Case(case) => vec![&case.word],
            CommandKind::Test(expr) => test_operands(expr),
            _ => Vec::new(),
        };
        self.expansions(
            words.into_iter().chain(redirect_words(&command.redirects)),
            Some(&why),
        );
        // Its own redirects and words are expanded before it runs, with the
        // bindings as they stand; what its body binds is cleared after. Found in
        // history: a group binding a loop variable, redirected to `"$WATCH"`,
        // lost its target and the earlier `: > "$WATCH"` stood as predicted.
        let targets = self.outputs(&command.redirects).unwrap_or_default();
        // What it prints into a redirect around it is not known either.
        if !prints_to_redirect(&command.redirects)
            && let Some(sink) = self.sink.clone()
        {
            for path in sink {
                self.write(&path, true, Err(why.clone()));
            }
        }
        if let CommandKind::Simple(simple) = &command.kind {
            let argv: Vec<Option<String>> = simple.words.iter().map(|w| self.literal(w)).collect();
            if self.change_dir(&argv) {
                return;
            }
            match python_of(command) {
                Some(embedded) => self.python(embedded.program, Some(why.clone())),
                None => self.forget_program_writes(simple, &command.redirects, Some(why.clone())),
            }
        }
        for target in targets {
            match target {
                Target::File { path, .. } | Target::Other(path) => {
                    self.write(&path, false, Err(why.clone()));
                }
                Target::Unnamed(_) => self.unfollowed.push(Unfollowed {
                    path: None,
                    why: why.clone(),
                }),
            }
        }
        // Whatever it bound, this did not see.
        if binds(command) {
            self.vars.clear();
        }
        // An exit that succeeds may have been taken; one that fails the call
        // leaves it unchecked, so the success assumed excludes it.
        if ends_with_success(command)
            || (deep
                // A subshell's exit ends only it; a function's body does not
                // run where it is defined.
                && !matches!(command.kind, CommandKind::Subshell(_) | CommandKind::Function(_))
                && bodies(&command.kind)
                    .into_iter()
                    .any(|body| any_command(body, &ends_with_success)))
        {
            self.maybe_stopped = true;
        }
        if !deep {
            return;
        }
        // A function's body runs where it is called, not here: an exit in it
        // stops nothing yet.
        let stopped = self.maybe_stopped;
        for items in bodies(&command.kind) {
            for item in items {
                if let Item::List(list) = item {
                    self.forget_list(list, why.clone());
                }
            }
        }
        if matches!(command.kind, CommandKind::Function(_)) {
            self.maybe_stopped = stopped;
        }
    }

    /// The paths a command names, in its words and in the assignments before it,
    /// as absolute paths: a whole word, or what follows `=` in one
    /// (`--out=dir`). Text with whitespace in it is not a path.
    fn told(&self, simple: &Simple, literal: &[Option<String>]) -> Vec<String> {
        let assigned = simple
            .assignments
            .iter()
            .filter_map(|assignment| self.literal(&assignment.value));
        literal
            .iter()
            .flatten()
            .cloned()
            .chain(assigned)
            .flat_map(|word| {
                let value = word.split_once('=').map(|(_, value)| value.to_string());
                std::iter::once(word).chain(value)
            })
            .filter(|word| !word.contains(char::is_whitespace) && looks_like_path(word))
            .filter_map(|word| self.resolve(&word))
            .collect()
    }

    /// A word's value, when the text determines it: literal text, a home tilde,
    /// or a plain `$name` this run knows the binding of.
    fn literal(&self, word: &Word) -> Option<String> {
        let mut out = String::new();
        for (at, segment) in word.segments.iter().enumerate() {
            match &segment.kind {
                SegmentKind::Literal(text) => out.push_str(text),
                SegmentKind::Tilde(Tilde::Home) if at == 0 => out.push_str(self.home),
                SegmentKind::Parameter(parameter) if parameter.subscript.is_none() => {
                    out.push_str(&self.parameter(parameter)?);
                }
                _ => return None,
            }
        }
        Some(out)
    }

    /// A `$name` this run knows the value of — one the text bound, or `HOME`
    /// and `PWD`, which the run itself carries — alone or under a prefix or
    /// suffix strip whose pattern the text spells out.
    fn parameter(&self, parameter: &Parameter) -> Option<String> {
        let value = match self.vars.get(&parameter.name) {
            Some(bound) => bound.clone(),
            None => match parameter.name.as_str() {
                "HOME" => self.home.to_string(),
                "PWD" => self.cwd.clone()?,
                _ => return None,
            },
        };
        match &parameter.op {
            None => Some(value),
            Some(ParameterOp::StripPrefix { longest, pattern }) => {
                Some(strip(&value, &glob(pattern)?, Strip::Prefix, *longest))
            }
            Some(ParameterOp::StripSuffix { longest, pattern }) => {
                Some(strip(&value, &glob(pattern)?, Strip::Suffix, *longest))
            }
            Some(_) => None,
        }
    }

    fn resolve(&self, word: &str) -> Option<String> {
        resolve(word, self.cwd.as_deref(), self.home)
    }
}

/// One piece of a shell pattern: a character, `*`, or `?`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    Char(char),
    Any,
    One,
}

/// A pattern word as pieces, when the text spells it out; a character class or
/// an expansion in it is not followed.
fn glob(word: &Word) -> Option<Vec<Piece>> {
    let mut out = Vec::new();
    for segment in &word.segments {
        match &segment.kind {
            SegmentKind::Literal(text) => out.extend(text.chars().map(Piece::Char)),
            SegmentKind::Glob(Glob::Any) => out.push(Piece::Any),
            SegmentKind::Glob(Glob::One) => out.push(Piece::One),
            _ => return None,
        }
    }
    Some(out)
}

/// Whether `pattern` matches all of `text`.
fn matches(pattern: &[Piece], text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    // The classic wildcard walk: on `*`, remember where to resume.
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while t < chars.len() {
        match pattern.get(p) {
            Some(Piece::Any) => {
                star = Some((p, t));
                p += 1;
            }
            Some(Piece::One) => {
                p += 1;
                t += 1;
            }
            Some(Piece::Char(c)) if *c == chars[t] => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    p = sp + 1;
                    t = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|piece| *piece == Piece::Any)
}

#[derive(Debug, Clone, Copy)]
enum Strip {
    Prefix,
    Suffix,
}

/// `${x#pat}`, `${x##pat}`, `${x%pat}`, `${x%%pat}`: the value with the shortest
/// or longest matching prefix or suffix removed, or unchanged when none matches.
fn strip(value: &str, pattern: &[Piece], end: Strip, longest: bool) -> String {
    let mut cuts: Vec<usize> = (0..=value.len())
        .filter(|at| value.is_char_boundary(*at))
        .collect();
    // Shortest first for a prefix means ascending; for a suffix, descending.
    if matches!(end, Strip::Suffix) != longest {
        cuts.reverse();
    }
    for at in cuts {
        let (removed, kept) = match end {
            Strip::Prefix => (&value[..at], &value[at..]),
            Strip::Suffix => (&value[at..], &value[..at]),
        };
        if matches(pattern, removed) {
            return kept.to_string();
        }
    }
    value.to_string()
}

/// Builtins that bind names this cannot see the values of, or run text it has
/// not read.
const REBINDS: &[&str] = &[
    "read",
    "mapfile",
    "readarray",
    "declare",
    "typeset",
    "local",
    "export",
    "unset",
    "eval",
    "source",
    ".",
    "shift",
    "getopts",
    "let",
];

/// Where a command's output lands.
enum Target {
    /// Stdout into a file, replacing or appending.
    File { path: String, append: bool },
    /// Another descriptor into a file.
    Other(String),
    /// A file whose name could not be known.
    Unnamed(Why),
}

/// `echo` as bash's builtin runs it: words joined by spaces, then a newline.
fn echo(args: &[String]) -> Result<String, Why> {
    let mut newline = true;
    let mut rest = args;
    while let Some((first, tail)) = rest.split_first() {
        let flags = first
            .strip_prefix('-')
            .filter(|f| !f.is_empty() && f.chars().all(|c| "neE".contains(c)));
        let Some(flags) = flags else { break };
        if flags.contains('e') {
            return Err(Why::Option("echo -e".to_string()));
        }
        newline &= !flags.contains('n');
        rest = tail;
    }
    let mut out = rest.join(" ");
    if newline {
        out.push('\n');
    }
    Ok(out)
}

/// `printf` with a format alone: its escapes, and `%%`.
fn printf(args: &[String]) -> Result<String, Why> {
    let [format] = args else {
        return Err(Why::Option("printf with arguments".to_string()));
    };
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(match chars.next() {
                Some('n') => '\n',
                Some('t') => '\t',
                Some('r') => '\r',
                Some('\\') => '\\',
                Some('"') => '"',
                Some('\'') => '\'',
                Some(other) => return Err(Why::Option(format!("printf \\{other}"))),
                None => '\\',
            }),
            '%' => match chars.next() {
                Some('%') => out.push('%'),
                Some(other) => return Err(Why::Option(format!("printf %{other}"))),
                None => return Err(Why::Option("printf %".to_string())),
            },
            c => out.push(c),
        }
    }
    Ok(out)
}

/// The command lists a compound carries.
fn bodies(kind: &CommandKind) -> Vec<&[Item]> {
    match kind {
        CommandKind::Simple(_) | CommandKind::Test(_) | CommandKind::Arithmetic(_) => Vec::new(),
        CommandKind::For(it) => vec![&it.body],
        CommandKind::ForArith(it) => vec![&it.body],
        CommandKind::While(it) => vec![&it.condition, &it.body],
        CommandKind::If(it) => {
            let mut out: Vec<&[Item]> = vec![&it.condition, &it.then];
            if let Some(otherwise) = &it.otherwise {
                out.push(otherwise);
            }
            out
        }
        CommandKind::Case(it) => it.arms.iter().map(|arm| arm.body.as_slice()).collect(),
        CommandKind::Subshell(items) | CommandKind::Group(items) => vec![items],
        CommandKind::Function(it) => vec![&it.body],
    }
}

/// Whether any command in `items`, at any depth, is one `wanted` says.
fn any_command(items: &[Item], wanted: &dyn Fn(&Command) -> bool) -> bool {
    items.iter().any(|item| match item {
        Item::List(list) => std::iter::once(&list.first)
            .chain(list.rest.iter().map(|link| &link.pipeline))
            .flat_map(|pipeline| &pipeline.commands)
            .any(|command| {
                wanted(command)
                    || bodies(&command.kind)
                        .into_iter()
                        .any(|body| any_command(body, wanted))
            }),
        Item::Comment(_) => false,
    })
}

/// A simple command's name, when it is one literal word.
/// An `exit` or `return` that may end the shell with success: bare, whose
/// status is the last command's, or `0`.
fn ends_with_success(command: &Command) -> bool {
    let CommandKind::Simple(simple) = &command.kind else {
        return false;
    };
    if !matches!(named(command), Some("exit" | "return")) {
        return false;
    }
    match simple.words.get(1).map(|word| word.segments.as_slice()) {
        None => true,
        Some([segment]) => match &segment.kind {
            SegmentKind::Literal(status) => status.trim() == "0",
            _ => true,
        },
        Some(_) => true,
    }
}

fn named(command: &Command) -> Option<&str> {
    let CommandKind::Simple(simple) = &command.kind else {
        return None;
    };
    match simple.words.first()?.segments.as_slice() {
        [segment] => match &segment.kind {
            SegmentKind::Literal(name) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// Whether a compound runs a `cd` anywhere inside it.
fn moves(command: &Command) -> bool {
    bodies(&command.kind)
        .into_iter()
        .any(|body| any_command(body, &|command| named(command) == Some("cd")))
}

/// Whether a loop body leaves the loop or the shell early anywhere inside it,
/// which unrolling every iteration would not honour.
fn controls_flow(body: &[Item]) -> bool {
    any_command(body, &|command| {
        matches!(
            named(command),
            Some("break" | "continue" | "exit" | "return")
        )
    })
}

/// Whether a command this does not follow could bind a name: an assignment, a
/// builtin that binds, arithmetic, a loop's variable, or any of those inside.
fn binds(command: &Command) -> bool {
    match &command.kind {
        CommandKind::Simple(simple) => {
            !simple.assignments.is_empty()
                || named(command).is_some_and(|name| REBINDS.contains(&name) || name == "printf")
        }
        CommandKind::Arithmetic(_)
        | CommandKind::ForArith(_)
        | CommandKind::For(_)
        | CommandKind::Function(_) => true,
        kind => bodies(kind)
            .into_iter()
            .any(|body| any_command(body, &binds)),
    }
}

/// A file that did not end up holding what was predicted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    pub path: String,
    /// What the file was to hold, or `None` if it was to be removed.
    pub predicted: Option<String>,
    /// What the file held after the call, or `None` if it did not exist.
    pub actual: Option<String>,
}

/// Compare a prediction with the files as they were after the call. Every predicted
/// path must be in `after`; one that is not is not judged.
pub fn check(predicted: &[Written], after: &Files) -> Vec<Divergence> {
    predicted
        .iter()
        .filter_map(|written| {
            let actual = after.get(&written.path)?;
            (actual != &written.text).then(|| Divergence {
                path: written.path.clone(),
                predicted: written.text.clone(),
                actual: actual.clone(),
            })
        })
        .collect()
}

/// Git subcommands that rewrite files in the working tree, whatever they name:
/// `git checkout main` as much as `git checkout a.txt`.
const REWRITES_TREE: &[&str] = &[
    "checkout",
    "switch",
    "stash",
    "reset",
    "pull",
    "rebase",
    "merge",
    "cherry-pick",
    "revert",
    "apply",
    "am",
    "clean",
];

/// The name of a program that may write any file: one the tables do not know,
/// a script run from a file, or git rewriting the working tree.
fn writes_anything(op: &Op, argv: &[String]) -> Option<String> {
    match op {
        Op::Unknown { .. } => Some(
            innermost(argv)
                .first()
                .map_or("", |head| basename(head))
                .to_string(),
        ),
        Op::Run { script } => Some(basename(script).to_string()),
        Op::Opaque { name } => Some(name.clone()),
        Op::Git(GitOp::Other { subcommand }) if REWRITES_TREE.contains(&subcommand.as_str()) => {
            Some(format!("git {subcommand}"))
        }
        _ => None,
    }
}

/// The words a redirect list expands: its file targets.
fn redirect_words(redirects: &[Redirect]) -> impl Iterator<Item = &Word> {
    redirects
        .iter()
        .filter_map(|redirect| match &redirect.target {
            RedirectTarget::File(word) => Some(word),
            _ => None,
        })
}

/// A `[[ ]]` test's operands.
fn test_operands(expr: &TestExpr) -> Vec<&Word> {
    match expr {
        TestExpr::Unary { operand, .. } => vec![operand],
        TestExpr::Binary { left, right, .. } => vec![left, right],
        TestExpr::Not(inner) => test_operands(inner),
        TestExpr::And(a, b) | TestExpr::Or(a, b) => {
            let mut words = test_operands(a);
            words.extend(test_operands(b));
            words
        }
    }
}

/// Whether an unquoted heredoc runs a command as it expands: its body is text
/// here, not a tree, so what it runs is not followed.
fn heredoc_runs(redirects: &[Redirect]) -> bool {
    redirects.iter().any(|redirect| match &redirect.target {
        RedirectTarget::Here(heredoc) => {
            !heredoc.quoted && (heredoc.body.contains("$(") || heredoc.body.contains('`'))
        }
        _ => false,
    })
}

/// Whether a redirect list sends stdout somewhere: `>`, `>>`, `>|`, `&>`,
/// `&>>`, `1>&2`.
fn prints_to_redirect(redirects: &[Redirect]) -> bool {
    redirects.iter().any(|redirect| {
        matches!(
            (redirect.op, redirect.fd),
            (
                RedirectOp::Write | RedirectOp::Append | RedirectOp::Clobber | RedirectOp::DupOut,
                Some(1),
            ) | (
                RedirectOp::Both | RedirectOp::BothAppend | RedirectOp::BothWord,
                _
            )
        )
    })
}

/// Commands that print nothing when they succeed, which the prediction
/// assumes: a builtin that binds or moves, and a file utility without `-v`.
fn quiet(name: Option<&str>, argv: &[Option<String>]) -> bool {
    const BUILTINS: &[&str] = &[
        "cd", "set", "export", "unset", "shift", "local", "declare", "exit", "return",
    ];
    const UTILITIES: &[&str] = &["rm", "mkdir", "cp", "mv", "touch", "chmod", "ln", "rmdir"];
    match name {
        Some(name) if BUILTINS.contains(&name) => true,
        Some(name) if UTILITIES.contains(&name) => argv.iter().skip(1).all(|arg| {
            arg.as_deref().is_some_and(|arg| {
                !(arg.starts_with('-') && !arg.starts_with("--") && arg.contains('v'))
                    && arg != "--verbose"
            })
        }),
        _ => false,
    }
}

/// The directory a glob's matches all lie under — the path up to its first
/// pattern character — or `None` for a path with none.
fn glob_root(path: &str) -> Option<&str> {
    let at = path.find(['*', '?', '['])?;
    Some(path[..at].rfind('/').map_or("", |slash| &path[..slash]))
}
