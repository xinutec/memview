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
//! therefore none. A name bound twice to two LITERALS is the exception, and it
//! is not a constant either — it is a set, reported as a bound.
//!
//! ⚠ **An imported name is not a path at all**, which is a stronger statement
//! than "not a constant" and a different one. `Image.open(p)` reads like
//! `p.open()` and means the opposite: the receiver is a library and the
//! argument is the file. See [`callable`] for why that is three named pairs and
//! not a rule about imports.
//!
//! ⚠ **Trusting only a name bound once is a limitation, not a principle, and
//! the shape of its replacement is already decided.** "Bound exactly once to a
//! literal" is constant propagation with a domain of two values, written by
//! hand for one language. **`for p in ['a.ts', 'b.ts']:` is fully determined by the text** —
//! it names two files, not none — and the same is true of the shell's 3,078
//! loops over a literal word list. What this cannot follow is a value that
//! depends on the world: a loop over `Path('.').glob(…)`, a name assigned in
//! both arms of an `if`, an argument built from `sys.argv`. The gap stands at
//! **2,777 of 26,536 operations that name no file**, 2026-08-25, and the census
//! of what it cannot name is in `docs/reader.md`. A loop over a glob or a
//! literal list is read; a loop over a name the program was handed is not, and
//! neither is a value built from `sys.argv`.
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

pub use crate::program::{Program, Ran, Refused, Tally, Use};

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
        candidates: scope.candidates,
        ranging: scope.ranging,
        imported: scope.imported,
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
    /// Names the program bound to SEVERAL literals and nothing else, by the
    /// candidates in order. One of them was the path; which one is not knowable
    /// without running it — see [`crate::program::Program::bounded`].
    ///
    /// ⚠ **Every binding has to be a literal.** One `p = compute()` among them
    /// and the set has a hole, so it bounds nothing and the name goes back to
    /// being opaque. A set with a hole in it is not a set.
    candidates: BTreeMap<String, Vec<String>>,
    /// Names a `for` bound to a language — a glob's pattern, or a written-out
    /// list. **Bound exactly once, and by that loop**: a name the program also
    /// assigns is not ranging over anything by the time it is used.
    ranging: BTreeMap<String, Value>,
    /// Names the program imported. **Not paths**, whatever else they are —
    /// which is what lets a call on one be read as the library call it is
    /// rather than as a method on a value nobody can name.
    imported: BTreeSet<String>,
    /// Every name the program bound itself, however it bound it. **A call on
    /// one of these is not a gap in this reader**: `def ri()` two lines up — or
    /// `ri = lambda: …`, which is how the corpus writes it as often — is not a
    /// library call anybody could teach it, and left in the worklist they crowd
    /// out the ones that can be.
    defined: BTreeSet<String>,
}

/// The language a `for` ranges over, when the text determines one.
///
/// Two shapes, and they are the two the corpus writes. A **glob** gives a
/// pattern; a **literal list** gives a set. Both are languages, which is what
/// separates them from `for p in files` — a name whose value came from
/// somewhere this cannot see, and which therefore has no space at all.
///
/// ⚠ **`sorted`, `list`, `set`, `tuple` and `reversed` are transparent.** Order
/// and duplicates are not part of a language, so the answer is the same
/// underneath them — and 250 of the corpus's 531 glob loops are written
/// `sorted(glob.glob(…))`.
///
/// ⚠ **`enumerate` is NOT**, and neither is `zip`: they yield tuples, so the
/// loop's first name is an index and its second is the path. Reading through
/// one would bind the wrong name to the language.
fn ranges_over(iterable: &Pair<Rule>) -> Option<Value> {
    let mut operands = iterable.clone().into_inner();
    let only = operands.next()?;
    // A second operand means an operator between them, and a computed value.
    if operands.next().is_some() || only.as_rule() != Rule::operand {
        return None;
    }
    let expr = only
        .into_inner()
        .next()
        .filter(|p| p.as_rule() == Rule::expr)?;
    let mut parts = expr.into_inner().peekable();
    let head = parts.next()?;
    match head.as_rule() {
        // `['out/a.txt', 'out/b.txt']` — the language written out.
        Rule::list => {
            if parts.next().is_some() {
                return None;
            }
            let args = head.into_inner().next()?;
            let mut paths = Vec::new();
            for arg in args.into_inner() {
                let mut parts = arg.into_inner();
                let value = parts.next().filter(|p| p.as_rule() == Rule::value)?;
                // ⚠ One member that is not a literal and the set has a hole, so
                // it bounds nothing — the rule `candidates` already keeps.
                paths.push(literal(&value)?);
            }
            paths.sort();
            paths.dedup();
            (paths.len() > 1).then_some(Value::OneOf(paths))
        }
        Rule::name => {
            let mut name = head.as_str().to_string();
            if name == "glob"
                && let Some(attr) = parts.next_if(|p| p.as_rule() == Rule::attr)
            {
                name = format!("glob.{}", attr.as_str().trim_start_matches('.'));
            }
            let call = parts.next().filter(|p| p.as_rule() == Rule::call)?;
            if parts.next().is_some() {
                return None;
            }
            match name.as_str() {
                "glob.glob" | "glob.iglob" => match single_argument(&call)? {
                    Value::Text(pattern) => Some(Value::Pattern(pattern)),
                    _ => None,
                },
                "sorted" | "list" | "set" | "tuple" | "reversed" => {
                    let args = call.into_inner().next()?;
                    let mut args = args.into_inner();
                    let arg = args.next()?;
                    if args.next().is_some() {
                        return None;
                    }
                    let inner = arg
                        .into_inner()
                        .next()
                        .filter(|p| p.as_rule() == Rule::value)?;
                    ranges_over(&inner)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The directory a pattern is rooted at — the text ahead of its first wildcard,
/// cut at the last `/`.
///
/// ⚠ **Rooted at, not contained in**, exactly as the shell reader's locus is:
/// the answer lies under this directory unless the pattern climbs out, and only
/// `..` does that. `*.log` has no locus and gets none.
fn locus(pattern: &str) -> Option<String> {
    let fixed = pattern
        .find(['*', '?', '['])
        .map_or(pattern, |at| &pattern[..at]);
    let dir = fixed.rsplit_once('/')?.0;
    (!dir.is_empty()).then(|| dir.to_string())
}

fn scope(elements: &[Pair<Rule>]) -> Scope {
    let mut ranging: BTreeMap<String, Value> = BTreeMap::new();
    let mut imported = BTreeSet::new();
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
                let importing = element
                    .clone()
                    .into_inner()
                    .next()
                    .is_some_and(|keyword| keyword.as_str() == "import");
                // ⚠ **`target` by name, not "everything after the keyword".**
                // The header now carries `ranges_over` too, and flattening that
                // would bind the loop to the words of its own iterable.
                // ⚠ **One name only.** `for k, v in d.items()` yields a pair,
                // so a language over the sequence belongs to neither name and
                // giving it to the first would bind a key to a set of paths.
                let over = element
                    .clone()
                    .into_inner()
                    .find(|part| part.as_rule() == Rule::ranges_over)
                    .and_then(|over| over.into_inner().find(|p| p.as_rule() == Rule::iterable))
                    .as_ref()
                    .and_then(ranges_over);
                let sole = element
                    .clone()
                    .into_inner()
                    .find(|part| part.as_rule() == Rule::target)
                    .filter(|target| target.clone().into_inner().count() == 1);
                if let (Some(over), Some(target)) = (over, sole) {
                    ranging.insert(target.as_str().trim().to_string(), over);
                }
                for target in inner
                    .filter(|part| part.as_rule() == Rule::target)
                    .flat_map(Pair::into_inner)
                {
                    if importing {
                        imported.insert(target.as_str().to_string());
                    }
                    bound
                        .entry(target.as_str().to_string())
                        .or_default()
                        .push(None);
                }
            }
            _ => {}
        }
    }
    let bound_once: BTreeMap<String, usize> = bound
        .iter()
        .map(|(name, ways)| (name.clone(), ways.len()))
        .collect();
    let defined = bound.keys().cloned().collect();
    let mut consts = BTreeMap::new();
    let mut candidates = BTreeMap::new();
    for (name, values) in bound {
        match values.as_slice() {
            [Some(value)] => {
                consts.insert(name, value.clone());
            }
            // ⚠ **Was `_ => None`, and that one line was the largest unnamed
            // shape in the corpus.** A name bound twice to two literals had BOTH
            // thrown away, so `p = 'a'; p = 'b'; open(p)` named nothing while the
            // reader held every path it could be.
            many if many.len() > 1 && many.iter().all(Option::is_some) => {
                let mut paths: Vec<String> = many.iter().flatten().cloned().collect();
                paths.sort();
                paths.dedup();
                // Bound to the same literal twice is one path, not a choice.
                match paths.len() {
                    1 => {
                        consts.insert(name, paths.remove(0));
                    }
                    _ => {
                        candidates.insert(name, paths);
                    }
                }
            }
            _ => {}
        }
    }
    // ⚠ **Bound once, by the loop that named the language.** A name the
    // program also assigns has left the loop's space by the time it is used,
    // and `bound` is where that shows: one entry means one binding.
    ranging.retain(|name, _| bound_once.get(name).is_some_and(|ways| *ways == 1));
    Scope {
        defined,
        ranging,
        imported,
        consts,
        candidates,
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
        // Neither a list nor a choice is a path, which is the same answer
        // `Unknown` gets. A set here would make the CONSTANTS pass bind a name
        // to one of its own candidates, which is how a set becomes a wrong
        // certainty.
        Value::List(_) | Value::OneOf(_) | Value::Pattern(_) | Value::Unknown => None,
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
        (Rule::string, None) => Some(
            text(head.as_str())
                .map(Value::Text)
                .or_else(|| shape(head.as_str()).map(Value::Pattern))
                .unwrap_or(Value::Unknown),
        ),
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

/// What an f-string still says about the path, as a glob.
///
/// ⚠ **[`text`] returns `None` here and that is right for a literal** — the
/// name is genuinely not in the text. What is wrong is throwing away everything
/// AROUND the hole: `f"data/{name}.stream"` is not unknowable, it is
/// `data/*.stream`, a language with a locus, the same object a `glob.glob`
/// argument produces. Measured 2026-08-26 (`--example fstring-shapes`): of 289
/// interpolated f-strings in a file operation's path argument, 44.3% carry a
/// literal directory and 33.9% a certain filename.
///
/// ⚠ **Refuses everything that is not path-shaped, and most f-strings are
/// not.** 94.3% of the corpus's are `print` formatting — `*=*`, `Bearer *`,
/// `#* [*] *` — and a rule that files those as bounded paths would invent
/// thousands of subjects. So a space, a URL scheme, or a literal that is
/// nothing but separators is refused, which is the direction that claims less.
fn shape(raw: &str) -> Option<String> {
    let quote_at = raw.find(['\'', '"'])?;
    let prefix = raw[..quote_at].to_ascii_lowercase();
    if !prefix.contains('f') {
        return None;
    }
    let body = &raw[quote_at..];
    let quote = body.chars().next()?;
    let fence = if body.starts_with(&quote.to_string().repeat(3)) {
        3
    } else {
        1
    };
    let inner = body.get(fence..body.len().saturating_sub(fence))?;
    let literal = prefix.contains('r');
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if !literal => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            },
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            // The hole. A format spec ends at the same `}` and is just as
            // unknown, so neither is followed. Adjacent holes are one run.
            '{' => {
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                }
                if !out.ends_with('*') {
                    out.push('*');
                }
            }
            c => out.push(c),
        }
    }
    path_shaped(&out).then_some(out)
}

/// Whether a rendered shape could be a path at all.
///
/// Each refusal is a population the census named, and every one of them is
/// bigger than the thing being gained.
fn path_shaped(pattern: &str) -> bool {
    if !pattern.contains('*') {
        return false;
    }
    let literal: String = pattern.chars().filter(|c| *c != '*').collect();
    // Whitespace says a sentence; a scheme says a URL, which is a subject but
    // not a file on this machine.
    if literal.trim().is_empty() || literal.contains([' ', '\t', '\n']) || pattern.contains("://") {
        return false;
    }
    // `*/*` and `*.` name neither a directory nor a file.
    if literal.chars().all(|c| c == '/' || c == '.') {
        return false;
    }
    // Something must be certain: a directory before the first hole, or an
    // extension after the last one.
    let located = pattern
        .split('*')
        .next()
        .is_some_and(|head| head.contains('/'));
    let extension = pattern
        .rsplit('*')
        .next()
        .is_some_and(|tail| tail.starts_with('.') && tail.len() > 1 && !tail.contains('/'));
    located || extension || literal.contains('/')
}

// ---- reading ----

/// A value, as far as it can be known. A string literal is `Text` whether or not
/// it turns out to name a file — deciding that is [`crate::shell_ops`]'s job,
/// and doing it twice would mean two rules to keep in step.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    /// `["ffmpeg", "-i", f]` — the shape `subprocess.run` is written in, and the
    /// only reason this exists. Every other reader of a `Value` treats it as
    /// unknown, which is what it is: a list is not a path.
    List(Vec<Value>),
    /// A language rather than a set: `⟦p⟧ ∈ L(captures/*.json)`. A glob's
    /// members were decided by the filesystem of the day and are gone; the
    /// pattern is not, and neither is the directory ahead of its first wildcard.
    ///
    /// ⚠ **Distinct from [`Value::OneOf`] on purpose.** Rendering a pattern as
    /// a set would claim a finite membership nobody can enumerate, and every
    /// other reader of a `Value` treats it as unknown for the same reason a set
    /// is treated as unknown: it is not a path.
    Pattern(String),
    /// One of a known finite set of literals — a name the program bound more
    /// than once, every binding a literal.
    ///
    /// ⚠ **Treated as unknown by everything except [`Reader::record`].** A set
    /// is not a path: joining onto it, or handing it to a command as a word,
    /// would need the choice this deliberately does not make.
    OneOf(Vec<String>),
    Unknown,
}

/// One argument of a call.
struct Arg {
    keyword: Option<String>,
    value: Value,
    /// The argument as written. Needed for `shell=True`, whose meaning is in the
    /// word `True` and not in any value this reader computes — and getting that
    /// wrong decides whether a string is a script or a program name.
    raw: String,
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
    /// Runs a command: `subprocess.run`, `os.system`. What it does to files is
    /// in the command, which [`crate::shell_files`] reads — this only says what
    /// was handed over, and whether a shell was on the other side.
    Command,
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
        // ⚠ **A library that opens the file it is GIVEN**, which is the
        // opposite shape from `p.open()` and reads identically. `Image`, `wave`
        // and `Store` are names the program imported, so they are not paths;
        // qualifying them is what puts the argument where the reader looks.
        // Only these three, because only these three are what the corpus
        // writes — `webbrowser.open` takes a URL, and a rule about "any
        // imported name with an `open`" would file one as a file.
        "Image.open" | "wave.open" => Call::Open,
        // A database is a file, and connecting to one uses it. Read rather than
        // written: whether a statement later changed it is in the SQL, which
        // this does not read, and an undercount beats an invention.
        "sqlite3.connect" | "Store.open" => Call::Database,
        "os.makedirs" | "os.mkdir" | "os.rmdir" => Call::Directory,
        "os.chdir" => Call::ChangeDir,
        // The top of this reader's worklist, 443 calls, and every one of them a
        // whole command whose files were invisible.
        "subprocess.run"
        | "subprocess.call"
        | "subprocess.check_call"
        | "subprocess.check_output"
        | "subprocess.Popen"
        | "os.system"
        | "os.popen" => Call::Command,
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
    candidates: BTreeMap<String, Vec<String>>,
    ranging: BTreeMap<String, Value>,
    imported: BTreeSet<String>,
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
            // ⚠ **A loop header is a statement too.** `for line in open(p)`
            // opens a file whatever the body does, and now that the header
            // holds its own iterable nothing else would read it.
            Rule::binder => {
                if let Some(iterable) = pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::ranges_over)
                    .and_then(|over| over.into_inner().find(|p| p.as_rule() == Rule::iterable))
                {
                    self.value(iterable);
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
                        // Joining onto a choice needs the choice made.
                        // `Path(p) / 'x.ts'` where `p` is one of two is one of
                        // two paths, and this reader does not multiply sets.
                        Value::List(_) | Value::OneOf(_) | Value::Pattern(_) | Value::Unknown => {
                            return Value::Unknown;
                        }
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
            Rule::string => text(head.as_str())
                .map(Value::Text)
                .or_else(|| shape(head.as_str()).map(Value::Pattern))
                .unwrap_or(Value::Unknown),
            // `(Path('src') / 'x.ts')` — brackets round one thing are that
            // thing, and pathlib's join is written that way as often as not.
            Rule::paren => match self.arguments(&head).as_slice() {
                [only] if only.keyword.is_none() => only.value.clone(),
                _ => Value::Unknown,
            },
            Rule::list => {
                let items = self.arguments(&head);
                // A keyword inside a list is not a list — a comprehension, or a
                // shape this has no reading for.
                if items.iter().all(|arg| arg.keyword.is_none()) {
                    Value::List(items.into_iter().map(|arg| arg.value).collect())
                } else {
                    Value::Unknown
                }
            }
            Rule::dict => {
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
                } else if self.imported.contains(&name) {
                    // ⚠ **One attribute, and only when the table knows the
                    // pair.** An imported name is not a path, but reading every
                    // call on one as a library call would take a shape like
                    // `OUT.write_text(s)` out of the file-operation count
                    // altogether — a rate rising because operations stopped
                    // being counted, which is the trap this reader is judged
                    // on. So an unknown pair keeps its old reading and stays in
                    // the denominator, named nothing.
                    let qualified = parts
                        .peek()
                        .filter(|part| part.as_rule() == Rule::attr)
                        .map(|attr| format!("{name}.{}", attr.as_str().trim_start_matches('.')))
                        .filter(|qualified| callable(qualified).is_some());
                    if let Some(qualified) = qualified {
                        parts.next();
                        name = qualified;
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
                    None => match self.consts.get(&name) {
                        Some(path) => Value::Text(path.clone()),
                        // Bound to several literals: the set, which `record`
                        // reports as a bound rather than resolving.
                        None => self
                            .candidates
                            .get(&name)
                            .map(|set| Value::OneOf(set.clone()))
                            // A loop variable, where the loop said what it
                            // ranged over: a pattern, or a written-out set.
                            .or_else(|| self.ranging.get(&name).cloned())
                            .unwrap_or(Value::Unknown),
                    },
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
            let mut raw = String::new();
            for part in arg.into_inner() {
                match part.as_rule() {
                    Rule::keyword => {
                        keyword = part
                            .into_inner()
                            .next()
                            .map(|name| name.as_str().to_string());
                    }
                    Rule::value => {
                        raw = part.as_str().trim().to_string();
                        value = self.value(part);
                    }
                    _ => {}
                }
            }
            out.push(Arg {
                keyword,
                value,
                raw,
            });
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
            Call::Command => {
                self.command(name, args);
                Value::Unknown
            }
            Call::Directory | Call::Nothing => Value::Unknown,
        }
    }

    /// A command handed to the system, recorded for the shell reader to follow.
    ///
    /// ⚠ **Whether a shell is on the other side is decided by `shell=`, not by
    /// the shape of the argument.** `subprocess.run("ls -la")` without it does
    /// NOT run a shell — Python looks for a program called `ls -la` and fails —
    /// so reading that string as a script would credit the program with work it
    /// did not do. `os.system` always has a shell; `subprocess` has one only
    /// when told.
    fn command(&mut self, name: &str, args: &[Arg]) {
        let through_a_shell = name.starts_with("os.")
            || args
                .iter()
                .any(|arg| arg.keyword.as_deref() == Some("shell") && arg.raw == "True");
        match positional(args, 0) {
            Some(Value::Text(text)) if through_a_shell => {
                self.out.ran.push(Ran::Script(text.clone()));
            }
            // A string with no shell is the program's own name, and nothing else
            // in the call is an argument to it that this can see.
            Some(Value::Text(text)) => self.out.ran.push(Ran::Argv(vec![text.clone()])),
            Some(Value::List(items)) => {
                let mut argv = Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        Value::Text(word) => argv.push(word.clone()),
                        // ⚠ **One unknown word makes the whole argv unusable.**
                        // `["ffmpeg", "-i", f]` with `f` computed would classify
                        // as an ffmpeg call over a file called `-i`, which is
                        // not a file anybody touched.
                        _ => {
                            *self.out.unresolved.entry(name.to_string()).or_insert(0) += 1;
                            return;
                        }
                    }
                }
                // `shell=True` with a list runs the FIRST word as a script and
                // passes the rest to the shell itself, which is a shape nobody
                // in this corpus writes; recorded as the argv it mostly is.
                self.out.ran.push(Ran::Argv(argv));
            }
            _ => {
                *self.out.unresolved.entry(name.to_string()).or_insert(0) += 1;
            }
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
            // One of a known set. Reported as the set and, when they agree on
            // one, as the directory — never as a use, because only one of them
            // happened. See [`crate::program::Program::bounded`].
            // A language whose members the filesystem of the day decided.
            // Bounded by the pattern and located at the directory ahead of its
            // first wildcard — never a use, because no file is named.
            Some(Value::Pattern(pattern)) => {
                if let Some(dir) = locus(&pattern) {
                    *self.out.located.entry(dir).or_insert(0) += 1;
                }
                *self.out.bounded.entry(pattern).or_insert(0) += 1;
            }
            Some(Value::OneOf(set)) if set.len() > 1 => {
                if let Some(dir) = shared_directory(&set) {
                    *self.out.located.entry(dir).or_insert(0) += 1;
                }
                *self.out.bounded.entry(render(&set)).or_insert(0) += 1;
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
            // As in the `/` operator above: a set is not a part.
            Value::List(_) | Value::OneOf(_) | Value::Pattern(_) | Value::Unknown => {
                return Value::Unknown;
            }
        }
    }
    match parts.is_empty() {
        true => Value::Unknown,
        false => Value::Text(parts.join("/")),
    }
}

/// The set as one word, so a report can print what a path could have been.
///
/// Sorted at the point the set is built, so this is stable across runs and two
/// programs writing the same choice in the other order produce one key.
fn render(set: &[String]) -> String {
    format!("{{{}}}", set.join(","))
}

/// The directory every candidate is rooted at, when there is one.
///
/// ⚠ **Every candidate, not the first.** A set whose members live in different
/// places has no locus, and picking one would name a directory the program may
/// never have touched. Relative and absolute are compared as written: they are
/// resolved against the shell's working directory one layer up, exactly as
/// `uses` are.
fn shared_directory(set: &[String]) -> Option<String> {
    let first = set.first()?;
    let dir = first.rsplit_once('/')?.0;
    // A leading `/` alone is the root and `rsplit_once` leaves it empty.
    let dir = if dir.is_empty() { "/" } else { dir };
    set.iter()
        .all(|path| {
            path.rsplit_once('/')
                .map(|(d, _)| if d.is_empty() { "/" } else { d })
                == Some(dir)
        })
        .then(|| dir.to_string())
}
