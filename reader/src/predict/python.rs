//! Python, followed the way the shell is: straight-line, assuming each call
//! succeeds, against the same files as the commands around it.
//!
//! It interprets what it can name — strings, paths, open files — and what a
//! write depends on that it cannot name makes the write a refusal
//! ([`Why::Python`]), never a guess. A block it does not follow (an `if` it cannot
//! decide, a loop, a function body) has every file it writes refused and
//! forgotten, as a shell loop does. An `assert` it can evaluate as false ends the
//! program there, so nothing after it is written.

use std::collections::{BTreeMap, BTreeSet};

mod json;

use super::python_re;
use super::{Held, Run, Unfollowed, Why};
use crate::syntax::python::ast::{
    Arg, BinOp, BoolOp, CmpOp, Comprehension, Expr, FPart, Module, Params, Singleton, Stmt,
    StmtKind, UnaryOp,
};

/// Methods that write their object to the path they are given: `img.save(p)`.
const SAVES: &[&str] = &["save", "savefig", "to_csv", "to_json", "to_parquet"];

/// How deep a call to a function the program defines is followed into
/// [`Eval::forget`]; a backstop against recursion, not a limit programs meet.
const DEPTH: usize = 8;

/// A list is a `Tuple` held by value: where Python shares one list between
/// names, this holds copies, so a change made through one name leaves the others
/// stale. Every change is therefore followed to every holder
/// ([`Eval::shared`]), or the holders are forgotten.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    None,
    /// A `pathlib.Path`, as its text.
    Path(String),
    /// What `open` returned, by resolved path.
    File {
        path: String,
        mode: Mode,
    },
    /// A module, or a function reached through one, by dotted name: `os.path.join`.
    Name(String),
    /// A function the program defines, by name.
    Defined(String),
    Tuple(Vec<Value>),
    /// A `dict`, its pairs in insertion order, each key once.
    Dict(Vec<(Value, Value)>),
    /// A `set`, each member once. Python gives it no order, so what depends on
    /// one — iterating it, `list` of it — is refused; `sorted` of it is not.
    Set(Vec<Value>),
    /// What `re.compile` returned: the pattern and its flags, compiled where used.
    Pattern {
        source: String,
        flags: i64,
    },
    /// A function this does not follow, as a value: a `lambda` that closes
    /// over a function's names, or takes defaults.
    Callable,
    /// A `lambda` written at module level, by its place among the program's.
    Lambda(usize),
    /// What a match returned: the text searched, each group's span in
    /// characters (`None` for one that took no part), and the groups' names.
    Match {
        text: String,
        spans: python_re::Spans,
        names: Vec<Option<String>>,
    },
    /// Not determined by the text, and why.
    Unknown(Why),
}

/// A function this evaluator knows, read once from its dotted name.
#[derive(Debug, Clone, Copy)]
enum Function {
    Open,
    Path,
    Join,
    ExpandUser,
    Exists,
    ChangeDir,
    Str,
    Len,
    Print,
    Exit,
    /// Runs shell text: `os.system`, `os.popen`.
    Shell,
    /// Runs an argv, or shell text when told `shell=True`, and waits for it.
    Spawn,
    /// Starts a command and does not wait for it: `subprocess.Popen`.
    Concurrent,
    /// Deletes its first argument.
    Delete,
    /// Deletes its first argument and everything under it.
    DeleteTree,
    /// Reads its first argument and writes its second.
    Transfer,
    /// `re.sub` and `re.subn`: a substitution, and with `subn` its count too.
    Substitute {
        counted: bool,
    },
    /// `re.compile`.
    Compile,
    /// `glob.glob(pattern)`: the matching paths, in no order.
    Glob,
    /// `os.listdir(dir)`: the names in it, in no order.
    ListDir,
    /// Makes or removes directories, which changes a listing untracked.
    MakesDirectories,
    /// `re.search`, `re.match`, `re.fullmatch`, `re.finditer`, `re.findall`.
    Matching(Matching),
    /// `re.escape`.
    Escape,
    /// A library call that only reads or computes: handed a path-shaped string,
    /// it writes none of them.
    Pure,
    /// A library that reads the file it is given: `Image.open(p)`.
    Reads,
    /// `dict()`, from nothing, a dict, pairs, or keywords.
    Dict,
    /// `set()`, from nothing or what can be listed.
    Set,
    /// `next(generator[, default])`, `any(...)`, `all(...)`: each stops where
    /// Python's does.
    Next,
    Any,
    All,
    /// `json.load(file)` and `json.loads(text)`.
    JsonLoad,
    JsonLoads,
    /// `json.dump(value, file)` and `json.dumps(value)`.
    JsonDump,
    JsonDumps,
    /// The sequences a loop can be unrolled over, when every element is known.
    Range,
    Enumerate,
    Zip,
    Sorted,
    List,
}

impl Function {
    /// `None` for a function it does not know: a library call.
    fn of(name: &str) -> Option<Self> {
        Some(match name {
            "open" | "io.open" => Function::Open,
            "Path" | "pathlib.Path" => Function::Path,
            "os.path.join" => Function::Join,
            "os.path.expanduser" => Function::ExpandUser,
            "os.path.exists" | "os.path.isfile" => Function::Exists,
            "os.chdir" => Function::ChangeDir,
            "str" => Function::Str,
            "len" => Function::Len,
            "range" => Function::Range,
            "enumerate" => Function::Enumerate,
            "zip" => Function::Zip,
            "sorted" => Function::Sorted,
            "list" | "tuple" => Function::List,
            "set" | "frozenset" => Function::Set,
            "dict" => Function::Dict,
            "next" => Function::Next,
            "any" => Function::Any,
            "all" => Function::All,
            "json.load" => Function::JsonLoad,
            "json.loads" => Function::JsonLoads,
            "json.dump" => Function::JsonDump,
            "json.dumps" => Function::JsonDumps,
            "print" => Function::Print,
            "sys.exit" | "exit" | "quit" | "os._exit" => Function::Exit,
            "os.system" | "os.popen" => Function::Shell,
            "subprocess.run"
            | "subprocess.call"
            | "subprocess.check_call"
            | "subprocess.check_output" => Function::Spawn,
            "subprocess.Popen" => Function::Concurrent,
            "os.remove" | "os.unlink" => Function::Delete,
            "shutil.rmtree" => Function::DeleteTree,
            "Image.open" | "PIL.Image.open" | "wave.open" => Function::Reads,
            "glob.glob" | "glob.iglob" => Function::Glob,
            "os.listdir" => Function::ListDir,
            "os.mkdir" | "os.makedirs" | "os.rmdir" | "os.removedirs" | "os.symlink"
            | "shutil.copytree" => Function::MakesDirectories,
            "os.scandir" | "os.walk" | "os.path.basename" | "os.path.dirname"
            | "os.path.splitext" | "os.path.isdir" | "os.path.relpath" | "os.getcwd"
            | "os.environ.get" | "sys.path.insert" | "sys.path.append" | "re.split"
            | "shlex.quote" | "shlex.split" | "textwrap.dedent" => Function::Pure,
            "re.sub" => Function::Substitute { counted: false },
            "re.subn" => Function::Substitute { counted: true },
            "re.compile" => Function::Compile,
            "re.search" => Function::Matching(Matching::Find(python_re::Anchor::Anywhere)),
            "re.match" => Function::Matching(Matching::Find(python_re::Anchor::Start)),
            "re.fullmatch" => Function::Matching(Matching::Find(python_re::Anchor::Whole)),
            "re.finditer" => Function::Matching(Matching::Iter),
            "re.findall" => Function::Matching(Matching::All),
            "re.escape" => Function::Escape,
            "os.rename" | "os.replace" | "shutil.move" | "shutil.copy" | "shutil.copy2"
            | "shutil.copyfile" => Function::Transfer,
            _ => return None,
        })
    }
}

/// A run of the program as an `if` found it: what [`Eval::both`] runs each arm
/// from.
#[derive(Clone)]
struct State<'a> {
    names: BTreeMap<String, Value>,
    frames: Vec<Frame>,
    /// Each function's name and whether a call to it is followed.
    functions: BTreeMap<String, bool>,
    defaults: BTreeMap<String, BTreeMap<String, Value>>,
    lambdas: usize,
    cwd: Option<String>,
    maybe_exited: bool,
    maybe_jumped: bool,
    shell: Run<'a>,
}

/// Names two arms left: alike, they stand; bound differently, or in one arm
/// only, they are unknown.
fn join_names(mine: &mut BTreeMap<String, Value>, theirs: BTreeMap<String, Value>, why: &Why) {
    let names: BTreeSet<String> = mine.keys().chain(theirs.keys()).cloned().collect();
    for name in names {
        if mine.get(&name) != theirs.get(&name) {
            mine.insert(name, Value::Unknown(why.clone()));
        }
    }
}

/// What a matching function returns: one match or `None`, every match, or
/// every match's text.
#[derive(Debug, Clone, Copy)]
enum Matching {
    Find(python_re::Anchor),
    Iter,
    All,
}

/// The parts of a path read as attributes: `p.parent`, `p.name`.
enum PathPart {
    Parent,
    Name,
}

impl PathPart {
    fn of(attr: &str) -> Option<Self> {
        match attr {
            "parent" => Some(PathPart::Parent),
            "name" => Some(PathPart::Name),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Read,
    /// Writes, the file already emptied or created by the `open`.
    Write,
}

/// Why a block stopped before its end.
enum Stop {
    /// It raised, or exited: nothing after this runs.
    Ended,
    Return(Value),
    Break,
    Continue,
}

/// How far a comprehension is run: to the end, or to the first element
/// (`next`), the first true one (`any`) or the first false one (`all`).
#[derive(Clone, Copy, PartialEq)]
enum Consumed {
    Whole,
    First,
    UntilTrue,
    UntilFalse,
}

/// What running a comprehension came to.
enum Ran {
    /// Every element it produced, each as the values of its parts.
    Rows(Vec<Vec<Value>>),
    /// Something in it is not known.
    Unknown(Value),
}

/// How many elements a comprehension is run for at most.
const MAX_ELEMENTS: usize = 10_000;

/// How many values a loop over a written-out list is run for, the backstop
/// [`crate::project`] uses for the shell's.
const MAX_UNROLL: usize = 256;

pub(super) fn run(shell: &mut Run<'_>, module: &Module) {
    let _ = Eval::new(shell).block(&module.body);
}

/// Every file `module` could write is refused for `why`: it runs in a place the
/// shell around it does not follow.
pub(super) fn forget(shell: &mut Run<'_>, module: &Module, why: &Why) {
    Eval::new(shell).forget(&module.body, why, 0);
}

struct Eval<'r, 'a, 'm> {
    shell: &'r mut Run<'a>,
    /// Python's own directory: `os.chdir` moves it and not the shell's.
    cwd: Option<String>,
    /// The module's names.
    names: BTreeMap<String, Value>,
    /// The calls being followed, innermost last.
    frames: Vec<Frame>,
    functions: BTreeMap<String, Def<'m>>,
    /// Each function's default arguments by parameter, evaluated once where it
    /// was defined, as Python does.
    defaults: BTreeMap<String, BTreeMap<String, Value>>,
    /// The module-level lambdas, as [`Value::Lambda`] numbers them.
    lambdas: Vec<(&'m Params, &'m Expr)>,
    /// A block not followed may have left with success — `sys.exit(0)`, a
    /// bare `exit()`: what follows runs only sometimes, to the end.
    maybe_exited: bool,
    /// A block not followed may have jumped — `return`, `break`,
    /// `continue`: what follows runs only sometimes, until the function or
    /// the loop it jumps out of ends.
    maybe_jumped: bool,
}

/// One call's names, and the ones it declared `global`.
#[derive(Default, Clone)]
struct Frame {
    locals: BTreeMap<String, Value>,
    globals: BTreeSet<String>,
}

/// A function the program defines.
#[derive(Clone, Copy)]
struct Def<'m> {
    params: &'m Params,
    body: &'m [Stmt],
    /// A call to it is not followed: a decorator may change what it does, it
    /// was defined in a block this does not know ran, or inside a function
    /// whose names it may read.
    unfollowed: bool,
}

fn construct(name: &str) -> Why {
    Why::Python(name.to_string())
}

impl<'r, 'a, 'm> Eval<'r, 'a, 'm> {
    fn new(shell: &'r mut Run<'a>) -> Self {
        let cwd = shell.cwd.clone();
        Self {
            shell,
            cwd,
            names: BTreeMap::from([("__name__".to_string(), Value::Str("__main__".to_string()))]),
            frames: Vec::new(),
            functions: BTreeMap::new(),
            defaults: BTreeMap::new(),
            lambdas: Vec::new(),
            maybe_exited: false,
            maybe_jumped: false,
        }
    }

    /// Binds `name` where Python would: in the call being followed unless it
    /// declared the name `global`, else in the module.
    fn set(&mut self, name: &str, value: Value) {
        match self.frames.last_mut() {
            Some(frame) if !frame.globals.contains(name) => {
                frame.locals.insert(name.to_string(), value);
            }
            _ => {
                self.names.insert(name.to_string(), value);
            }
        }
    }

    fn unset(&mut self, name: &str) {
        match self.frames.last_mut() {
            Some(frame) if !frame.globals.contains(name) => {
                frame.locals.remove(name);
            }
            _ => {
                self.names.remove(name);
            }
        }
    }

    fn block(&mut self, body: &'m [Stmt]) -> Result<(), Stop> {
        for (at, stmt) in body.iter().enumerate() {
            if self.maybe_exited || self.maybe_jumped {
                self.forget(&body[at..], &construct("after a jump"), 0);
                return Ok(());
            }
            self.stmt(stmt)?;
        }
        Ok(())
    }

    fn stmt(&mut self, stmt: &'m Stmt) -> Result<(), Stop> {
        match &stmt.kind {
            StmtKind::Expr(expr) => {
                self.expr(expr)?;
            }
            StmtKind::Assign { targets, value } => {
                let value = self.expr(value)?;
                for target in targets {
                    match target {
                        Expr::Subscript {
                            value: container,
                            index,
                        } => self.store(container, index, value.clone())?,
                        _ => self.bind(target, value.clone()),
                    }
                }
            }
            StmtKind::AugAssign { target, op, value } => {
                let right = self.expr(value)?;
                match target {
                    Expr::Name(name) => {
                        let left = self.name(name);
                        let joined = binary(left.clone(), *op, right);
                        // `xs += ys` extends the list every holder shares.
                        if let (Value::Tuple(_), BinOp::Add) = (&left, op) {
                            let why = construct("list +=");
                            self.changed(target, &left, Some(joined), &why);
                        } else {
                            self.set(name, joined);
                        }
                    }
                    Expr::Subscript { value: owner, .. } | Expr::Attribute { value: owner, .. } => {
                        let old = self.expr(owner)?;
                        self.escape(&right, &construct("item assignment"));
                        self.changed(owner, &old, None, &construct("item assignment"));
                    }
                    _ => {}
                }
            }
            StmtKind::Import(aliases) => {
                for alias in aliases {
                    match &alias.asname {
                        Some(asname) => {
                            self.set(asname, Value::Name(alias.name.clone()));
                        }
                        None => {
                            let head = alias.name.split('.').next().unwrap_or(&alias.name);
                            self.set(head, Value::Name(head.to_string()));
                        }
                    }
                }
            }
            StmtKind::ImportFrom {
                module,
                level,
                names,
            } => {
                for alias in names.iter().filter(|alias| alias.name != "*") {
                    let dotted = match (module, level) {
                        (Some(module), 0) => format!("{module}.{}", alias.name),
                        _ => alias.name.clone(),
                    };
                    let bound = alias.asname.as_ref().unwrap_or(&alias.name);
                    self.set(bound, Value::Name(dotted));
                }
            }
            StmtKind::Assert { test, .. } => {
                if let Value::Bool(false) = truth(self.expr(test)?) {
                    return Err(Stop::Ended);
                }
            }
            StmtKind::Raise { exc, .. } => {
                // What it raises is built first, and building it may write.
                if let Some(exc) = exc {
                    self.expr(exc)?;
                }
                return Err(Stop::Ended);
            }
            StmtKind::Delete(targets) => {
                for target in targets {
                    self.delete(target)?;
                }
            }
            StmtKind::If { test, body, orelse } => {
                let tested = self.expr(test)?;
                match truth(tested.clone()) {
                    Value::Bool(true) => self.block(body)?,
                    Value::Bool(false) => self.block(orelse)?,
                    // Named for why the test is not known: that is what to build.
                    // Both arms, joined, where they end alike.
                    _ if self.both(
                        body,
                        orelse,
                        &construct(&format!("if on {}", cause(&tested))),
                    ) => {}
                    _ => {
                        let why = construct(&format!("if on {}", cause(&tested)));
                        self.forget(body, &why, 0);
                        self.forget(orelse, &why, 0);
                        self.note_jumps(body, false);
                        self.note_jumps(orelse, false);
                    }
                }
            }
            StmtKind::For {
                body,
                orelse,
                target,
                iter,
            } => {
                let over = self.expr(iter)?;
                match self.iterable(over) {
                    Value::Tuple(values) if values.len() <= MAX_UNROLL => {
                        // A possible `break` or `continue` in it ends with it.
                        let outer = std::mem::replace(&mut self.maybe_jumped, false);
                        let broke = self.unrolled(target, values, body);
                        let possibly = std::mem::replace(&mut self.maybe_jumped, outer);
                        if broke? {
                            return Ok(());
                        }
                        // After a possible `break`, `else` only sometimes runs.
                        if possibly {
                            self.forget(orelse, &construct("after a jump"), 0);
                            self.note_jumps(orelse, false);
                        } else {
                            self.block(orelse)?;
                        }
                    }
                    unlisted => {
                        let why = construct(&format!("for over {}", cause(&unlisted)));
                        self.unbind(target, &why);
                        self.forget(body, &why, 0);
                        self.forget(orelse, &why, 0);
                        self.note_jumps(body, true);
                        self.note_jumps(orelse, false);
                    }
                }
            }
            // Its test runs too, as often as the body does: what it calls or
            // binds is forgotten with the body.
            StmtKind::While { test, body, orelse } => {
                self.forget_expr(test, &construct("while"), 0);
                self.forget(body, &construct("while"), 0);
                self.forget(orelse, &construct("while"), 0);
                self.note_jumps(body, true);
                self.note_jumps(orelse, false);
            }
            StmtKind::With { items, body } => {
                for item in items {
                    let value = self.expr(&item.context)?;
                    if let Some(var) = &item.var {
                        self.bind(var, value);
                    }
                }
                self.block(body)?;
            }
            // Its decorators and defaults run once, here.
            StmtKind::FunctionDef {
                name,
                params,
                body,
                decorators,
            } => {
                for decorator in decorators {
                    self.forget_expr(decorator, &construct("decorator"), 0);
                }
                let mut defaults = BTreeMap::new();
                for param in params.args.iter().chain(&params.kwonly) {
                    if let Some(default) = &param.default {
                        let value = match (default, self.expr(default)?) {
                            // A tuple of plain values cannot change.
                            (Expr::Tuple(_), Value::Tuple(items))
                                if !items.iter().any(|item| matches!(item, Value::Tuple(_))) =>
                            {
                                Value::Tuple(items)
                            }
                            // One list every call shares: which calls changed
                            // it is not followed.
                            (_, Value::Tuple(_)) => Value::Unknown(construct("mutable default")),
                            (_, value) => value,
                        };
                        defaults.insert(param.name.clone(), value);
                    }
                }
                self.defaults.insert(name.clone(), defaults);
                self.define(name, params, body, decorators);
            }
            // A raise the handlers may catch ends the body there; what the
            // handlers then do is not followed.
            StmtKind::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                match self.block(body) {
                    Ok(()) => self.block(orelse)?,
                    Err(Stop::Ended) if !handlers.is_empty() => {
                        for handler in handlers {
                            self.forget(&handler.body, &construct("except"), 0);
                            self.note_jumps(&handler.body, false);
                        }
                    }
                    Err(stop) => {
                        self.block(finalbody)?;
                        return Err(stop);
                    }
                }
                self.block(finalbody)?;
            }
            StmtKind::Global(names) => {
                if let Some(frame) = self.frames.last_mut() {
                    frame.globals.extend(names.iter().cloned());
                }
            }
            StmtKind::Return(value) => {
                let value = match value {
                    Some(value) => self.expr(value)?,
                    None => Value::None,
                };
                return Err(Stop::Return(value));
            }
            StmtKind::Break => return Err(Stop::Break),
            StmtKind::Continue => return Err(Stop::Continue),
            StmtKind::Comment(_) | StmtKind::Pass | StmtKind::Nonlocal(_) => {}
        }
        Ok(())
    }

    fn bind(&mut self, target: &Expr, value: Value) {
        match (target, value) {
            (Expr::Name(name), value) => {
                self.set(name, value);
            }
            (Expr::Tuple(targets) | Expr::List(targets), Value::Tuple(values))
                if targets.len() == values.len() =>
            {
                for (target, value) in targets.iter().zip(values) {
                    self.bind(target, value);
                }
            }
            (Expr::Tuple(targets) | Expr::List(targets), _) => {
                for target in targets {
                    self.unbind(target, &construct("unpacking"));
                }
            }
            // An item or an attribute: whatever list holds it has changed, and
            // the value may now be reached through it.
            (target, value) => {
                let why = construct("item assignment");
                self.escape(&value, &why);
                if let Some(name) = root(target) {
                    let held = self.name(name);
                    self.changed(&Expr::Name(name.to_string()), &held, None, &why);
                }
            }
        }
    }

    fn delete(&mut self, target: &'m Expr) -> Result<(), Stop> {
        match target {
            Expr::Name(name) => self.unset(name),
            Expr::Subscript {
                value: container,
                index,
            } => self.remove_items(container, index)?,
            Expr::Attribute { value: owner, .. } => {
                let old = self.expr(owner)?;
                self.changed(owner, &old, None, &construct("del"));
            }
            // `del a[0], b`: each in turn.
            Expr::Tuple(targets) | Expr::List(targets) => {
                for target in targets {
                    self.delete(target)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// `container[index] = value`, followed on a list this knows at an index or
    /// a step-less slice it can compute; otherwise the list is forgotten.
    fn store(&mut self, container: &'m Expr, index: &'m Expr, value: Value) -> Result<(), Stop> {
        let old = self.expr(container)?;
        let why = construct("item assignment");
        if let Value::Dict(pairs) = &old {
            let at = self.expr(index)?;
            let now = key(&at).map(|_| Value::Dict(put(pairs.clone(), at, value.clone())));
            if now.is_none() {
                self.escape(&value, &why);
            }
            self.changed(container, &old, now, &why);
            return Ok(());
        }
        let Value::Tuple(items) = &old else {
            self.escape(&value, &why);
            self.changed(container, &old, None, &why);
            return Ok(());
        };
        let now = match index {
            Expr::Slice {
                lower,
                upper,
                step: None,
            } => {
                let (lower, upper) = self.bounds(lower, upper)?;
                match (
                    bound(lower, items.len(), 0),
                    bound(upper, items.len(), items.len()),
                    &value,
                ) {
                    (Ok(from), Ok(to), Value::Tuple(put)) => {
                        let mut now = items[..from].to_vec();
                        now.extend(put.iter().cloned());
                        now.extend(items[to.max(from)..].iter().cloned());
                        Some(now)
                    }
                    _ => None,
                }
            }
            index => match self.expr(index)? {
                Value::Int(at) => {
                    let at = position(items.len(), at)?;
                    let mut now = items.clone();
                    now[at] = value.clone();
                    Some(now)
                }
                _ => None,
            },
        };
        if now.is_none() {
            self.escape(&value, &why);
        }
        self.changed(container, &old, now.map(Value::Tuple), &why);
        Ok(())
    }

    /// `del container[index]`, followed as [`Self::store`] is.
    fn remove_items(&mut self, container: &'m Expr, index: &'m Expr) -> Result<(), Stop> {
        let old = self.expr(container)?;
        let why = construct("del");
        if let Value::Dict(pairs) = &old {
            let at = self.expr(index)?;
            let now = match find_key(pairs, &at) {
                Some(found) => {
                    let mut now = pairs.clone();
                    now.remove(found);
                    Some(Value::Dict(now))
                }
                None if key(&at).is_some() => return Err(Stop::Ended),
                None => None,
            };
            self.changed(container, &old, now, &why);
            return Ok(());
        }
        let Value::Tuple(items) = &old else {
            self.changed(container, &old, None, &why);
            return Ok(());
        };
        let now = match index {
            Expr::Slice {
                lower,
                upper,
                step: None,
            } => {
                let (lower, upper) = self.bounds(lower, upper)?;
                match (
                    bound(lower, items.len(), 0),
                    bound(upper, items.len(), items.len()),
                ) {
                    (Ok(from), Ok(to)) => {
                        let mut now = items[..from].to_vec();
                        now.extend(items[to.max(from)..].iter().cloned());
                        Some(now)
                    }
                    _ => None,
                }
            }
            index => match self.expr(index)? {
                Value::Int(at) => {
                    let at = position(items.len(), at)?;
                    let mut now = items.clone();
                    now.remove(at);
                    Some(now)
                }
                _ => None,
            },
        };
        self.changed(container, &old, now.map(Value::Tuple), &why);
        Ok(())
    }

    fn bounds(
        &mut self,
        lower: &'m Option<Box<Expr>>,
        upper: &'m Option<Box<Expr>>,
    ) -> Result<(Option<Value>, Option<Value>), Stop> {
        let mut side = |part: &'m Option<Box<Expr>>| -> Result<Option<Value>, Stop> {
            part.as_deref().map(|part| self.expr(part)).transpose()
        };
        Ok((side(lower)?, side(upper)?))
    }

    /// A list changed in place through `target`, which held `old`. Every holder
    /// of `old`, under any name in any frame, is forgotten; then `target`, when
    /// it is a name, holds `now` if the change was followed. A change through an
    /// item or an attribute forgets the name it starts from.
    fn changed(&mut self, target: &Expr, old: &Value, now: Option<Value>, why: &Why) {
        self.shared(old, why);
        // A dict's views change with it: what holds one is forgotten too.
        if let Value::Dict(pairs) = old {
            for view in views(pairs) {
                self.shared(&view, why);
            }
        }
        match (target, now) {
            (Expr::Name(name), Some(now)) => self.set(name, now),
            _ => {
                if let Some(name) = root(target) {
                    let held = self.name(name);
                    if container(&held) {
                        self.shared(&held, why);
                        self.set(name, Value::Unknown(why.clone()));
                    }
                }
            }
        }
    }

    /// Forgets every binding that holds `list` or holds something holding it.
    /// Equal lists that are not the same one are forgotten too: this cannot tell
    /// them apart.
    fn shared(&mut self, list: &Value, why: &Why) {
        if !container(list) {
            return;
        }
        let frames = self.frames.iter_mut().map(|frame| &mut frame.locals);
        for bindings in std::iter::once(&mut self.names).chain(frames) {
            for held in bindings.values_mut() {
                if holds(held, list) {
                    *held = Value::Unknown(why.clone());
                }
            }
        }
    }

    /// `value` is now reachable from something this does not follow, which may
    /// change any list in it at any later point.
    fn escape(&mut self, value: &Value, why: &Why) {
        match value {
            Value::Tuple(items) => {
                self.shared(value, why);
                for item in items {
                    self.escape(item, why);
                }
            }
            Value::Dict(pairs) => {
                self.shared(value, why);
                for (_, item) in pairs {
                    self.escape(item, why);
                }
            }
            Value::Set(_) => self.shared(value, why),
            _ => {}
        }
    }

    /// Everything a run of this program holds, for an `if` to run both arms from.
    fn state(&self) -> State<'a> {
        State {
            names: self.names.clone(),
            frames: self.frames.clone(),
            functions: self
                .functions
                .keys()
                .map(|name| (name.clone(), self.functions[name].unfollowed))
                .collect(),
            defaults: self.defaults.clone(),
            lambdas: self.lambdas.len(),
            cwd: self.cwd.clone(),
            maybe_exited: self.maybe_exited,
            maybe_jumped: self.maybe_jumped,
            shell: self.shell.clone(),
        }
    }

    fn restore(&mut self, state: State<'a>) {
        self.names = state.names;
        self.frames = state.frames;
        self.functions
            .retain(|name, _| state.functions.contains_key(name));
        for (name, unfollowed) in &state.functions {
            if let Some(def) = self.functions.get_mut(name) {
                def.unfollowed = *unfollowed;
            }
        }
        self.defaults = state.defaults;
        self.lambdas.truncate(state.lambdas);
        self.cwd = state.cwd;
        self.maybe_exited = state.maybe_exited;
        self.maybe_jumped = state.maybe_jumped;
        *self.shell = state.shell;
    }

    /// Both arms of an `if` this cannot decide, each from the state before
    /// it, joined: a name they bind alike keeps its value and one they bind
    /// differently is unknown; files join as the shell's do. `false`, with
    /// nothing done, where the arms end differently — a raise, a return, a
    /// jump — or make a function or lambda, which a join does not hold.
    fn both(&mut self, body: &'m [Stmt], orelse: &'m [Stmt], why: &Why) -> bool {
        let defines = |block: &[Stmt]| {
            block
                .iter()
                .any(|stmt| matches!(stmt.kind, StmtKind::FunctionDef { .. }))
        };
        if defines(body) || defines(orelse) {
            return false;
        }
        let before = self.state();
        let then = self.block(body);
        let taken = self.state();
        let before_shell = before.shell.clone();
        self.restore(before.clone());
        let otherwise = self.block(orelse);
        let fits = then.is_ok()
            && otherwise.is_ok()
            && taken.lambdas == before.lambdas
            && self.lambdas.len() == before.lambdas
            && taken.frames.len() == self.frames.len()
            && taken.maybe_exited == self.maybe_exited
            && taken.maybe_jumped == self.maybe_jumped;
        if !fits {
            self.restore(before);
            return false;
        }
        self.shell.join(taken.shell, &before_shell, why);
        join_names(&mut self.names, taken.names, why);
        for (mine, theirs) in self.frames.iter_mut().zip(taken.frames) {
            join_names(&mut mine.locals, theirs.locals, why);
            mine.globals.extend(theirs.globals);
        }
        self.defaults
            .retain(|name, held| taken.defaults.get(name) == Some(held));
        if self.cwd != taken.cwd {
            self.cwd = None;
        }
        // A function one arm defined, deeper in it, is not there in the other.
        for (name, def) in self.functions.iter_mut() {
            if !before.functions.contains_key(name) {
                def.unfollowed = true;
            }
        }
        true
    }

    /// A loop's elements, its body run for each: `true` when it broke.
    fn unrolled(
        &mut self,
        target: &Expr,
        values: Vec<Value>,
        body: &'m [Stmt],
    ) -> Result<bool, Stop> {
        for value in values {
            self.bind(target, value);
            match self.block(body) {
                Ok(()) | Err(Stop::Continue) => {}
                Err(Stop::Break) => return Ok(true),
                Err(stop) => return Err(stop),
            }
        }
        Ok(false)
    }

    /// A block not followed may have jumped out of what holds it, or left the
    /// program with success; what follows then runs only sometimes. A
    /// `raise`, or an exit with a failing status, fails the call, which the
    /// check then skips: the success assumed excludes it.
    fn note_jumps(&mut self, body: &[Stmt], in_loop: bool) {
        let (exits, jumps) = jumps_in(body, in_loop);
        self.maybe_exited |= exits;
        self.maybe_jumped |= jumps;
    }

    /// Runs a comprehension as Python does, its loop variables bound only
    /// inside it: `parts` evaluated for each element its generators produce,
    /// stopping where `consumed` says.
    fn comprehension(
        &mut self,
        parts: &[&'m Expr],
        generators: &'m [Comprehension],
        consumed: Consumed,
    ) -> Result<Ran, Stop> {
        let mut names = BTreeSet::new();
        for generator in generators {
            names_in(&generator.target, &mut names);
        }
        let saved: Vec<(String, Option<Value>)> = names
            .into_iter()
            .map(|name| {
                let held = self.bound(&name);
                (name, held)
            })
            .collect();
        let mut rows = Vec::new();
        let mut budget = MAX_ELEMENTS;
        let ran = self.produce(parts, generators, consumed, &mut rows, &mut budget);
        for (name, held) in saved {
            match held {
                Some(value) => self.set(&name, value),
                None => self.unset(&name),
            }
        }
        Ok(match ran? {
            Some(why) => Ran::Unknown(why),
            None => Ran::Rows(rows),
        })
    }

    /// One generator's elements, and under each the generators after it.
    /// `Some` with the unknown value when one is not known, and `Ok(None)` when
    /// it ran, to the end or to where `consumed` stops it (then `rows` says).
    fn produce(
        &mut self,
        parts: &[&'m Expr],
        generators: &'m [Comprehension],
        consumed: Consumed,
        rows: &mut Vec<Vec<Value>>,
        budget: &mut usize,
    ) -> Result<Option<Value>, Stop> {
        let Some((generator, rest)) = generators.split_first() else {
            let mut row = Vec::with_capacity(parts.len());
            for part in parts {
                row.push(self.expr(part)?);
            }
            // Where `any` or `all` stops depends on this element's truth.
            if matches!(consumed, Consumed::UntilTrue | Consumed::UntilFalse)
                && let undecided @ Value::Unknown(_) = truth(row[0].clone())
            {
                return Ok(Some(undecided));
            }
            rows.push(row);
            return Ok(None);
        };
        let over = self.expr(&generator.iter)?;
        let items = match self.iterable(over) {
            Value::Tuple(items) => items,
            unlisted => return Ok(Some(unlisted)),
        };
        for item in items {
            if *budget == 0 {
                return Ok(Some(Value::Unknown(construct("comprehension length"))));
            }
            *budget -= 1;
            self.bind(&generator.target, item);
            let mut passes = true;
            for test in &generator.ifs {
                match truth(self.expr(test)?) {
                    Value::Bool(true) => {}
                    Value::Bool(false) => {
                        passes = false;
                        break;
                    }
                    other => return Ok(Some(other)),
                }
            }
            if !passes {
                continue;
            }
            let before = rows.len();
            if let Some(unknown) = self.produce(parts, rest, consumed, rows, budget)? {
                return Ok(Some(unknown));
            }
            if consumed != Consumed::Whole && rows.len() > before && self.stopped_at(rows, consumed)
            {
                return Ok(None);
            }
        }
        Ok(None)
    }

    /// Whether the last row ends a comprehension consumed partly.
    fn stopped_at(&self, rows: &[Vec<Value>], consumed: Consumed) -> bool {
        let Some(last) = rows.last() else {
            return false;
        };
        match consumed {
            Consumed::Whole => false,
            Consumed::First => true,
            Consumed::UntilTrue => matches!(truth(last[0].clone()), Value::Bool(true)),
            Consumed::UntilFalse => matches!(truth(last[0].clone()), Value::Bool(false)),
        }
    }

    /// A name's value in the scope `set` writes to, if bound there.
    fn bound(&self, name: &str) -> Option<Value> {
        match self.frames.last() {
            Some(frame) if !frame.globals.contains(name) => frame.locals.get(name).cloned(),
            _ => self.names.get(name).cloned(),
        }
    }

    /// A generator written as a call's argument, consumed as that call does:
    /// its rows, or `Unknown` when it is not known.
    fn generator(&mut self, expr: &'m Expr, consumed: Consumed) -> Result<Value, Stop> {
        let Expr::GeneratorExp { elt, generators } = expr else {
            return self.expr(expr);
        };
        Ok(match self.comprehension(&[elt], generators, consumed)? {
            Ran::Rows(rows) => {
                Value::Tuple(rows.into_iter().map(|mut row| row.remove(0)).collect())
            }
            Ran::Unknown(value) => {
                self.escape_comprehension(expr, &construct("comprehension"));
                self.forget_expr(expr, &construct("comprehension"), 0);
                unknown(&value, "comprehension")
            }
        })
    }

    /// A comprehension shares the elements of what it ranges over, not the
    /// list itself: `[l.strip() for l in lines]` leaves `lines` known. Names
    /// read anywhere else in it escape whole.
    /// A comprehension this could not run: what it reads escapes and what it
    /// calls is forgotten, and its value is unknown for the reason it stopped.
    fn unfollowed_comprehension(&mut self, expr: &'m Expr, stopped: &Value) -> Value {
        let why = construct(&format!("comprehension over {}", cause(stopped)));
        self.escape_comprehension(expr, &why);
        self.forget_expr(expr, &why, 0);
        Value::Unknown(why)
    }

    fn escape_comprehension(&mut self, expr: &Expr, why: &Why) {
        let (parts, generators): (Vec<&Expr>, &[Comprehension]) = match expr {
            Expr::ListComp { elt, generators }
            | Expr::SetComp { elt, generators }
            | Expr::GeneratorExp { elt, generators } => (vec![elt], generators),
            Expr::DictComp {
                key,
                value,
                generators,
            } => (vec![key, value], generators),
            other => return self.escape_names(other, why),
        };
        for part in parts {
            self.escape_names(part, why);
        }
        for generator in generators {
            match ranged_over(&generator.iter) {
                Some(names) => {
                    for name in names {
                        if let Value::Tuple(items) = self.name(name) {
                            for item in &items {
                                self.escape(item, why);
                            }
                        }
                    }
                }
                None => self.escape_names(&generator.iter, why),
            }
            for test in &generator.ifs {
                self.escape_names(test, why);
            }
        }
    }

    /// Every list this knows is forgotten: code it does not follow ran.
    fn forget_lists(&mut self, why: &Why) {
        let frames = self.frames.iter_mut().map(|frame| &mut frame.locals);
        for bindings in std::iter::once(&mut self.names).chain(frames) {
            for held in bindings.values_mut() {
                if container(held) {
                    *held = Value::Unknown(why.clone());
                }
            }
        }
    }

    /// The lists named inside an expression this does not evaluate escape into
    /// its value; a function the program defines, or a lambda, called there may
    /// change any list.
    fn escape_names(&mut self, expr: &Expr, why: &Why) {
        let mut names = Vec::new();
        names_read(expr, &mut names);
        for name in names {
            match self.name(&name) {
                Value::Defined(_) | Value::Callable | Value::Lambda(_) => self.forget_lists(why),
                held => self.escape(&held, why),
            }
        }
    }

    fn unbind(&mut self, target: &Expr, why: &Why) {
        match target {
            Expr::Name(name) => {
                self.set(name, Value::Unknown(why.clone()));
            }
            Expr::Tuple(targets) | Expr::List(targets) => {
                for target in targets {
                    self.unbind(target, why);
                }
            }
            Expr::Starred(inner) => self.unbind(inner, why),
            // `a, x[0] = f()`: the list `x` starts from has changed.
            target => {
                if let Some(name) = root(target) {
                    let held = self.name(name);
                    self.changed(&Expr::Name(name.to_string()), &held, None, why);
                }
            }
        }
    }

    fn name(&self, name: &str) -> Value {
        let local = self
            .frames
            .last()
            .filter(|frame| !frame.globals.contains(name))
            .and_then(|frame| frame.locals.get(name));
        match local.or_else(|| self.names.get(name)) {
            Some(value) => value.clone(),
            None => Value::Name(name.to_string()),
        }
    }

    fn define(&mut self, name: &str, params: &'m Params, body: &'m [Stmt], decorators: &[Expr]) {
        self.functions.insert(
            name.to_string(),
            Def {
                params,
                body,
                // Defined inside a function, it reads that function's names,
                // which a call's own frame does not hold.
                unfollowed: !decorators.is_empty() || !self.frames.is_empty(),
            },
        );
        self.set(name, Value::Defined(name.to_string()));
    }

    fn expr(&mut self, expr: &'m Expr) -> Result<Value, Stop> {
        Ok(match expr {
            Expr::Name(name) => self.name(name),
            Expr::Str(text) => Value::Str(text.clone()),
            Expr::Number(text) => text
                .parse()
                .map_or(Value::Unknown(construct("number")), Value::Int),
            Expr::Singleton(Singleton::True) => Value::Bool(true),
            Expr::Singleton(Singleton::False) => Value::Bool(false),
            Expr::Singleton(Singleton::None) => Value::None,
            Expr::FString(parts) => self.fstring(parts)?,
            Expr::Attribute { value, attr } => {
                let value = self.expr(value)?;
                self.attribute(value, attr)
            }
            Expr::Call { func, args } => self.call(func, args)?,
            Expr::BinOp { left, op, right } => {
                let left = self.expr(left)?;
                let right = self.expr(right)?;
                binary(left, *op, right)
            }
            Expr::UnaryOp {
                op: UnaryOp::Not,
                operand,
            } => match truth(self.expr(operand)?) {
                Value::Bool(b) => Value::Bool(!b),
                other => other,
            },
            Expr::BoolOp { op, values } => {
                let mut last = Value::Bool(matches!(op, BoolOp::And));
                for value in values {
                    last = self.expr(value)?;
                    match (op, truth(last.clone())) {
                        (BoolOp::And, Value::Bool(false)) | (BoolOp::Or, Value::Bool(true)) => {
                            break;
                        }
                        (_, Value::Bool(_)) => {}
                        (_, other) => return Ok(other),
                    }
                }
                last
            }
            Expr::Compare { left, rest } => {
                let mut left = self.expr(left)?;
                let mut out = Value::Bool(true);
                for (op, right) in rest {
                    let right = self.expr(right)?;
                    match compare(&left, *op, &right) {
                        Value::Bool(true) => {}
                        other => {
                            out = other;
                            break;
                        }
                    }
                    left = right;
                }
                out
            }
            Expr::IfExp { test, body, orelse } => match truth(self.expr(test)?) {
                Value::Bool(true) => self.expr(body)?,
                Value::Bool(false) => self.expr(orelse)?,
                other => other,
            },
            Expr::NamedExpr { target, value } => {
                let value = self.expr(value)?;
                self.set(target, value.clone());
                value
            }
            Expr::Tuple(items) | Expr::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.expr(item)?);
                }
                Value::Tuple(values)
            }
            Expr::Subscript { value, index } => {
                let value = self.expr(value)?;
                match &**index {
                    Expr::Slice {
                        lower,
                        upper,
                        step: None,
                    } => {
                        let mut side =
                            |part: &'m Option<Box<Expr>>| -> Result<Option<Value>, Stop> {
                                part.as_deref().map(|part| self.expr(part)).transpose()
                            };
                        let (lower, upper) = (side(lower)?, side(upper)?);
                        slice(value, lower, upper)
                    }
                    index => {
                        let index = self.expr(index)?;
                        item(value, index)?
                    }
                }
            }
            Expr::UnaryOp {
                op: UnaryOp::USub,
                operand,
            } => match self.expr(operand)? {
                Value::Int(n) => n
                    .checked_neg()
                    .map_or(Value::Unknown(construct("int")), Value::Int),
                Value::Unknown(why) => Value::Unknown(why),
                _ => Value::Unknown(construct("operator -")),
            },
            // Followed where it can be: written at module level, with plain
            // parameters. One inside a function reads that function's names.
            Expr::Lambda { params, body }
                if self.frames.is_empty()
                    && params.kwonly.is_empty()
                    && params.vararg.is_none()
                    && params.kwarg.is_none()
                    && params.args.iter().all(|param| param.default.is_none()) =>
            {
                self.lambdas.push((params, body));
                Value::Lambda(self.lambdas.len() - 1)
            }
            Expr::Lambda { .. } => Value::Callable,
            Expr::Bytes(_) => Value::Unknown(construct("bytes")),
            // A list or dict comprehension runs at once, in order.
            Expr::ListComp { elt, generators } => {
                match self.comprehension(&[elt], generators, Consumed::Whole)? {
                    Ran::Rows(rows) => {
                        Value::Tuple(rows.into_iter().map(|mut row| row.remove(0)).collect())
                    }
                    Ran::Unknown(value) => self.unfollowed_comprehension(expr, &value),
                }
            }
            Expr::DictComp {
                key: at,
                value,
                generators,
            } => match self.comprehension(&[at, value], generators, Consumed::Whole)? {
                Ran::Rows(rows) => {
                    let mut pairs = Vec::with_capacity(rows.len());
                    for row in rows {
                        let mut row = row.into_iter();
                        let (Some(at), Some(value)) = (row.next(), row.next()) else {
                            continue;
                        };
                        if key(&at).is_none() {
                            return Ok(unknown(&at, "dict key"));
                        }
                        pairs = put(pairs, at, value);
                    }
                    Value::Dict(pairs)
                }
                Ran::Unknown(value) => self.unfollowed_comprehension(expr, &value),
            },
            Expr::SetComp { elt, generators } => {
                match self.comprehension(&[elt], generators, Consumed::Whole)? {
                    Ran::Rows(rows) => {
                        set_of(rows.into_iter().map(|mut row| row.remove(0)).collect())
                    }
                    Ran::Unknown(value) => self.unfollowed_comprehension(expr, &value),
                }
            }
            // A generator runs as it is consumed, which is followed only where
            // it is written as a call's argument.
            Expr::GeneratorExp { .. } => {
                self.escape_comprehension(expr, &construct("comprehension"));
                self.forget_expr(expr, &construct("comprehension"), 0);
                Value::Unknown(construct("comprehension"))
            }
            Expr::Dict(entries) => {
                let mut pairs = Vec::with_capacity(entries.len());
                for (at, value) in entries {
                    let value = self.expr(value)?;
                    match at {
                        Some(at) => {
                            let at = self.expr(at)?;
                            if key(&at).is_none() {
                                return Ok(unknown(&at, "dict key"));
                            }
                            pairs = put(pairs, at, value);
                        }
                        // `**other`
                        None => match value {
                            Value::Dict(more) => {
                                for (at, value) in more {
                                    pairs = put(pairs, at, value);
                                }
                            }
                            other => return Ok(unknown(&other, "dict unpacking")),
                        },
                    }
                }
                Value::Dict(pairs)
            }
            Expr::Set(items) if !items.iter().any(|item| matches!(item, Expr::Starred(_))) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.expr(item)?);
                }
                set_of(values)
            }
            Expr::Set(_) | Expr::Starred(_) => {
                self.escape_names(expr, &construct("value"));
                Value::Unknown(construct("value"))
            }
            Expr::Singleton(Singleton::Ellipsis) | Expr::UnaryOp { .. } | Expr::Slice { .. } => {
                Value::Unknown(construct("value"))
            }
        })
    }

    fn fstring(&mut self, parts: &'m [FPart]) -> Result<Value, Stop> {
        let mut out = String::new();
        for part in parts {
            match part {
                FPart::Text(text) => out.push_str(text),
                FPart::Field {
                    value,
                    conversion: None,
                    spec: None,
                } => match self.expr(value)? {
                    Value::Str(text) | Value::Path(text) => out.push_str(&text),
                    Value::Int(n) => out.push_str(&n.to_string()),
                    Value::Unknown(why) => return Ok(Value::Unknown(why)),
                    _ => return Ok(Value::Unknown(construct("f-string"))),
                },
                FPart::Field { .. } => return Ok(Value::Unknown(construct("f-string"))),
            }
        }
        Ok(Value::Str(out))
    }

    fn attribute(&self, value: Value, attr: &str) -> Value {
        match value {
            Value::Name(dotted) if dotted == "re" && python_re::flag(attr).is_some() => {
                Value::Int(python_re::flag(attr).unwrap_or_default())
            }
            Value::Name(dotted) => Value::Name(format!("{dotted}.{attr}")),
            Value::Path(path) => match PathPart::of(attr) {
                Some(PathPart::Parent) => Value::Path(parent(&path)),
                Some(PathPart::Name) => {
                    Value::Str(path.rsplit('/').next().unwrap_or(&path).to_string())
                }
                None => Value::Unknown(Why::Python(format!("path.{attr}"))),
            },
            Value::Unknown(why) => Value::Unknown(why),
            _ => Value::Unknown(Why::Python(format!("attribute {attr}"))),
        }
    }

    /// A call's arguments. A generator written as one is run to the end when
    /// `whole`: the callee is one this knows consumes it so.
    fn args(
        &mut self,
        args: &'m [Arg],
        whole: bool,
    ) -> Result<(Vec<Value>, BTreeMap<&'m str, Value>), Stop> {
        let mut positional = Vec::new();
        let mut keyword = BTreeMap::new();
        for arg in args {
            match arg {
                Arg::Positional(expr @ Expr::GeneratorExp { .. }) if whole => {
                    positional.push(self.generator(expr, Consumed::Whole)?);
                }
                Arg::Positional(expr) => positional.push(self.expr(expr)?),
                Arg::Keyword(name, expr) => {
                    keyword.insert(name.as_str(), self.expr(expr)?);
                }
                Arg::Starred(expr) | Arg::DoubleStarred(expr) => {
                    self.expr(expr)?;
                    positional.push(Value::Unknown(construct("*args")));
                }
            }
        }
        Ok((positional, keyword))
    }

    fn call(&mut self, func: &'m Expr, args: &'m [Arg]) -> Result<Value, Stop> {
        if let Expr::Attribute { value, attr } = func {
            let receiver = self.expr(value)?;
            if !matches!(receiver, Value::Name(_)) {
                // `''.join(g)` and `xs.extend(g)` run a generator to its end.
                let whole = matches!(
                    (&receiver, attr.as_str()),
                    (Value::Str(_), "join") | (Value::Tuple(_), "extend")
                );
                let (positional, keyword) = self.args(args, whole)?;
                if let Value::Tuple(items) = receiver {
                    return Ok(self.list_method(value, items, attr, positional, &keyword));
                }
                if let Value::Dict(pairs) = receiver {
                    return self.dict_method(value, pairs, attr, positional, &keyword);
                }
                if let Value::Set(items) = receiver {
                    return Ok(self.set_method(value, items, attr, positional, &keyword));
                }
                if let Value::Str(text) = &receiver {
                    return string_method(text, attr, &positional);
                }
                if let Value::Pattern { source, flags } = &receiver
                    && let counted @ ("sub" | "subn") = attr.as_str()
                {
                    let arg =
                        |at: usize, key: &str| positional.get(at).or_else(|| keyword.get(key));
                    let (pattern, flags) = (Value::Str(source.clone()), Value::Int(*flags));
                    let repl = arg(0, "repl").unwrap_or(&Value::None).clone();
                    let string = arg(1, "string").unwrap_or(&Value::None).clone();
                    let count = arg(2, "count").unwrap_or(&Value::Int(0)).clone();
                    if let Value::Defined(_) | Value::Lambda(_) = repl {
                        return self.substitute_calling(
                            &pattern,
                            &flags,
                            &repl,
                            &string,
                            &count,
                            counted == "subn",
                        );
                    }
                    return Ok(substitute(
                        &pattern,
                        &flags,
                        &repl,
                        &string,
                        &count,
                        counted == "subn",
                    ));
                }
                if let Value::Pattern { source, flags } = &receiver
                    && let Some(kind) = Matching::of_method(attr)
                    && positional.len() == 1
                    && keyword.is_empty()
                {
                    return Ok(matching(
                        kind,
                        &Value::Str(source.clone()),
                        &Value::Int(*flags),
                        &positional[0],
                    ));
                }
                if let Value::Match { text, spans, names } = &receiver {
                    return match_method(text, spans, names, attr, &positional);
                }
                return Ok(self.method(receiver, attr, positional, &keyword));
            }
        }
        let callee = self.expr(func)?;
        let function = match &callee {
            Value::Name(name) => Function::of(name),
            _ => None,
        };
        // `next`, `any` and `all` stop a generator where Python does.
        let partly = match function {
            Some(Function::Next) => Some(Consumed::First),
            Some(Function::Any) => Some(Consumed::UntilTrue),
            Some(Function::All) => Some(Consumed::UntilFalse),
            _ => None,
        };
        if let (Some(consumed), Some(Arg::Positional(generator @ Expr::GeneratorExp { .. }))) =
            (partly, args.first())
        {
            let value = self.generator(generator, consumed)?;
            let default = match args.get(1) {
                Some(Arg::Positional(default)) => Some(self.expr(default)?),
                _ => None,
            };
            return consume(function, value, default);
        }
        let whole = matches!(
            function,
            Some(
                Function::List | Function::Sorted | Function::Dict | Function::Any | Function::All
            )
        );
        let (positional, keyword) = self.args(args, whole)?;
        let name = match callee {
            Value::Name(name) => name,
            Value::Defined(defined) => return self.call_defined(&defined, positional, keyword),
            Value::Lambda(at) => return self.call_lambda(at, positional, &keyword),
            // A function reached through an expression this does not follow
            // (`handlers[k](f)`, a lambda) may write a file handed to it, and a
            // lambda may change a list it closes over.
            callee => {
                let why = construct("call");
                if let Value::Callable = callee {
                    self.forget_lists(&why);
                }
                for value in positional.iter().chain(keyword.values()) {
                    self.stranger_writes(value, &why, true);
                    self.escape(value, &why);
                }
                return Ok(Value::Unknown(why));
            }
        };
        let first = positional.first().cloned();
        let function = Function::of(&name);
        Ok(match function {
            Some(Function::Open) => {
                let mode = positional.get(1).or_else(|| keyword.get("mode")).cloned();
                let plain = plain_text(&keyword, positional.len() > 2);
                self.open_plain(first, mode, plain)
            }
            Some(Function::Path | Function::Join) => {
                let mut path = String::new();
                for part in &positional {
                    match text(part) {
                        Some(part) => path = join(&path, &part),
                        None => return Ok(unknown(part, &name)),
                    }
                }
                match function {
                    Some(Function::Join) => Value::Str(path),
                    _ if path.is_empty() => Value::Path(".".to_string()),
                    _ => Value::Path(path),
                }
            }
            Some(Function::ExpandUser) => match first.as_ref().and_then(text) {
                Some(path) => Value::Str(self.expand_user(&path)),
                None => unknown(first.as_ref().unwrap_or(&Value::None), "expanduser"),
            },
            Some(Function::Exists) => match first.as_ref().and_then(text) {
                Some(path) => self.exists(&path),
                None => Value::Unknown(construct("exists")),
            },
            Some(Function::ChangeDir) => {
                self.cwd = first
                    .as_ref()
                    .and_then(text)
                    .and_then(|path| self.resolve(&path));
                Value::None
            }
            // A `for` over a `Tuple` runs once per element.
            Some(Function::Range) => range(&positional),
            Some(Function::Enumerate) => {
                let start = positional.get(1).or_else(|| keyword.get("start"));
                match (first, start) {
                    (Some(Value::Tuple(values)), None | Some(Value::Int(_))) => {
                        let start = match start {
                            Some(Value::Int(n)) => *n,
                            _ => 0,
                        };
                        Value::Tuple(
                            values
                                .into_iter()
                                .enumerate()
                                .map(|(at, value)| {
                                    Value::Tuple(vec![Value::Int(start + at as i64), value])
                                })
                                .collect(),
                        )
                    }
                    (Some(other), _) => unknown(&other, "enumerate"),
                    (None, _) => Value::Unknown(construct("enumerate")),
                }
            }
            Some(Function::Zip) => {
                let rows: Option<Vec<Vec<Value>>> = positional
                    .iter()
                    .map(|value| match value {
                        Value::Tuple(values) => Some(values.clone()),
                        _ => None,
                    })
                    .collect();
                match rows {
                    Some(rows) if !rows.is_empty() => {
                        let shortest = rows.iter().map(Vec::len).min().unwrap_or(0);
                        Value::Tuple(
                            (0..shortest)
                                .map(|at| {
                                    Value::Tuple(rows.iter().map(|row| row[at].clone()).collect())
                                })
                                .collect(),
                        )
                    }
                    _ => Value::Unknown(construct("zip")),
                }
            }
            Some(Function::Sorted) => match first {
                Some(Value::Tuple(values)) if keyword.is_empty() => sorted(values),
                Some(Value::Dict(pairs)) if keyword.is_empty() => {
                    sorted(pairs.into_iter().map(|(at, _)| at).collect())
                }
                Some(Value::Set(items)) if keyword.is_empty() => sorted(items),
                Some(other) => unknown(&other, "sorted"),
                None => Value::Unknown(construct("sorted")),
            },
            Some(Function::List) => match first {
                Some(Value::Tuple(values)) => Value::Tuple(values),
                Some(Value::Dict(pairs)) => {
                    Value::Tuple(pairs.into_iter().map(|(at, _)| at).collect())
                }
                Some(Value::Str(text)) => {
                    Value::Tuple(text.chars().map(|c| Value::Str(c.to_string())).collect())
                }
                Some(other) => unknown(&other, "list"),
                None => Value::Tuple(Vec::new()),
            },
            Some(function @ (Function::Next | Function::Any | Function::All)) => consume(
                Some(function),
                first.unwrap_or(Value::None),
                positional.get(1).cloned(),
            )?,
            Some(Function::Set) => match first {
                None => Value::Set(Vec::new()),
                Some(Value::Set(items)) => Value::Set(items),
                Some(other) => match self.iterable(other) {
                    Value::Tuple(items) => set_of(items),
                    other => unknown(&other, "set"),
                },
            },
            Some(Function::Glob) => match (first, keyword.is_empty()) {
                (Some(Value::Str(pattern)), true) => self.glob(&pattern, false),
                (Some(other), _) => unknown(&other, "glob"),
                (None, _) => Value::Unknown(construct("glob")),
            },
            Some(Function::ListDir) => {
                let dir = match first {
                    None => Some(".".to_string()),
                    Some(value) => text(&value),
                };
                match dir.and_then(|dir| self.resolve(&dir)) {
                    Some(dir) => match self.shell.listing(&dir) {
                        Ok(Some(names)) => Value::Set(names.into_iter().map(Value::Str).collect()),
                        // `listdir` of a directory that is not there raises.
                        Ok(None) => return Err(Stop::Ended),
                        Err(why) => Value::Unknown(why),
                    },
                    None => Value::Unknown(construct("os.listdir")),
                }
            }
            Some(Function::MakesDirectories) => {
                self.shell.listings_changed();
                Value::None
            }
            Some(Function::Dict) => {
                let mut pairs = match first {
                    None => Vec::new(),
                    Some(Value::Dict(pairs)) => pairs,
                    Some(Value::Tuple(items)) => {
                        let mut pairs = Vec::new();
                        for item in items {
                            match item {
                                Value::Tuple(pair)
                                    if pair.len() == 2 && key(&pair[0]).is_some() =>
                                {
                                    let mut pair = pair.into_iter();
                                    let (at, value) = (pair.next(), pair.next());
                                    if let (Some(at), Some(value)) = (at, value) {
                                        pairs = put(pairs, at, value);
                                    }
                                }
                                other => return Ok(unknown(&other, "dict")),
                            }
                        }
                        pairs
                    }
                    Some(other) => return Ok(unknown(&other, "dict")),
                };
                // Keywords arrive sorted here, not in call order.
                if keyword.len() > 1 {
                    return Ok(Value::Unknown(construct("dict keywords")));
                }
                for (at, value) in &keyword {
                    pairs = put(pairs, Value::Str((*at).to_string()), value.clone());
                }
                Value::Dict(pairs)
            }
            Some(Function::JsonLoad) => match first {
                Some(Value::File {
                    path,
                    mode: Mode::Read,
                }) if keyword.is_empty() => match self.read_value(&path) {
                    Value::Str(text) => loaded(&text)?,
                    other => other,
                },
                Some(other) => unknown(&other, "json.load"),
                None => Value::Unknown(construct("json.load")),
            },
            Some(Function::JsonLoads) => match first {
                Some(Value::Str(text)) if keyword.is_empty() => loaded(&text)?,
                Some(other) => unknown(&other, "json.loads"),
                None => Value::Unknown(construct("json.loads")),
            },
            Some(Function::JsonDumps) => match (first, layout(&keyword)) {
                (Some(value), Ok(layout)) => match json::dump(&value, &layout) {
                    Ok(text) => Value::Str(text),
                    Err(construct_name) => unknown(&value, &construct_name),
                },
                (_, Err(why)) => Value::Unknown(why),
                (None, _) => Value::Unknown(construct("json.dumps")),
            },
            Some(Function::JsonDump) => {
                let file = positional.get(1).or_else(|| keyword.get("fp")).cloned();
                let mut rest = keyword.clone();
                rest.remove("fp");
                let text = match (&first, layout(&rest)) {
                    (Some(value), Ok(layout)) => {
                        json::dump(value, &layout).map_err(|construct_name| match value {
                            Value::Unknown(why) => why.clone(),
                            _ => construct(&construct_name),
                        })
                    }
                    (_, Err(why)) => Err(why),
                    (None, _) => Err(construct("json.dump")),
                };
                match file {
                    Some(Value::File {
                        path,
                        mode: Mode::Write,
                    }) => self.shell.write(&path, true, text),
                    Some(other) => self.forget_value(&other, &construct("json.dump")),
                    None => {}
                }
                Value::None
            }
            Some(Function::Str) => match first {
                Some(Value::Str(text) | Value::Path(text)) => Value::Str(text),
                Some(Value::Int(n)) => Value::Str(n.to_string()),
                Some(Value::Bool(b)) => Value::Str(if b { "True" } else { "False" }.to_string()),
                Some(Value::None) => Value::Str("None".to_string()),
                Some(other) => unknown(&other, "str"),
                None => Value::Str(String::new()),
            },
            Some(Function::Len) => match first {
                Some(Value::Str(text)) => Value::Int(text.chars().count() as i64),
                Some(Value::Tuple(items)) => Value::Int(items.len() as i64),
                Some(Value::Dict(pairs)) => Value::Int(pairs.len() as i64),
                Some(Value::Set(items)) => Value::Int(items.len() as i64),
                _ => Value::Unknown(construct("len")),
            },
            Some(Function::Print) => {
                if let Some(file) = keyword.get("file") {
                    self.forget_value(file, &construct("print"));
                }
                Value::None
            }
            Some(Function::Exit) => return Err(Stop::Ended),
            Some(function @ (Function::Shell | Function::Spawn | Function::Concurrent)) => {
                let why = Why::Python(name.clone());
                // `stdout=open(f, 'w')`: the command's output lands in a file.
                for value in keyword.values() {
                    if let Value::File {
                        mode: Mode::Write, ..
                    } = value
                    {
                        self.forget_value(value, &why);
                    }
                }
                let concurrent = matches!(function, Function::Concurrent).then_some(why);
                self.command(function, first.as_ref(), &keyword, concurrent);
                Value::Unknown(construct("subprocess"))
            }
            Some(Function::Delete) => {
                self.forget_value(first.as_ref().unwrap_or(&Value::None), &Why::Python(name));
                Value::None
            }
            Some(Function::DeleteTree) => {
                self.forget_tree(first.as_ref().unwrap_or(&Value::None), &Why::Python(name));
                Value::None
            }
            Some(Function::Reads | Function::Pure) => {
                Value::Unknown(Why::Python(format!("call {name}")))
            }
            Some(Function::Matching(kind)) => {
                let arg = |at: usize, key: &str| positional.get(at).or_else(|| keyword.get(key));
                matching(
                    kind,
                    arg(0, "pattern").unwrap_or(&Value::None),
                    arg(2, "flags").unwrap_or(&Value::Int(0)),
                    arg(1, "string").unwrap_or(&Value::None),
                )
            }
            Some(Function::Substitute { counted })
                if matches!(
                    positional.get(1).or_else(|| keyword.get("repl")),
                    Some(Value::Defined(_) | Value::Lambda(_))
                ) =>
            {
                let arg = |at: usize, key: &str| positional.get(at).or_else(|| keyword.get(key));
                let args = [
                    arg(0, "pattern").unwrap_or(&Value::None).clone(),
                    arg(4, "flags").unwrap_or(&Value::Int(0)).clone(),
                    arg(1, "repl").unwrap_or(&Value::None).clone(),
                    arg(2, "string").unwrap_or(&Value::None).clone(),
                    arg(3, "count").unwrap_or(&Value::Int(0)).clone(),
                ];
                return self
                    .substitute_calling(&args[0], &args[1], &args[2], &args[3], &args[4], counted);
            }
            Some(Function::Substitute { counted }) => {
                let arg = |at: usize, key: &str| positional.get(at).or_else(|| keyword.get(key));
                substitute(
                    arg(0, "pattern").unwrap_or(&Value::None),
                    arg(4, "flags").unwrap_or(&Value::Int(0)),
                    arg(1, "repl").unwrap_or(&Value::None),
                    arg(2, "string").unwrap_or(&Value::None),
                    arg(3, "count").unwrap_or(&Value::Int(0)),
                    counted,
                )
            }
            Some(Function::Compile) => {
                let flags = positional.get(1).or_else(|| keyword.get("flags"));
                match (first.as_ref(), flags) {
                    (Some(Value::Str(source)), None) => Value::Pattern {
                        source: source.clone(),
                        flags: 0,
                    },
                    (Some(Value::Str(source)), Some(Value::Int(flags))) => Value::Pattern {
                        source: source.clone(),
                        flags: *flags,
                    },
                    (Some(Value::Unknown(why)), _) | (_, Some(Value::Unknown(why))) => {
                        Value::Unknown(why.clone())
                    }
                    _ => Value::Unknown(construct("re pattern")),
                }
            }
            Some(Function::Escape) => match first {
                Some(Value::Str(text)) => Value::Str(python_re::escape(&text)),
                Some(other) => unknown(&other, "re.escape"),
                None => Value::Unknown(construct("re.escape")),
            },
            Some(Function::Transfer) => {
                for path in positional.iter().take(2) {
                    self.forget_value(path, &Why::Python(name.clone()));
                }
                Value::None
            }
            // A library call: what it returns is not known, and a path or a file
            // open for writing handed to it may be written.
            None => {
                let why = Why::Python(format!("call {name}"));
                for value in positional.iter().chain(keyword.values()) {
                    self.stranger_writes(value, &why, true);
                    self.escape(value, &why);
                }
                Value::Unknown(why)
            }
        })
    }

    /// A module-level lambda called: its body in a frame of its own. Too few or
    /// too many arguments raise, as in Python.
    fn call_lambda(
        &mut self,
        at: usize,
        positional: Vec<Value>,
        keyword: &BTreeMap<&str, Value>,
    ) -> Result<Value, Stop> {
        let (params, body) = self.lambdas[at];
        if self.frames.len() >= DEPTH {
            return Ok(Value::Unknown(construct("lambda")));
        }
        let mut frame = Frame::default();
        let mut positional = positional.into_iter();
        for param in &params.args {
            let value = positional
                .next()
                .or_else(|| keyword.get(param.name.as_str()).cloned())
                .ok_or(Stop::Ended)?;
            frame.locals.insert(param.name.clone(), value);
        }
        if positional.next().is_some() {
            return Err(Stop::Ended);
        }
        self.frames.push(frame);
        let result = self.expr(body);
        self.frames.pop();
        result
    }

    /// `re.sub` with a function as the replacement: called once per match with
    /// the match, each call's text put in its place.
    fn substitute_calling(
        &mut self,
        pattern: &Value,
        flags: &Value,
        repl: &Value,
        string: &Value,
        count: &Value,
        counted: bool,
    ) -> Result<Value, Stop> {
        let (Value::Str(source), Value::Int(flags), Value::Str(text), Value::Int(count)) =
            (pattern, flags, string, count)
        else {
            for value in [pattern, flags, string, count] {
                if let Value::Unknown(why) = value {
                    return Ok(Value::Unknown(why.clone()));
                }
            }
            return Ok(Value::Unknown(construct("re.sub")));
        };
        let found = python_re::compile(source, *flags).and_then(|compiled| {
            Ok((
                python_re::names(&compiled),
                python_re::find_all(&compiled, text)?,
            ))
        });
        let (names, found) = match found {
            Ok(found) => found,
            Err(why) => return Ok(Value::Unknown(construct(why))),
        };
        let chars: Vec<char> = text.chars().collect();
        let (mut out, mut last, mut done) = (String::new(), 0, 0);
        for spans in found {
            if *count > 0 && done == *count {
                break;
            }
            let (start, end) = spans[0].expect("group 0 always matches");
            out.extend(&chars[last..start]);
            let found = Value::Match {
                text: text.clone(),
                spans,
                names: names.clone(),
            };
            let replaced = match repl {
                Value::Defined(name) => self.call_defined(name, vec![found], BTreeMap::new())?,
                Value::Lambda(at) => self.call_lambda(*at, vec![found], &BTreeMap::new())?,
                other => return Ok(unknown(other, "re replacement function")),
            };
            match replaced {
                Value::Str(replaced) => out.push_str(&replaced),
                other => return Ok(unknown(&other, "re replacement function")),
            }
            last = end;
            done += 1;
        }
        out.extend(&chars[last..]);
        Ok(if counted {
            Value::Tuple(vec![Value::Str(out), Value::Int(done)])
        } else {
            Value::Str(out)
        })
    }

    /// A call to a function the program defines, followed in a frame of its own
    /// with its parameters bound.
    fn call_defined(
        &mut self,
        name: &str,
        positional: Vec<Value>,
        keyword: BTreeMap<&str, Value>,
    ) -> Result<Value, Stop> {
        let why = Why::Python(format!("function {name}"));
        let Some(def) = self.functions.get(name).copied() else {
            return Ok(Value::Unknown(why));
        };
        if def.unfollowed || self.frames.len() >= DEPTH {
            self.forget(def.body, &why, 0);
            // Its `return` ends with it; an exit ends everything.
            self.maybe_exited |= jumps_in(def.body, false).0;
            return Ok(Value::Unknown(why));
        }
        let mut frame = Frame::default();
        let mut positional = positional.into_iter();
        for param in def.params.args.iter().chain(&def.params.kwonly) {
            let value = match (
                positional.next(),
                keyword.get(param.name.as_str()),
                &param.default,
            ) {
                (Some(value), _, _) => value,
                (None, Some(value), _) => value.clone(),
                (None, None, Some(_)) => self
                    .defaults
                    .get(name)
                    .and_then(|defaults| defaults.get(&param.name))
                    .cloned()
                    .unwrap_or_else(|| Value::Unknown(why.clone())),
                (None, None, None) => Value::Unknown(why.clone()),
            };
            frame.locals.insert(param.name.clone(), value);
        }
        for rest in [&def.params.vararg, &def.params.kwarg]
            .into_iter()
            .flatten()
        {
            frame
                .locals
                .insert(rest.clone(), Value::Unknown(why.clone()));
        }
        self.frames.push(frame);
        // A possible early `return` ends with the call, and leaves what it
        // returned unknown.
        let outer = std::mem::replace(&mut self.maybe_jumped, false);
        let result = self.block(def.body);
        let possibly = std::mem::replace(&mut self.maybe_jumped, outer);
        self.frames.pop();
        match result {
            Err(Stop::Ended) => Err(Stop::Ended),
            _ if possibly => Ok(Value::Unknown(construct("an early return"))),
            Ok(()) | Err(Stop::Break | Stop::Continue) => Ok(Value::None),
            Err(Stop::Return(value)) => Ok(value),
        }
    }

    fn method(
        &mut self,
        receiver: Value,
        method: &str,
        args: Vec<Value>,
        keyword: &BTreeMap<&str, Value>,
    ) -> Value {
        if SAVES.contains(&method) && !matches!(receiver, Value::Str(_) | Value::Path(_)) {
            let why = Why::Python(format!("method {method}"));
            self.forget_value(args.first().unwrap_or(&Value::None), &why);
            return Value::Unknown(why);
        }
        match receiver {
            Value::Path(path) => self.path_method(&path, method, args, keyword),
            Value::File { path, mode } => match (method, mode) {
                ("read", Mode::Read) if args.is_empty() => self.read_value(&path),
                ("readlines", Mode::Read) if args.is_empty() => lines_of(self.read_value(&path)),
                ("write", Mode::Write) => {
                    let written = match args.into_iter().next() {
                        Some(Value::Str(text)) => Ok(text),
                        Some(Value::Unknown(why)) => Err(why),
                        _ => Err(construct("write")),
                    };
                    self.shell.write(&path, true, written);
                    Value::None
                }
                ("close" | "flush", _) => Value::None,
                (_, Mode::Write) => {
                    self.shell
                        .write(&path, false, Err(Why::Python(format!("file.{method}"))));
                    Value::None
                }
                _ => Value::Unknown(Why::Python(format!("file.{method}"))),
            },
            // An object this does not know may write a file handed to it. Not a
            // path-shaped string: an unknown object is most often text, and its
            // `replace('src/a.ts', …)` names no file.
            other => {
                let why = Why::Python(format!("method {method}"));
                for value in args.iter().chain(keyword.values()) {
                    self.stranger_writes(value, &why, false);
                    self.escape(value, &why);
                }
                match other {
                    Value::Unknown(unknown) => Value::Unknown(unknown),
                    _ => Value::Unknown(why),
                }
            }
        }
    }

    /// A call this does not follow may write each file `value` names or holds
    /// open, inside a list or a dict too: `fmt.run(['a.py', 'b.py'])`. A
    /// path-shaped string counts where `strings` says it does.
    fn stranger_writes(&mut self, value: &Value, why: &Why, strings: bool) {
        match value {
            Value::Tuple(items) => {
                for item in items {
                    self.stranger_writes(item, why, strings);
                }
            }
            Value::Dict(pairs) => {
                for (at, item) in pairs {
                    self.stranger_writes(at, why, strings);
                    self.stranger_writes(item, why, strings);
                }
            }
            value if (strings && written_by_a_stranger(value)) || holds_a_file(value) => {
                self.forget_value(value, why);
            }
            _ => {}
        }
    }

    /// A method of a list: the changes it makes followed where modelled, the
    /// list forgotten where not.
    fn list_method(
        &mut self,
        target: &Expr,
        items: Vec<Value>,
        method: &str,
        args: Vec<Value>,
        keyword: &BTreeMap<&str, Value>,
    ) -> Value {
        let old = Value::Tuple(items.clone());
        let why = Why::Python(format!("list.{method}"));
        let mut items = items;
        let now = match (method, args.as_slice()) {
            ("copy", []) if keyword.is_empty() => return old,
            ("index" | "count", _) => return Value::Unknown(why),
            _ if !keyword.is_empty() => None,
            ("append", [value]) => {
                items.push(value.clone());
                Some(items)
            }
            ("extend", [Value::Tuple(more)]) => {
                items.extend(more.iter().cloned());
                Some(items)
            }
            ("extend", [Value::Str(text)]) => {
                items.extend(text.chars().map(|c| Value::Str(c.to_string())));
                Some(items)
            }
            ("insert", [Value::Int(at), value]) => {
                let len = items.len() as i64;
                let at = if *at < 0 {
                    (at + len).max(0)
                } else {
                    (*at).min(len)
                };
                items.insert(at as usize, value.clone());
                Some(items)
            }
            ("clear", []) => Some(Vec::new()),
            ("reverse", []) => {
                items.reverse();
                Some(items)
            }
            _ => None,
        };
        let followed = now.is_some();
        if !followed {
            for value in &args {
                self.escape(value, &why);
            }
        }
        self.changed(target, &old, now.map(Value::Tuple), &why);
        if followed {
            Value::None
        } else {
            Value::Unknown(why)
        }
    }

    /// A method of a dict: what it reads computed, the changes it makes
    /// followed where modelled, the dict forgotten where not.
    fn dict_method(
        &mut self,
        target: &Expr,
        pairs: Vec<(Value, Value)>,
        method: &str,
        args: Vec<Value>,
        keyword: &BTreeMap<&str, Value>,
    ) -> Result<Value, Stop> {
        let old = Value::Dict(pairs.clone());
        let why = Why::Python(format!("dict.{method}"));
        let [keys, values, items] = views(&pairs);
        let mut changed: Option<Vec<(Value, Value)>> = None;
        let result = match (method, args.as_slice(), keyword.is_empty()) {
            ("keys", [], true) => return Ok(keys),
            ("values", [], true) => return Ok(values),
            ("items", [], true) => return Ok(items),
            ("copy", [], true) => return Ok(old),
            ("get", [at] | [at, _], true) => {
                if key(at).is_none() {
                    return Ok(unknown(at, "dict key"));
                }
                return Ok(match find_key(&pairs, at) {
                    Some(found) => pairs[found].1.clone(),
                    None => args.get(1).cloned().unwrap_or(Value::None),
                });
            }
            ("setdefault", [at] | [at, _], true) if key(at).is_some() => {
                match find_key(&pairs, at) {
                    Some(found) => return Ok(pairs[found].1.clone()),
                    None => {
                        let value = args.get(1).cloned().unwrap_or(Value::None);
                        changed = Some(put(pairs, at.clone(), value.clone()));
                        value
                    }
                }
            }
            ("pop", [at] | [at, _], true) if key(at).is_some() => match find_key(&pairs, at) {
                Some(found) => {
                    let mut now = pairs;
                    let (_, value) = now.remove(found);
                    changed = Some(now);
                    value
                }
                None => match args.get(1) {
                    Some(default) => return Ok(default.clone()),
                    None => return Err(Stop::Ended),
                },
            },
            ("update", [Value::Dict(more)], true) => {
                let mut now = pairs;
                for (at, value) in more {
                    now = put(now, at.clone(), value.clone());
                }
                changed = Some(now);
                Value::None
            }
            // Keywords arrive sorted here, not in call order: one is enough.
            ("update", [], false) if keyword.len() == 1 => {
                let mut now = pairs;
                for (at, value) in keyword {
                    now = put(now, Value::Str((*at).to_string()), value.clone());
                }
                changed = Some(now);
                Value::None
            }
            ("clear", [], true) => {
                changed = Some(Vec::new());
                Value::None
            }
            _ => {
                for value in args.iter().chain(keyword.values()) {
                    self.escape(value, &why);
                }
                Value::Unknown(why.clone())
            }
        };
        self.changed(target, &old, changed.map(Value::Dict), &why);
        Ok(result)
    }

    /// A method of a set: `add`, `discard` and `update` followed, the set
    /// forgotten on any other.
    fn set_method(
        &mut self,
        target: &Expr,
        items: Vec<Value>,
        method: &str,
        args: Vec<Value>,
        keyword: &BTreeMap<&str, Value>,
    ) -> Value {
        let old = Value::Set(items.clone());
        let why = Why::Python(format!("set.{method}"));
        let now = match (method, args.as_slice(), keyword.is_empty()) {
            ("copy", [], true) => return old,
            ("add", [member], true) if key(member).is_some() => {
                Some(set_of(items.into_iter().chain([member.clone()]).collect()))
            }
            ("discard", [member], true) if key(member).is_some() => Some(Value::Set(
                items
                    .into_iter()
                    .filter(|item| key(item) != key(member))
                    .collect(),
            )),
            ("update", [more], true) => match self.iterable(more.clone()) {
                Value::Tuple(more) | Value::Set(more) => {
                    Some(set_of(items.into_iter().chain(more).collect()))
                }
                _ => None,
            },
            _ => None,
        };
        let followed = now.is_some();
        self.changed(target, &old, now, &why);
        if followed {
            Value::None
        } else {
            Value::Unknown(why)
        }
    }

    /// The paths a glob pattern matches, in no order: `glob.glob`'s strings,
    /// or with `paths` `Path.glob`'s paths. One directory's names only — a
    /// wildcard before the last `/`, `**`, or a `[` class is refused. The
    /// `glob` module skips a name starting with `.` unless the pattern does;
    /// `pathlib` does not.
    fn glob(&mut self, pattern: &str, paths: bool) -> Value {
        let (dir, name) = match pattern.rsplit_once('/') {
            Some((dir, name)) => (Some(dir), name),
            None => (None, pattern),
        };
        let wild = |text: &str| text.contains(['*', '?', '[']);
        if dir.is_some_and(wild) || name.contains("**") || name.contains('[') {
            return Value::Unknown(construct("glob pattern"));
        }
        let listed = match self.resolve(dir.filter(|dir| !dir.is_empty()).unwrap_or(".")) {
            Some(resolved) => self.shell.listing(&resolved),
            None => return Value::Unknown(construct("path")),
        };
        let names = match listed {
            Ok(Some(names)) => names,
            // A directory that is not there matches nothing.
            Ok(None) => Vec::new(),
            Err(why) => return Value::Unknown(why),
        };
        let pieces = glob_pieces(name);
        let hidden = paths || name.starts_with('.');
        let joined = |found: &str| match dir {
            Some(dir) => format!("{dir}/{found}"),
            None => found.to_string(),
        };
        Value::Set(
            names
                .iter()
                .filter(|found| hidden || !found.starts_with('.'))
                .filter(|found| wildcard(&pieces, &found.chars().collect::<Vec<_>>()))
                .map(|found| {
                    if paths {
                        Value::Path(joined(found))
                    } else {
                        Value::Str(joined(found))
                    }
                })
                .collect(),
        )
    }

    fn path_method(
        &mut self,
        path: &str,
        method: &str,
        args: Vec<Value>,
        keyword: &BTreeMap<&str, Value>,
    ) -> Value {
        match method {
            "read_text" => match (self.resolve(path), plain_text(keyword, !args.is_empty())) {
                (Some(resolved), Ok(())) => self.read_value(&resolved),
                (_, Err(why)) => Value::Unknown(why),
                (None, _) => Value::Unknown(construct("path")),
            },
            "write_text" => {
                let plain = plain_text(keyword, args.len() > 1);
                let written = match (args.into_iter().next(), plain) {
                    (_, Err(why)) => Err(why),
                    (Some(Value::Str(text)), Ok(())) => Ok(text),
                    (Some(Value::Unknown(why)), Ok(())) => Err(why),
                    _ => Err(construct("write_text")),
                };
                match self.resolve(path) {
                    Some(resolved) => self.shell.write(&resolved, false, written),
                    None => self.unnamed(construct("path")),
                }
                Value::None
            }
            "open" => {
                let plain = plain_text(keyword, args.len() > 1);
                let mode = args
                    .into_iter()
                    .next()
                    .or_else(|| keyword.get("mode").cloned());
                self.open_plain(Some(Value::Path(path.to_string())), mode, plain)
            }
            "exists" | "is_file" => self.exists(path),
            "expanduser" => Value::Path(self.expand_user(path)),
            "resolve" | "absolute" => match self.resolve(path) {
                Some(resolved) => Value::Path(resolved),
                None => Value::Unknown(construct("path")),
            },
            "joinpath" => {
                let mut out = path.to_string();
                for part in &args {
                    match text(part) {
                        Some(part) => out = join(&out, &part),
                        None => return unknown(part, "joinpath"),
                    }
                }
                Value::Path(out)
            }
            "glob" => match (args.as_slice(), keyword.is_empty()) {
                ([Value::Str(pattern)], true) => self.glob(&join(path, pattern), true),
                _ => Value::Unknown(construct("path.glob")),
            },
            "iterdir" if args.is_empty() => match self.resolve(path) {
                Some(dir) => match self.shell.listing(&dir) {
                    Ok(Some(names)) => Value::Set(
                        names
                            .into_iter()
                            .map(|name| Value::Path(join(path, &name)))
                            .collect(),
                    ),
                    Ok(None) => Value::Unknown(Why::Missing),
                    Err(why) => Value::Unknown(why),
                },
                None => Value::Unknown(construct("path")),
            },
            "mkdir" | "rmdir" | "symlink_to" => {
                self.shell.listings_changed();
                Value::None
            }
            "is_dir" | "stat" | "iterdir" | "rglob" => {
                Value::Unknown(Why::Python(format!("path.{method}")))
            }
            _ => {
                self.forget_value(
                    &Value::Path(path.to_string()),
                    &Why::Python(format!("path.{method}")),
                );
                Value::Unknown(Why::Python(format!("path.{method}")))
            }
        }
    }

    /// `open(file, mode)`. A writing mode empties or creates the file at once, as
    /// CPython does, so what reaches it later is appended.
    fn open(&mut self, file: Option<Value>, mode: Option<Value>) -> Value {
        let Some(path) = file.as_ref().and_then(text) else {
            let why = match file {
                Some(Value::Unknown(why)) => why,
                _ => construct("open"),
            };
            // Opened to write, at a path the program cannot name.
            let reading = match &mode {
                None => true,
                Some(Value::Str(mode)) => !mode.contains(['w', 'a', 'x', '+']),
                Some(_) => false,
            };
            if !reading {
                self.unnamed_write(why.clone());
            }
            return Value::Unknown(why);
        };
        let Some(resolved) = self.resolve(&path) else {
            return Value::Unknown(construct("path"));
        };
        let mode = match mode {
            None => "r".to_string(),
            Some(Value::Str(mode)) => mode,
            Some(_) => {
                self.shell
                    .write(&resolved, false, Err(construct("open mode")));
                return Value::Unknown(construct("open mode"));
            }
        };
        let binary = mode.contains('b') || mode.contains('+');
        match mode.chars().find(|c| matches!(c, 'r' | 'w' | 'a' | 'x')) {
            _ if binary && mode.contains(['w', 'a', 'x', '+']) => {
                self.shell
                    .write(&resolved, false, Err(Why::Python(format!("open {mode}"))));
                Value::Unknown(Why::Python(format!("open {mode}")))
            }
            _ if binary => Value::Unknown(Why::Python(format!("open {mode}"))),
            Some('w' | 'x') => {
                self.shell.write(&resolved, false, Ok(String::new()));
                Value::File {
                    path: resolved,
                    mode: Mode::Write,
                }
            }
            Some('a') => {
                self.shell.write(&resolved, true, Ok(String::new()));
                Value::File {
                    path: resolved,
                    mode: Mode::Write,
                }
            }
            _ => Value::File {
                path: resolved,
                mode: Mode::Read,
            },
        }
    }

    /// What a `for` runs over: a file open for reading is its lines.
    fn iterable(&mut self, value: Value) -> Value {
        match value {
            Value::File {
                path,
                mode: Mode::Read,
            } => lines_of(self.read_value(&path)),
            // A string ranges over its characters.
            Value::Str(text) => {
                Value::Tuple(text.chars().map(|c| Value::Str(c.to_string())).collect())
            }
            // A dict ranges over its keys.
            Value::Dict(pairs) => Value::Tuple(pairs.into_iter().map(|(at, _)| at).collect()),
            other => other,
        }
    }

    /// `open` of a plain UTF-8 text file, or, with `plain` the reason it is not,
    /// a file this does not follow: written in a way it does not model when
    /// opened to write.
    fn open_plain(
        &mut self,
        file: Option<Value>,
        mode: Option<Value>,
        plain: Result<(), Why>,
    ) -> Value {
        let opened = self.open(file, mode);
        match plain {
            Ok(()) => opened,
            Err(why) => {
                if let Value::File {
                    mode: Mode::Write, ..
                } = &opened
                {
                    self.forget_value(&opened, &why);
                }
                Value::Unknown(why)
            }
        }
    }

    /// A text file's content as Python reads it. Text mode turns `\r\n` and
    /// a lone `\r` into `\n`, which this does not model: such a text is
    /// refused.
    fn read_value(&mut self, path: &str) -> Value {
        match self.shell.read(path) {
            Held::Text(text) if text.contains('\r') => Value::Unknown(construct("carriage return")),
            Held::Text(text) => Value::Str(text),
            Held::Absent => Value::Unknown(Why::Missing),
            Held::Unknown => Value::Unknown(Why::NotRead),
            Held::OneOf(_) => Value::Unknown(Why::Branches),
        }
    }

    fn exists(&mut self, path: &str) -> Value {
        match self
            .resolve(path)
            .map(|resolved| self.shell.read(&resolved))
        {
            Some(Held::Text(_)) => Value::Bool(true),
            Some(Held::Absent) => Value::Bool(false),
            Some(Held::OneOf(members)) if members.iter().all(|m| matches!(m, Held::Text(_))) => {
                Value::Bool(true)
            }
            Some(Held::OneOf(_)) => Value::Unknown(Why::Branches),
            Some(Held::Unknown) => Value::Unknown(Why::NotRead),
            None => Value::Unknown(construct("path")),
        }
    }

    /// A path as Python resolves it: against its own directory, with no `~`.
    fn resolve(&self, path: &str) -> Option<String> {
        if path.starts_with('~') || path.contains('$') {
            return None;
        }
        crate::shell_ops::resolve(path, self.cwd.as_deref(), self.shell.home)
    }

    fn expand_user(&self, path: &str) -> String {
        match path.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                format!("{}{rest}", self.shell.home)
            }
            _ => path.to_string(),
        }
    }

    fn unnamed(&mut self, why: Why) {
        self.shell.unfollowed.push(Unfollowed { path: None, why });
    }

    /// A write to a path the program cannot name may have been to any file, so
    /// nothing predicted before it survives and nothing read after it is known.
    /// Found live: a loop over `grep -rl`'s output edited three files, and a
    /// later loop over those files by name was predicted from their old text.
    fn unnamed_write(&mut self, why: Why) {
        self.unnamed(why.clone());
        self.shell.forget_everything(why);
    }

    /// The file a value names or holds open is changed in a way this does not follow.
    fn forget_value(&mut self, value: &Value, why: &Why) {
        let path = match value {
            Value::File { path, .. } => Some(path.clone()),
            other => text(other).and_then(|path| self.resolve(&path)),
        };
        match path {
            Some(path) => self.shell.write(&path, false, Err(why.clone())),
            None => self.unnamed_write(why.clone()),
        }
    }

    /// A command run from Python, as the shell text it amounts to: followed in
    /// its own directory, or with `why` what it writes refused. A command the text
    /// does not give is refused without a path.
    fn command(
        &mut self,
        function: Function,
        first: Option<&Value>,
        keyword: &BTreeMap<&str, Value>,
        why: Option<Why>,
    ) {
        let shell = matches!(function, Function::Shell)
            || matches!(keyword.get("shell"), Some(Value::Bool(true)));
        let script = match (shell, first) {
            (true, Some(Value::Str(text))) => Some(text.clone()),
            (false, Some(Value::Tuple(words))) => words
                .iter()
                .map(|word| text(word).map(|word| quoted(&word)))
                .collect::<Option<Vec<_>>>()
                .map(|words| words.join(" ")),
            (false, Some(Value::Str(program) | Value::Path(program))) => Some(quoted(program)),
            _ => None,
        };
        let cwd = match keyword.get("cwd") {
            None => self.cwd.clone(),
            Some(dir) => text(dir).and_then(|dir| self.resolve(&dir)),
        };
        let followed = script.is_some_and(|script| self.shell.child(&script, cwd, why.clone()));
        if !followed {
            // A command this cannot read may write any file, as an unknown
            // program does: named for its program where the text gives it.
            let program = match first {
                Some(Value::Tuple(words)) => words.first().and_then(text),
                Some(value) => text(value),
                None => None,
            }
            .and_then(|head| head.split_whitespace().next().map(str::to_string))
            .map(|head| crate::shell_ops::basename(&head).to_string())
            .unwrap_or_else(|| "subprocess".to_string());
            // The paths it was told, in its argv or its `env=`.
            let mut words: Vec<Value> = match first {
                Some(Value::Tuple(words)) => words.clone(),
                _ => Vec::new(),
            };
            if let Some(Value::Dict(env)) = keyword.get("env") {
                words.extend(env.iter().map(|(_, value)| value.clone()));
            }
            let told: Vec<String> = words
                .iter()
                .filter_map(text)
                .filter(|word| {
                    !word.contains(char::is_whitespace) && crate::shell_ops::looks_like_path(word)
                })
                .filter_map(|word| self.resolve(&word))
                .collect();
            self.unnamed(why.unwrap_or_else(|| construct("subprocess")));
            self.shell.unknown_program(program, &told);
        }
    }

    /// A directory a value names is removed in a way this does not follow, with
    /// everything under it.
    fn forget_tree(&mut self, value: &Value, why: &Why) {
        match text(value).and_then(|path| self.resolve(&path)) {
            Some(path) => self.shell.forget_tree(&path, why.clone()),
            None => self.unnamed(why.clone()),
        }
    }

    /// Every file `body` could write is refused for `why` and forgotten, and every
    /// name it assigns becomes unknown — a block that is not followed.
    fn forget(&mut self, body: &'m [Stmt], why: &Why, depth: usize) {
        // A block not followed may change a list without assigning a name.
        self.forget_lists(why);
        for name in assigned(body) {
            self.set(&name, Value::Unknown(why.clone()));
        }
        for stmt in body {
            self.forget_stmt(stmt, why, depth);
        }
    }

    fn forget_stmt(&mut self, stmt: &'m Stmt, why: &Why, depth: usize) {
        let (exprs, blocks): (Vec<&'m Expr>, Vec<&'m [Stmt]>) = match &stmt.kind {
            StmtKind::Expr(expr) => (vec![expr], vec![]),
            StmtKind::Assign { targets, value } => {
                (targets.iter().chain([value]).collect(), vec![])
            }
            StmtKind::AugAssign { value, .. } => (vec![value], vec![]),
            StmtKind::Assert { test, msg } => (
                [Some(test), msg.as_ref()].into_iter().flatten().collect(),
                vec![],
            ),
            StmtKind::Return(value) => (value.iter().collect(), vec![]),
            StmtKind::Raise { exc, cause } => (
                [exc.as_ref(), cause.as_ref()]
                    .into_iter()
                    .flatten()
                    .collect(),
                vec![],
            ),
            StmtKind::If { test, body, orelse } | StmtKind::While { test, body, orelse } => {
                (vec![test], vec![body, orelse])
            }
            StmtKind::For {
                iter, body, orelse, ..
            } => (vec![iter], vec![body, orelse]),
            StmtKind::With { items, body } => {
                (items.iter().map(|item| &item.context).collect(), vec![body])
            }
            StmtKind::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                let mut blocks: Vec<&'m [Stmt]> = vec![body, orelse, finalbody];
                blocks.extend(handlers.iter().map(|handler| handler.body.as_slice()));
                (vec![], blocks)
            }
            // A definition runs nothing; a call to it is forgotten where it is
            // made, since which definition holds is not known.
            StmtKind::FunctionDef {
                name,
                params,
                body,
                decorators,
            } => {
                self.define(name, params, body, decorators);
                if let Some(def) = self.functions.get_mut(name) {
                    def.unfollowed = true;
                }
                (vec![], vec![])
            }
            StmtKind::Delete(_)
            | StmtKind::Import(_)
            | StmtKind::ImportFrom { .. }
            | StmtKind::Comment(_)
            | StmtKind::Pass
            | StmtKind::Global(_)
            | StmtKind::Nonlocal(_)
            | StmtKind::Break
            | StmtKind::Continue => (vec![], vec![]),
        };
        if let StmtKind::Import(_) | StmtKind::ImportFrom { .. } = &stmt.kind {
            let _ = self.stmt(stmt);
        }
        if let StmtKind::With { items, .. } = &stmt.kind {
            for item in items {
                if let Some(var) = &item.var {
                    let value = self.static_value(&item.context);
                    self.bind(var, value);
                }
            }
        }
        for expr in exprs {
            self.forget_expr(expr, why, depth);
        }
        for block in blocks {
            for stmt in block {
                self.forget_stmt(stmt, why, depth);
            }
        }
    }

    /// Every write a call inside `expr` could make, refused for `why`.
    fn forget_expr(&mut self, expr: &'m Expr, why: &Why, depth: usize) {
        // `while (line := f.readline()):` binds where it is not followed.
        let mut bound = Vec::new();
        walrus_in(expr, &mut bound);
        for name in bound {
            self.set(&name, Value::Unknown(why.clone()));
        }
        let mut calls = Vec::new();
        calls_in(expr, &mut calls);
        for (func, args) in calls {
            match func {
                Expr::Attribute { value, attr } => {
                    let receiver = self.static_value(value);
                    match (&receiver, attr.as_str()) {
                        (Value::Name(module), _) => {
                            self.forget_function(&format!("{module}.{attr}"), args, why);
                        }
                        (_, attr) if SAVES.contains(&attr) => {
                            if let Some(Arg::Positional(target)) = args.first() {
                                let target = self.static_value(target);
                                self.forget_value(&target, why);
                            }
                        }
                        // A file opened in this same expression is counted at its `open`.
                        (
                            Value::File {
                                mode: Mode::Write, ..
                            },
                            _,
                        ) if !matches!(**value, Expr::Call { .. }) => {
                            self.forget_value(&receiver, why);
                        }
                        (
                            Value::Path(_),
                            "write_text" | "write_bytes" | "touch" | "unlink" | "rename"
                            | "replace" | "open",
                        ) => self.forget_value(&receiver, why),
                        (
                            Value::Unknown(_),
                            "write_text" | "write_bytes" | "touch" | "unlink" | "rename"
                            | "replace",
                        ) => self.unnamed_write(why.clone()),
                        _ => {}
                    }
                }
                Expr::Name(name) => match self.name(name) {
                    Value::Defined(defined) => {
                        if depth < DEPTH
                            && let Some(def) = self.functions.get(&defined).copied()
                        {
                            for stmt in def.body {
                                self.forget_stmt(stmt, why, depth + 1);
                            }
                        }
                    }
                    Value::Name(dotted) => self.forget_function(&dotted, args, why),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn forget_function(&mut self, name: &str, args: &'m [Arg], why: &Why) {
        let positional: Vec<&'m Expr> = args
            .iter()
            .filter_map(|arg| match arg {
                Arg::Positional(expr) => Some(expr),
                _ => None,
            })
            .collect();
        let targets: Vec<&'m Expr> = match Function::of(name) {
            Some(Function::Open) => {
                let mode = positional.get(1).copied().or_else(|| {
                    args.iter().find_map(|arg| match arg {
                        Arg::Keyword(key, expr) if key == "mode" => Some(expr),
                        _ => None,
                    })
                });
                let writes = match mode.map(|mode| self.static_value(mode)) {
                    None => false,
                    Some(Value::Str(mode)) => mode.contains(['w', 'a', 'x', '+']),
                    Some(_) => true,
                };
                positional
                    .first()
                    .copied()
                    .filter(|_| writes)
                    .into_iter()
                    .collect()
            }
            Some(Function::Delete) => positional.first().copied().into_iter().collect(),
            Some(Function::DeleteTree) => {
                if let Some(target) = positional.first() {
                    let value = self.static_value(target);
                    self.forget_tree(&value, why);
                }
                vec![]
            }
            Some(Function::Transfer) => positional.iter().take(2).copied().collect(),
            Some(function @ (Function::Shell | Function::Spawn | Function::Concurrent)) => {
                let first = positional
                    .first()
                    .map(|expr| self.static_value(expr))
                    .unwrap_or(Value::None);
                let mut keyword = BTreeMap::new();
                for arg in args {
                    if let Arg::Keyword(key, expr) = arg {
                        let value = self.static_value(expr);
                        keyword.insert(key.as_str(), value);
                    }
                }
                self.command(function, Some(&first), &keyword, Some(why.clone()));
                vec![]
            }
            None => {
                for arg in args {
                    if let Arg::Positional(expr) | Arg::Keyword(_, expr) = arg {
                        let value = self.static_value(expr);
                        self.stranger_writes(&value, why, true);
                    }
                }
                vec![]
            }
            _ => vec![],
        };
        for target in targets {
            let value = self.static_value(target);
            self.forget_value(&value, why);
        }
    }

    /// What `expr` evaluates to with nothing run — names as they stand, and any
    /// call left unknown. For naming the target of a write that is not followed.
    fn static_value(&mut self, expr: &'m Expr) -> Value {
        match expr {
            Expr::Name(name) => self.name(name),
            Expr::Str(text) => Value::Str(text.clone()),
            Expr::Singleton(Singleton::True) => Value::Bool(true),
            Expr::Tuple(items) | Expr::List(items) => {
                Value::Tuple(items.iter().map(|item| self.static_value(item)).collect())
            }
            Expr::Attribute { value, attr } => {
                let value = self.static_value(value);
                self.attribute(value, attr)
            }
            Expr::BinOp { left, op, right } => {
                let left = self.static_value(left);
                let right = self.static_value(right);
                binary(left, *op, right)
            }
            Expr::Call { func, args } => {
                let callee = self.static_value(func);
                let parts: Vec<Value> = args
                    .iter()
                    .map(|arg| match arg {
                        Arg::Positional(expr) => self.static_value(expr),
                        _ => Value::Unknown(construct("*args")),
                    })
                    .collect();
                let function = match &callee {
                    Value::Name(name) => Function::of(name),
                    _ => None,
                };
                match function {
                    Some(Function::Path | Function::Join) => {
                        let mut path = String::new();
                        for part in &parts {
                            match text(part) {
                                Some(part) => path = join(&path, &part),
                                None => return unknown(part, "path"),
                            }
                        }
                        if matches!(function, Some(Function::Join)) {
                            Value::Str(path)
                        } else {
                            Value::Path(path)
                        }
                    }
                    Some(Function::Open) => {
                        match parts
                            .first()
                            .and_then(text)
                            .and_then(|path| self.resolve(&path))
                        {
                            Some(path) => Value::File {
                                path,
                                mode: Mode::Write,
                            },
                            None => Value::Unknown(construct("open")),
                        }
                    }
                    _ => Value::Unknown(construct("call")),
                }
            }
            _ => Value::Unknown(construct("value")),
        }
    }
}

/// Whether a function this does not know may write what it is handed: a file
/// ([`holds_a_file`]), or a string shaped like a path. Other strings are text,
/// and so is one spanning lines: a doc comment starts with `///`.
fn written_by_a_stranger(value: &Value) -> bool {
    match value {
        Value::Str(text) => !text.contains('\n') && crate::shell_ops::looks_like_path(text),
        other => holds_a_file(other),
    }
}

/// A `Path`, or a file open for writing.
fn holds_a_file(value: &Value) -> bool {
    matches!(
        value,
        Value::Path(_)
            | Value::File {
                mode: Mode::Write,
                ..
            }
    )
}

/// One shell word holding exactly `word`.
fn quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// `re.sub(pattern, repl, string, count, flags)`, where the two regex engines agree;
/// refused by what differs where they do not. `counted` is `re.subn`.
fn substitute(
    pattern: &Value,
    flags: &Value,
    repl: &Value,
    string: &Value,
    count: &Value,
    counted: bool,
) -> Value {
    let refused = |why: &str| Value::Unknown(Why::Python(why.to_string()));
    let (source, flags, template, text, count) = match (pattern, flags, repl, string, count) {
        (_, _, Value::Callable | Value::Defined(_) | Value::Name(_), _, _) => {
            return refused("re replacement function");
        }
        (
            Value::Str(source),
            Value::Int(flags),
            Value::Str(template),
            Value::Str(text),
            Value::Int(count),
        ) if *count >= 0 => (source, *flags, template, text, *count as usize),
        (Value::Unknown(why), ..)
        | (_, Value::Unknown(why), ..)
        | (_, _, Value::Unknown(why), ..)
        | (_, _, _, Value::Unknown(why), _)
        | (.., Value::Unknown(why)) => return Value::Unknown(why.clone()),
        _ => return refused("re.sub"),
    };
    match python_re::compile(source, flags)
        .and_then(|pattern| python_re::sub(&pattern, template, text, count))
    {
        Ok((out, done)) if counted => Value::Tuple(vec![Value::Str(out), Value::Int(done as i64)]),
        Ok((out, _)) => Value::Str(out),
        Err(why) => refused(why),
    }
}

/// A value as the text of a path or a string, when it is one.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) | Value::Path(text) => Some(text.clone()),
        _ => None,
    }
}

fn unknown(value: &Value, construct_: &str) -> Value {
    match value {
        Value::Unknown(why) => Value::Unknown(why.clone()),
        _ => Value::Unknown(construct(construct_)),
    }
}

/// `os.path.join` and `Path /`: a part that is absolute starts again.
fn join(base: &str, part: &str) -> String {
    if part.starts_with('/') || base.is_empty() {
        part.to_string()
    } else if base.ends_with('/') {
        format!("{base}{part}")
    } else {
        format!("{base}/{part}")
    }
}

fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) => "/".to_string(),
        Some((head, _)) => head.to_string(),
        None => ".".to_string(),
    }
}

/// Python's truth of a value, where the text decides it.
fn truth(value: Value) -> Value {
    match value {
        Value::Bool(b) => Value::Bool(b),
        Value::Str(text) => Value::Bool(!text.is_empty()),
        Value::Int(n) => Value::Bool(n != 0),
        Value::None => Value::Bool(false),
        Value::Tuple(items) => Value::Bool(!items.is_empty()),
        Value::Dict(pairs) => Value::Bool(!pairs.is_empty()),
        Value::Set(items) => Value::Bool(!items.is_empty()),
        Value::Match { .. } => Value::Bool(true),
        Value::Unknown(why) => Value::Unknown(why),
        _ => Value::Unknown(construct("truth")),
    }
}

fn binary(left: Value, op: BinOp, right: Value) -> Value {
    match (left, op, right) {
        (Value::Int(a), BinOp::BitOr, Value::Int(b)) => Value::Int(a | b),
        (Value::Str(a), BinOp::Add, Value::Str(b)) => Value::Str(a + &b),
        (Value::Tuple(mut a), BinOp::Add, Value::Tuple(b)) => {
            a.extend(b);
            Value::Tuple(a)
        }
        (Value::Set(a), op @ (BinOp::BitOr | BinOp::BitAnd | BinOp::Sub), Value::Set(b)) => {
            let within =
                |items: &[Value], item: &Value| items.iter().any(|other| key(other) == key(item));
            match op {
                BinOp::BitOr => set_of(a.into_iter().chain(b).collect()),
                BinOp::BitAnd => {
                    Value::Set(a.into_iter().filter(|item| within(&b, item)).collect())
                }
                _ => Value::Set(a.into_iter().filter(|item| !within(&b, item)).collect()),
            }
        }
        (Value::Int(a), op @ (BinOp::Add | BinOp::Sub | BinOp::Mult), Value::Int(b)) => {
            let n = match op {
                BinOp::Add => a.checked_add(b),
                BinOp::Sub => a.checked_sub(b),
                _ => a.checked_mul(b),
            };
            n.map_or(Value::Unknown(construct("int")), Value::Int)
        }
        (Value::Path(a), BinOp::Div, Value::Str(b) | Value::Path(b)) => Value::Path(join(&a, &b)),
        (Value::Unknown(why), _, _) | (_, _, Value::Unknown(why)) => Value::Unknown(why),
        (_, op, _) => Value::Unknown(Why::Python(format!("operator {}", op.symbol()))),
    }
}

fn compare(left: &Value, op: CmpOp, right: &Value) -> Value {
    match (left, op, right) {
        (_, CmpOp::In | CmpOp::NotIn, Value::Dict(pairs)) => match key(left) {
            Some(_) => Value::Bool(find_key(pairs, left).is_some() == (op == CmpOp::In)),
            None => unknown(left, "dict key"),
        },
        (_, CmpOp::In | CmpOp::NotIn, Value::Set(items)) => match key(left) {
            Some(wanted) => Value::Bool(
                items.iter().any(|item| key(item).as_ref() == Some(&wanted)) == (op == CmpOp::In),
            ),
            None => unknown(left, "set member"),
        },
        (Value::Str(a), CmpOp::In, Value::Str(b)) => Value::Bool(b.contains(a.as_str())),
        (Value::Str(a), CmpOp::NotIn, Value::Str(b)) => Value::Bool(!b.contains(a.as_str())),
        (Value::Str(a), CmpOp::Eq, Value::Str(b)) => Value::Bool(a == b),
        (Value::Str(a), CmpOp::NotEq, Value::Str(b)) => Value::Bool(a != b),
        (Value::Int(a), CmpOp::Eq, Value::Int(b)) => Value::Bool(a == b),
        (Value::Int(a), CmpOp::NotEq, Value::Int(b)) => Value::Bool(a != b),
        (Value::Unknown(why), _, _) | (_, _, Value::Unknown(why)) => Value::Unknown(why.clone()),
        (_, op, _) => Value::Unknown(Why::Python(format!("comparison {}", op.symbol()))),
    }
}

/// A method on a string. `Err` where Python raises: `index` of what is not there.
fn string_method(text: &str, method: &str, args: &[Value]) -> Result<Value, Stop> {
    let found = |at: Option<usize>| Value::Int(at.map_or(-1, |at| at as i64));
    Ok(match (method, args) {
        ("find", [Value::Str(needle)]) => found(char_find(text, needle)),
        ("rfind", [Value::Str(needle)]) => found(char_rfind(text, needle)),
        ("index", [Value::Str(needle)]) => found(Some(char_find(text, needle).ok_or(Stop::Ended)?)),
        ("rindex", [Value::Str(needle)]) => {
            found(Some(char_rfind(text, needle).ok_or(Stop::Ended)?))
        }
        ("replace", [Value::Str(old), Value::Str(new)]) => {
            Value::Str(text.replace(old.as_str(), new))
        }
        ("replace", [Value::Str(old), Value::Str(new), Value::Int(count)]) => {
            if *count < 0 {
                Value::Str(text.replace(old.as_str(), new))
            } else if old.is_empty() {
                Value::Unknown(construct("replace ''"))
            } else {
                Value::Str(text.replacen(old.as_str(), new, *count as usize))
            }
        }
        ("splitlines", []) => splitlines(text),
        ("split", []) => Value::Tuple(
            text.split(python_space)
                .filter(|word| !word.is_empty())
                .map(|word| Value::Str(word.to_string()))
                .collect(),
        ),
        ("split", [Value::Str(sep)]) if !sep.is_empty() => Value::Tuple(
            text.split(sep.as_str())
                .map(|part| Value::Str(part.to_string()))
                .collect(),
        ),
        ("join", [Value::Tuple(parts)]) => {
            let parts: Option<Vec<&str>> = parts
                .iter()
                .map(|part| match part {
                    Value::Str(part) => Some(part.as_str()),
                    _ => None,
                })
                .collect();
            match parts {
                Some(parts) => Value::Str(parts.join(text)),
                None => Value::Unknown(construct("join")),
            }
        }
        ("strip", []) => Value::Str(text.trim_matches(python_space).to_string()),
        ("rstrip", []) => Value::Str(text.trim_end_matches(python_space).to_string()),
        ("lstrip", []) => Value::Str(text.trim_start_matches(python_space).to_string()),
        ("startswith", [Value::Str(prefix)]) => Value::Bool(text.starts_with(prefix.as_str())),
        ("endswith", [Value::Str(suffix)]) => Value::Bool(text.ends_with(suffix.as_str())),
        ("count", [Value::Str(needle)]) if !needle.is_empty() => {
            Value::Int(text.matches(needle.as_str()).count() as i64)
        }
        (_, args) => match args.iter().find_map(|arg| match arg {
            Value::Unknown(why) => Some(why.clone()),
            _ => None,
        }) {
            Some(why) => Value::Unknown(why),
            None => Value::Unknown(Why::Python(format!("str.{method}"))),
        },
    })
}

/// Python's whitespace, `str.isspace`: Unicode's, and the separators `\x1c` to
/// `\x1f`, which Rust does not count.
fn python_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// Whether an `open`, `read_text` or `write_text` is of plain UTF-8 text:
/// only `mode` and a UTF-8 `encoding` are taken; `newline=`, another
/// encoding, `errors=` or a further positional argument change what is read
/// or written, and refuse it.
fn plain_text(keyword: &BTreeMap<&str, Value>, more: bool) -> Result<(), Why> {
    if more {
        return Err(construct("open arguments"));
    }
    for (name, value) in keyword {
        match (*name, value) {
            ("mode", _) => {}
            ("encoding", Value::Str(encoding))
                if matches!(encoding.to_ascii_lowercase().as_str(), "utf-8" | "utf8") => {}
            (name, _) => return Err(construct(&format!("open {name}"))),
        }
    }
    Ok(())
}

/// `str.splitlines`: a break at `\n`, `\r` or `\r\n`, none kept, and no empty
/// last line after a final break. Python also breaks on `\v`, `\f`, the
/// separators and NEL, which this does not follow.
fn splitlines(text: &str) -> Value {
    if text.contains([
        '\x0b', '\x0c', '\x1c', '\x1d', '\x1e', '\u{85}', '\u{2028}', '\u{2029}',
    ]) {
        return Value::Unknown(construct("splitlines"));
    }
    let mut lines: Vec<Value> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(['\n', '\r']).unwrap_or(rest.len());
        lines.push(Value::Str(rest[..end].to_string()));
        rest = &rest[end..];
        rest = rest
            .strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix(['\n', '\r']))
            .unwrap_or(rest);
    }
    Value::Tuple(lines)
}

/// `range(stop)`, `range(start, stop)`, `range(start, stop, step)`.
fn range(args: &[Value]) -> Value {
    let ints: Option<Vec<i64>> = args
        .iter()
        .map(|arg| match arg {
            Value::Int(n) => Some(*n),
            _ => None,
        })
        .collect();
    let (start, stop, step) = match ints.as_deref() {
        Some([stop]) => (0, *stop, 1),
        Some([start, stop]) => (*start, *stop, 1),
        Some([start, stop, step]) if *step != 0 => (*start, *stop, *step),
        _ => return Value::Unknown(construct("range")),
    };
    let mut out = Vec::new();
    let mut at = start;
    while (step > 0 && at < stop) || (step < 0 && at > stop) {
        if out.len() > MAX_UNROLL {
            return Value::Unknown(construct("range"));
        }
        out.push(Value::Int(at));
        at += step;
    }
    Value::Tuple(out)
}

/// `sorted` of strings or of integers, as Python orders each; anything else
/// compares by rules this does not follow.
/// Python's order between two values, where both are of a type it orders and
/// this models: strings by code point, integers, and tuples element by
/// element.
fn order(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
        // A path orders by its parts: `a/x` before `a-b/x`.
        (Value::Path(a), Value::Path(b)) => Some(a.split('/').cmp(b.split('/'))),
        (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        (Value::Tuple(a), Value::Tuple(b)) => {
            for (a, b) in a.iter().zip(b) {
                match order(a, b)? {
                    std::cmp::Ordering::Equal => {}
                    other => return Some(other),
                }
            }
            Some(a.len().cmp(&b.len()))
        }
        _ => None,
    }
}

fn sorted(values: Vec<Value>) -> Value {
    // Every pair must be ordered: one that is not raises, or is not modelled.
    let mut sortable = true;
    for pair in values.windows(2) {
        sortable &= order(&pair[0], &pair[1]).is_some();
    }
    let same = values.iter().all(|v| matches!(v, Value::Tuple(_)))
        || values.iter().all(|v| matches!(v, Value::Path(_)));
    if sortable && same {
        let mut values = values;
        // Stable, as Python's is; the check above covered neighbours only,
        // so any pair found unordered while sorting refuses it.
        let mut refused = false;
        values.sort_by(|a, b| {
            order(a, b).unwrap_or_else(|| {
                refused = true;
                std::cmp::Ordering::Equal
            })
        });
        return if refused {
            Value::Unknown(construct("sorted"))
        } else {
            Value::Tuple(values)
        };
    }
    if values.iter().all(|v| matches!(v, Value::Str(_))) {
        let mut texts: Vec<String> = values
            .into_iter()
            .filter_map(|v| match v {
                Value::Str(text) => Some(text),
                _ => None,
            })
            .collect();
        texts.sort();
        return Value::Tuple(texts.into_iter().map(Value::Str).collect());
    }
    if values.iter().all(|v| matches!(v, Value::Int(_))) {
        let mut ints: Vec<i64> = values
            .into_iter()
            .filter_map(|v| match v {
                Value::Int(n) => Some(n),
                _ => None,
            })
            .collect();
        ints.sort_unstable();
        return Value::Tuple(ints.into_iter().map(Value::Int).collect());
    }
    Value::Unknown(construct("sorted"))
}

/// A text as its lines, each with its newline, as iterating an open file
/// yields them.
fn lines_of(value: Value) -> Value {
    match value {
        Value::Str(text) => Value::Tuple(
            text.split_inclusive('\n')
                .map(|line| Value::Str(line.to_string()))
                .collect(),
        ),
        Value::Unknown(why) => Value::Unknown(why),
        _ => Value::Unknown(construct("lines")),
    }
}

/// Where `needle` first occurs in `text`, counted in code points as Python counts.
fn char_find(text: &str, needle: &str) -> Option<usize> {
    text.find(needle).map(|at| text[..at].chars().count())
}

fn char_rfind(text: &str, needle: &str) -> Option<usize> {
    text.rfind(needle).map(|at| text[..at].chars().count())
}

/// A slice bound as Python resolves it: absent is the end it stands for, a
/// negative one counts from the end, and either is clamped to the sequence.
fn bound(value: Option<Value>, len: usize, absent: usize) -> Result<usize, Why> {
    let len = len as i64;
    let at = match value {
        None | Some(Value::None) => return Ok(absent),
        Some(Value::Int(at)) if at < 0 => at + len,
        Some(Value::Int(at)) => at,
        Some(Value::Unknown(why)) => return Err(why),
        Some(_) => return Err(construct("slice")),
    };
    Ok(at.clamp(0, len) as usize)
}

fn slice(value: Value, lower: Option<Value>, upper: Option<Value>) -> Value {
    let len = match &value {
        Value::Str(text) => text.chars().count(),
        Value::Tuple(items) => items.len(),
        Value::Unknown(why) => return Value::Unknown(why.clone()),
        _ => return Value::Unknown(construct("slice")),
    };
    let (from, to) = match (bound(lower, len, 0), bound(upper, len, len)) {
        (Ok(from), Ok(to)) => (from, to.max(from)),
        (Err(why), _) | (_, Err(why)) => return Value::Unknown(why),
    };
    match value {
        Value::Str(text) => Value::Str(text.chars().skip(from).take(to - from).collect()),
        Value::Tuple(items) => Value::Tuple(items[from..to].to_vec()),
        _ => Value::Unknown(construct("slice")),
    }
}

impl Matching {
    /// A compiled pattern's method of the same name.
    fn of_method(method: &str) -> Option<Self> {
        Some(match method {
            "search" => Matching::Find(python_re::Anchor::Anywhere),
            "match" => Matching::Find(python_re::Anchor::Start),
            "fullmatch" => Matching::Find(python_re::Anchor::Whole),
            "finditer" => Matching::Iter,
            "findall" => Matching::All,
            _ => return None,
        })
    }
}

/// A matching function's result over a text: a match or `None`, every match,
/// or `findall`'s texts (the whole match with no groups, the one group's text
/// with one, a tuple of each group's text with more).
fn matching(kind: Matching, pattern: &Value, flags: &Value, string: &Value) -> Value {
    let (Value::Str(source), Value::Int(flags), Value::Str(text)) = (pattern, flags, string) else {
        for value in [pattern, flags, string] {
            if let Value::Unknown(why) = value {
                return Value::Unknown(why.clone());
            }
        }
        return Value::Unknown(construct("re match"));
    };
    let compiled = match kind {
        Matching::Find(_) => python_re::compile_once(source, *flags),
        Matching::Iter | Matching::All => python_re::compile(source, *flags),
    };
    let compiled = match compiled {
        Ok(compiled) => compiled,
        Err(why) => return Value::Unknown(construct(why)),
    };
    let names = python_re::names(&compiled);
    let found = |spans: python_re::Spans| Value::Match {
        text: text.clone(),
        spans,
        names: names.clone(),
    };
    let result = match kind {
        Matching::Find(anchor) => {
            python_re::find(&compiled, text, anchor).map(|spans| spans.map_or(Value::None, found))
        }
        Matching::Iter => python_re::find_all(&compiled, text)
            .map(|all| Value::Tuple(all.into_iter().map(found).collect())),
        Matching::All => python_re::find_all(&compiled, text).map(|all| {
            Value::Tuple(
                all.into_iter()
                    .map(|spans| {
                        let texts: Vec<Value> = spans
                            .iter()
                            .map(|span| match group_text(text, *span) {
                                Value::None => Value::Str(String::new()),
                                found => found,
                            })
                            .collect();
                        match texts.len() {
                            1 => texts[0].clone(),
                            2 => texts[1].clone(),
                            _ => Value::Tuple(texts[1..].to_vec()),
                        }
                    })
                    .collect(),
            )
        }),
    };
    result.unwrap_or_else(|why| Value::Unknown(construct(why)))
}

/// A group's text, or `None` for one that took no part.
fn group_text(text: &str, span: Option<(usize, usize)>) -> Value {
    span.map_or(Value::None, |(start, end)| {
        Value::Str(text.chars().skip(start).take(end - start).collect())
    })
}

/// Which group an argument names: its number, or its name. `Err` where Python
/// raises, `Ok(None)` when the argument is not known.
fn group_index(names: &[Option<String>], wanted: &Value) -> Result<Option<usize>, Stop> {
    match wanted {
        Value::Int(at) => usize::try_from(*at)
            .ok()
            .filter(|at| *at < names.len())
            .map(Some)
            .ok_or(Stop::Ended),
        Value::Str(name) => names
            .iter()
            .position(|named| named.as_deref() == Some(name.as_str()))
            .map(Some)
            .ok_or(Stop::Ended),
        _ => Ok(None),
    }
}

/// A method of a match.
fn match_method(
    text: &str,
    spans: &python_re::Spans,
    names: &[Option<String>],
    method: &str,
    args: &[Value],
) -> Result<Value, Stop> {
    let group = |wanted: &Value| -> Result<Option<Option<(usize, usize)>>, Stop> {
        Ok(group_index(names, wanted)?.map(|at| spans[at]))
    };
    let whole = Value::Int(0);
    Ok(match (method, args) {
        ("group", []) => group_text(text, spans[0]),
        ("group", [one]) => match group(one)? {
            Some(span) => group_text(text, span),
            None => unknown(one, "match.group"),
        },
        ("group", many) => {
            let mut texts = Vec::with_capacity(many.len());
            for wanted in many {
                match group(wanted)? {
                    Some(span) => texts.push(group_text(text, span)),
                    None => return Ok(unknown(wanted, "match.group")),
                }
            }
            Value::Tuple(texts)
        }
        ("groups", [] | [_]) => {
            let default = args.first().cloned().unwrap_or(Value::None);
            Value::Tuple(
                spans[1..]
                    .iter()
                    .map(|span| match group_text(text, *span) {
                        Value::None => default.clone(),
                        found => found,
                    })
                    .collect(),
            )
        }
        ("groupdict", []) => Value::Dict(
            names
                .iter()
                .zip(spans)
                .filter_map(|(name, span)| {
                    Some((Value::Str(name.clone()?), group_text(text, *span)))
                })
                .collect(),
        ),
        (edge @ ("start" | "end" | "span"), [] | [_]) => {
            let wanted = args.first().unwrap_or(&whole);
            let Some(span) = group(wanted)? else {
                return Ok(unknown(wanted, "match span"));
            };
            let (start, end) = span.map_or((-1, -1), |(start, end)| (start as i64, end as i64));
            match edge {
                "start" => Value::Int(start),
                "end" => Value::Int(end),
                _ => Value::Tuple(vec![Value::Int(start), Value::Int(end)]),
            }
        }
        _ => Value::Unknown(Why::Python(format!("match.{method}"))),
    })
}

/// A glob name pattern: `*` and `?`, the rest literal.
enum GlobPiece {
    Char(char),
    Any,
    One,
}

fn glob_pieces(pattern: &str) -> Vec<GlobPiece> {
    pattern
        .chars()
        .map(|c| match c {
            '*' => GlobPiece::Any,
            '?' => GlobPiece::One,
            c => GlobPiece::Char(c),
        })
        .collect()
}

/// Whether `name` matches the pieces whole.
fn wildcard(pieces: &[GlobPiece], name: &[char]) -> bool {
    match pieces.split_first() {
        None => name.is_empty(),
        Some((GlobPiece::Any, rest)) => (0..=name.len()).any(|at| wildcard(rest, &name[at..])),
        Some((GlobPiece::One, rest)) => !name.is_empty() && wildcard(rest, &name[1..]),
        Some((GlobPiece::Char(c), rest)) => name.first() == Some(c) && wildcard(rest, &name[1..]),
    }
}

/// `next`, `any` or `all` of what a generator produced (run only as far as
/// each looks) or of a list: `next` of a list raises in Python.
fn consume(
    function: Option<Function>,
    value: Value,
    default: Option<Value>,
) -> Result<Value, Stop> {
    let Value::Tuple(items) = value else {
        return Ok(unknown(&value, "iterator"));
    };
    Ok(match function {
        Some(Function::Next) => match (items.into_iter().next(), default) {
            (Some(first), _) => first,
            (None, Some(default)) => default,
            (None, None) => return Err(Stop::Ended),
        },
        Some(function @ (Function::Any | Function::All)) => {
            let any = matches!(function, Function::Any);
            for item in items {
                match truth(item) {
                    Value::Bool(b) if b == any => return Ok(Value::Bool(any)),
                    Value::Bool(_) => {}
                    other => return Ok(other),
                }
            }
            Value::Bool(!any)
        }
        _ => Value::Unknown(construct("iterator")),
    })
}

/// `json.loads` of a text: its value, the call raising where Python's does,
/// or refused by the construct not modelled.
fn loaded(text: &str) -> Result<Value, Stop> {
    match json::parse(text) {
        Ok(value) => Ok(value),
        Err(None) => Err(Stop::Ended),
        Err(Some(refused)) => Ok(Value::Unknown(construct(&refused))),
    }
}

/// How `json.dump` was asked to lay its text out, or the keyword refused.
fn layout(keyword: &BTreeMap<&str, Value>) -> Result<json::Layout, Why> {
    let indent = match keyword.get("indent") {
        None | Some(Value::None) => None,
        Some(Value::Int(n)) => Some(" ".repeat(usize::try_from(*n).unwrap_or(0))),
        Some(Value::Str(text)) => Some(text.clone()),
        Some(other) => return Err(refusal(other, "json indent")),
    };
    let mut layout = json::Layout::new(indent);
    for (name, value) in keyword {
        match (*name, value) {
            ("indent", _) => {}
            ("sort_keys", Value::Bool(b)) => layout.sort_keys = *b,
            ("ensure_ascii", Value::Bool(b)) => layout.ensure_ascii = *b,
            ("separators", Value::Tuple(pair)) => match pair.as_slice() {
                [Value::Str(item), Value::Str(key)] => {
                    layout.item = item.clone();
                    layout.key = key.clone();
                }
                _ => return Err(construct("json separators")),
            },
            (name, other) => return Err(refusal(other, &format!("json {name}"))),
        }
    }
    Ok(layout)
}

/// Why a value is not what a construct needs: its own reason when unknown.
fn refusal(value: &Value, name: &str) -> Why {
    match value {
        Value::Unknown(why) => why.clone(),
        _ => construct(name),
    }
}

/// Why a value could not be used, for a refusal's name: an unknown value's
/// own reason, without repeating `python`, or the kind of value it is.
fn cause(value: &Value) -> String {
    match value {
        Value::Unknown(why) => {
            let name = why.census_name();
            name.strip_prefix("python ").unwrap_or(&name).to_string()
        }
        Value::Tuple(items) => format!("{} values", items.len()),
        Value::Str(_) => "a string".to_string(),
        Value::Int(_) => "an int".to_string(),
        Value::Path(_) => "a path".to_string(),
        Value::File { .. } => "a file".to_string(),
        Value::Name(name) => name.clone(),
        Value::Defined(name) => format!("function {name}"),
        Value::Pattern { .. } => "a pattern".to_string(),
        Value::Callable | Value::Lambda(_) => "a lambda".to_string(),
        Value::Match { .. } => "a match".to_string(),
        Value::Set(_) => "a set".to_string(),
        Value::Bool(_) | Value::None | Value::Dict(_) => "a value".to_string(),
    }
}

/// Whether a block may exit with success, and whether it may jump out of
/// what holds it: a `return`, or a `break` or `continue` not inside a loop of
/// its own. A function it defines is not run here.
fn jumps_in(body: &[Stmt], in_loop: bool) -> (bool, bool) {
    let (mut exits, mut jumps) = (false, false);
    let mut merge = |(e, j): (bool, bool)| {
        exits |= e;
        jumps |= j;
    };
    for stmt in body {
        match &stmt.kind {
            StmtKind::Return(_) => merge((false, true)),
            StmtKind::Break | StmtKind::Continue => merge((false, !in_loop)),
            StmtKind::Expr(expr) => merge((exits_with_success(expr), false)),
            StmtKind::If { body, orelse, .. } | StmtKind::For { body, orelse, .. } => {
                let looped = matches!(stmt.kind, StmtKind::For { .. });
                merge(jumps_in(body, in_loop || looped));
                merge(jumps_in(orelse, in_loop));
            }
            StmtKind::While { body, orelse, .. } => {
                merge(jumps_in(body, true));
                merge(jumps_in(orelse, in_loop));
            }
            StmtKind::With { body, .. } => merge(jumps_in(body, in_loop)),
            StmtKind::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                for block in [body, orelse, finalbody] {
                    merge(jumps_in(block, in_loop));
                }
                for handler in handlers {
                    merge(jumps_in(&handler.body, in_loop));
                }
            }
            _ => {}
        }
    }
    (exits, jumps)
}

/// `sys.exit()`, `exit(0)` and the like: an exit whose status may be success.
/// A nonzero number or a message fails the call.
fn exits_with_success(expr: &Expr) -> bool {
    let Expr::Call { func, args } = expr else {
        return false;
    };
    let name = match &**func {
        Expr::Name(name) => name.clone(),
        Expr::Attribute { value, attr } => match &**value {
            Expr::Name(module) => format!("{module}.{attr}"),
            _ => return false,
        },
        _ => return false,
    };
    if !matches!(name.as_str(), "sys.exit" | "exit" | "quit" | "os._exit") {
        return false;
    }
    match args.first() {
        None => true,
        Some(Arg::Positional(Expr::Number(n))) => n.trim() == "0",
        Some(Arg::Positional(Expr::Str(_) | Expr::FString(_))) => false,
        Some(_) => true,
    }
}

/// Where `index` lands in a sequence of `len`; `Err` where Python raises.
fn position(len: usize, index: i64) -> Result<usize, Stop> {
    let at = if index < 0 { index + len as i64 } else { index };
    usize::try_from(at)
        .ok()
        .filter(|at| *at < len)
        .ok_or(Stop::Ended)
}

/// Whether `value` is `list` or holds it at any depth.
fn holds(value: &Value, list: &Value) -> bool {
    value == list
        || match value {
            Value::Tuple(items) => items.iter().any(|item| holds(item, list)),
            Value::Dict(pairs) => pairs.iter().any(|(_, item)| holds(item, list)),
            _ => false,
        }
}

/// A value the program can change in place: a list, a dict or a set.
fn container(value: &Value) -> bool {
    matches!(value, Value::Tuple(_) | Value::Dict(_) | Value::Set(_))
}

/// A set of `values`, each once. A member that cannot be one refuses it.
fn set_of(values: Vec<Value>) -> Value {
    let mut members: Vec<Value> = Vec::with_capacity(values.len());
    for value in values {
        if key(&value).is_none() {
            return unknown(&value, "set member");
        }
        if !members.iter().any(|member| key(member) == key(&value)) {
            members.push(value);
        }
    }
    Value::Set(members)
}

/// A dict's `keys()`, `values()` and `items()`, as this holds them.
fn views(pairs: &[(Value, Value)]) -> [Value; 3] {
    [
        Value::Tuple(pairs.iter().map(|(key, _)| key.clone()).collect()),
        Value::Tuple(pairs.iter().map(|(_, value)| value.clone()).collect()),
        Value::Tuple(
            pairs
                .iter()
                .map(|(key, value)| Value::Tuple(vec![key.clone(), value.clone()]))
                .collect(),
        ),
    ]
}

/// Whether `value` can be a key, and the key it is: `True` and `1` are one
/// key in Python.
fn key(value: &Value) -> Option<Value> {
    match value {
        Value::Bool(b) => Some(Value::Int(i64::from(*b))),
        Value::Str(_) | Value::Int(_) | Value::None | Value::Path(_) => Some(value.clone()),
        _ => None,
    }
}

/// Where `wanted` is among a dict's keys.
fn find_key(pairs: &[(Value, Value)], wanted: &Value) -> Option<usize> {
    let wanted = key(wanted)?;
    pairs
        .iter()
        .position(|(held, _)| key(held).as_ref() == Some(&wanted))
}

/// `pairs` with `value` under `at`: in place when the key is there, last when
/// it is new.
fn put(mut pairs: Vec<(Value, Value)>, at: Value, value: Value) -> Vec<(Value, Value)> {
    match find_key(&pairs, &at) {
        Some(found) => pairs[found].1 = value,
        None => pairs.push((at, value)),
    }
    pairs
}

/// The name an item or attribute chain starts from: `rows` in `rows[0].x`.
fn root(target: &Expr) -> Option<&str> {
    match target {
        Expr::Name(name) => Some(name),
        Expr::Subscript { value, .. } | Expr::Attribute { value, .. } => root(value),
        _ => None,
    }
}

/// The names `:=` binds anywhere in an expression.
fn walrus_in(expr: &Expr, out: &mut Vec<String>) {
    each_expr(expr, &mut |inner| {
        if let Expr::NamedExpr { target, .. } = inner {
            out.push(target.clone());
        }
    });
}

/// Calls `visit` on `expr` and every expression inside it.
fn each_expr(expr: &Expr, visit: &mut impl FnMut(&Expr)) {
    visit(expr);
    let mut each = |inner: &Expr| each_expr(inner, visit);
    match expr {
        Expr::Name(_) | Expr::Number(_) | Expr::Singleton(_) | Expr::Str(_) | Expr::Bytes(_) => {}
        Expr::FString(parts) => fstring_exprs(parts, &mut each),
        Expr::Attribute { value: inner, .. }
        | Expr::UnaryOp { operand: inner, .. }
        | Expr::Lambda { body: inner, .. }
        | Expr::NamedExpr { value: inner, .. }
        | Expr::Starred(inner) => each(inner),
        Expr::Call { func, args } => {
            each(func);
            for arg in args {
                match arg {
                    Arg::Positional(inner)
                    | Arg::Starred(inner)
                    | Arg::Keyword(_, inner)
                    | Arg::DoubleStarred(inner) => each(inner),
                }
            }
        }
        Expr::Subscript { value, index } => {
            each(value);
            each(index);
        }
        Expr::Slice { lower, upper, step } => {
            for part in [lower, upper, step].into_iter().flatten() {
                each(part);
            }
        }
        Expr::BinOp { left, right, .. } => {
            each(left);
            each(right);
        }
        Expr::Compare { left, rest } => {
            each(left);
            for (_, right) in rest {
                each(right);
            }
        }
        Expr::IfExp { test, body, orelse } => {
            each(test);
            each(body);
            each(orelse);
        }
        Expr::BoolOp { values: items, .. }
        | Expr::Tuple(items)
        | Expr::List(items)
        | Expr::Set(items) => items.iter().for_each(each),
        Expr::Dict(pairs) => {
            for (key, value) in pairs {
                if let Some(key) = key {
                    each(key);
                }
                each(value);
            }
        }
        Expr::ListComp { elt, generators }
        | Expr::SetComp { elt, generators }
        | Expr::GeneratorExp { elt, generators } => {
            each(elt);
            for generator in generators {
                each(&generator.target);
                each(&generator.iter);
                generator.ifs.iter().for_each(&mut each);
            }
        }
        Expr::DictComp {
            key,
            value,
            generators,
        } => {
            each(key);
            each(value);
            for generator in generators {
                each(&generator.target);
                each(&generator.iter);
                generator.ifs.iter().for_each(&mut each);
            }
        }
    }
}

fn fstring_exprs(parts: &[FPart], each: &mut impl FnMut(&Expr)) {
    for part in parts {
        if let FPart::Field { value, spec, .. } = part {
            each(value);
            if let Some(spec) = spec {
                fstring_exprs(spec, each);
            }
        }
    }
}

/// Every name an expression reads, anywhere in it. A comprehension's own
/// variables are counted too, which only forgets more.
fn names_read(expr: &Expr, out: &mut Vec<String>) {
    each_expr(expr, &mut |inner| {
        if let Expr::Name(name) = inner {
            out.push(name.clone());
        }
    });
}

/// The names a loop ranges over the elements of, when that is all it reads:
/// `lines`, `enumerate(lines)`, `zip(a, b)`.
fn ranged_over(iter: &Expr) -> Option<Vec<&str>> {
    match iter {
        Expr::Name(name) => Some(vec![name]),
        Expr::Call { func, args } => {
            let Expr::Name(function) = &**func else {
                return None;
            };
            if !["enumerate", "zip", "sorted", "reversed", "list", "tuple"]
                .contains(&function.as_str())
            {
                return None;
            }
            let mut names = Vec::new();
            for arg in args {
                match arg {
                    Arg::Positional(inner) => names.extend(ranged_over(inner)?),
                    _ => return None,
                }
            }
            Some(names)
        }
        _ => None,
    }
}

/// `value[index]`. `Err` where Python raises: an index past either end.
fn item(value: Value, index: Value) -> Result<Value, Stop> {
    Ok(match (value, index) {
        (Value::Str(text), Value::Int(index)) => {
            let at = position(text.chars().count(), index)?;
            Value::Str(text.chars().nth(at).map(String::from).unwrap_or_default())
        }
        (Value::Tuple(mut items), Value::Int(index)) => {
            let at = position(items.len(), index)?;
            items.swap_remove(at)
        }
        (Value::Match { text, spans, names }, wanted) => match group_index(&names, &wanted)? {
            Some(at) => group_text(&text, spans[at]),
            None => unknown(&wanted, "match.group"),
        },
        (Value::Dict(_), Value::Unknown(why)) => Value::Unknown(why),
        (Value::Dict(mut pairs), at) => match find_key(&pairs, &at) {
            Some(found) => pairs.swap_remove(found).1,
            None if key(&at).is_some() => return Err(Stop::Ended),
            None => Value::Unknown(construct("dict key")),
        },
        (Value::Unknown(why), _) | (_, Value::Unknown(why)) => Value::Unknown(why),
        _ => Value::Unknown(construct("subscript")),
    })
}

/// Names a block assigns, anywhere in it: after a block that is not followed,
/// each holds a value this cannot know.
fn assigned(body: &[Stmt]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for stmt in body {
        match &stmt.kind {
            StmtKind::Assign { targets, .. } => {
                for target in targets {
                    names_in(target, &mut out);
                }
            }
            StmtKind::AugAssign { target, .. } => names_in(target, &mut out),
            StmtKind::For {
                target,
                body,
                orelse,
                ..
            } => {
                names_in(target, &mut out);
                out.extend(assigned(body));
                out.extend(assigned(orelse));
            }
            StmtKind::If { body, orelse, .. } | StmtKind::While { body, orelse, .. } => {
                out.extend(assigned(body));
                out.extend(assigned(orelse));
            }
            StmtKind::With { items, body } => {
                for item in items {
                    if let Some(var) = &item.var {
                        names_in(var, &mut out);
                    }
                }
                out.extend(assigned(body));
            }
            StmtKind::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                out.extend(assigned(body));
                out.extend(assigned(orelse));
                out.extend(assigned(finalbody));
                for handler in handlers {
                    out.extend(assigned(&handler.body));
                    out.extend(handler.name.clone());
                }
            }
            _ => {}
        }
    }
    out
}

fn names_in(target: &Expr, out: &mut BTreeSet<String>) {
    match target {
        Expr::Name(name) => {
            out.insert(name.clone());
        }
        Expr::Tuple(items) | Expr::List(items) => {
            for item in items {
                names_in(item, out);
            }
        }
        Expr::Starred(inner) => names_in(inner, out),
        _ => {}
    }
}

/// Every call inside `expr`, outermost first, as its function and arguments.
fn calls_in<'m>(expr: &'m Expr, out: &mut Vec<(&'m Expr, &'m [Arg])>) {
    match expr {
        Expr::Call { func, args } => {
            out.push((func, args));
            calls_in(func, out);
            for arg in args {
                match arg {
                    Arg::Positional(inner)
                    | Arg::Starred(inner)
                    | Arg::Keyword(_, inner)
                    | Arg::DoubleStarred(inner) => calls_in(inner, out),
                }
            }
        }
        Expr::Attribute { value: inner, .. }
        | Expr::UnaryOp { operand: inner, .. }
        | Expr::Lambda { body: inner, .. }
        | Expr::NamedExpr { value: inner, .. }
        | Expr::Starred(inner) => calls_in(inner, out),
        Expr::Subscript { value, index } => {
            calls_in(value, out);
            calls_in(index, out);
        }
        Expr::Slice { lower, upper, step } => {
            for part in [lower, upper, step].into_iter().flatten() {
                calls_in(part, out);
            }
        }
        Expr::BinOp { left, right, .. } => {
            calls_in(left, out);
            calls_in(right, out);
        }
        Expr::Compare { left, rest } => {
            calls_in(left, out);
            for (_, right) in rest {
                calls_in(right, out);
            }
        }
        Expr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                calls_in(part, out);
            }
        }
        Expr::BoolOp { values: items, .. }
        | Expr::Tuple(items)
        | Expr::List(items)
        | Expr::Set(items) => {
            for item in items {
                calls_in(item, out);
            }
        }
        Expr::Dict(pairs) => {
            for (key, value) in pairs {
                if let Some(key) = key {
                    calls_in(key, out);
                }
                calls_in(value, out);
            }
        }
        Expr::FString(parts) => {
            for part in parts {
                if let FPart::Field { value, .. } = part {
                    calls_in(value, out);
                }
            }
        }
        Expr::ListComp { elt, generators }
        | Expr::SetComp { elt, generators }
        | Expr::GeneratorExp { elt, generators } => {
            calls_in(elt, out);
            for generator in generators {
                calls_in(&generator.iter, out);
                for test in &generator.ifs {
                    calls_in(test, out);
                }
            }
        }
        Expr::DictComp {
            key,
            value,
            generators,
        } => {
            calls_in(key, out);
            calls_in(value, out);
            for generator in generators {
                calls_in(&generator.iter, out);
                for test in &generator.ifs {
                    calls_in(test, out);
                }
            }
        }
        Expr::Name(_) | Expr::Number(_) | Expr::Singleton(_) | Expr::Str(_) | Expr::Bytes(_) => {}
    }
}
