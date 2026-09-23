//! A [`Module`] back to text, as a pure function of the tree.
//!
//! One canonical form: four-space indentation, one statement per line, strings
//! quoted as `repr` quotes them, parentheses only where precedence needs them —
//! and around every tuple, except as a subscript, where `x[1:2, 3]` cannot carry
//! them. Layout is not in the tree, so it cannot come back out; the round-trip
//! law checks that this form reads back to the tree it came from.

use std::cell::Cell;

use super::ast::*;

thread_local! {
    /// The quote of the f-string being printed, which a literal nested in one of
    /// its fields must not reuse: before 3.12 that ends the outer string.
    static INSIDE: Cell<Option<char>> = const { Cell::new(None) };
}

pub fn print(module: &Module) -> String {
    let mut out = String::new();
    block(&mut out, &module.body, 0);
    out
}

fn block(out: &mut String, body: &[Stmt], depth: usize) {
    for stmt in body {
        statement(out, stmt, depth);
    }
}

fn line(out: &mut String, depth: usize, text: &str, comment: Option<&String>) {
    out.push_str(&"    ".repeat(depth));
    out.push_str(text);
    if let Some(comment) = comment {
        out.push_str("  #");
        out.push_str(comment);
    }
    out.push('\n');
}

fn statement(out: &mut String, stmt: &Stmt, depth: usize) {
    let comment = stmt.comment.as_ref();
    match &stmt.kind {
        StmtKind::Comment(text) => line(out, depth, &format!("#{text}"), None),
        StmtKind::If { test, body, orelse } => {
            line(out, depth, &format!("if {}:", expr(test, NAMED)), comment);
            block(out, body, depth + 1);
            else_branch(out, orelse, depth);
        }
        StmtKind::For {
            target,
            iter,
            body,
            orelse,
        } => {
            line(
                out,
                depth,
                &format!("for {} in {}:", expr(target, TUPLE), expr(iter, TUPLE)),
                comment,
            );
            block(out, body, depth + 1);
            if !orelse.is_empty() {
                line(out, depth, "else:", None);
                block(out, orelse, depth + 1);
            }
        }
        StmtKind::While { test, body, orelse } => {
            line(
                out,
                depth,
                &format!("while {}:", expr(test, NAMED)),
                comment,
            );
            block(out, body, depth + 1);
            if !orelse.is_empty() {
                line(out, depth, "else:", None);
                block(out, orelse, depth + 1);
            }
        }
        StmtKind::With { items, body } => {
            let items: Vec<String> = items
                .iter()
                .map(|item| match &item.var {
                    Some(var) => format!("{} as {}", expr(&item.context, TEST), expr(var, TUPLE)),
                    None => expr(&item.context, TEST),
                })
                .collect();
            line(out, depth, &format!("with {}:", items.join(", ")), comment);
            block(out, body, depth + 1);
        }
        StmtKind::FunctionDef {
            decorators,
            name,
            params: p,
            body,
        } => {
            for decorator in decorators {
                line(out, depth, &format!("@{}", expr(decorator, NAMED)), None);
            }
            line(out, depth, &format!("def {name}({}):", params(p)), comment);
            block(out, body, depth + 1);
        }
        StmtKind::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            line(out, depth, "try:", comment);
            block(out, body, depth + 1);
            for handler in handlers {
                let header = match (&handler.kind, &handler.name) {
                    (None, _) => "except:".to_string(),
                    (Some(kind), None) => format!("except {}:", expr(kind, TEST)),
                    (Some(kind), Some(name)) => format!("except {} as {name}:", expr(kind, TEST)),
                };
                line(out, depth, &header, None);
                block(out, &handler.body, depth + 1);
            }
            if !orelse.is_empty() {
                line(out, depth, "else:", None);
                block(out, orelse, depth + 1);
            }
            if !finalbody.is_empty() {
                line(out, depth, "finally:", None);
                block(out, finalbody, depth + 1);
            }
        }
        simple => line(out, depth, &simple_statement(simple), comment),
    }
}

/// An `else` holding exactly one `if` is written back as the `elif` it was.
fn else_branch(out: &mut String, orelse: &[Stmt], depth: usize) {
    match orelse {
        [] => {}
        [
            Stmt {
                kind: StmtKind::If { test, body, orelse },
                comment,
            },
        ] => {
            line(
                out,
                depth,
                &format!("elif {}:", expr(test, NAMED)),
                comment.as_ref(),
            );
            block(out, body, depth + 1);
            else_branch(out, orelse, depth);
        }
        _ => {
            line(out, depth, "else:", None);
            block(out, orelse, depth + 1);
        }
    }
}

fn simple_statement(kind: &StmtKind) -> String {
    match kind {
        StmtKind::Expr(value) => expr(value, TEST),
        StmtKind::Assign { targets, value } => {
            let mut parts: Vec<String> = targets.iter().map(|t| expr(t, TUPLE)).collect();
            parts.push(expr(value, TUPLE));
            parts.join(" = ")
        }
        StmtKind::AugAssign { target, op, value } => {
            format!(
                "{} {}= {}",
                expr(target, TUPLE),
                op.symbol(),
                expr(value, TUPLE)
            )
        }
        StmtKind::Import(names) => format!("import {}", aliases(names)),
        StmtKind::ImportFrom {
            module,
            level,
            names,
        } => format!(
            "from {}{} import {}",
            ".".repeat(*level as usize),
            module.as_deref().unwrap_or(""),
            aliases(names)
        ),
        StmtKind::Assert { test, msg } => match msg {
            Some(msg) => format!("assert {}, {}", expr(test, TEST), expr(msg, TEST)),
            None => format!("assert {}", expr(test, TEST)),
        },
        StmtKind::Delete(targets) => format!("del {}", list(targets, TUPLE)),
        StmtKind::Pass => "pass".to_string(),
        StmtKind::Break => "break".to_string(),
        StmtKind::Continue => "continue".to_string(),
        StmtKind::Return(None) => "return".to_string(),
        StmtKind::Return(Some(value)) => format!("return {}", expr(value, TUPLE)),
        StmtKind::Raise { exc, cause } => match (exc, cause) {
            (None, _) => "raise".to_string(),
            (Some(exc), None) => format!("raise {}", expr(exc, TEST)),
            (Some(exc), Some(cause)) => {
                format!("raise {} from {}", expr(exc, TEST), expr(cause, TEST))
            }
        },
        StmtKind::Global(names) => format!("global {}", names.join(", ")),
        StmtKind::Nonlocal(names) => format!("nonlocal {}", names.join(", ")),
        // Compounds and comments are written by `statement`.
        _ => String::new(),
    }
}

fn aliases(names: &[Alias]) -> String {
    names
        .iter()
        .map(|alias| match &alias.asname {
            Some(asname) => format!("{} as {asname}", alias.name),
            None => alias.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn params(p: &Params) -> String {
    let mut out: Vec<String> = p.args.iter().map(param).collect();
    match &p.vararg {
        Some(name) => out.push(format!("*{name}")),
        None if !p.kwonly.is_empty() => out.push("*".to_string()),
        None => {}
    }
    out.extend(p.kwonly.iter().map(param));
    if let Some(name) = &p.kwarg {
        out.push(format!("**{name}"));
    }
    out.join(", ")
}

fn param(p: &Param) -> String {
    match &p.default {
        Some(default) => format!("{}={}", p.name, expr(default, TEST)),
        None => p.name.clone(),
    }
}

fn list(items: &[Expr], min: u8) -> String {
    items
        .iter()
        .map(|item| expr(item, min))
        .collect::<Vec<_>>()
        .join(", ")
}

// Precedence, lowest first. An expression is bracketed when it binds more
// loosely than its place requires.
const TUPLE: u8 = 0;
const NAMED: u8 = 1;
const LAMBDA: u8 = 2;
const TEST: u8 = 2;
const IFEXP: u8 = 3;
const OR: u8 = 4;
const AND: u8 = 5;
const NOT: u8 = 6;
const COMPARE: u8 = 7;
const BITOR: u8 = 8;
const BITXOR: u8 = 9;
const BITAND: u8 = 10;
const SHIFT: u8 = 11;
const ARITH: u8 = 12;
const TERM: u8 = 13;
const UNARY: u8 = 14;
const POWER: u8 = 15;
const PRIMARY: u8 = 16;

fn binds(e: &Expr) -> u8 {
    match e {
        Expr::NamedExpr { .. } => NAMED,
        Expr::Lambda { .. } => LAMBDA,
        Expr::IfExp { .. } => IFEXP,
        Expr::BoolOp { op: BoolOp::Or, .. } => OR,
        Expr::BoolOp {
            op: BoolOp::And, ..
        } => AND,
        Expr::UnaryOp {
            op: UnaryOp::Not, ..
        } => NOT,
        Expr::Compare { .. } => COMPARE,
        Expr::BinOp { op, .. } => binop(*op),
        Expr::UnaryOp { .. } => UNARY,
        _ => PRIMARY,
    }
}

fn binop(op: BinOp) -> u8 {
    match op {
        BinOp::BitOr => BITOR,
        BinOp::BitXor => BITXOR,
        BinOp::BitAnd => BITAND,
        BinOp::LShift | BinOp::RShift => SHIFT,
        BinOp::Add | BinOp::Sub => ARITH,
        BinOp::Mult | BinOp::MatMult | BinOp::Div | BinOp::FloorDiv | BinOp::Mod => TERM,
        BinOp::Pow => POWER,
    }
}

/// An expression, bracketed if it binds more loosely than `min`.
fn expr(e: &Expr, min: u8) -> String {
    let text = bare(e);
    if binds(e) < min {
        format!("({text})")
    } else {
        text
    }
}

fn bare(e: &Expr) -> String {
    match e {
        Expr::Name(name) => name.clone(),
        Expr::Number(number) => number.clone(),
        Expr::Singleton(Singleton::True) => "True".to_string(),
        Expr::Singleton(Singleton::False) => "False".to_string(),
        Expr::Singleton(Singleton::None) => "None".to_string(),
        Expr::Singleton(Singleton::Ellipsis) => "...".to_string(),
        Expr::Str(text) => quoted(text, INSIDE.get()),
        Expr::Bytes(bytes) => bytes_literal(bytes),
        Expr::FString(parts) => fstring(parts),
        Expr::Attribute { value, attr } => {
            // `1 .real`: a number followed by `.` would read as a float.
            let base = match value.as_ref() {
                Expr::Number(_) => format!("({})", bare(value)),
                _ => expr(value, PRIMARY),
            };
            format!("{base}.{attr}")
        }
        Expr::Call { func, args } => {
            let args: Vec<String> = args.iter().map(argument).collect();
            format!("{}({})", expr(func, PRIMARY), args.join(", "))
        }
        Expr::Subscript { value, index } => {
            format!("{}[{}]", expr(value, PRIMARY), index_text(index))
        }
        Expr::Slice { lower, upper, step } => {
            let part = |p: &Option<Box<Expr>>| p.as_ref().map_or(String::new(), |e| expr(e, TEST));
            match step {
                Some(_) => format!("{}:{}:{}", part(lower), part(upper), part(step)),
                None => format!("{}:{}", part(lower), part(upper)),
            }
        }
        Expr::BinOp { left, op, right } => {
            let level = binop(*op);
            if *op == BinOp::Pow {
                // Right-associative, and a unary on the left must be bracketed.
                format!("{} ** {}", expr(left, PRIMARY), expr(right, UNARY))
            } else {
                format!(
                    "{} {} {}",
                    expr(left, level),
                    op.symbol(),
                    expr(right, level + 1)
                )
            }
        }
        Expr::UnaryOp { op, operand } => match op {
            UnaryOp::Not => format!("not {}", expr(operand, NOT)),
            UnaryOp::Invert => format!("~{}", expr(operand, UNARY)),
            UnaryOp::UAdd => format!("+{}", expr(operand, UNARY)),
            UnaryOp::USub => format!("-{}", expr(operand, UNARY)),
        },
        Expr::BoolOp { op, values } => {
            let (word, level) = match op {
                BoolOp::And => (" and ", AND),
                BoolOp::Or => (" or ", OR),
            };
            // One more than the level: a nested BoolOp of the same kind would be
            // flattened, so it has to keep its brackets to stay a separate node.
            values
                .iter()
                .map(|v| expr(v, level + 1))
                .collect::<Vec<_>>()
                .join(word)
        }
        Expr::Compare { left, rest } => {
            let mut out = expr(left, BITOR);
            for (op, right) in rest {
                out.push(' ');
                out.push_str(op.symbol());
                out.push(' ');
                out.push_str(&expr(right, BITOR));
            }
            out
        }
        Expr::IfExp { test, body, orelse } => format!(
            "{} if {} else {}",
            expr(body, OR),
            expr(test, OR),
            expr(orelse, IFEXP)
        ),
        Expr::Lambda { params: p, body } => {
            let p = params(p);
            if p.is_empty() {
                format!("lambda: {}", expr(body, TEST))
            } else {
                format!("lambda {p}: {}", expr(body, TEST))
            }
        }
        Expr::NamedExpr { target, value } => format!("{target} := {}", expr(value, TEST)),
        Expr::Tuple(items) => match items.as_slice() {
            [] => "()".to_string(),
            [only] => format!("({},)", expr(only, NAMED)),
            _ => format!("({})", list(items, NAMED)),
        },
        Expr::List(items) => format!("[{}]", list(items, NAMED)),
        Expr::Set(items) => format!("{{{}}}", list(items, NAMED)),
        Expr::Dict(entries) => {
            let entries: Vec<String> = entries
                .iter()
                .map(|(key, value)| match key {
                    Some(key) => format!("{}: {}", expr(key, TEST), expr(value, TEST)),
                    None => format!("**{}", expr(value, BITOR)),
                })
                .collect();
            format!("{{{}}}", entries.join(", "))
        }
        Expr::Starred(inner) => format!("*{}", expr(inner, BITOR)),
        Expr::ListComp { elt, generators } => {
            format!("[{}{}]", expr(elt, NAMED), comprehension(generators))
        }
        Expr::SetComp { elt, generators } => {
            format!("{{{}{}}}", expr(elt, NAMED), comprehension(generators))
        }
        Expr::GeneratorExp { elt, generators } => {
            format!("({}{})", expr(elt, NAMED), comprehension(generators))
        }
        Expr::DictComp {
            key,
            value,
            generators,
        } => format!(
            "{{{}: {}{}}}",
            expr(key, TEST),
            expr(value, TEST),
            comprehension(generators)
        ),
    }
}

/// A subscript's index: a tuple bare, since a slice cannot sit in brackets.
fn index_text(index: &Expr) -> String {
    match index {
        // A walrus keeps its brackets here: `x[a := 1, 2]` is not Python.
        Expr::Tuple(items) if !items.is_empty() => {
            let text = list(items, TEST);
            if items.len() == 1 {
                format!("{text},")
            } else {
                text
            }
        }
        other => expr(other, NAMED),
    }
}

fn argument(arg: &Arg) -> String {
    match arg {
        Arg::Positional(value) => expr(value, NAMED),
        Arg::Starred(value) => format!("*{}", expr(value, TEST)),
        Arg::Keyword(name, value) => format!("{name}={}", expr(value, TEST)),
        Arg::DoubleStarred(value) => format!("**{}", expr(value, TEST)),
    }
}

fn comprehension(generators: &[Comprehension]) -> String {
    let mut out = String::new();
    for g in generators {
        out.push_str(&format!(
            " for {} in {}",
            expr(&g.target, TUPLE),
            expr(&g.iter, OR)
        ));
        for cond in &g.ifs {
            out.push_str(&format!(" if {}", expr(cond, OR)));
        }
    }
    out
}

/// A string as `repr` writes it: single quotes unless the text holds one and no
/// double. `avoid` is the enclosing f-string's quote, which a nested literal must
/// not reuse before 3.12.
fn quoted(text: &str, avoid: Option<char>) -> String {
    let quote = match avoid {
        Some('\'') => '"',
        Some(_) => '\'',
        None if text.contains('\'') && !text.contains('"') => '"',
        None => '\'',
    };
    format!("{quote}{}{quote}", escaped(text, quote, false))
}

fn escaped(text: &str, quote: char, braces: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '{' if braces => out.push_str("{{"),
            '}' if braces => out.push_str("}}"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

fn bytes_literal(bytes: &[u8]) -> String {
    let quote = if bytes.contains(&b'\'') && !bytes.contains(&b'"') {
        '"'
    } else {
        '\''
    };
    let mut out = format!("b{quote}");
    for &b in bytes {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b if b as char == quote => {
                out.push('\\');
                out.push(b as char);
            }
            0x20..=0x7e => out.push(b as char),
            _ => out.push_str(&format!("\\x{b:02x}")),
        }
    }
    out.push(quote);
    out
}

fn fstring(parts: &[FPart]) -> String {
    let quote = if INSIDE.get() == Some('\'') {
        '"'
    } else {
        '\''
    };
    format!("f{quote}{}{quote}", fparts(parts, quote))
}

fn fparts(parts: &[FPart], quote: char) -> String {
    let mut out = String::new();
    for part in parts {
        match part {
            FPart::Text(text) => out.push_str(&escaped(text, quote, true)),
            FPart::Field {
                value,
                conversion,
                spec,
            } => {
                let mut inner = field_expr(value, quote);
                // A field opening with `{` would read as a doubled brace.
                if inner.starts_with('{') {
                    inner.insert(0, ' ');
                }
                out.push('{');
                out.push_str(&inner);
                if let Some(conversion) = conversion {
                    out.push('!');
                    out.push(*conversion);
                }
                if let Some(spec) = spec {
                    out.push(':');
                    out.push_str(&fparts(spec, quote));
                }
                out.push('}');
            }
        }
    }
    out
}

/// A field's expression, every string literal in it quoted with the other quote.
fn field_expr(value: &Expr, quote: char) -> String {
    let outer = INSIDE.replace(Some(quote));
    let text = expr(value, IFEXP);
    INSIDE.set(outer);
    text
}
