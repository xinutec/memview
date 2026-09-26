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

use super::python_re;
use super::{Held, Run, Unfollowed, Why};
use crate::syntax::python::ast::{
    Arg, BinOp, BoolOp, CmpOp, Expr, FPart, Module, Params, Singleton, Stmt, StmtKind, UnaryOp,
};

/// Methods that write their object to the path they are given: `img.save(p)`.
const SAVES: &[&str] = &["save", "savefig", "to_csv", "to_json", "to_parquet"];

/// How deep a call to a function the program defines is followed into
/// [`Eval::forget`]; a backstop against recursion, not a limit programs meet.
const DEPTH: usize = 8;

#[derive(Debug, Clone)]
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
    /// What `re.compile` returned: the pattern and its flags, compiled where used.
    Pattern {
        source: String,
        flags: i64,
    },
    /// A function this does not follow, as a value: a `lambda`.
    Callable,
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
    /// `re.escape`.
    Escape,
    /// A library call that only reads or computes: handed a path-shaped string,
    /// it writes none of them.
    Pure,
    /// A library that reads the file it is given: `Image.open(p)`.
    Reads,
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
            "glob.glob" | "glob.iglob" | "os.listdir" | "os.scandir" | "os.walk"
            | "os.path.basename" | "os.path.dirname" | "os.path.splitext" | "os.path.isdir"
            | "os.path.relpath" | "os.getcwd" | "os.environ.get" | "json.dumps" | "json.loads"
            | "sys.path.insert" | "sys.path.append" | "re.search" | "re.match" | "re.fullmatch"
            | "re.findall" | "re.finditer" | "re.split" | "shlex.quote" | "shlex.split"
            | "textwrap.dedent" => Function::Pure,
            "re.sub" => Function::Substitute { counted: false },
            "re.subn" => Function::Substitute { counted: true },
            "re.compile" => Function::Compile,
            "re.escape" => Function::Escape,
            "os.rename" | "os.replace" | "shutil.move" | "shutil.copy" | "shutil.copy2"
            | "shutil.copyfile" => Function::Transfer,
            _ => return None,
        })
    }
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
}

/// One call's names, and the ones it declared `global`.
#[derive(Default)]
struct Frame {
    locals: BTreeMap<String, Value>,
    globals: BTreeSet<String>,
}

/// A function the program defines.
#[derive(Clone, Copy)]
struct Def<'m> {
    params: &'m Params,
    body: &'m [Stmt],
    /// A decorator may change what a call does, so such a call is not followed.
    decorated: bool,
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
        for stmt in body {
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
                    self.bind(target, value.clone());
                }
            }
            StmtKind::AugAssign { target, op, value } => {
                let right = self.expr(value)?;
                if let Expr::Name(name) = target {
                    let left = self.name(name);
                    let joined = binary(left, *op, right);
                    self.set(name, joined);
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
            StmtKind::Raise { .. } => return Err(Stop::Ended),
            StmtKind::Delete(targets) => {
                for target in targets {
                    if let Expr::Name(name) = target {
                        self.unset(name);
                    }
                }
            }
            StmtKind::If { test, body, orelse } => match truth(self.expr(test)?) {
                Value::Bool(true) => self.block(body)?,
                Value::Bool(false) => self.block(orelse)?,
                _ => {
                    self.forget(body, &construct("if"), 0);
                    self.forget(orelse, &construct("if"), 0);
                }
            },
            StmtKind::For {
                body,
                orelse,
                target,
                iter,
            } => match self.expr(iter)? {
                Value::Tuple(values) if values.len() <= MAX_UNROLL => {
                    for value in values {
                        self.bind(target, value);
                        match self.block(body) {
                            Ok(()) | Err(Stop::Continue) => {}
                            Err(Stop::Break) => return Ok(()),
                            Err(stop) => return Err(stop),
                        }
                    }
                    self.block(orelse)?;
                }
                _ => {
                    self.unbind(target, &construct("for"));
                    self.forget(body, &construct("for"), 0);
                    self.forget(orelse, &construct("for"), 0);
                }
            },
            StmtKind::While { body, orelse, .. } => {
                self.forget(body, &construct("while"), 0);
                self.forget(orelse, &construct("while"), 0);
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
            StmtKind::FunctionDef {
                name,
                params,
                body,
                decorators,
            } => self.define(name, params, body, decorators),
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
            // An attribute or an item of something: no string or path changes.
            _ => {}
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
            _ => {}
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
                decorated: !decorators.is_empty(),
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
            Expr::Lambda { .. } => Value::Callable,
            Expr::Bytes(_) => Value::Unknown(construct("bytes")),
            Expr::ListComp { .. }
            | Expr::SetComp { .. }
            | Expr::GeneratorExp { .. }
            | Expr::DictComp { .. } => {
                self.forget_expr(expr, &construct("comprehension"), 0);
                Value::Unknown(construct("comprehension"))
            }
            Expr::Singleton(Singleton::Ellipsis)
            | Expr::UnaryOp { .. }
            | Expr::Slice { .. }
            | Expr::Set(_)
            | Expr::Dict(_)
            | Expr::Starred(_) => Value::Unknown(construct("value")),
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

    fn args(&mut self, args: &'m [Arg]) -> Result<(Vec<Value>, BTreeMap<&'m str, Value>), Stop> {
        let mut positional = Vec::new();
        let mut keyword = BTreeMap::new();
        for arg in args {
            match arg {
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
                let (positional, keyword) = self.args(args)?;
                if let Value::Str(text) = &receiver {
                    return string_method(text, attr, &positional);
                }
                if let Value::Pattern { source, flags } = &receiver
                    && let counted @ ("sub" | "subn") = attr.as_str()
                {
                    let arg =
                        |at: usize, key: &str| positional.get(at).or_else(|| keyword.get(key));
                    return Ok(substitute(
                        &Value::Str(source.clone()),
                        &Value::Int(*flags),
                        arg(0, "repl").unwrap_or(&Value::None),
                        arg(1, "string").unwrap_or(&Value::None),
                        arg(2, "count").unwrap_or(&Value::Int(0)),
                        counted == "subn",
                    ));
                }
                return Ok(self.method(receiver, attr, positional, &keyword));
            }
        }
        let callee = self.expr(func)?;
        let (positional, keyword) = self.args(args)?;
        let name = match callee {
            Value::Name(name) => name,
            Value::Defined(defined) => return self.call_defined(&defined, positional, keyword),
            _ => return Ok(Value::Unknown(construct("call"))),
        };
        let first = positional.first().cloned();
        let function = Function::of(&name);
        Ok(match function {
            Some(Function::Open) => {
                let mode = positional.get(1).or_else(|| keyword.get("mode")).cloned();
                self.open(first, mode)
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
            Some(Function::Str) => match first {
                Some(Value::Str(text) | Value::Path(text)) => Value::Str(text),
                Some(Value::Int(n)) => Value::Str(n.to_string()),
                Some(other) => unknown(&other, "str"),
                None => Value::Str(String::new()),
            },
            Some(Function::Len) => match first {
                Some(Value::Str(text)) => Value::Int(text.chars().count() as i64),
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
                    if written_by_a_stranger(value) {
                        self.forget_value(value, &why);
                    }
                }
                Value::Unknown(why)
            }
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
        if def.decorated || self.frames.len() >= DEPTH {
            self.forget(def.body, &why, 0);
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
                (None, None, Some(default)) => self.expr(default)?,
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
        let result = self.block(def.body);
        self.frames.pop();
        match result {
            Ok(()) | Err(Stop::Break | Stop::Continue) => Ok(Value::None),
            Err(Stop::Return(value)) => Ok(value),
            Err(Stop::Ended) => Err(Stop::Ended),
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
                    if holds_a_file(value) {
                        self.forget_value(value, &why);
                    }
                }
                match other {
                    Value::Unknown(unknown) => Value::Unknown(unknown),
                    _ => Value::Unknown(why),
                }
            }
        }
    }

    fn path_method(
        &mut self,
        path: &str,
        method: &str,
        args: Vec<Value>,
        keyword: &BTreeMap<&str, Value>,
    ) -> Value {
        match method {
            "read_text" => match self.resolve(path) {
                Some(resolved) => self.read_value(&resolved),
                None => Value::Unknown(construct("path")),
            },
            "write_text" => {
                let written = match args.into_iter().next() {
                    Some(Value::Str(text)) => Ok(text),
                    Some(Value::Unknown(why)) => Err(why),
                    _ => Err(construct("write_text")),
                };
                match self.resolve(path) {
                    Some(resolved) => self.shell.write(&resolved, false, written),
                    None => self.unnamed(construct("path")),
                }
                Value::None
            }
            "open" => {
                let mode = args
                    .into_iter()
                    .next()
                    .or_else(|| keyword.get("mode").cloned());
                self.open(Some(Value::Path(path.to_string())), mode)
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
            "mkdir" | "is_dir" | "stat" | "iterdir" | "glob" | "rglob" => {
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

    fn read_value(&mut self, path: &str) -> Value {
        match self.shell.read(path) {
            Held::Text(text) => Value::Str(text),
            Held::Absent => Value::Unknown(Why::Missing),
            Held::Unknown => Value::Unknown(Why::NotRead),
        }
    }

    fn exists(&mut self, path: &str) -> Value {
        match self
            .resolve(path)
            .map(|resolved| self.shell.read(&resolved))
        {
            Some(Held::Text(_)) => Value::Bool(true),
            Some(Held::Absent) => Value::Bool(false),
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

    /// The file a value names or holds open is changed in a way this does not follow.
    fn forget_value(&mut self, value: &Value, why: &Why) {
        let path = match value {
            Value::File { path, .. } => Some(path.clone()),
            other => text(other).and_then(|path| self.resolve(&path)),
        };
        match path {
            Some(path) => self.shell.write(&path, false, Err(why.clone())),
            None => self.unnamed(why.clone()),
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
            self.unnamed(why.unwrap_or_else(|| construct("subprocess")));
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
            // A definition runs nothing; a call to it is forgotten where it is made.
            StmtKind::FunctionDef {
                name,
                params,
                body,
                decorators,
            } => {
                self.define(name, params, body, decorators);
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
                        ) => self.unnamed(why.clone()),
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
                        if written_by_a_stranger(&value) {
                            self.forget_value(&value, why);
                        }
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
/// ([`holds_a_file`]), or a string shaped like a path. Other strings are text.
fn written_by_a_stranger(value: &Value) -> bool {
    match value {
        Value::Str(text) => crate::shell_ops::looks_like_path(text),
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
        Value::Unknown(why) => Value::Unknown(why),
        _ => Value::Unknown(construct("truth")),
    }
}

fn binary(left: Value, op: BinOp, right: Value) -> Value {
    match (left, op, right) {
        (Value::Int(a), BinOp::BitOr, Value::Int(b)) => Value::Int(a | b),
        (Value::Str(a), BinOp::Add, Value::Str(b)) => Value::Str(a + &b),
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
        ("strip", []) => Value::Str(text.trim().to_string()),
        ("rstrip", []) => Value::Str(text.trim_end().to_string()),
        ("lstrip", []) => Value::Str(text.trim_start().to_string()),
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

/// `value[index]`. `Err` where Python raises: an index past either end.
fn item(value: Value, index: Value) -> Result<Value, Stop> {
    let at = |len: usize, index: i64| -> Result<usize, Stop> {
        let at = if index < 0 { index + len as i64 } else { index };
        usize::try_from(at)
            .ok()
            .filter(|at| *at < len)
            .ok_or(Stop::Ended)
    };
    Ok(match (value, index) {
        (Value::Str(text), Value::Int(index)) => {
            let at = at(text.chars().count(), index)?;
            Value::Str(text.chars().nth(at).map(String::from).unwrap_or_default())
        }
        (Value::Tuple(mut items), Value::Int(index)) => {
            let at = at(items.len(), index)?;
            items.swap_remove(at)
        }
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
