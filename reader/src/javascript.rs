//! The JavaScript Claude has actually written, and nothing more.
//!
//! A third language after the shell and Python, and it is here for the same
//! reason both of those are: the work is invisible without it. Measured over the
//! union corpus on 2026-08-22, **11,748 Bash calls mention a JavaScript runtime
//! and 3,824 carry a program in a flag** — and inside them are 1,790
//! `readFileSync` calls, 214 `writeFileSync`, 1,909 `require` and 670 `import`,
//! every one a file an agent touched that no `Read`, no `Write` and no `sed`
//! recorded. The sniffed heredoc census agrees from the other side: TypeScript
//! or JavaScript is the largest carried language at 1,445 bodies and 2.0 MB.
//!
//! ⚠ **This corrects a ranking, and the older one is worth keeping in view.**
//! `docs/reader.md` listed `node -e` under "not done — a query tool, not an
//! editor", on a count of 724 calls with 23 writes. That was `node -e` alone and
//! by distinct payload; it missed the heredocs, the `--input-type=module`
//! shape, and every read. Reads are most of what a projection is for, so the
//! decision moved when the denominator did.
//!
//! Built like [`crate::python`]: a grammar (`javascript.pest`) for the syntax,
//! this module for the meaning, and a report — `cargo run --bin
//! javascript-report` — ranking the calls it could not read, which is what
//! decides what to teach it next.
//!
//! **It is not an interpreter.** It evaluates nothing, imports nothing and
//! follows no control flow. What it cannot resolve to a literal it counts and
//! drops — an unread call is an undercount, an invented path is a lie.
//!
//! The one variable it trusts is a name bound exactly once to a literal, the
//! same rule and the same reason as the Python reader:
//!
//! ```javascript
//! const p = "src/geo/velocity.ts";
//! const s = fs.readFileSync(p, "utf8");
//! fs.writeFileSync(p, s.replace("a", "b"));
//! ```
//!
//! A name bound twice, bound to anything computed, or bound by a pattern, a
//! `for` or a `catch` is not a constant.
//!
//! ⚠ **A template literal with a `${…}` in it is computed**, exactly as an
//! f-string is, and is recorded as unresolved rather than guessed at. A template
//! with no substitution is an ordinary string, because that is all it is.

use std::collections::{BTreeMap, BTreeSet};

use pest::Parser;
use pest::iterators::Pair;
use pest_derive::Parser;

pub use crate::program::{Program, Ran, Refused, Tally, Use, Why};

#[derive(Parser)]
#[grammar = "javascript.pest"]
struct JavaScriptParser;

/// Read a JavaScript program.
///
/// The grammar accepts punctuation it has no reading for, so this does not fail
/// — what it cannot make sense of shows up as an unknown call rather than as a
/// parse error.
pub fn read(source: &str) -> Program {
    if let Some(why) = did_not_run(source) {
        return Program {
            did_not_run: Some(why),
            ..Program::default()
        };
    }
    let Ok(mut parsed) = JavaScriptParser::parse(Rule::program, source) else {
        return Program::default();
    };
    let elements: Vec<Pair<Rule>> = parsed
        .next()
        .expect("program always yields one pair")
        .into_inner()
        .collect();
    let scope = scope(&elements);
    let mut reader = Reader {
        consts: scope.consts,
        defined: scope.defined,
        out: Program::default(),
    };
    for element in elements {
        reader.element(element);
    }
    reader.out
}

/// Whether the text is JavaScript no runtime would accept, so nothing in it ran.
///
/// ⚠ **Deliberately only the shape that can be decided by counting**, and for
/// the reason [`crate::python::did_not_run`] gives: a wrong answer here
/// *deletes* real file operations rather than inventing them, so the test has to
/// be one nothing valid can trip. A string or a template that never closes, or a
/// bracket that never closes, is that shape — a heredoc cut short mid-program is
/// how it happens in this corpus.
///
/// It is NOT a validity check. `javascript.pest` accepts punctuation it has no
/// reading for by design, and a syntax gate would cost far more than it is
/// worth. In particular an unbalanced bracket *inside a comment* is not
/// possible here, because comments are consumed by the same scan.
pub fn did_not_run(source: &str) -> Option<&'static str> {
    let chars: Vec<char> = source.chars().collect();
    let mut stack: Vec<char> = Vec::new();
    // The last character that was not whitespace or a comment. It is what tells
    // a regex literal from a division, by the same rule the grammar uses:
    // division needs a left operand, so a `/` in operand position opens a regex.
    //
    // ⚠ **Without this the scanner deletes working programs.** `.replace(/['"]/g,
    // "")` holds an apostrophe inside a character class; read as an ordinary
    // quote it opens a string that never closes, and the whole program — file
    // writes and all — is thrown away as one that never ran. Caught by a test,
    // not by the corpus, which is the only reason it is not still there.
    let mut prev: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let here = chars[i];
        match here {
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let mut end = i + 2;
                while end + 1 < chars.len() && !(chars[end] == '*' && chars[end + 1] == '/') {
                    end += 1;
                }
                if end + 1 >= chars.len() {
                    return Some("a block comment that never closes");
                }
                i = end + 2;
            }
            // A `/` where a value is expected opens a regex; anywhere else it
            // is division or the start of a comment (both handled above).
            '/' if prev.is_none_or(|c| "([{,;:=!&|?+-*/%~^<>".contains(c)) => {
                match regex(&chars, i) {
                    Some(next) => i = next,
                    // Not a regex after all — an unterminated one is far more
                    // likely to be a division this rule guessed wrong about, so
                    // it is stepped over rather than reported.
                    None => i += 1,
                }
                prev = Some('/');
                continue;
            }
            quote @ ('"' | '\'') => match string(&chars, i, quote) {
                Some(next) => i = next,
                None => return Some("a string that never closes"),
            },
            // A template may span lines, so only the end of the text closes it.
            '`' => match template(&chars, i) {
                Some(next) => i = next,
                None => return Some("a template literal that never closes"),
            },
            open @ ('(' | '[' | '{') => {
                stack.push(open);
                i += 1;
            }
            close @ (')' | ']' | '}') => {
                let wanted = match close {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                // ⚠ **A stray closer is not reported.** `}` on its own is how a
                // program that was cut off at the FRONT looks — a heredoc whose
                // opening lines were lost — and this reader would rather read the
                // half it has than throw the file uses away.
                if stack.last() == Some(&wanted) {
                    stack.pop();
                }
                i += 1;
            }
            _ => i += 1,
        }
        if !here.is_whitespace() {
            prev = Some(here);
        }
    }
    if stack.is_empty() {
        None
    } else {
        Some("a bracket that never closes")
    }
}

/// Past a regex literal, or `None` if it never closes.
///
/// A character class may hold an unescaped `/`, which is why this is not a
/// search for the next slash.
fn regex(chars: &[char], at: usize) -> Option<usize> {
    let mut i = at + 1;
    let mut in_class = false;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            // A regex literal does not span lines.
            '\n' => return None,
            '[' => {
                in_class = true;
                i += 1;
            }
            ']' if in_class => {
                in_class = false;
                i += 1;
            }
            '/' if !in_class => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// Past a quoted string, or `None` if it never closes.
fn string(chars: &[char], at: usize, quote: char) -> Option<usize> {
    let mut i = at + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            // A single-quoted or double-quoted literal does not span lines, so a
            // newline closes nothing and means the quote was never closed.
            '\n' => return None,
            c if c == quote => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// Past a template literal, or `None` if it never closes.
fn template(chars: &[char], at: usize) -> Option<usize> {
    let mut i = at + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '`' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

// ---- constants ----

/// The names a program binds, and which of them stay knowable.
struct Scope {
    consts: BTreeMap<String, String>,
    defined: BTreeSet<String>,
}

/// Which names are bound exactly once to a literal.
///
/// Walks the whole tree rather than the top level: `for (const f of files)` sits
/// inside brackets, and a name rebound there while a literal binding stands
/// somewhere else is the one way a constant becomes a wrong path.
fn scope(elements: &[Pair<Rule>]) -> Scope {
    let mut bound: BTreeMap<String, Vec<Option<String>>> = BTreeMap::new();
    for element in elements {
        collect(element.clone(), &mut bound);
    }
    let mut consts = BTreeMap::new();
    let mut defined = BTreeSet::new();
    for (name, bindings) in bound {
        if let [Some(literal)] = bindings.as_slice() {
            consts.insert(name.clone(), literal.clone());
        }
        defined.insert(name);
    }
    Scope { consts, defined }
}

/// Every binding in the subtree: the name, and the literal it was bound to when
/// it was bound to one.
fn collect(pair: Pair<Rule>, bound: &mut BTreeMap<String, Vec<Option<String>>>) {
    match pair.as_rule() {
        Rule::assign => {
            let mut inner = pair.clone().into_inner();
            let Some(name) = inner.next() else { return };
            let literal = inner
                .find(|p| p.as_rule() == Rule::value)
                .and_then(|value| self_literal(&value));
            bound
                .entry(name.as_str().to_string())
                .or_default()
                .push(literal);
            for part in pair.into_inner() {
                collect(part, bound);
            }
        }
        // Bound to something this cannot know: a pattern, a loop, a catch, a
        // function name, an augmented assignment.
        Rule::destructure | Rule::binder | Rule::augmented | Rule::target | Rule::pattern => {
            for name in names(&pair) {
                bound.entry(name).or_default().push(None);
            }
            for part in pair.into_inner() {
                collect(part, bound);
            }
        }
        _ => {
            for part in pair.into_inner() {
                collect(part, bound);
            }
        }
    }
}

/// Every `name` in a subtree, in order.
fn names(pair: &Pair<Rule>) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack: Vec<Pair<Rule>> = pair.clone().into_inner().collect();
    while let Some(part) = stack.pop() {
        if part.as_rule() == Rule::name {
            out.push(part.as_str().to_string());
        } else {
            stack.extend(part.into_inner());
        }
    }
    out
}

/// A value that is exactly one string literal, as its text.
fn self_literal(value: &Pair<Rule>) -> Option<String> {
    let mut operands = value
        .clone()
        .into_inner()
        .filter(|p| p.as_rule() == Rule::operand);
    let only = operands.next()?;
    if operands.next().is_some() {
        return None;
    }
    let mut parts = only.into_inner();
    let expr = parts.next()?;
    if expr.as_rule() != Rule::expr || parts.next().is_some() {
        return None;
    }
    let mut inner = expr.into_inner();
    let atom = inner.next()?;
    // A literal with anything chained onto it is a call's result, not a path.
    if inner.next().is_some() {
        return None;
    }
    text(&atom)
}

/// The text a string or template atom stands for, when it stands for one.
fn text(atom: &Pair<Rule>) -> Option<String> {
    match atom.as_rule() {
        Rule::string => {
            let raw = atom.as_str();
            let body = &raw[1..raw.len().saturating_sub(1)];
            Some(unescape(body))
        }
        // ⚠ **Only a template with nothing substituted into it.** `` `${d}/x` ``
        // is computed, and a reader that dropped the `${d}` would produce the
        // path `/x`, which is not a file anybody touched.
        Rule::template => {
            let raw = atom.as_str();
            let body = &raw[1..raw.len().saturating_sub(1)];
            (!body.contains("${")).then(|| unescape(body))
        }
        _ => None,
    }
}

/// The escapes that change what path a literal names.
fn unescape(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

// ---- reading ----

/// A value, as far as it can be known.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    /// `["-i", f]` — the second argument of `spawnSync`, and the only reason
    /// this exists. Everywhere else it is as unknown as `Unknown`: a list is
    /// not a path.
    List(Vec<Value>),
    /// No value — and [`Why`] not, carried so that [`Reader::use_of`] can file
    /// the REASON an operation named nothing, not merely the fact. The same
    /// carrier the Python reader's `Unknown` is, for the same census.
    Unknown(Why),
}

/// One argument of a call, or one entry of an object literal.
struct Arg {
    keyword: Option<String>,
    value: Value,
}

/// Modules named by their module, so `fs.readFileSync` is one name rather than a
/// `readFileSync` method on something called `fs`. The same list Python's
/// `MODULES` is, for the same reason.
const MODULES: &[&str] = &[
    "fs",
    "fsp",
    "path",
    "os",
    "process",
    "child_process",
    "cp",
    "util",
    "crypto",
    "url",
    "JSON",
    "Object",
    "Array",
    "Math",
    "String",
    "Number",
    "Promise",
    "console",
    "zlib",
    "stream",
    "readline",
    "assert",
    "http",
    "https",
    "net",
    "Buffer",
    "Date",
];

/// Keywords the grammar leaves as ordinary names. None is a call, whatever
/// bracket follows.
const KEYWORDS: &[&str] = &[
    "if",
    "else",
    "for",
    "while",
    "do",
    "return",
    "switch",
    "case",
    "default",
    "break",
    "continue",
    "throw",
    "try",
    "catch",
    "finally",
    "function",
    "class",
    "new",
    "delete",
    "typeof",
    "instanceof",
    "in",
    "of",
    "void",
    "yield",
    "await",
    "async",
    "const",
    "let",
    "var",
    "this",
    "super",
    "null",
    "undefined",
    "true",
    "false",
    "export",
    "default_",
    "extends",
    "static",
    "get",
    "set",
];

/// Calls that are understood and touch no file. Without this list the worklist
/// of things still to teach this reader is headed by `console.log` forever.
const NOTHING: &[&str] = &[
    "console.log",
    "console.error",
    "console.warn",
    "console.info",
    "console.table",
    "console.dir",
    "JSON.parse",
    "JSON.stringify",
    "Object.keys",
    "Object.values",
    "Object.entries",
    "Object.assign",
    "Object.fromEntries",
    "Object.freeze",
    "Array.from",
    "Array.isArray",
    "Number",
    "String",
    "Boolean",
    "parseInt",
    "parseFloat",
    "isNaN",
    "Math.round",
    "Math.floor",
    "Math.ceil",
    "Math.abs",
    "Math.min",
    "Math.max",
    "Math.sqrt",
    "Math.pow",
    "Math.random",
    "Date",
    "Date.now",
    "Promise.all",
    "Promise.allSettled",
    "Promise.race",
    "Promise.resolve",
    "Promise.reject",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "process.exit",
    "process.on",
    "encodeURIComponent",
    "decodeURIComponent",
    "Buffer.from",
    "structuredClone",
    "fetch",
    "assert",
    "assert.equal",
    "assert.deepEqual",
    "assert.strictEqual",
    "path.basename",
    "path.extname",
    "path.relative",
    "path.parse",
    "path.sep",
    "process.cwd",
    "os.tmpdir",
    "os.homedir",
    "String.raw",
    // Surfaced by the first run of `javascript-report` over the corpus, in
    // worklist order: the geometry in the health app's route matching, and the
    // collection constructors around it.
    "Math.cos",
    "Math.sin",
    "Math.tan",
    "Math.atan",
    "Math.atan2",
    "Math.asin",
    "Math.acos",
    "Math.hypot",
    "Math.log",
    "Math.exp",
    "Math.sign",
    "Math.trunc",
    "Math.PI",
    "Date.parse",
    "Date.UTC",
    "Set",
    "Map",
    "WeakMap",
    "WeakSet",
    "BigInt",
    "Symbol",
    "RegExp",
    "Error",
    "TypeError",
    "Object.defineProperty",
    "Object.getPrototypeOf",
    "Object.create",
    "Number.isInteger",
    "Number.isFinite",
    "Number.parseFloat",
    "Number.parseInt",
    "Array.of",
    "queueMicrotask",
    "require.resolve",
];

/// Methods that are understood and touch no file — string and array handling,
/// and the reads and writes on a handle whose `open` was already counted.
const NOTHING_METHODS: &[&str] = &[
    "replace",
    "replaceAll",
    "split",
    "join",
    "trim",
    "trimStart",
    "trimEnd",
    "slice",
    "splice",
    "substring",
    "substr",
    "toString",
    "toLowerCase",
    "toUpperCase",
    "padStart",
    "padEnd",
    "startsWith",
    "endsWith",
    "includes",
    "indexOf",
    "lastIndexOf",
    "match",
    "matchAll",
    "test",
    "exec",
    "concat",
    "push",
    "pop",
    "shift",
    "unshift",
    "map",
    "filter",
    "reduce",
    "forEach",
    "find",
    "findIndex",
    "some",
    "every",
    "sort",
    "reverse",
    "flat",
    "flatMap",
    "keys",
    "values",
    "entries",
    "has",
    "get",
    "add",
    "delete",
    "then",
    "catch",
    "finally",
    "toFixed",
    "toISOString",
    "getTime",
    "json",
    "text",
    "log",
    "error",
    "warn",
    "info",
    "close",
    "end",
    "on",
    "once",
    "emit",
    "pipe",
    "query",
    "execute",
    "release",
    "connect",
    "destroy",
];

/// What a call does with the path it is given. The closed set, read once here.
#[derive(Debug, Clone, Copy)]
enum Call {
    /// Reads its first argument: `readFileSync`, `createReadStream`.
    Read,
    /// Writes its first argument: `writeFileSync`, `createWriteStream`.
    Write,
    /// Deletes its first argument: `unlinkSync`, `rmSync`.
    Delete,
    /// Reads the first and writes the second: `copyFileSync`, `renameSync`.
    Transfer,
    /// Looks inside a directory: `readdirSync`, `globSync`.
    Walk,
    /// A module this program loaded, which is a file it read: `require`,
    /// `import`. Filtered by the caller's path rule, so `require("fs")` names
    /// nothing and `require("./dist/db/pool.js")` names a file.
    Module,
    /// Names a path without using it: `path.join`, `path.resolve`.
    Join,
    /// A directory, which this cannot attribute — it names a place, not a file.
    Directory,
    /// Moves the program's working directory.
    ChangeDir,
    /// Runs a command through a shell: `execSync("cd x && ls")`.
    Shell,
    /// Runs a program directly, with its arguments as a list: `spawnSync(p, a)`.
    /// No shell, so nothing re-splits the words — the same distinction
    /// `Op::RemoteRun` draws.
    Spawn,
    /// Understood, and touches no file.
    Nothing,
}

/// The one place a function name is read. `None` means "not taught yet".
fn callable(name: &str) -> Option<Call> {
    // The same function reached three ways — `fs.readFileSync`,
    // `fs.promises.readFile`, and a destructured bare `readFileSync` — is one
    // entry, because what it does to a file does not depend on how it was
    // imported. The bare spellings are what `const { readFileSync } =
    // require("fs")` leaves behind, and that is how this corpus writes it.
    let bare = name.rsplit('.').next().unwrap_or(name);
    Some(match bare {
        "readFileSync" | "readFile" | "createReadStream" | "openSync" | "opendirSync" => Call::Read,
        "writeFileSync" | "writeFile" | "appendFileSync" | "appendFile" | "createWriteStream"
        | "outputFileSync" => Call::Write,
        "unlinkSync" | "unlink" | "rmSync" | "rm" | "rmdirSync" | "removeSync" => Call::Delete,
        "copyFileSync" | "copyFile" | "cpSync" | "renameSync" | "rename" => Call::Transfer,
        "readdirSync" | "readdir" | "globSync" | "glob" => Call::Walk,
        "mkdirSync" | "mkdir" | "mkdtempSync" => Call::Directory,
        "chdir" => Call::ChangeDir,
        // ⚠ `exec`/`execSync` take a COMMAND LINE and run it through `/bin/sh`;
        // `spawn`/`execFile` take a program and an argv and run neither through
        // a shell. Node's own manual draws the line there, and it decides
        // whether the text is a script or a filename.
        "execSync" | "exec" | "execAsync" => Call::Shell,
        "spawnSync" | "spawn" | "execFileSync" | "execFile" => Call::Spawn,
        _ => match name {
            "require" | "import" => Call::Module,
            "path.join" | "path.resolve" | "path.normalize" => Call::Join,
            _ if KEYWORDS.contains(&name) || NOTHING.contains(&name) => Call::Nothing,
            _ if NOTHING_METHODS.contains(&bare) && name.contains('.') => Call::Nothing,
            _ => return None,
        },
    })
}

struct Reader {
    consts: BTreeMap<String, String>,
    /// Every name the program bound itself, however it bound it — see
    /// `named_call`.
    defined: BTreeSet<String>,
    out: Program,
}

impl Reader {
    /// One statement.
    fn element(&mut self, pair: Pair<Rule>) {
        match pair.as_rule() {
            Rule::expr => {
                self.expr(pair);
            }
            Rule::assign | Rule::destructure => {
                if let Some(value) = pair.into_inner().find(|p| p.as_rule() == Rule::value) {
                    self.value(value);
                }
            }
            Rule::declare => {
                for part in pair.into_inner() {
                    self.element(part);
                }
            }
            // `import { x } from "./a.js"` — the module is a file the program
            // read, named plainly, and it is 670 of them in this corpus.
            Rule::import_stmt => {
                if let Some(string) = pair.into_inner().find(|p| p.as_rule() == Rule::string) {
                    *self.out.calls.entry("import".to_string()).or_insert(0) += 1;
                    match text(&string) {
                        Some(path) => self.out.uses.push(Use { path, write: false }),
                        None => {
                            *self.out.unresolved.entry("import".to_string()).or_insert(0) += 1;
                            *self.out.why.entry(Why::Expression).or_insert(0) += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// A value: its operands, and what the operators between them leave known.
    fn value(&mut self, pair: Pair<Rule>) -> Value {
        // `value` and `open_value` differ only in whether they may cross a
        // newline; what they mean is identical, so one function reads both.
        let mut values = Vec::new();
        let mut operators = 0usize;
        for part in pair.into_inner() {
            match part.as_rule() {
                Rule::operand => values.push(self.operand(part)),
                Rule::binop => operators += 1,
                Rule::declare | Rule::assign => self.element(part),
                _ => {}
            }
        }
        match values.len() {
            0 => Value::Unknown(Why::Expression),
            1 if operators == 0 => values.remove(0),
            _ => Value::Unknown(Why::Expression),
        }
    }

    /// An operand, which a unary operator makes unknowable — except `await`,
    /// which changes what a value *is* and not what it names.
    fn operand(&mut self, pair: Pair<Rule>) -> Value {
        let mut value = Value::Unknown(Why::Expression);
        let mut plain = true;
        for part in pair.into_inner() {
            match part.as_rule() {
                Rule::expr => value = self.expr(part),
                Rule::unary if part.as_str() == "await" => {}
                _ => plain = false,
            }
        }
        if plain {
            value
        } else {
            Value::Unknown(Why::Expression)
        }
    }

    /// An atom and the calls chained onto it.
    fn expr(&mut self, pair: Pair<Rule>) -> Value {
        let mut parts = pair.into_inner().peekable();
        let Some(head) = parts.next() else {
            return Value::Unknown(Why::Expression);
        };
        let mut value = match head.as_rule() {
            Rule::string | Rule::template => match text(&head) {
                Some(literal) => Value::Text(literal),
                None => Value::Unknown(Why::Expression),
            },
            Rule::paren => match self.arguments(&head).as_slice() {
                [only] if only.keyword.is_none() => only.value.clone(),
                _ => Value::Unknown(Why::Expression),
            },
            Rule::array => {
                let items = self.arguments(&head);
                if items.iter().all(|arg| arg.keyword.is_none()) {
                    Value::List(items.into_iter().map(|arg| arg.value).collect())
                } else {
                    Value::Unknown(Why::Expression)
                }
            }
            Rule::object => {
                self.arguments(&head);
                Value::Unknown(Why::Expression)
            }
            Rule::name => {
                let mut name = head.as_str().to_string();
                if MODULES.contains(&name.as_str()) {
                    while let Some(attr) = parts.next_if(|p| p.as_rule() == Rule::attr) {
                        name.push('.');
                        name.push_str(attr.as_str().trim_start_matches(['.', '?']));
                    }
                }
                match parts.next_if(|p| p.as_rule() == Rule::call) {
                    Some(call) => {
                        let args = self.arguments(&call);
                        self.named_call(&name, &args)
                    }
                    None if name.contains('.') => Value::Unknown(Why::Expression),
                    None => match self.consts.get(&name) {
                        Some(path) => Value::Text(path.clone()),
                        None => Value::Unknown(self.opaque(&name)),
                    },
                }
            }
            _ => Value::Unknown(Why::Expression),
        };
        while let Some(part) = parts.next() {
            value = match part.as_rule() {
                Rule::attr => {
                    let method = part.as_str().trim_start_matches(['.', '?']).to_string();
                    match parts.next_if(|p| p.as_rule() == Rule::call) {
                        Some(call) => {
                            let args = self.arguments(&call);
                            self.method_call(&value, &method, &args)
                        }
                        None => Value::Unknown(Why::Expression),
                    }
                }
                Rule::call | Rule::subscript => {
                    self.arguments(&part);
                    Value::Unknown(Why::Expression)
                }
                _ => Value::Unknown(Why::Expression),
            };
        }
        value
    }

    /// The arguments of a call, an object literal or a bracketed list.
    fn arguments(&mut self, pair: &Pair<Rule>) -> Vec<Arg> {
        let mut out = Vec::new();
        for args in pair.clone().into_inner() {
            if args.as_rule() != Rule::args {
                continue;
            }
            for arg in args.into_inner() {
                if arg.as_rule() != Rule::arg {
                    continue;
                }
                let mut keyword = None;
                let mut value = Value::Unknown(Why::Expression);
                for part in arg.into_inner() {
                    match part.as_rule() {
                        Rule::keyword => {
                            keyword = Some(
                                part.as_str()
                                    .trim_end_matches(':')
                                    .trim()
                                    .trim_matches(['"', '\''])
                                    .to_string(),
                            );
                        }
                        Rule::value | Rule::open_value => value = self.value(part),
                        Rule::declare => self.element(part),
                        _ => {}
                    }
                }
                out.push(Arg { keyword, value });
            }
        }
        out
    }

    /// A call by name: `fs.readFileSync(p)`, `require("./x.js")`.
    fn named_call(&mut self, name: &str, args: &[Arg]) -> Value {
        let Some(call) = callable(name) else {
            // A function the program defined itself — or destructured out of a
            // `require` — is not a gap in this reader, and left in the worklist
            // they crowd out the ones that are. `initPool`, `parseCapturedDay`
            // and `projectPointToSegment` were all in the first run's top
            // twenty, and none of them is a library call anybody could teach.
            if !self.defined.contains(name) {
                *self.out.unknown.entry(name.to_string()).or_insert(0) += 1;
            }
            return Value::Unknown(Why::Expression);
        };
        if !matches!(call, Call::Nothing) {
            *self.out.calls.entry(name.to_string()).or_insert(0) += 1;
        }
        let first = positional(args, 0);
        match call {
            Call::Read => self.use_of(name, first, false),
            // ⚠ **A bare specifier is a PACKAGE, not a file in this tree.**
            // `require("fs")`, `require("node:fs")` and `require("@angular/
            // compiler")` name nothing on disk here, and the last one has a
            // slash in it — so the caller's `looks_like_path` would have let it
            // through and recorded a read of a file that does not exist. Node's
            // own rule is the one used: a specifier is a path when it starts
            // with `./`, `../`, `/` or `~`, and a package otherwise.
            Call::Module => match first {
                Some(Value::Text(spec)) if is_a_path(spec) => self.use_of(name, first, false),
                Some(Value::Text(_)) => Value::Unknown(Why::Expression),
                _ => self.use_of(name, first, false),
            },
            Call::Write => self.use_of(name, first, true),
            Call::Delete => self.use_of(name, first, true),
            Call::Transfer => {
                self.use_of(name, first, false);
                self.use_of(name, positional(args, 1), true);
                Value::Unknown(Why::Expression)
            }
            // A directory listing is a read of a place, not of a file. Counted
            // as a call and attributed to nothing, the same answer Python's
            // `Call::Walk` gives.
            Call::Walk | Call::Directory => Value::Unknown(Why::Expression),
            Call::ChangeDir => {
                self.out.chdir = true;
                Value::Unknown(Why::Expression)
            }
            Call::Shell => {
                match first {
                    Some(Value::Text(script)) => self.out.ran.push(Ran::Script(script.clone())),
                    other => {
                        let why = match other {
                            Some(Value::Unknown(why)) => *why,
                            None => Why::Absent,
                            Some(_) => Why::Expression,
                        };
                        *self.out.unresolved.entry(name.to_string()).or_insert(0) += 1;
                        *self.out.why.entry(why).or_insert(0) += 1;
                    }
                }
                Value::Unknown(Why::Expression)
            }
            Call::Spawn => {
                let program = match first {
                    Some(Value::Text(word)) => word.clone(),
                    other => {
                        let why = match other {
                            Some(Value::Unknown(why)) => *why,
                            None => Why::Absent,
                            Some(_) => Why::Expression,
                        };
                        *self.out.unresolved.entry(name.to_string()).or_insert(0) += 1;
                        *self.out.why.entry(why).or_insert(0) += 1;
                        return Value::Unknown(why);
                    }
                };
                let mut argv = vec![program];
                // The argv list is optional — `spawnSync("ls")` is legal — but
                // one unknown word in it makes the whole call unusable, for the
                // reason the Python reader gives: a hole in the middle of an
                // argv turns the next flag into a filename.
                if let Some(Value::List(items)) = positional(args, 1) {
                    for item in items {
                        match item {
                            Value::Text(word) => argv.push(word.clone()),
                            item => {
                                let why = match item {
                                    Value::Unknown(why) => *why,
                                    _ => Why::Expression,
                                };
                                *self.out.unresolved.entry(name.to_string()).or_insert(0) += 1;
                                *self.out.why.entry(why).or_insert(0) += 1;
                                return Value::Unknown(why);
                            }
                        }
                    }
                }
                self.out.ran.push(Ran::Argv(argv));
                Value::Unknown(Why::Expression)
            }
            Call::Join => join(args),
            Call::Nothing => Value::Unknown(Why::Expression),
        }
    }

    /// A method call on a value this reader may or may not know.
    fn method_call(&mut self, _on: &Value, method: &str, args: &[Arg]) -> Value {
        // Reached through a value rather than a module — `fsp.readFile(p)` after
        // `const fsp = require("fs/promises")`, and `db.query(sql)`. The name
        // table is the same one, because what a function does to a file does not
        // depend on what it was reached through.
        // ⚠ **The bare-method table is consulted FIRST.** `s.replace(…)` is a
        // string method and reaches `callable` as the bare word `replace`, which
        // the module table has no entry for — so without this the worklist filled
        // up with `replace`, `split` and `map`, which is the failure
        // `NOTHING_METHODS` exists to prevent.
        if NOTHING_METHODS.contains(&method) {
            return Value::Unknown(Why::Expression);
        }
        match callable(method) {
            Some(Call::Nothing) => Value::Unknown(Why::Expression),
            Some(_) => self.named_call(method, args),
            None => {
                *self.out.unknown.entry(method.to_string()).or_insert(0) += 1;
                Value::Unknown(Why::Expression)
            }
        }
    }

    /// Record one file use, or the fact that its path was not knowable.
    fn use_of(&mut self, call: &str, value: Option<&Value>, write: bool) -> Value {
        match value {
            Some(Value::Text(path)) => {
                self.out.uses.push(Use {
                    path: path.clone(),
                    write,
                });
                Value::Text(path.clone())
            }
            // The path was not knowable. Count the miss under the call AND
            // under the reason — same total, two questions: where the misses
            // are, and what rule would stop them. Python's `record` does the
            // same, and `Tally.why` is checked against `unresolved` in a test.
            other => {
                let why = match other {
                    Some(Value::Unknown(why)) => *why,
                    None => Why::Absent,
                    Some(_) => Why::Expression,
                };
                *self.out.unresolved.entry(call.to_string()).or_insert(0) += 1;
                *self.out.why.entry(why).or_insert(0) += 1;
                Value::Unknown(why)
            }
        }
    }

    /// Why a bare name has no value here — the census row a miss lands in.
    fn opaque(&self, name: &str) -> Why {
        if self.defined.contains(name) {
            Why::Computed
        } else {
            Why::Outside
        }
    }
}

/// Whether a module specifier names a file in this tree rather than a package.
///
/// Node's own rule, and the reason it is here rather than left to the caller:
/// `@angular/compiler` passes any "looks like a path" test ever written, and it
/// is not a path.
fn is_a_path(spec: &str) -> bool {
    spec.starts_with("./")
        || spec.starts_with("../")
        || spec.starts_with('/')
        || spec.starts_with('~')
}

/// The nth argument that was given positionally.
fn positional(args: &[Arg], n: usize) -> Option<&Value> {
    args.iter()
        .filter(|arg| arg.keyword.is_none())
        .nth(n)
        .map(|arg| &arg.value)
}

/// `path.join("src", "x.ts")` — known only when every part is.
fn join(args: &[Arg]) -> Value {
    let mut joined = String::new();
    for arg in args.iter().filter(|arg| arg.keyword.is_none()) {
        match &arg.value {
            Value::Text(part) if joined.is_empty() => joined = part.clone(),
            Value::Text(part) => {
                joined = format!("{}/{part}", joined.trim_end_matches('/'));
            }
            Value::List(_) | Value::Unknown(_) => {
                return Value::Unknown(Why::Expression);
            }
        }
    }
    if joined.is_empty() {
        Value::Unknown(Why::Expression)
    } else {
        Value::Text(joined)
    }
}
