//! What a command will leave in the files it writes, predicted from its text and
//! the state it is given — see `docs/execution-model.md`, "Two settings, one
//! evaluator".
//!
//! A pure function. It opens nothing and runs nothing: the text of each file it
//! depends on is an argument ([`Files`]), and [`needs`] says which those are. From
//! history nothing is given, and only what the text alone determines is predicted;
//! before a live call the console reads the files and passes them in.
//!
//! **Straight-line shell, in order.** Top-level commands run one after another,
//! and a write replaces or extends what the file held at that point. What this
//! cannot follow — a program whose output is not modelled, a pipeline, a loop, a
//! word with an expansion in it — yields no prediction for the files it writes,
//! and an [`Unfollowed`] that names why. A file it cannot follow is forgotten from
//! then on, so a later append to it is not guessed at either. A program that
//! writes files itself — `sed -i`, `cp`, `rm` — has them named by the shell
//! tables and refused, so no write it knows of goes unmentioned.
//!
//! **The prediction assumes each command succeeds.** A write after `||` is only
//! sometimes made, and is not followed. Whether the call really went that way is
//! for the check after it to say.

use std::collections::BTreeMap;

use crate::shell::Reached;
use crate::shell_files::files_of;
use crate::shell_ops::{basename, classify, resolve, unwrap_command};
use crate::syntax::ast::{
    AndOr, Command, CommandKind, Connector, Item, Pipeline, Redirect, RedirectOp, RedirectTarget,
    Script, SegmentKind, Simple, Tilde, Word,
};
use crate::syntax::print::print_value;

/// What is known of the files a prediction may depend on, by absolute path:
/// `Some(text)`, or `None` for a file that does not exist. A path with no entry is
/// not known at all.
pub type Files = BTreeMap<String, Option<String>>;

/// A file as it will be after the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub path: String,
    pub text: String,
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
    /// Several commands joined by `|`.
    Pipeline,
    /// A write inside a loop, a branch, a group or a function.
    Compound,
    /// Only run when something before it failed: after `||`.
    Sometimes,
    /// Run in the background, so not in order.
    Background,
    /// A descriptor other than stdout into the file, whose text is not modelled.
    Descriptor,
    /// A relative path after a `cd` this could not follow.
    Directory,
}

/// What a command will write, and what it writes that could not be followed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Prediction {
    /// In the order each file was first written.
    pub written: Vec<Written>,
    pub unfollowed: Vec<Unfollowed>,
}

/// The files whose current text `predict` depends on, in the order it asks.
pub fn needs(script: &Script, cwd: &str, home: &str) -> Vec<String> {
    let nothing = Files::new();
    let mut run = Run::new(cwd, home, &nothing);
    run.items(&script.items);
    run.asked
}

/// What `script`, run in `cwd`, will leave in the files it writes.
pub fn predict(script: &Script, cwd: &str, home: &str, files: &Files) -> Prediction {
    let mut run = Run::new(cwd, home, files);
    run.items(&script.items);
    let written = run
        .order
        .iter()
        .filter_map(|path| match run.now.get(path) {
            Some(Held::Text(text)) => Some(Written {
                path: path.clone(),
                text: text.clone(),
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
#[derive(Debug, Clone)]
enum Held {
    Text(String),
    Absent,
    Unknown,
}

struct Run<'a> {
    cwd: Option<String>,
    home: &'a str,
    given: &'a Files,
    /// Files this run has written or found out about, by path.
    now: BTreeMap<String, Held>,
    /// Paths written, in first-write order.
    order: Vec<String>,
    /// Paths whose given text was asked for and not there.
    asked: Vec<String>,
    unfollowed: Vec<Unfollowed>,
}

impl<'a> Run<'a> {
    fn new(cwd: &str, home: &'a str, given: &'a Files) -> Self {
        Self {
            cwd: Some(cwd.to_string()),
            home,
            given,
            now: BTreeMap::new(),
            order: Vec::new(),
            asked: Vec::new(),
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
        if list.background {
            self.forget_list(list, Why::Background);
            return;
        }
        self.pipeline(&list.first);
        let mut sometimes = false;
        for link in &list.rest {
            sometimes |= link.connector == Connector::Or;
            if sometimes {
                self.forget_pipeline(&link.pipeline, Why::Sometimes);
            } else {
                self.pipeline(&link.pipeline);
            }
        }
    }

    fn pipeline(&mut self, pipeline: &Pipeline) {
        match pipeline.commands.as_slice() {
            [only] => self.command(only),
            _ => self.forget_pipeline(pipeline, Why::Pipeline),
        }
    }

    fn command(&mut self, command: &Command) {
        match &command.kind {
            CommandKind::Simple(simple) => self.simple(simple, &command.redirects),
            _ => {
                self.forget_command(command, Why::Compound);
                // A `cd` in a compound that runs in this shell moves it somewhere
                // this did not follow.
                if !matches!(command.kind, CommandKind::Subshell(_)) && moves(command) {
                    self.cwd = None;
                }
            }
        }
    }

    fn simple(&mut self, simple: &Simple, redirects: &[Redirect]) {
        let argv: Vec<Option<String>> = simple.words.iter().map(|w| self.literal(w)).collect();
        let name = argv.first().cloned().flatten();
        if name.as_deref() == Some("cd") {
            self.cwd = match argv.get(1) {
                Some(Some(to)) => self.resolve(to),
                None => Some(self.home.to_string()),
                Some(None) => None,
            };
            return;
        }
        // `tee` writes its input to the files it names, as well as to stdout.
        if name.as_deref() == Some("tee") {
            self.tee(&argv, redirects);
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
            Some(other) => Err(Why::Program(other.to_string())),
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
        // Nothing redirected in: the terminal, or whatever the caller wired up.
        input.unwrap_or(Err(Why::Program("stdin".to_string())))
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

    /// A file's text as this run has left it, or as it was given.
    fn read(&mut self, path: &str) -> Held {
        if let Some(held) = self.now.get(path) {
            return held.clone();
        }
        match self.given.get(path) {
            Some(Some(text)) => Held::Text(text.clone()),
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

    fn forget_pipeline(&mut self, pipeline: &Pipeline, why: Why) {
        for command in &pipeline.commands {
            self.forget_command(command, why.clone());
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
        let written: Vec<String> = files_of(&op, Reached::Always)
            .into_iter()
            .filter(|file| file.write)
            .map(|file| file.path)
            .collect();
        if written.is_empty() {
            return;
        }
        let why = why.unwrap_or_else(|| {
            let program = unwrap_command(&argv)
                .first()
                .map_or("", |head| basename(head));
            Why::Program(program.to_string())
        });
        // A path the text names is forgotten; one that came out of an expansion
        // could be any file, so it is refused without one.
        let named: Vec<String> = literal
            .iter()
            .flatten()
            .filter_map(|word| self.resolve(word))
            .collect();
        for path in written {
            if named.contains(&path) {
                self.write(&path, false, Err(why.clone()));
            } else {
                self.unfollowed.push(Unfollowed {
                    path: None,
                    why: Why::Expansion,
                });
            }
        }
    }

    fn forget_command(&mut self, command: &Command, why: Why) {
        if let CommandKind::Simple(simple) = &command.kind {
            self.forget_program_writes(simple, &command.redirects, Some(why.clone()));
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

    /// A word's value, when the text determines it.
    fn literal(&self, word: &Word) -> Option<String> {
        let mut out = String::new();
        for (at, segment) in word.segments.iter().enumerate() {
            match &segment.kind {
                SegmentKind::Literal(text) => out.push_str(text),
                SegmentKind::Tilde(Tilde::Home) if at == 0 => out.push_str(self.home),
                _ => return None,
            }
        }
        Some(out)
    }

    fn resolve(&self, word: &str) -> Option<String> {
        resolve(word, self.cwd.as_deref(), self.home)
    }
}

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
    pub predicted: String,
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
            (actual.as_deref() != Some(written.text.as_str())).then(|| Divergence {
                path: written.path.clone(),
                predicted: written.text.clone(),
                actual: actual.clone(),
            })
        })
        .collect()
}
