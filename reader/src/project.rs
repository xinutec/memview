//! The flat command list, read off the faithful tree instead of the grammar.
//!
//! [`crate::shell`] and [`crate::syntax`] answer the same first question — *which
//! commands does this script run* — from two grammars that share no code. That is
//! how the tree was built: an independent second reader is what makes a
//! disagreement mean something. But two answers with nothing between them is also
//! how a repository comes to hold two truths, and `docs/reader.md` says so in as
//! many words: **a coverage figure from one says nothing about the other.**
//!
//! This module is the thing between them. It projects a [`Script`] onto the
//! [`Simple`] list the rest of the chain consumes, so the two readers can be run
//! over one corpus and *compared* — `cargo run -p reader --bin projection` is the
//! instrument, and what it prints is the size of the port that would replace one
//! with the other.
//!
//! ## What a projection is allowed to lose
//!
//! Everything the tree knows and [`Simple`] cannot hold: which words were quoted,
//! where a loop ended, whether `time` timed a pipeline or a command. The flat
//! reader loses those because its grammar never had them; this one loses them on
//! purpose, at one place, with a name.
//!
//! ⚠ **It does not invent the words the old grammar left behind.** `done`, `fi`,
//! `esac`, `do` and the `for f in …` header reach [`crate::shell_ops`] as ordinary
//! commands today, and three separate tables downstream exist to take them back
//! out again — [`crate::shell_ops::unwrap_command`]'s keyword arm, the `NoFiles`
//! verbs, and the depth counting in [`crate::shell_files`]. A tree has the
//! structure those tables are reconstructing, so this emits none of them. That is
//! the one difference the comparison cannot call a defect on either side, and the
//! `projection` report subtracts it before counting.
//!
//! ## What it must not lose
//!
//! The order, the scopes, and [`Reached`]. Those are not spelling: a command
//! attributed to the wrong subshell resolves against the wrong directory, and a
//! branch recorded as certain claims a file use that never happened.

use crate::shell::{Reached, Redirect, Simple};
use crate::syntax::ast::{
    Arith, ArrayElement, Assignment, Brace, Command, CommandKind, Connector, Item, Parameter,
    ParameterOp, RedirectOp, RedirectTarget, Script, Segment, SegmentKind, Subscript, TestExpr,
    Word,
};
use crate::syntax::print::print_value as value;

/// Every simple command the script runs, in running order.
pub fn project(script: &Script) -> Vec<Simple> {
    let mut walk = Walk {
        out: Vec::new(),
        next: 0,
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
                            target: value(word),
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
                    flat.argv.push(binding(assignment));
                }
                for word in &simple.words {
                    self.expansions(word, scope, reached);
                    flat.argv.push(value(word));
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
            // ⚠ **A loop body keeps the condition of the loop itself**, which is
            // not the same as saying it ran. Whether an iteration is certain
            // depends on the loop's kind and on what it iterates over, and that
            // decision lives in [`crate::shell_files`] — one layer up, where the
            // words have been resolved. Demoting here would take it away from the
            // only place that can make it and answer `Sometimes` for a `for` over
            // a literal list, which certainly runs.
            CommandKind::For(loop_) => {
                for word in &loop_.words {
                    self.expansions(word, scope, reached);
                }
                self.items(&loop_.body, scope, reached);
            }
            CommandKind::While(loop_) => {
                self.items(&loop_.condition, scope, reached);
                self.items(&loop_.body, scope, reached);
            }
            CommandKind::ForArith(loop_) => {
                for part in [&loop_.init, &loop_.condition, &loop_.step]
                    .into_iter()
                    .flatten()
                {
                    self.arithmetic(part, scope, reached);
                }
                self.items(&loop_.body, scope, reached);
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
                    flat.argv.push(value(word));
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

/// `FOO=bar`, as the flat reader's grammar leaves it: one argv word, stripped
/// back off by [`crate::shell_ops::unwrap_command`] a layer later.
fn binding(assignment: &Assignment) -> String {
    format!(
        "{}{}={}",
        assignment.name,
        if assignment.append { "+" } else { "" },
        value(&assignment.value)
    )
}
