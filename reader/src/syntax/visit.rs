//! Every command in a tree, wherever it sits: in a list, a compound's body, or a
//! script held inside a word — `$( )`, `<( )`, a parameter's operand, arithmetic.
//!
//! The matches are exhaustive on purpose: a new node that can hold a script does
//! not compile until this walk says whether it descends.

use super::ast::{
    Arith, ArrayElement, Brace, Command, CommandKind, Item, Parameter, ParameterOp, RedirectTarget,
    Segment, SegmentKind, Subscript, TestExpr, Word,
};

/// Calls `each` on every command in `items`, outer before inner.
pub fn commands<'t>(items: &'t [Item], each: &mut impl FnMut(&'t Command)) {
    for item in items {
        let Item::List(list) = item else {
            continue;
        };
        for pipeline in std::iter::once(&list.first).chain(list.rest.iter().map(|l| &l.pipeline)) {
            for command in &pipeline.commands {
                self::command(command, each);
            }
        }
    }
}

fn command<'t>(command: &'t Command, each: &mut impl FnMut(&'t Command)) {
    each(command);
    for redirect in &command.redirects {
        match &redirect.target {
            RedirectTarget::File(word) => words([word], each),
            RedirectTarget::Fd(_) | RedirectTarget::Close | RedirectTarget::Here(_) => {}
        }
    }
    match &command.kind {
        CommandKind::Simple(simple) => {
            words(simple.assignments.iter().map(|a| &a.value), each);
            words(&simple.words, each);
        }
        CommandKind::For(loop_) => {
            words(&loop_.words, each);
            commands(&loop_.body, each);
        }
        CommandKind::While(loop_) => {
            commands(&loop_.condition, each);
            commands(&loop_.body, each);
        }
        CommandKind::If(conditional) => {
            commands(&conditional.condition, each);
            commands(&conditional.then, each);
            if let Some(otherwise) = &conditional.otherwise {
                commands(otherwise, each);
            }
        }
        CommandKind::Case(case) => {
            words([&case.word], each);
            for arm in &case.arms {
                words(&arm.patterns, each);
                commands(&arm.body, each);
            }
        }
        CommandKind::Test(test) => self::test(test, each),
        CommandKind::Subshell(items) | CommandKind::Group(items) => commands(items, each),
        CommandKind::Function(function) => commands(&function.body, each),
        CommandKind::Arithmetic(arith) => arithmetic(arith, each),
        CommandKind::ForArith(loop_) => {
            for part in [&loop_.init, &loop_.condition, &loop_.step]
                .into_iter()
                .flatten()
            {
                arithmetic(part, each);
            }
            commands(&loop_.body, each);
        }
    }
}

fn words<'t>(words: impl IntoIterator<Item = &'t Word>, each: &mut impl FnMut(&'t Command)) {
    for word in words {
        for segment in &word.segments {
            self::segment(segment, each);
        }
    }
}

fn segment<'t>(segment: &'t Segment, each: &mut impl FnMut(&'t Command)) {
    match &segment.kind {
        SegmentKind::Substitution(substitution) => commands(&substitution.items, each),
        SegmentKind::ProcessSubstitution(process) => commands(&process.items, each),
        SegmentKind::Parameter(parameter) => self::parameter(parameter, each),
        SegmentKind::Array(elements) => {
            for ArrayElement { key, value } in elements {
                words(key.iter().chain([value]), each);
            }
        }
        SegmentKind::Brace(Brace::Alternatives(alternatives)) => words(alternatives, each),
        SegmentKind::Arithmetic(arith) => arithmetic(arith, each),
        SegmentKind::Literal(_)
        | SegmentKind::Glob(_)
        | SegmentKind::Tilde(_)
        | SegmentKind::Brace(Brace::Range { .. }) => {}
    }
}

fn parameter<'t>(parameter: &'t Parameter, each: &mut impl FnMut(&'t Command)) {
    match &parameter.subscript {
        Some(Subscript::Index(word)) => words([word], each),
        Some(Subscript::All | Subscript::Joined) | None => {}
    }
    match &parameter.op {
        Some(
            ParameterOp::Default { word, .. }
            | ParameterOp::Assign { word, .. }
            | ParameterOp::Error { word, .. }
            | ParameterOp::Alternate { word, .. }
            | ParameterOp::StripPrefix { pattern: word, .. }
            | ParameterOp::StripSuffix { pattern: word, .. },
        ) => words([word], each),
        Some(ParameterOp::Replace(replace)) => {
            words(
                std::iter::once(&replace.pattern).chain(&replace.replacement),
                each,
            );
        }
        Some(ParameterOp::Substring { offset, length }) => {
            arithmetic(offset, each);
            if let Some(length) = length {
                arithmetic(length, each);
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

fn arithmetic<'t>(arith: &'t Arith, each: &mut impl FnMut(&'t Command)) {
    match arith {
        Arith::Expansion(segment) => self::segment(segment, each),
        Arith::Based { digits, .. } => arithmetic(digits, each),
        Arith::Unary { operand, .. } | Arith::Postfix { operand, .. } => arithmetic(operand, each),
        Arith::Binary { left, right, .. } => {
            arithmetic(left, each);
            arithmetic(right, each);
        }
        Arith::Ternary {
            condition,
            then,
            otherwise,
        } => {
            for part in [condition, then, otherwise] {
                arithmetic(part, each);
            }
        }
        Arith::Assign { target, value, .. } => {
            arithmetic(target, each);
            arithmetic(value, each);
        }
        Arith::Sequence(parts) | Arith::Spliced(parts) => {
            for part in parts {
                arithmetic(part, each);
            }
        }
        Arith::Number(_) | Arith::Variable(_) => {}
    }
}

fn test<'t>(test: &'t TestExpr, each: &mut impl FnMut(&'t Command)) {
    match test {
        TestExpr::Unary { operand, .. } => words([operand], each),
        TestExpr::Binary { left, right, .. } => words([left, right], each),
        TestExpr::Not(inner) => self::test(inner, each),
        TestExpr::And(left, right) | TestExpr::Or(left, right) => {
            self::test(left, each);
            self::test(right, each);
        }
    }
}
