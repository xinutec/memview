//! The flat command list, read off the faithful tree.
//!
//! [`read`] is what the chain parses with. This module projects a [`Script`] onto
//! the [`Simple`] list the rest of the chain consumes, which is also what lets
//! [`crate::shell`] — the same question answered from a grammar sharing no code —
//! be diffed against it over a whole corpus: `--bin projection`.
//!
//! ## What a projection is allowed to lose
//!
//! Everything the tree knows and [`Simple`] cannot hold: which words were quoted,
//! where a loop ended, whether `time` timed a pipeline or a command. The flat
//! reader loses those because its grammar never had them; this one loses them on
//! purpose, at one place, with a name.
//!
//! ⚠ **It does not invent the words the old grammar left behind — with one
//! exception, and the exception is the rule.** `done`, `fi`, `esac` and `do`
//! reach [`crate::shell_ops`] as ordinary commands today, and three tables
//! downstream exist to take them back out again. They are not emitted here: a
//! tree has the structure those tables are reconstructing, and none of those
//! words carries anything.
//!
//! A loop HEAD does. `for f in */` is not a command either, but it holds the
//! list the variable ranged over — and for a glob that is the only place the
//! pattern appears, which is what lets `$f/package.json` be recorded as a subset
//! of `*/package.json` rather than as nothing at all. So the head is emitted and
//! the closers are not. `shell_files::ran` draws the same line for the same
//! reason. The `projection` report subtracts the rest before counting.
//!
//! ⚠ **A nested script must be read by the reader that read its parent.**
//! [`crate::shell`] hides a heredoc body inside its own delimiter so a re-parse
//! can still find it, and only that reader decodes the marker — so a flat outer
//! parse feeding a tree inner one loses every `bash -c 'python3 - <<PY … PY'`,
//! silently, which is a third of the corpus's wrapper commands.
//!
//! ## What it must not lose
//!
//! The order, the scopes, and [`Reached`]. Those are not spelling: a command
//! attributed to the wrong subshell resolves against the wrong directory, and a
//! branch recorded as certain claims a file use that never happened.

use std::collections::BTreeMap;

use crate::shell::{Reached, Redirect, Simple};
use crate::syntax::ast::{
    Arith, ArrayElement, Assignment, Brace, Command, CommandKind, Connector, ForLoop, Item,
    Parameter, ParameterOp, RedirectOp, RedirectTarget, Script, Segment, SegmentKind, Subscript,
    TestExpr, Word,
};
use crate::syntax::print::print_value as value;

/// Every simple command the script *says*, in running order.
///
/// One command per command written, which is what makes this comparable with
/// [`crate::shell::parse`] — see `--bin projection`.
pub fn project(script: &Script) -> Vec<Simple> {
    walk(script, false)
}

/// Every simple command the script *ran*: as [`project`], with the loops the
/// text already determines run out into their iterations.
///
/// ⚠ **This is where the reader stops looking commands up and starts evaluating
/// them**, and it is the same step [`crate::shell_files`] takes on the flat list
/// — moved here because the tree has the loop, where that one had to find the
/// `done` by counting keywords. A `for` over a literal word list says exactly
/// what happened, and reading it as a header plus a body full of `$f` throws
/// away the largest single class of unnamed subject there is.
///
/// The two entry points are the same distinction the flat chain draws between
/// `shell::parse` and its `unrolled`: what the text says, and what it did.
pub fn run_out(script: &Script) -> Vec<Simple> {
    walk(script, true)
}

/// **The chain's reader**: what one script ran, off the tree.
///
/// ⚠ **This is the entry point the artefacts are built from, and it is
/// deliberately not [`crate::shell::parse`].** That one stays, because a
/// comparison needs two answers and `--bin projection` is what keeps this one
/// honest — the moment the flat grammar becomes a call to this, nothing checks
/// either of them again.
///
/// A refusal is a refusal: there is no falling back to the other reader. Two
/// readers taking turns would mean no command's reading could be attributed to
/// either, and the 16 commands this refuses where the grammar does not are
/// outnumbered four to one by the 66 the other way.
pub fn read(script: &str) -> Result<Vec<Simple>, crate::syntax::Refusal> {
    Ok(run_out(&crate::syntax::parse(script)?))
}

fn walk(script: &Script, unroll: bool) -> Vec<Simple> {
    let mut walk = Walk {
        out: Vec::new(),
        next: 0,
        unroll,
        bound: BTreeMap::new(),
    };
    walk.items(&script.items, &[], Reached::Always);
    let mut out = walk.out;
    crate::shell::forget_discarded_status(&mut out);
    out
}

/// The walk's own state: what has been found, and the last subshell id handed
/// out. `scope` and `reached` are passed down rather than held, because they are
/// properties of the position and not of the walk.
struct Walk {
    out: Vec<Simple>,
    next: usize,
    unroll: bool,
    /// What each loop variable holds on the iteration being walked. Empty
    /// except inside a loop [`run_out`] is running out.
    ///
    /// ⚠ **A map rather than one binding, because loops nest** — and the inner
    /// one is walked once per outer value, so both are standing at the same
    /// time. Restored on the way out rather than cleared: `for f` inside
    /// `for f` shadows and then gives the name back.
    bound: BTreeMap<String, String>,
}

impl Walk {
    fn items(&mut self, items: &[Item], scope: &[usize], reached: Reached) {
        for item in items {
            let Item::List(list) = item else {
                continue; // A comment runs nothing.
            };
            // The condition accumulates along the list and is reset by the `;`
            // between lists, exactly as the flat reader does it: in `a && b; c`,
            // `b` needs `a` to have worked and `c` needs nothing.
            let mut here = Reached::Always;
            self.pipeline(&list.first.commands, scope, reached.and(here));
            for link in &list.rest {
                here = here.and(match link.connector {
                    Connector::And => Reached::OnSuccess,
                    Connector::Or => Reached::Sometimes,
                });
                self.pipeline(&link.pipeline.commands, scope, reached.and(here));
            }
        }
    }

    /// ⚠ **`time` and `!` are dropped, and dropping them is the projection.**
    /// They are fields on the pipeline here and `argv[0]` over there, which is
    /// the misparse `ast::Pipeline` documents: `time a | b` times the pipeline
    /// where `nohup a | b` wraps one command. The flat chain cannot express the
    /// difference and strips `time` as a wrapper anyway, so the tree's answer is
    /// the one that survives.
    fn pipeline(&mut self, commands: &[Command], scope: &[usize], reached: Reached) {
        for command in commands {
            self.command(command, scope, reached);
        }
    }

    fn command(&mut self, command: &Command, scope: &[usize], reached: Reached) {
        let mut flat = Simple {
            argv: Vec::new(),
            reached,
            scope: scope.to_vec(),
            redirects: Vec::new(),
            heredocs: Vec::new(),
        };
        // The redirections are read first because a `< <(ls)` runs its commands
        // before the one it feeds, and because a heredoc body belongs to the
        // command whatever the operator's position on the line.
        for redirect in &command.redirects {
            match &redirect.target {
                RedirectTarget::File(word) => {
                    self.expansions(word, scope, reached);
                    if !matches!(redirect.op, RedirectOp::HereString) {
                        flat.redirects.push(Redirect {
                            target: self.word(word),
                            // Only `<` reads. `<>` opens for both and can create
                            // the file, so it counts as a write — the direction
                            // this reader errs in everywhere else.
                            write: !matches!(redirect.op, RedirectOp::Read),
                        });
                    }
                }
                RedirectTarget::Here(here) => flat.heredocs.push(here.body.clone()),
                // `2>&1` and `>&-` name a descriptor, not a file.
                RedirectTarget::Fd(_) | RedirectTarget::Close => {}
            }
        }
        match &command.kind {
            CommandKind::Simple(simple) => {
                for assignment in &simple.assignments {
                    self.expansions(&assignment.value, scope, reached);
                    flat.argv.push(self.binding(assignment));
                }
                for word in &simple.words {
                    self.expansions(word, scope, reached);
                    flat.argv.push(self.word(word));
                }
            }
            // ⚠ **`[[ … ]]` is grammar, and it is projected back to the words it
            // was written as** rather than to nothing. Not because the words are
            // wanted — `[[` is a `NoFiles` verb downstream — but because a
            // command that vanishes is a command the comparison cannot line up,
            // and because the operands hold expansions that really do run.
            CommandKind::Test(test) => {
                flat.argv.push("[[".to_string());
                self.test(test, &mut flat, scope, reached);
                flat.argv.push("]]".to_string());
            }
            // `( … )` forks a shell and keeps what it changes; `{ … }` does not,
            // which is the only reason the two are separate nodes.
            CommandKind::Subshell(items) => {
                let inner = self.descend(scope);
                self.items(items, &inner, reached);
            }
            CommandKind::Group(items) => self.items(items, scope, reached),
            // The condition runs whatever the answer turns out to be; neither
            // branch is certain, and recording one as certain is the one error
            // this reader is built not to make.
            CommandKind::If(conditional) => {
                self.items(&conditional.condition, scope, reached);
                let branch = reached.and(Reached::Sometimes);
                self.items(&conditional.then, scope, branch);
                if let Some(otherwise) = &conditional.otherwise {
                    self.items(otherwise, scope, branch);
                }
            }
            // ⚠ **A loop body is certain only if the loop certainly ran it**,
            // and the rule is bash's: a `while` or `until` tests before the
            // first iteration, so empty input runs the body no times; a `for`
            // over words that are all written out runs once per word — including
            // a glob, because with `nullglob` off a pattern matching nothing
            // expands to itself and the body runs once with the pattern as the
            // value. A `for` over a `$(…)` or a variable can have an empty list.
            //
            // ⚠ **`select` is uncertain whatever it ranges over**: it reads from
            // the terminal, and end-of-file runs the body no times at all.
            CommandKind::For(loop_) => {
                for word in &loop_.words {
                    self.expansions(word, scope, reached);
                }
                // ⚠ **A loop HEAD is emitted where a `done` is not, and the
                // difference is what each carries.** `done` is not a command and
                // holds nothing; `for f in */` is not a command either, but it
                // holds the one thing the body cannot — the list the variable
                // ranged over. For a glob that is the only place the pattern
                // appears, and it is what lets `$f/package.json` be recorded as
                // a subset of `*/package.json` instead of as nothing at all.
                // `shell_files::ran` draws the same line for the same reason.
                let head = std::iter::once(if loop_.select { "select" } else { "for" }.to_string())
                    .chain([loop_.name.clone(), "in".to_string()])
                    .chain(loop_.words.iter().map(|word| self.word(word)));
                self.out.push(Simple {
                    argv: head.collect(),
                    ..flat.clone()
                });
                // The redirections belong to the head now, and `flat` keeps an
                // empty argv so the push at the end of this function skips it.
                flat.redirects.clear();
                flat.heredocs.clear();
                // ⚠ **Asked before the unrolling and not after, so both entry
                // points answer the same.** A list the text determines says the
                // body ran, whether or not this walk is the one running it out —
                // `for i in $(seq 1 3)` holds an expansion and is still certain,
                // because nothing outside the text decides what it prints.
                let values = self.values(loop_);
                let certain = values.as_ref().is_some_and(|values| !values.is_empty())
                    || (!loop_.select && !loop_.words.iter().any(has_expansion));
                let body = self.iterated(certain, reached);
                match values.filter(|_| self.unroll) {
                    // ⚠ **An empty list is an answer, not a failure.** `seq 3 1`
                    // prints nothing, so the body ran no times and none of it is
                    // emitted — which is why this arm is taken on a `Some` that
                    // is empty rather than treated as "could not tell".
                    Some(values) => self.iterations(loop_, &values, scope, body),
                    None => self.items(&loop_.body, scope, body),
                }
            }
            CommandKind::While(loop_) => {
                self.items(&loop_.condition, scope, reached);
                let body = self.iterated(false, reached);
                self.items(&loop_.body, scope, body);
            }
            CommandKind::ForArith(loop_) => {
                for part in [&loop_.init, &loop_.condition, &loop_.step]
                    .into_iter()
                    .flatten()
                {
                    self.arithmetic(part, scope, reached);
                }
                // The condition is arithmetic on values nothing here evaluates,
                // so whether it held even once is not knowable from the text.
                let body = self.iterated(false, reached);
                self.items(&loop_.body, scope, body);
            }
            // ⚠ **The subject is not an arm.** `case $(readlink -f "$p") in`
            // really does run `readlink`, whichever way the match goes. The arms
            // are alternatives, so at most one of them ran; the patterns are
            // globs and name no command.
            CommandKind::Case(case) => {
                self.expansions(&case.word, scope, reached);
                for arm in &case.arms {
                    self.items(&arm.body, scope, reached.and(Reached::Sometimes));
                }
            }
            // ⚠ **Defining a function runs none of it**, and the body is kept
            // anyway because a call site names no files at all. So it lands in
            // "runs sometimes and the text cannot say when".
            CommandKind::Function(function) => {
                self.items(&function.body, scope, reached.and(Reached::Sometimes));
            }
            CommandKind::Arithmetic(arith) => self.arithmetic(arith, scope, reached),
        }
        if !flat.argv.is_empty() || !flat.redirects.is_empty() || !flat.heredocs.is_empty() {
            self.out.push(flat);
        }
    }

    /// The operands of a `[[ … ]]`, in the order written.
    ///
    /// The operators are not projected: they are grammar with no argv spelling
    /// that reads back as itself, and nothing downstream asks what a test tested.
    fn test(&mut self, test: &TestExpr, flat: &mut Simple, scope: &[usize], reached: Reached) {
        match test {
            TestExpr::Unary { operand, .. } => {
                self.expansions(operand, scope, reached);
                flat.argv.push(value(operand));
            }
            TestExpr::Binary { left, right, .. } => {
                for word in [left, right] {
                    self.expansions(word, scope, reached);
                    flat.argv.push(self.word(word));
                }
            }
            TestExpr::Not(inner) => self.test(inner, flat, scope, reached),
            TestExpr::And(left, right) | TestExpr::Or(left, right) => {
                self.test(left, flat, scope, reached);
                self.test(right, flat, scope, reached);
            }
        }
    }

    /// Every command a word runs before the word is a word, at any depth.
    ///
    /// ⚠ **The depth is the point**, and it is the one place the flat reader was
    /// silently wrong for 8,300 commands: a `$( … )` inside double quotes was one
    /// opaque token, so its commands were never attributed to anybody. Here a
    /// quoted substitution is the same node as an unquoted one, so there is no
    /// second path to forget about — but the operands of a `${x:-$(…)}`, the
    /// elements of an array literal and the operands of arithmetic are all words
    /// too, and each of them is walked below for the same reason.
    fn expansions(&mut self, word: &Word, scope: &[usize], reached: Reached) {
        for segment in &word.segments {
            self.segment(segment, scope, reached);
        }
    }

    fn segment(&mut self, segment: &Segment, scope: &[usize], reached: Reached) {
        match &segment.kind {
            // A substitution and a process substitution each hold a whole script
            // and each fork a shell, so each gets its own scope: a `cd` inside
            // one must not move the directory of the command it belongs to.
            SegmentKind::Substitution(substitution) => {
                let inner = self.descend(scope);
                self.items(&substitution.items, &inner, reached);
            }
            SegmentKind::ProcessSubstitution(process) => {
                let inner = self.descend(scope);
                self.items(&process.items, &inner, reached);
            }
            SegmentKind::Parameter(parameter) => self.parameter(parameter, scope, reached),
            SegmentKind::Array(elements) => {
                for ArrayElement { key, value } in elements {
                    for word in key.iter().chain([value]) {
                        self.expansions(word, scope, reached);
                    }
                }
            }
            SegmentKind::Brace(Brace::Alternatives(words)) => {
                for word in words {
                    self.expansions(word, scope, reached);
                }
            }
            SegmentKind::Arithmetic(arith) => self.arithmetic(arith, scope, reached),
            // Literal text, a glob, a tilde and a brace range run nothing.
            SegmentKind::Literal(_)
            | SegmentKind::Glob(_)
            | SegmentKind::Tilde(_)
            | SegmentKind::Brace(Brace::Range { .. }) => {}
        }
    }

    /// A parameter's operands, which are words: `${x:-$(date)}` runs `date` when
    /// `x` is unset, and the text cannot say whether it was — so the command is
    /// recorded under the condition standing here, which is what
    /// [`Reached::Sometimes`] means everywhere else.
    fn parameter(&mut self, parameter: &Parameter, scope: &[usize], reached: Reached) {
        let inside = reached.and(Reached::Sometimes);
        if let Some(Subscript::Index(word)) = &parameter.subscript {
            self.expansions(word, scope, inside);
        }
        match &parameter.op {
            Some(
                ParameterOp::Default { word, .. }
                | ParameterOp::Assign { word, .. }
                | ParameterOp::Error { word, .. }
                | ParameterOp::Alternate { word, .. },
            ) => self.expansions(word, scope, inside),
            Some(
                ParameterOp::StripPrefix { pattern, .. } | ParameterOp::StripSuffix { pattern, .. },
            ) => {
                self.expansions(pattern, scope, inside);
            }
            Some(ParameterOp::Replace(replace)) => {
                self.expansions(&replace.pattern, scope, inside);
                if let Some(replacement) = &replace.replacement {
                    self.expansions(replacement, scope, inside);
                }
            }
            Some(ParameterOp::Substring { offset, length }) => {
                self.arithmetic(offset, scope, inside);
                if let Some(length) = length {
                    self.arithmetic(length, scope, inside);
                }
            }
            Some(
                ParameterOp::Case { .. }
                | ParameterOp::Transform(_)
                | ParameterOp::Length
                | ParameterOp::Indirect,
            )
            | None => {}
        }
    }

    /// `$(( … ))` is a number, but its operands are expansions and one of them
    /// may be a whole script: `$(( $(wc -l < f) + 1 ))`.
    fn arithmetic(&mut self, arith: &Arith, scope: &[usize], reached: Reached) {
        match arith {
            Arith::Expansion(segment) => self.segment(segment, scope, reached),
            Arith::Based { digits, .. } => self.arithmetic(digits, scope, reached),
            Arith::Unary { operand, .. } | Arith::Postfix { operand, .. } => {
                self.arithmetic(operand, scope, reached);
            }
            Arith::Binary { left, right, .. } => {
                self.arithmetic(left, scope, reached);
                self.arithmetic(right, scope, reached);
            }
            Arith::Ternary {
                condition,
                then,
                otherwise,
            } => {
                self.arithmetic(condition, scope, reached);
                // Exactly one arm is evaluated, and the text cannot say which.
                let branch = reached.and(Reached::Sometimes);
                self.arithmetic(then, scope, branch);
                self.arithmetic(otherwise, scope, branch);
            }
            Arith::Assign { target, value, .. } => {
                self.arithmetic(target, scope, reached);
                self.arithmetic(value, scope, reached);
            }
            Arith::Sequence(parts) => {
                for part in parts {
                    self.arithmetic(part, scope, reached);
                }
            }
            Arith::Number(_) | Arith::Variable(_) => {}
        }
    }

    /// What a loop body's condition becomes, given whether the loop certainly
    /// ran it at least once.
    ///
    /// ⚠ **A statement about RUNNING, so [`project`] does not make it.** That
    /// entry point says what the text holds, one command per command written,
    /// and a body that may run zero times is a fact about execution rather than
    /// about the text — the flat chain draws the line in exactly the same place,
    /// leaving its bodies unconditional until `shell_files` runs the loops out.
    /// Making it here too would put a decision from one layer into the
    /// comparison of another, and `--bin projection` would report a
    /// disagreement that is only a difference of stage.
    fn iterated(&self, certain: bool, reached: Reached) -> Reached {
        if certain || !self.unroll {
            reached
        } else {
            reached.and(Reached::Sometimes)
        }
    }

    /// The body once per value, with the loop's own variable standing for it.
    ///
    /// The name is restored afterwards rather than removed, so a `for f` nested
    /// inside another `for f` gives the outer one its value back — and so a
    /// command *after* the loop still sees nothing bound, which is the truthful
    /// answer: the name holds the last value, and this walk has no way to say
    /// "the last one" except by naming it, which would be a claim about a value
    /// rather than about the text.
    fn iterations(&mut self, loop_: &ForLoop, values: &[String], scope: &[usize], body: Reached) {
        let shadowed = self.bound.remove(&loop_.name);
        for value in values {
            self.bound.insert(loop_.name.clone(), value.clone());
            self.items(&loop_.body, scope, body);
        }
        self.bound.remove(&loop_.name);
        if let Some(outer) = shadowed {
            self.bound.insert(loop_.name.clone(), outer);
        }
    }

    /// The values a `for` ranges over, when the text determines every one of
    /// them — and `None` the moment it does not.
    ///
    /// A glob is answered by the filesystem of the day, which is gone, and a
    /// `$(…)` by running something, which never happens here. The **one**
    /// exception is `$(seq …)`, which is arithmetic on numbers already written
    /// down rather than a question for anybody: 1,029 corpus loops, the largest
    /// unrun class there was and larger than every glob put together.
    fn values(&self, loop_: &ForLoop) -> Option<Vec<String>> {
        if loop_.select {
            return None;
        }
        // `$(seq 1 18)` is one word, so it is asked about before the general
        // rule — which would refuse it, and correctly, as an expansion.
        if let [only] = &loop_.words[..]
            && let Some(numbers) = crate::shell_files::counted(&self.word(only))
        {
            return Some(numbers);
        }
        let values: Option<Vec<String>> = loop_
            .words
            .iter()
            .map(|word| {
                word.segments
                    .iter()
                    .all(|segment| {
                        matches!(
                            segment.kind,
                            SegmentKind::Literal(_) | SegmentKind::Tilde(_)
                        )
                    })
                    .then(|| self.word(word))
                    .filter(|value| !value.is_empty())
            })
            .collect();
        // A word list is never empty — bash prints a bare `for f` back as
        // `for f in "$@"`, which holds an expansion and never reaches here — so
        // the only empty answer comes from `seq`, above, and is a real one.
        let values = values?;
        // ⚠ Two caps, because they bound different things: this one is on the
        // commands produced, and [`crate::shell_files::counted`]'s is on the
        // list itself, so `seq 1 100000000` is never built at all.
        let body = count(&loop_.body);
        (values.len().checked_mul(body)? <= crate::shell_files::MAX_UNROLL).then_some(values)
    }

    /// A word as `argv` holds it, with whatever a loop has bound standing in.
    ///
    /// ⚠ **Substitution happens on the TREE, not on the printed string.** The
    /// flat chain expanded `$f` by rewriting text, which meant knowing every
    /// spelling of a parameter and getting `${f}` right by hand. Here the two
    /// spellings are one node, so replacing it is exact and `${f}x` needs no
    /// special case at all.
    ///
    /// A parameter carrying an operator is left alone: `${f%.txt}` is a
    /// transduction this reader does not perform, and putting the value in
    /// without applying it would be worse than leaving the question open.
    fn word(&self, word: &Word) -> String {
        if self.bound.is_empty() {
            return value(word);
        }
        value(&Word {
            segments: word
                .segments
                .iter()
                .map(|segment| match &segment.kind {
                    SegmentKind::Parameter(parameter)
                        if parameter.op.is_none()
                            && parameter.subscript.is_none()
                            && self.bound.contains_key(&parameter.name) =>
                    {
                        Segment {
                            kind: SegmentKind::Literal(self.bound[&parameter.name].clone()),
                            span: segment.span,
                        }
                    }
                    _ => segment.clone(),
                })
                .collect(),
            span: word.span,
        })
    }

    /// `FOO=bar`, as the flat reader's grammar leaves it: one argv word,
    /// stripped back off by [`crate::shell_ops::unwrap_command`] a layer later.
    fn binding(&self, assignment: &Assignment) -> String {
        format!(
            "{}{}={}",
            assignment.name,
            if assignment.append { "+" } else { "" },
            self.word(&assignment.value)
        )
    }

    /// A fresh scope one level inside `outer`.
    ///
    /// Ids rather than a depth, because depth alone cannot tell
    /// `(cd a && x); (cd b && y)` from one group holding both.
    fn descend(&mut self, outer: &[usize]) -> Vec<usize> {
        self.next += 1;
        let mut inner = outer.to_vec();
        inner.push(self.next);
        inner
    }
}

/// Whether a word holds anything only running something could answer.
///
/// ⚠ **A glob is not one of them.** With `nullglob` off a pattern matching
/// nothing expands to itself, so `for f in *.log` runs its body at least once
/// whatever the directory held — which is why a glob loop is certain where a
/// `$(…)` one is not.
fn has_expansion(word: &Word) -> bool {
    word.segments.iter().any(|segment| {
        matches!(
            segment.kind,
            SegmentKind::Parameter(_)
                | SegmentKind::Substitution(_)
                | SegmentKind::ProcessSubstitution(_)
                | SegmentKind::Arithmetic(_)
                | SegmentKind::Brace(_)
                | SegmentKind::Array(_)
        )
    })
}

/// How many commands a list holds, counting into everything that carries one.
///
/// The multiplicand of the unrolling cap, so it has to count what the walk will
/// actually emit: an inner loop's body is walked once per outer value, and a
/// count that stopped at the top level would let a nested pair through.
fn count(items: &[Item]) -> usize {
    items
        .iter()
        .filter_map(|item| match item {
            Item::List(list) => Some(list),
            Item::Comment(_) => None,
        })
        .flat_map(|list| {
            std::iter::once(&list.first).chain(list.rest.iter().map(|link| &link.pipeline))
        })
        .flat_map(|pipeline| &pipeline.commands)
        .map(|command| match &command.kind {
            CommandKind::Simple(_) | CommandKind::Test(_) | CommandKind::Arithmetic(_) => 1,
            CommandKind::Subshell(items) | CommandKind::Group(items) => count(items),
            CommandKind::If(conditional) => {
                count(&conditional.condition)
                    + count(&conditional.then)
                    + conditional.otherwise.as_deref().map_or(0, count)
            }
            CommandKind::For(loop_) => count(&loop_.body).max(1),
            CommandKind::While(loop_) => count(&loop_.condition) + count(&loop_.body),
            CommandKind::ForArith(loop_) => count(&loop_.body).max(1),
            CommandKind::Case(case) => case.arms.iter().map(|arm| count(&arm.body)).sum(),
            CommandKind::Function(function) => count(&function.body),
        })
        .sum()
}
