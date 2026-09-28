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
//! nothing since could have changed it. What this
//! cannot follow — a program whose output is not modelled, a branch,
//! a word with an expansion in it — yields no prediction for the files it writes,
//! and an [`Unfollowed`] that names why. A file it cannot follow is forgotten from
//! then on, so a later append to it is not guessed at either. A program that
//! writes files itself — `sed -i`, `cp`, `rm` — has them named by the shell
//! tables and refused, so no write it knows of goes unmentioned. One the tables
//! do not know may have written anything, and nothing known survives it. A
//! script handed to another shell — `bash -c`, `nix-shell --run` — is followed
//! in place.
//!
//! **The prediction assumes each command succeeds.** A write after `||` is only
//! sometimes made, and is not followed. Whether the call really went that way is
//! for the check after it to say.

use std::collections::BTreeMap;

mod python;
mod python_re;
pub mod sed;

use crate::shell::Reached;
use crate::shell_files::files_of;
use crate::shell_ops::{GitOp, Op, basename, classify, innermost, resolve};
use crate::syntax::ast::{
    AndOr, Command, CommandKind, Connector, Item, Pipeline, Redirect, RedirectOp, RedirectTarget,
    Script, SegmentKind, Simple, Tilde, Word,
};
use crate::syntax::embed::{Program, python_of};
use crate::syntax::print::print_value;

/// What is known of the files a prediction may depend on, by absolute path:
/// `Some(text)`, or `None` for a file that does not exist. A path with no entry is
/// not known at all.
pub type Files = BTreeMap<String, Option<String>>;

/// What the evaluator may be shown, asked for when a command needs it. It reads
/// and never runs — `docs/execution-model.md`, "Sight". This crate has no
/// implementation that touches a disk: a [`Files`] answers from what it holds,
/// which is how history (nothing) and a test (a fixture) are shown.
pub trait Sight {
    /// `Some(Some(text))`, `Some(None)` for a file that does not exist, or `None`
    /// when the file cannot be shown.
    fn file(&self, path: &str) -> Option<Option<String>>;
}

impl Sight for Files {
    fn file(&self, path: &str) -> Option<Option<String>> {
        self.get(path).cloned()
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
            other => format!("{other:?}").to_lowercase(),
        }
    }
}

/// What a command will write, and what it writes that could not be followed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Prediction {
    /// In the order each file was first written.
    pub written: Vec<Written>,
    pub unfollowed: Vec<Unfollowed>,
}

/// The files `predict` asked for and was shown nothing of, in the order it asked.
pub fn needs(script: &Script, cwd: &str, home: &str) -> Vec<String> {
    let nothing = Files::new();
    let mut run = Run::new(cwd, home, &nothing);
    run.items(&script.items);
    run.asked
}

/// What `script`, run in `cwd`, will leave in the files it writes.
pub fn predict(script: &Script, cwd: &str, home: &str, sight: &dyn Sight) -> Prediction {
    let mut run = Run::new(cwd, home, sight);
    run.items(&script.items);
    let written = run
        .order
        .iter()
        .filter_map(|path| match run.now.get(path) {
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
        .collect();
    Prediction {
        written,
        unfollowed: run.unfollowed,
    }
}

/// A file as the run has left it so far.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Held {
    Text(String),
    Absent,
    Unknown,
}

struct Run<'a> {
    cwd: Option<String>,
    home: &'a str,
    sight: &'a dyn Sight,
    /// Variables the text bound to a literal, still known to hold it.
    vars: BTreeMap<String, String>,
    /// Inside a pipeline member after the first: stdin is the pipe.
    piped: bool,
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
}

impl<'a> Run<'a> {
    fn new(cwd: &str, home: &'a str, sight: &'a dyn Sight) -> Self {
        Self {
            cwd: Some(cwd.to_string()),
            home,
            sight,
            vars: BTreeMap::new(),
            piped: false,
            now: BTreeMap::new(),
            order: Vec::new(),
            asked: Vec::new(),
            trees: Vec::new(),
            unfollowed: Vec::new(),
        }
    }

    fn items(&mut self, items: &[Item]) {
        for item in items {
            if let Item::List(list) = item {
                self.list(list);
            }
        }
    }

    fn list(&mut self, list: &AndOr) {
        // A background job is a child: its `cd` stays with it.
        if list.background {
            let cwd = self.cwd.clone();
            self.forget_list(list, Why::Background);
            self.cwd = cwd;
            return;
        }
        self.pipeline(&list.first);
        let mut sometimes = false;
        for link in &list.rest {
            sometimes |= link.connector == Connector::Or;
            if sometimes {
                // A `cd` that only sometimes ran leaves the shell somewhere unknown.
                let cwd = self.cwd.clone();
                self.forget_pipeline(&link.pipeline, Why::Sometimes);
                if self.cwd != cwd {
                    self.cwd = None;
                }
            } else {
                self.pipeline(&link.pipeline);
            }
        }
    }

    /// Each member of a longer pipeline runs in a subshell of its own, all at
    /// once: it keeps its `cd` and bindings inside, sees the files as they
    /// were before the pipeline, and reads the pipe as its stdin. A path two
    /// members both change is left unknown, since their order is not. Found
    /// live: a heredoc edit followed by `2>&1 | grep` was refused whole.
    fn pipeline(&mut self, pipeline: &Pipeline) {
        let members = pipeline.commands.as_slice();
        if let [only] = members {
            self.command(only);
            return;
        }
        let (cwd, vars, outer) = (self.cwd.clone(), self.vars.clone(), self.piped);
        let (base_now, base_trees) = (self.now.clone(), self.trees.clone());
        let mut merged = base_now.clone();
        let mut merged_trees = base_trees.clone();
        let mut changed_by: BTreeMap<String, usize> = BTreeMap::new();
        for (at, member) in members.iter().enumerate() {
            self.now = base_now.clone();
            self.trees = base_trees.clone();
            self.piped = outer || at > 0;
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
    }

    fn command(&mut self, command: &Command) {
        match &command.kind {
            CommandKind::Simple(simple) => self.simple(command, simple),
            // A group is its commands; a subshell the same, with its `cd` and
            // its bindings kept inside.
            CommandKind::Group(items) if command.redirects.is_empty() => self.items(items),
            CommandKind::Subshell(items) if command.redirects.is_empty() => {
                let (cwd, vars) = (self.cwd.clone(), self.vars.clone());
                self.items(items);
                self.cwd = cwd;
                self.vars = vars;
            }
            // A loop over words the text spells out runs its body once per word,
            // the variable bound to each; it stays bound to the last, as bash
            // leaves it.
            CommandKind::For(it) if !it.select && command.redirects.is_empty() => {
                let values: Option<Vec<String>> =
                    it.words.iter().map(|w| self.literal(w)).collect();
                match values {
                    Some(values) => {
                        for value in values {
                            self.vars.insert(it.name.clone(), value);
                            self.items(&it.body);
                        }
                    }
                    None => {
                        self.vars.remove(&it.name);
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

    fn simple(&mut self, command: &Command, simple: &Simple) {
        let redirects = command.redirects.as_slice();
        let argv: Vec<Option<String>> = simple.words.iter().map(|w| self.literal(w)).collect();
        let name = argv.first().cloned().flatten();
        if self.change_dir(&argv) {
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
        if name.as_deref().is_some_and(|name| REBINDS.contains(&name)) {
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
        let now = match (text, append) {
            (Ok(text), false) => Ok(text),
            (Ok(text), true) => match self.read(path) {
                Held::Text(before) => Ok(before + &text),
                Held::Absent => Ok(text),
                Held::Unknown => Err(Why::NotRead),
            },
            (Err(why), _) => Err(why),
        };
        match now {
            Ok(text) => {
                self.now.insert(path.to_string(), Held::Text(text));
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
            self.forget_everything(Why::Program(program));
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
        for target in self.outputs(&command.redirects).unwrap_or_default() {
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
        for items in bodies(&command.kind) {
            for item in items {
                if let Item::List(list) = item {
                    self.forget_list(list, why.clone());
                }
            }
        }
    }

    /// A word's value, when the text determines it: literal text, a home tilde,
    /// or a plain `$name` this run knows the binding of.
    fn literal(&self, word: &Word) -> Option<String> {
        let mut out = String::new();
        for (at, segment) in word.segments.iter().enumerate() {
            match &segment.kind {
                SegmentKind::Literal(text) => out.push_str(text),
                SegmentKind::Tilde(Tilde::Home) if at == 0 => out.push_str(self.home),
                SegmentKind::Parameter(parameter)
                    if parameter.subscript.is_none() && parameter.op.is_none() =>
                {
                    out.push_str(self.vars.get(&parameter.name)?);
                }
                _ => return None,
            }
        }
        Some(out)
    }

    fn resolve(&self, word: &str) -> Option<String> {
        resolve(word, self.cwd.as_deref(), self.home)
    }
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

/// Whether a compound runs a `cd` anywhere inside it.
fn moves(command: &Command) -> bool {
    bodies(&command.kind).into_iter().flatten().any(|item| match item {
        Item::List(list) => std::iter::once(&list.first)
            .chain(list.rest.iter().map(|link| &link.pipeline))
            .flat_map(|pipeline| &pipeline.commands)
            .any(|command| match &command.kind {
                CommandKind::Simple(simple) => simple
                    .words
                    .first()
                    .is_some_and(|w| matches!(w.segments.as_slice(), [s] if s.kind == SegmentKind::Literal("cd".to_string()))),
                _ => moves(command),
            }),
        Item::Comment(_) => false,
    })
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

/// The directory a glob's matches all lie under — the path up to its first
/// pattern character — or `None` for a path with none.
fn glob_root(path: &str) -> Option<&str> {
    let at = path.find(['*', '?', '['])?;
    Some(path[..at].rfind('/').map_or("", |slash| &path[..slash]))
}
