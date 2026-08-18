//! The Python Claude has actually written, and nothing more.
//!
//! A second language after the shell, and it is here for the same reason that
//! one is: the work is invisible without it. **7,494 of the corpus's Bash calls
//! are Python** — 4,563 heredocs fed to `python3 -` and 2,931 `python -c` — and
//! inside them are 1,735 `write_text` calls and 1,812 `open(…, 'w')`s, every one
//! a file changed by an agent that no `Write`, no `Edit` and no `sed` recorded.
//!
//! Built like [`crate::shell`]: a grammar (`python.pest`) for the syntax, this
//! module for the meaning, and a report — `cargo run --bin python-report` —
//! ranking what it could not read, which is what decides what to teach it next.
//! Indentation — the awkward part of Python for a generated parser, though not
//! an impossible one — is skipped rather than solved: block structure never
//! decides which file was written.
//!
//! **It is not an interpreter.** It evaluates nothing, imports nothing and
//! follows no control flow. What it cannot resolve to a literal it counts and
//! drops — an unread call is an undercount, an invented path is a lie.
//!
//! The one variable it does trust is a name bound exactly once to a literal,
//! because that is the corpus's commonest shape by a distance:
//!
//! ```python
//! p = Path('src/geo/velocity.ts')
//! s = p.read_text()
//! p.write_text(s.replace('a', 'b'))
//! ```
//!
//! A name bound twice, bound to anything computed, or bound by a `for`, an `as`
//! or an `import` is not a constant: `for p in files:` names every file and
//! therefore none.
//!
//! ⚠ **That last rule is a limitation, not a principle, and the shape of its
//! replacement is already decided.** "Bound exactly once to a literal" is
//! constant propagation with a domain of two values, written by hand for one
//! language. **`for p in ['a.ts', 'b.ts']:` is fully determined by the text** —
//! it names two files, not none — and the same is true of the shell's 3,078
//! loops over a literal word list. What this cannot follow is a value that
//! depends on the world: a loop over `Path('.').glob(…)`, a name assigned in
//! both arms of an `if`, an argument built from `sys.argv`. Measured, the gap is
//! **3,694 of 13,828 operations that name no file** — "computed, f-string, loop
//! variable" — and the loop-variable share of that comes back with the value
//! domain in the README's Roadmap.
//!
//! What stays, whatever replaces it: **an undetermined value is recorded as
//! undetermined and never approximated.** An unread call is an undercount; an
//! invented path is a lie.

use std::collections::{BTreeMap, BTreeSet};

use pest::Parser;
use pest::iterators::Pair;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "python.pest"]
struct PythonParser;

/// A file a Python program used, as the program named it.
///
/// The path is **unresolved on purpose**: what it is relative to is the
/// directory the shell was in, which is one layer up in
/// [`crate::shell_files`] — and that is also where the rule about which words
/// may become paths at all already lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    pub path: String,
    /// `true` when the program changed the file: an `open` in a writing mode, a
    /// `write_text`, an `os.remove`.
    pub write: bool,
}

/// What one Python program did with files, and what it did that this could not
/// read.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Program {
    pub uses: Vec<Use>,
    /// File operations recognised, by name — `open`, `write_text`, `os.remove`.
    pub calls: BTreeMap<String, usize>,
    /// Those among them whose path was not knowable: an f-string, a loop
    /// variable, a computed join. Counted rather than guessed at, because the
    /// size of what is being dropped is the only honest way to read the rest.
    pub unresolved: BTreeMap<String, usize>,
    /// Every other call, by name. **The worklist**: what tops this is what the
    /// reader should learn next, exactly as the grammar was grown.
    pub unknown: BTreeMap<String, usize>,
    /// Whether the program moved its own working directory.
    ///
    /// It cannot be followed — `os.chdir(sys.argv[1])` has no value here — so
    /// the honest response is to stop trusting relative paths out of that
    /// program. The caller enforces it; this only reports it.
    pub chdir: bool,
    /// Set when the text is not Python any interpreter would accept, so the
    /// program **raised a `SyntaxError` and ran none of itself**.
    ///
    /// ⚠ **`uses` is empty whenever this is set, and that is the point.** A
    /// permissive grammar reads a broken program as happily as a working one
    /// and hands back the paths it mentions — which are then recorded as work
    /// that happened. Soundness here is the same property the shell oracle
    /// asserts: never claim an operation the machine did not perform.
    pub did_not_run: Option<&'static str>,
}

/// What the Python across many programs did — the report's view, and the
/// worklist for growing this reader.
#[derive(Debug, Default)]
pub struct Tally {
    pub programs: usize,
    /// File uses found, before the caller's rules about what may be a path.
    pub uses: usize,
    /// Those that survived those rules and were attributed to somebody. Filled
    /// in by the caller, because the rule is the caller's.
    pub kept: usize,
    /// Those the caller's rules then turned away, by which rule. Filled in by
    /// the caller for the same reason `kept` is.
    ///
    /// `uses == kept + refused.total()` — every use is one or the other, and
    /// there is a test that says so.
    pub refused: Refused,
    pub calls: BTreeMap<String, usize>,
    pub unresolved: BTreeMap<String, usize>,
    pub unknown: BTreeMap<String, usize>,
    /// Programs that moved their own working directory, whose relative paths
    /// are therefore not trusted.
    pub chdir: usize,
}

/// Why a use this reader *did* resolve still did not become a path.
///
/// ⚠ **Three different facts, and only the first is an unknown of the kind
/// [`Program::unresolved`] holds.** The program named a file plainly; what
/// stopped it is a rule of the layer above — which directory to read it
/// against, or whether a word may be a path at all. Kept apart because a rule
/// that turns away thousands is worth revisiting and an unknowable value is not.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Refused {
    /// The program called `os.chdir`, so its relative paths name no file this
    /// reader can find. The argument is usually computed, so the move cannot be
    /// followed and the paths cannot be trusted.
    pub moved: usize,
    /// Relative, with no directory to resolve it against — the shell's own `cd`
    /// went somewhere this reader could not follow either.
    pub no_directory: usize,
    /// Not shaped like a path, by the same rule a shell operand goes through.
    pub not_a_path: usize,
}

impl Refused {
    pub fn total(&self) -> usize {
        self.moved + self.no_directory + self.not_a_path
    }

    pub fn merge(&mut self, other: &Refused) {
        self.moved += other.moved;
        self.no_directory += other.no_directory;
        self.not_a_path += other.not_a_path;
    }
}

impl Tally {
    /// Fold one program's findings in.
    pub fn absorb(&mut self, program: Program) {
        self.programs += 1;
        self.uses += program.uses.len();
        self.chdir += usize::from(program.chdir);
        merge(&mut self.calls, program.calls);
        merge(&mut self.unresolved, program.unresolved);
        merge(&mut self.unknown, program.unknown);
    }

    /// Fold another tally in — a nested shell's Python is this script's Python.
    pub fn merge(&mut self, other: Tally) {
        self.programs += other.programs;
        self.uses += other.uses;
        self.kept += other.kept;
        self.refused.merge(&other.refused);
        self.chdir += other.chdir;
        merge(&mut self.calls, other.calls);
        merge(&mut self.unresolved, other.unresolved);
        merge(&mut self.unknown, other.unknown);
    }
}

fn merge(into: &mut BTreeMap<String, usize>, from: BTreeMap<String, usize>) {
    for (name, n) in from {
        *into.entry(name).or_insert(0) += n;
    }
}

/// Read a Python program.
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
    let Ok(mut parsed) = PythonParser::parse(Rule::program, source) else {
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

/// Whether the text is Python no interpreter would accept, so nothing in it ran.
///
/// One shape, because one shape is what the corpus has and a wrong answer here
/// *deletes* real file operations rather than inventing them: **an escaped
/// outer quote inside an f-string's replacement field.** `f"{d[\"k\"]}"` is
/// written that way to survive the shell's quoting, and the backslashes reach
/// the interpreter.
///
/// ⚠ **Checked against the interpreters that ran this corpus, not assumed.**
/// `SyntaxError` on 3.9.6 ("f-string expression part cannot include a
/// backslash") and on 3.12.14 ("unexpected character after line continuation
/// character"). PEP 701 lifted two neighbouring things on 3.12 and left this
/// one an error: the unescaped nested quote `f"{d["k"]}"` now runs, and so does
/// a backslash inside a *nested string*, `f"{'\n'.join(x)}"`. An earlier rule
/// here fired on any backslash and threw away two programs that worked, which
/// is why the test is the escaped quote and not the backslash.
///
/// **Measured 2026-08-17 against CPython 3.12.14** — `--example python-raised`
/// hands every program to `ast.parse`. Of 12,240 distinct programs it refuses
/// 72; this flags 39 and flags nothing CPython accepts. Of the 33 left, 19 hold
/// an unexpanded shell variable and so ran perfectly well once the shell had
/// substituted it, and 14 are genuinely broken — a heredoc cut short, mostly.
///
/// This is deliberately not a general validity check. `python.pest` accepts
/// punctuation it has no reading for by design, and turning it into a Python
/// syntax gate would cost far more than those last 14 programs are worth.
pub fn did_not_run(source: &str) -> Option<&'static str> {
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '#' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            // ⚠ **Every string is consumed whole, not only the f-strings.** This
            // corpus rewrites its own source — `s.replace('f"{a}\\n"', …)` — so an
            // f-string appears *inside* an ordinary literal as data. Scanning for
            // `f"` without tracking which quotes are open read four such programs
            // as broken and threw away the files they really did touch.
            '"' | '\'' => match string(&chars, i, "") {
                Ok(next) => i = next,
                Err(why) => return Some(why),
            },
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                // A name is a prefix only if the quote comes straight after it;
                // otherwise it is an ordinary identifier and `format` is not `f`.
                let word: String = chars[start..i].iter().collect();
                if matches!(chars.get(i), Some('"' | '\''))
                    && word.len() <= 2
                    && word.chars().all(|c| "rbufRBUF".contains(c))
                {
                    match string(&chars, i, &word) {
                        Ok(next) => i = next,
                        Err(why) => return Some(why),
                    }
                }
            }
            _ => i += 1,
        }
    }
    never_closed(&chars)
}

/// A string or bracket that is opened and never closed.
///
/// **A closure check, not a syntax check.** It asks nothing about whether the
/// statements mean anything — only whether every quote and bracket that opens
/// has a partner. `python.pest` declines to validate Python by design, so that
/// no program is ever rejected whole; this stays inside that rule, because an
/// unclosed literal cannot be a program under ANY reading.
///
/// ⚠ **A pass of its own rather than a branch in [`string`], because the two
/// disagree about a newline on purpose.** `string` stops a one-line literal at
/// its line's end, mirroring the grammar so an unterminated quote cannot
/// swallow the rest of the program. That recovery is right for reading and
/// wrong for judging: here the quote must actually be found, so the scan runs
/// to the end of the source.
///
/// Measured over all 12,240 distinct programs against CPython 3.12: catches
/// **11**, every one of which CPython refuses, and **over-claims none** — the
/// direction that costs, since flagging a program discards every file it named
/// (memview #1033).
fn never_closed(chars: &[char]) -> Option<&'static str> {
    let mut i = 0;
    let mut depth = 0i64;
    while i < chars.len() {
        match chars[i] {
            '#' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\\' => i += 2,
            '(' | '[' | '{' => {
                depth += 1;
                i += 1;
            }
            ')' | ']' | '}' => {
                depth -= 1;
                i += 1;
            }
            quote @ ('"' | '\'') => {
                let triple = chars.get(i + 1) == Some(&quote) && chars.get(i + 2) == Some(&quote);
                let width = if triple { 3 } else { 1 };
                i += width;
                loop {
                    if i >= chars.len() {
                        return Some(if triple {
                            "a triple-quoted string is never closed"
                        } else {
                            "a string is never closed"
                        });
                    }
                    if chars[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if chars[i] == quote
                        && (!triple
                            || (chars.get(i + 1) == Some(&quote)
                                && chars.get(i + 2) == Some(&quote)))
                    {
                        i += width;
                        break;
                    }
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    // ⚠ Only an unclosed OPEN counts. A surplus close is left alone: the
    // corpus writes `python3 -c` fragments whose brackets were balanced by the
    // shell around them, and a negative depth says nothing about the program.
    (depth > 0).then_some("a bracket is never closed")
}

/// Consume one string literal, starting at its opening quote, and return where
/// it ends. `Err` is the finding: this program raised before it ran.
fn string(chars: &[char], at: usize, prefix: &str) -> Result<usize, &'static str> {
    let quote = chars[at];
    let triple = chars.get(at + 1) == Some(&quote) && chars.get(at + 2) == Some(&quote);
    let raw = prefix.to_lowercase().contains('r');
    let formatted = prefix.to_lowercase().contains('f');
    let mut i = at + if triple { 3 } else { 1 };
    // Replacement-field depth, counted only where it can mean anything.
    let mut depth = 0usize;
    while i < chars.len() {
        // A one-line literal stops at its line's end — the same rule
        // `python.pest` follows, so an unterminated quote cannot swallow the
        // rest of the program.
        if !triple && chars[i] == '\n' {
            return Ok(i);
        }
        if triple
            && chars[i] == quote
            && chars.get(i + 1) == Some(&quote)
            && chars.get(i + 2) == Some(&quote)
        {
            return Ok(i + 3);
        }
        if !triple && chars[i] == quote && depth == 0 {
            return Ok(i + 1);
        }
        match chars[i] {
            '{' if formatted && chars.get(i + 1) == Some(&'{') => i += 1,
            '{' if formatted => depth += 1,
            '}' if formatted => depth = depth.saturating_sub(1),
            // ⚠ **The escaped quote, not any backslash.** PEP 701 allows a
            // backslash inside a replacement field on 3.12 — `f"{'\n'.join(x)}"`
            // runs there — and flagging those discarded two programs that
            // worked. What both 3.9 and 3.12 refuse is the *outer* quote
            // escaped, which is what surviving the shell's quoting produces.
            '\\' if formatted && depth > 0 && chars.get(i + 1) == Some(&quote) => {
                return Err("an escaped quote in an f-string replacement field");
            }
            // Outside a replacement field a backslash escapes the next
            // character, which is how `"a\"b"` stays one string.
            '\\' if !raw => i += 1,
            _ => {}
        }
        i += 1;
    }
    Ok(i)
}

// ---- constants ----

/// What a program's own names mean, before any of it is read.
struct Scope {
    /// Names bound exactly once, to a path literal — the only variables this
    /// trusts.
    consts: BTreeMap<String, String>,
    /// Every name the program bound itself, however it bound it. **A call on
    /// one of these is not a gap in this reader**: `def ri()` two lines up — or
    /// `ri = lambda: …`, which is how the corpus writes it as often — is not a
    /// library call anybody could teach it, and left in the worklist they crowd
    /// out the ones that can be.
    defined: BTreeSet<String>,
}

fn scope(elements: &[Pair<Rule>]) -> Scope {
    let mut bound: BTreeMap<String, Vec<Option<String>>> = BTreeMap::new();
    for element in elements {
        let mut inner = element.clone().into_inner();
        match element.as_rule() {
            Rule::assign => {
                if let (Some(name), Some(value)) = (inner.next(), inner.next()) {
                    bound
                        .entry(name.as_str().to_string())
                        .or_default()
                        .push(literal(&value));
                }
            }
            Rule::augmented => {
                if let Some(name) = inner.next() {
                    bound
                        .entry(name.as_str().to_string())
                        .or_default()
                        .push(None);
                }
            }
            // `for p in files`, `with … as f`, `import json`: bound, and to
            // nothing this can name.
            Rule::binder => {
                for target in inner.skip(1).flat_map(Pair::into_inner) {
                    bound
                        .entry(target.as_str().to_string())
                        .or_default()
                        .push(None);
                }
            }
            _ => {}
        }
    }
    Scope {
        defined: bound.keys().cloned().collect(),
        consts: bound
            .into_iter()
            .filter_map(|(name, values)| match values.as_slice() {
                [Some(value)] => Some((name, value.clone())),
                _ => None,
            })
            .collect(),
    }
}

/// The literal a `value` is, if the whole of it is one.
///
/// `'src/x.ts'` and `Path('src/x.ts')` are constants; `base + name` is not, and
/// neither is `'src/x.ts' if flag else 'y.ts'` — which is why this is asked of
/// the whole right-hand side rather than of its first operand.
fn literal(value: &Pair<Rule>) -> Option<String> {
    let mut operands = value.clone().into_inner();
    let only = operands.next()?;
    // A second operand means an operator between them, and a computed value.
    if operands.next().is_some() || only.as_rule() != Rule::operand {
        return None;
    }
    let mut parts = only.into_inner();
    let expr = parts.next().filter(|p| p.as_rule() == Rule::expr)?;
    let mut parts = expr.into_inner().peekable();
    let head = parts.next()?;
    if head.as_rule() == Rule::string {
        return parts
            .next()
            .is_none()
            .then(|| text(head.as_str()))
            .flatten();
    }
    // `Path('x')` and `pathlib.Path('x')` name a file without opening one.
    let mut name = head.as_str().to_string();
    if name == "pathlib"
        && let Some(attr) = parts.next_if(|p| p.as_rule() == Rule::attr)
    {
        name = attr.into_inner().next()?.as_str().to_string();
    }
    if !matches!(name.as_str(), "Path" | "PurePath") {
        return None;
    }
    let call = parts.next().filter(|p| p.as_rule() == Rule::call)?;
    if parts.next().is_some() {
        return None;
    }
    match single_argument(&call)? {
        Value::Text(path) => Some(path),
        Value::Unknown => None,
    }
}

/// The one literal argument of a call, without reading the call for uses — the
/// constants pass must have no effect of its own.
fn single_argument(call: &Pair<Rule>) -> Option<Value> {
    let args = call.clone().into_inner().next()?;
    let mut args = args.into_inner();
    let arg = args.next()?;
    if args.next().is_some() {
        return None;
    }
    let value = arg
        .into_inner()
        .next()
        .filter(|p| p.as_rule() == Rule::value)?;
    let mut operands = value.into_inner();
    let only = operands.next()?;
    if operands.next().is_some() {
        return None;
    }
    let expr = only.into_inner().next()?;
    let mut parts = expr.into_inner();
    let head = parts.next()?;
    match (head.as_rule(), parts.next()) {
        (Rule::string, None) => text(head.as_str()).map(Value::Text),
        _ => Some(Value::Unknown),
    }
}

/// The value of a string literal, or `None` when it has none: an f-string with
/// a placeholder in it is a shape, not a path.
fn text(raw: &str) -> Option<String> {
    let quote_at = raw.find(['\'', '"'])?;
    let prefix = raw[..quote_at].to_ascii_lowercase();
    let body = &raw[quote_at..];
    let quote = body.chars().next()?;
    let fence = if body.starts_with(&quote.to_string().repeat(3)) {
        3
    } else {
        1
    };
    let inner = body.get(fence..body.len().saturating_sub(fence))?;
    let literal = prefix.contains('r');
    let formatted = prefix.contains('f');
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' if !literal => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            },
            '{' if formatted => match chars.next() {
                Some('{') => out.push('{'),
                // `f"{root}/x.ts"` names a file whose name is not here.
                _ => return None,
            },
            c => out.push(c),
        }
    }
    Some(out)
}

// ---- reading ----

/// A value, as far as it can be known. A string literal is `Text` whether or not
/// it turns out to name a file — deciding that is [`crate::shell_ops`]'s job,
/// and doing it twice would mean two rules to keep in step.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    Unknown,
}

/// One argument of a call.
struct Arg {
    keyword: Option<String>,
    value: Value,
}

/// Modules whose functions are named by their module, so `os.remove` is one name
/// rather than a `remove` method on something called `os`.
const MODULES: &[&str] = &[
    "os",
    "shutil",
    "glob",
    "json",
    "pathlib",
    "subprocess",
    "sys",
    "re",
    "csv",
    "io",
    "yaml",
    "time",
    "datetime",
    "math",
    "random",
    "hashlib",
    "sqlite3",
    "tempfile",
    "textwrap",
    "itertools",
    "collections",
    "argparse",
    "base64",
    "urllib",
    "typing",
    "traceback",
    "difflib",
    "ast",
];

/// Keywords, which the grammar leaves as ordinary names — the same choice
/// `shell.pest` makes with `do` and `done`, and for the same reason: a keyword
/// rule would have to decide what `print(x if y else z)` is before knowing.
/// None of them is a call, whatever bracket follows.
const KEYWORDS: &[&str] = &[
    "in", "not", "and", "or", "is", "if", "else", "elif", "for", "while", "return", "yield",
    "assert", "del", "raise", "with", "as", "lambda", "pass", "break", "continue", "from",
    "import", "global", "nonlocal", "try", "except", "finally", "def", "class", "None", "True",
    "False", "await", "async",
];

/// Calls that are understood and touch no file.
///
/// The same distinction the shell's `Verb::NoFiles` draws, and for the same
/// reason: without it the worklist of things still to teach this reader is
/// headed by `print` forever.
const NOTHING: &[&str] = &[
    "print",
    "len",
    "str",
    "int",
    "float",
    "bool",
    "list",
    "dict",
    "set",
    "tuple",
    "sorted",
    "enumerate",
    "range",
    "zip",
    "map",
    "filter",
    "sum",
    "min",
    "max",
    "any",
    "all",
    "isinstance",
    "repr",
    "type",
    "input",
    "abs",
    "round",
    "chr",
    "ord",
    "hex",
    "format",
    "vars",
    "dir",
    "getattr",
    "setattr",
    "hasattr",
    "next",
    "iter",
    "reversed",
    "divmod",
    "id",
    "hash",
    "bytes",
    "bytearray",
    "frozenset",
    "slice",
    "callable",
    "super",
    "json.dumps",
    "json.loads",
    // Its argument is a handle, and the `open` that made it is already counted.
    "json.load",
    "json.dump",
    "yaml.safe_load",
    "yaml.safe_dump",
    "csv.reader",
    "csv.writer",
    "csv.DictReader",
    "csv.DictWriter",
    "re.compile",
    "re.sub",
    "re.search",
    "re.match",
    "re.fullmatch",
    "re.findall",
    "re.finditer",
    "re.escape",
    "re.split",
    "sys.exit",
    "time.sleep",
    "time.time",
    "collections.defaultdict",
    "collections.Counter",
    "textwrap.dedent",
    "datetime.datetime",
    "math.floor",
    "math.ceil",
    "math.radians",
    "math.cos",
    "math.sin",
    "math.tan",
    "math.atan2",
    "math.hypot",
    "math.log",
    "math.exp",
    "math.pi",
    "math.degrees",
    "math.sqrt",
    "sys.path.insert",
    "sys.path.append",
    "datetime.fromisoformat",
    "struct.unpack",
    "struct.pack",
    "hashlib.sha256",
    "hashlib.md5",
    "itertools.groupby",
    "argparse.ArgumentParser",
    "Counter",
    "defaultdict",
    "os.getcwd",
    "os.getenv",
    "os.path.dirname",
    "os.path.basename",
    "os.path.splitext",
    "os.path.getsize",
    "os.path.getmtime",
    "difflib.unified_diff",
];

/// Methods that are understood and touch no file: string handling, and the read
/// and write calls on a handle whose `open` was already counted.
///
/// `replace` is deliberately absent from every table here. It is `str.replace`
/// nine times in ten and `Path.replace` once, and the two mean opposite things —
/// so it is read as neither.
const NOTHING_METHODS: &[&str] = &[
    "write",
    "writelines",
    "read",
    "readline",
    "readlines",
    "close",
    "seek",
    "tell",
    "flush",
    "strip",
    "lstrip",
    "rstrip",
    "split",
    "rsplit",
    "splitlines",
    "join",
    "startswith",
    "endswith",
    "format",
    "encode",
    "decode",
    "append",
    "extend",
    "insert",
    "pop",
    "get",
    "items",
    "keys",
    "values",
    "update",
    "add",
    "setdefault",
    "sort",
    "index",
    "count",
    "lower",
    "upper",
    "title",
    "group",
    "groups",
    "groupdict",
    "sub",
    "findall",
    "match",
    "search",
    "finditer",
    "removeprefix",
    "removesuffix",
    "ljust",
    "rjust",
    "zfill",
    "isdigit",
    "isalpha",
    "replace",
    // Database work: the file was named at `connect`, and what the SQL does to
    // it is not readable from here.
    "execute",
    "executemany",
    "executescript",
    "fetchone",
    "fetchall",
    "fetchmany",
    "commit",
    "rollback",
    "cursor",
    // Numbers, dates and text, in the shapes the corpus's analysis scripts use.
    "unpack",
    "pack",
    "total_seconds",
    "timestamp",
    "isoformat",
    "strftime",
    "astype",
    "mean",
    "median",
    "round",
    "unescape",
    "find",
    "rfind",
    "hexdigest",
    "digest",
    "array",
    "asarray",
    "norm",
    "percentile",
    "std",
    "sum",
    "argmin",
    "argmax",
    "reshape",
    "flatten",
    "tolist",
    "most_common",
    "convert",
    "crop",
    "resize",
    "thumbnail",
    "getroot",
    "iter",
    "findtext",
    "capitalize",
    "casefold",
    "partition",
    "rpartition",
    "expandtabs",
    "translate",
    "center",
    "swapcase",
    "islower",
    "isupper",
    "isspace",
    "isnumeric",
    "isidentifier",
];

/// Predicates that ask *about* a file without using it.
///
/// Recorded as nothing, deliberately. `if not p.exists(): …` is the shape of
/// code that decides whether to touch a file at all, and counting the question
/// as an answer would credit an agent with work it may have skipped.
const PREDICATES: &[&str] = &[
    "exists",
    "is_file",
    "is_dir",
    "is_symlink",
    "stat",
    "lstat",
    "samefile",
];

/// What a named function does — **the closed set, named once**.
///
/// The reader below matches on this rather than on the name, so the string is
/// read exactly once, at [`callable`], and a function nobody taught it cannot
/// slip through a catch-all arm pretending to be understood. The same shape
/// `shell_ops::Verb` has, for the same reason.
#[derive(Debug, Clone, Copy)]
enum Call {
    /// `open(path, mode)` — the mode decides the direction.
    Open,
    /// Names a path without using it: `Path('x')`, `os.path.abspath(p)`.
    Name,
    /// Joins its arguments into one path: `os.path.join('src', 'x.ts')`.
    Join,
    /// Deletes its first argument.
    Delete,
    /// Reads the first and writes the second: `shutil.copy`, `os.rename`.
    Transfer,
    /// Looks inside a directory: `glob.glob`, `os.walk`.
    Walk,
    /// Opens a database, which is a file like any other.
    Database,
    /// A directory, which this cannot attribute — `mkdir` names a place, not a
    /// file, and `makedirs` names several at once.
    Directory,
    /// Moves the program's working directory.
    ChangeDir,
    /// Understood, and touches no file.
    ///
    /// Distinct from a name that is absent, and the distinction is the whole
    /// worklist: without it the list of calls still to teach this reader is
    /// headed by `print` forever.
    Nothing,
}

/// The one place a function name is read. `None` means "not taught yet".
fn callable(name: &str) -> Option<Call> {
    Some(match name {
        "open" | "io.open" | "codecs.open" => Call::Open,
        "Path" | "PurePath" | "pathlib.Path" | "pathlib.PurePath" | "os.fspath"
        | "os.path.abspath" | "os.path.realpath" | "os.path.normpath" | "os.path.expanduser" => {
            Call::Name
        }
        "os.path.join" => Call::Join,
        "os.remove" | "os.unlink" | "shutil.rmtree" => Call::Delete,
        "os.rename" | "os.replace" | "shutil.move" | "shutil.copy" | "shutil.copy2"
        | "shutil.copyfile" => Call::Transfer,
        "glob.glob" | "glob.iglob" | "os.listdir" | "os.scandir" | "os.walk" => Call::Walk,
        // A database is a file, and connecting to one uses it. Read rather than
        // written: whether a statement later changed it is in the SQL, which
        // this does not read, and an undercount beats an invention.
        "sqlite3.connect" => Call::Database,
        "os.makedirs" | "os.mkdir" | "os.rmdir" => Call::Directory,
        "os.chdir" => Call::ChangeDir,
        // `x not in ('a.md', 'b.md')` is a keyword before a bracket, not a call.
        _ if KEYWORDS.contains(&name) || NOTHING.contains(&name) => Call::Nothing,
        _ => return None,
    })
}

/// What a method does to the value it is called on — the closed set again, and
/// read once at [`method_of`].
#[derive(Debug, Clone, Copy)]
enum Method {
    /// Replaces the receiver: `write_text`, `touch`.
    Write,
    /// Reads it: `read_text`, `read_bytes`.
    Read,
    /// Deletes it: `unlink`.
    Delete,
    /// Reads the receiver and writes the argument.
    Rename,
    /// `open(mode)` on a path.
    Open,
    /// Looks inside the receiver: `glob`, `iterdir`.
    Walk,
    /// Puts something *into* the argument: `img.save(p)`, `frame.to_csv(p)`.
    Save,
    /// Extends the receiver: `joinpath`.
    Join,
    /// The same path under another spelling: `resolve`, `expanduser`.
    Same,
    /// A directory.
    Directory,
    /// Understood, and touches no file: string handling, and the reads and
    /// writes on a handle whose `open` was already counted.
    Nothing,
}

fn method_of(name: &str) -> Option<Method> {
    Some(match name {
        "write_text" | "write_bytes" | "touch" => Method::Write,
        "read_text" | "read_bytes" => Method::Read,
        "unlink" => Method::Delete,
        "rename" => Method::Rename,
        "open" => Method::Open,
        "glob" | "rglob" | "iterdir" => Method::Walk,
        "save" | "savefig" | "to_csv" | "to_json" | "to_parquet" => Method::Save,
        "joinpath" => Method::Join,
        "resolve" | "expanduser" | "absolute" => Method::Same,
        "mkdir" | "rmdir" => Method::Directory,
        _ if PREDICATES.contains(&name) || NOTHING_METHODS.contains(&name) => Method::Nothing,
        _ => return None,
    })
}

struct Reader {
    consts: BTreeMap<String, String>,
    defined: BTreeSet<String>,
    out: Program,
}

impl Reader {
    /// One statement. An assignment's right-hand side is read like any other
    /// expression — `s = open(p).read()` opens a file whatever it does with it.
    fn element(&mut self, pair: Pair<Rule>) {
        match pair.as_rule() {
            Rule::expr => {
                self.expr(pair);
            }
            Rule::assign => {
                if let Some(value) = pair.into_inner().find(|p| p.as_rule() == Rule::value) {
                    self.value(value);
                }
            }
            _ => {}
        }
    }

    /// A value: its operands, and what the operators between them leave known.
    fn value(&mut self, pair: Pair<Rule>) -> Value {
        let mut values = Vec::new();
        let mut operators = Vec::new();
        for part in pair.into_inner() {
            match part.as_rule() {
                Rule::operand => values.push(self.operand(part)),
                Rule::binop => operators.push(part.as_str().to_string()),
                _ => {}
            }
        }
        match values.len() {
            0 => Value::Unknown,
            1 => values.remove(0),
            // `Path(root) / 'src' / 'x.ts'` — pathlib's join, and the only
            // arithmetic here that leaves a path behind.
            _ if operators.iter().all(|op| op == "/") => {
                let mut joined = String::new();
                for value in values {
                    match value {
                        Value::Text(part) if joined.is_empty() => joined = part,
                        Value::Text(part) => {
                            joined = format!("{}/{part}", joined.trim_end_matches('/'));
                        }
                        Value::Unknown => return Value::Unknown,
                    }
                }
                Value::Text(joined)
            }
            _ => Value::Unknown,
        }
    }

    /// An operand, which a unary operator makes unknowable — `*paths` is a list,
    /// not a path.
    fn operand(&mut self, pair: Pair<Rule>) -> Value {
        let mut value = Value::Unknown;
        let mut plain = true;
        for part in pair.into_inner() {
            match part.as_rule() {
                Rule::expr => value = self.expr(part),
                _ => plain = false,
            }
        }
        if plain { value } else { Value::Unknown }
    }

    /// An atom and the calls chained onto it.
    fn expr(&mut self, pair: Pair<Rule>) -> Value {
        let mut parts = pair.into_inner().peekable();
        let Some(head) = parts.next() else {
            return Value::Unknown;
        };
        let mut value = match head.as_rule() {
            Rule::string => text(head.as_str()).map_or(Value::Unknown, Value::Text),
            // `(Path('src') / 'x.ts')` — brackets round one thing are that
            // thing, and pathlib's join is written that way as often as not.
            Rule::paren => match self.arguments(&head).as_slice() {
                [only] if only.keyword.is_none() => only.value.clone(),
                _ => Value::Unknown,
            },
            Rule::list | Rule::dict => {
                self.arguments(&head);
                Value::Unknown
            }
            Rule::name => {
                // A module's function is named by its module: `os.path.exists`
                // is one name, where `p.exists` is a method on a value.
                let mut name = head.as_str().to_string();
                if MODULES.contains(&name.as_str()) {
                    while let Some(attr) = parts.next_if(|p| p.as_rule() == Rule::attr) {
                        name.push('.');
                        name.push_str(attr.as_str().trim_start_matches('.'));
                    }
                }
                match parts.next_if(|p| p.as_rule() == Rule::call) {
                    Some(call) => {
                        let args = self.arguments(&call);
                        self.named_call(&name, &args)
                    }
                    // A module is not a value, and neither is an attribute of
                    // one: only a name bound to a literal is.
                    None if name.contains('.') => Value::Unknown,
                    None => self
                        .consts
                        .get(&name)
                        .map_or(Value::Unknown, |path| Value::Text(path.clone())),
                }
            }
            _ => Value::Unknown,
        };
        while let Some(part) = parts.next() {
            value = match part.as_rule() {
                Rule::attr => {
                    let method = part.as_str().trim_start_matches('.').to_string();
                    match parts.next_if(|p| p.as_rule() == Rule::call) {
                        Some(call) => {
                            let args = self.arguments(&call);
                            self.method_call(&value, &method, &args)
                        }
                        // An attribute is not a call: `p.parent` is some other
                        // path, and this does not know which.
                        None => Value::Unknown,
                    }
                }
                Rule::call | Rule::subscript => {
                    self.arguments(&part);
                    Value::Unknown
                }
                _ => Value::Unknown,
            };
        }
        value
    }

    /// The arguments of a call, each read as an expression in its own right —
    /// so a call inside one, `json.dump(data, open(p, 'w'))`, is counted where
    /// it stands.
    fn arguments(&mut self, pair: &Pair<Rule>) -> Vec<Arg> {
        let mut out = Vec::new();
        let Some(args) = pair
            .clone()
            .into_inner()
            .find(|p| p.as_rule() == Rule::args)
        else {
            return out;
        };
        for arg in args.into_inner() {
            let mut keyword = None;
            let mut value = Value::Unknown;
            for part in arg.into_inner() {
                match part.as_rule() {
                    Rule::keyword => {
                        keyword = part
                            .into_inner()
                            .next()
                            .map(|name| name.as_str().to_string());
                    }
                    Rule::value => value = self.value(part),
                    _ => {}
                }
            }
            out.push(Arg { keyword, value });
        }
        out
    }

    /// A call by name: understood, the program's own, or one for the worklist.
    fn named_call(&mut self, name: &str, args: &[Arg]) -> Value {
        match callable(name) {
            Some(does) => self.call(does, name, args),
            // A function the program defined itself is not a gap in this
            // reader, and left in the worklist they crowd out the ones that are.
            None if self.defined.contains(name) => Value::Unknown,
            None => {
                *self.out.unknown.entry(name.to_string()).or_insert(0) += 1;
                Value::Unknown
            }
        }
    }

    /// The same for a method, which is filed under a leading dot so the two
    /// worklists cannot be confused for one another.
    fn method_call(&mut self, receiver: &Value, name: &str, args: &[Arg]) -> Value {
        match method_of(name) {
            Some(does) => self.method(receiver, does, name, args),
            None => {
                *self.out.unknown.entry(format!(".{name}")).or_insert(0) += 1;
                Value::Unknown
            }
        }
    }

    /// A call, once its name has been read.
    fn call(&mut self, does: Call, name: &str, args: &[Arg]) -> Value {
        let arg = |n: usize| positional(args, n).cloned();
        match does {
            Call::Open => {
                let write = writes(named(args, 1, "mode"));
                self.record(name, arg(0), write);
                Value::Unknown
            }
            Call::Name => arg(0).unwrap_or(Value::Unknown),
            Call::Join => join(args),
            Call::Delete => {
                self.record(name, arg(0), true);
                Value::Unknown
            }
            Call::Transfer => {
                self.record(name, arg(0), false);
                self.record(name, arg(1), true);
                Value::Unknown
            }
            Call::Walk | Call::Database => {
                self.record(name, arg(0), false);
                Value::Unknown
            }
            Call::ChangeDir => {
                self.out.chdir = true;
                Value::Unknown
            }
            Call::Directory | Call::Nothing => Value::Unknown,
        }
    }

    /// A method call, which is a file operation only when the value it is called
    /// on is a path this knows.
    fn method(&mut self, receiver: &Value, does: Method, name: &str, args: &[Arg]) -> Value {
        let receiver = || Some(receiver.clone());
        match does {
            Method::Write => {
                self.record(name, receiver(), true);
                Value::Unknown
            }
            Method::Read | Method::Walk => {
                self.record(name, receiver(), false);
                Value::Unknown
            }
            // Deleting is changing, and this is the shape `rm` takes here.
            Method::Delete => {
                self.record(name, receiver(), true);
                Value::Unknown
            }
            Method::Rename => {
                self.record(name, receiver(), false);
                self.record(name, positional(args, 0).cloned(), true);
                Value::Unknown
            }
            Method::Open => {
                let write = writes(named(args, 0, "mode"));
                self.record(name, receiver(), write);
                Value::Unknown
            }
            // `img.save('out.png')`, `fig.savefig(p)`, `frame.to_csv(p)` — here
            // the receiver is a picture or a table and the *argument* is the
            // file, which is the other way round from every method above.
            Method::Save => {
                self.record(name, positional(args, 0).cloned(), true);
                Value::Unknown
            }
            Method::Join => match (receiver(), positional(args, 0)) {
                (Some(Value::Text(base)), Some(Value::Text(rest))) => {
                    Value::Text(format!("{}/{rest}", base.trim_end_matches('/')))
                }
                _ => Value::Unknown,
            },
            // The same path under another spelling.
            Method::Same => receiver().unwrap_or(Value::Unknown),
            Method::Directory | Method::Nothing => Value::Unknown,
        }
    }

    /// Record a file operation — or, when its path is not knowable, the fact
    /// that one was missed.
    fn record(&mut self, call: &str, value: Option<Value>, write: bool) {
        *self.out.calls.entry(call.to_string()).or_insert(0) += 1;
        match value {
            Some(Value::Text(path)) if !path.is_empty() => {
                self.out.uses.push(Use { path, write });
            }
            _ => *self.out.unresolved.entry(call.to_string()).or_insert(0) += 1,
        }
    }
}

/// The nth positional argument.
fn positional(args: &[Arg], n: usize) -> Option<&Value> {
    args.iter()
        .filter(|arg| arg.keyword.is_none())
        .nth(n)
        .map(|arg| &arg.value)
}

/// An argument given either by position or by name — `open(p, 'w')` and
/// `open(p, mode='w')` are the same call.
fn named<'a>(args: &'a [Arg], n: usize, keyword: &str) -> Option<&'a Value> {
    args.iter()
        .find(|arg| arg.keyword.as_deref() == Some(keyword))
        .map(|arg| &arg.value)
        .or_else(|| positional(args, n))
}

/// Whether an `open` mode changes the file. An unknown mode is a read: that is
/// the default, and guessing the other way invents a change.
fn writes(mode: Option<&Value>) -> bool {
    match mode {
        Some(Value::Text(mode)) => mode.contains(['w', 'a', 'x', '+']),
        _ => false,
    }
}

/// `os.path.join('src', 'geo', 'x.ts')` — a path when every part is known.
fn join(args: &[Arg]) -> Value {
    let mut parts = Vec::new();
    for arg in args.iter().filter(|arg| arg.keyword.is_none()) {
        match &arg.value {
            Value::Text(part) => parts.push(part.trim_end_matches('/').to_string()),
            Value::Unknown => return Value::Unknown,
        }
    }
    match parts.is_empty() {
        true => Value::Unknown,
        false => Value::Text(parts.join("/")),
    }
}
