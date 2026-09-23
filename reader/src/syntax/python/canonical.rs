//! The tree in CPython's own shape, as JSON: node names, field names, and the way
//! CPython files arguments, parameters and f-strings — so the second gate can put it
//! beside `ast.parse` of the same text and compare node for node.
//!
//! A number goes as the text written, `{"_": "Num", "text": "0x10"}`; the gate
//! evaluates it in CPython, which is the authority on what a literal is worth.
//! Comments are left out, because CPython drops them.

use serde_json::{Value, json};

use super::ast::*;

pub fn canonical(module: &Module) -> Value {
    json!({ "_": "Module", "body": statements(&module.body) })
}

fn statements(body: &[Stmt]) -> Value {
    Value::Array(
        body.iter()
            .filter(|stmt| !matches!(stmt.kind, StmtKind::Comment(_)))
            .map(|stmt| statement(&stmt.kind))
            .collect(),
    )
}

fn opt(e: &Option<Expr>) -> Value {
    e.as_ref().map_or(Value::Null, expr)
}

fn opt_box(e: &Option<Box<Expr>>) -> Value {
    e.as_deref().map_or(Value::Null, expr)
}

fn exprs(items: &[Expr]) -> Value {
    Value::Array(items.iter().map(expr).collect())
}

fn statement(kind: &StmtKind) -> Value {
    match kind {
        StmtKind::Comment(_) => Value::Null,
        StmtKind::Expr(value) => json!({ "_": "Expr", "value": expr(value) }),
        StmtKind::Assign { targets, value } => {
            json!({ "_": "Assign", "targets": exprs(targets), "value": expr(value) })
        }
        StmtKind::AugAssign { target, op, value } => json!({
            "_": "AugAssign", "target": expr(target), "op": binop(*op), "value": expr(value)
        }),
        StmtKind::Import(names) => json!({ "_": "Import", "names": aliases(names) }),
        StmtKind::ImportFrom {
            module,
            level,
            names,
        } => json!({
            "_": "ImportFrom", "module": module, "names": aliases(names), "level": level
        }),
        StmtKind::Assert { test, msg } => {
            json!({ "_": "Assert", "test": expr(test), "msg": opt(msg) })
        }
        StmtKind::Delete(targets) => json!({ "_": "Delete", "targets": exprs(targets) }),
        StmtKind::Pass => json!({ "_": "Pass" }),
        StmtKind::Break => json!({ "_": "Break" }),
        StmtKind::Continue => json!({ "_": "Continue" }),
        StmtKind::Return(value) => json!({ "_": "Return", "value": opt(value) }),
        StmtKind::Raise { exc, cause } => {
            json!({ "_": "Raise", "exc": opt(exc), "cause": opt(cause) })
        }
        StmtKind::Global(names) => json!({ "_": "Global", "names": names }),
        StmtKind::Nonlocal(names) => json!({ "_": "Nonlocal", "names": names }),
        StmtKind::If { test, body, orelse } => json!({
            "_": "If", "test": expr(test), "body": statements(body), "orelse": statements(orelse)
        }),
        StmtKind::For {
            target,
            iter,
            body,
            orelse,
        } => json!({
            "_": "For", "target": expr(target), "iter": expr(iter),
            "body": statements(body), "orelse": statements(orelse)
        }),
        StmtKind::While { test, body, orelse } => json!({
            "_": "While", "test": expr(test), "body": statements(body), "orelse": statements(orelse)
        }),
        StmtKind::With { items, body } => json!({
            "_": "With",
            "items": items.iter().map(|item| json!({
                "_": "withitem", "context_expr": expr(&item.context), "optional_vars": opt(&item.var)
            })).collect::<Vec<_>>(),
            "body": statements(body)
        }),
        StmtKind::FunctionDef {
            decorators,
            name,
            params,
            body,
        } => json!({
            "_": "FunctionDef", "name": name, "args": arguments(params),
            "body": statements(body), "decorator_list": exprs(decorators)
        }),
        StmtKind::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => json!({
            "_": "Try",
            "body": statements(body),
            "handlers": handlers.iter().map(|h| json!({
                "_": "ExceptHandler", "type": opt(&h.kind), "name": h.name, "body": statements(&h.body)
            })).collect::<Vec<_>>(),
            "orelse": statements(orelse),
            "finalbody": statements(finalbody)
        }),
    }
}

fn aliases(names: &[Alias]) -> Value {
    Value::Array(
        names
            .iter()
            .map(|a| json!({ "_": "alias", "name": a.name, "asname": a.asname }))
            .collect(),
    )
}

/// CPython's `arguments`: defaults trail the positional parameters, and a
/// keyword-only parameter with no default has `None` in `kw_defaults`.
fn arguments(p: &Params) -> Value {
    let arg = |name: &str| json!({ "_": "arg", "arg": name });
    json!({
        "_": "arguments",
        "posonlyargs": [],
        "args": p.args.iter().map(|a| arg(&a.name)).collect::<Vec<_>>(),
        "vararg": p.vararg.as_deref().map(arg),
        "kwonlyargs": p.kwonly.iter().map(|a| arg(&a.name)).collect::<Vec<_>>(),
        "kw_defaults": p.kwonly.iter().map(|a| opt(&a.default)).collect::<Vec<_>>(),
        "kwarg": p.kwarg.as_deref().map(arg),
        "defaults": p.args.iter().filter_map(|a| a.default.as_ref().map(expr)).collect::<Vec<_>>()
    })
}

fn expr(e: &Expr) -> Value {
    match e {
        Expr::Name(name) => json!({ "_": "Name", "id": name }),
        Expr::Number(text) => json!({ "_": "Num", "text": text }),
        Expr::Singleton(s) => json!({ "_": "Const", "value": match s {
            Singleton::True => "True",
            Singleton::False => "False",
            Singleton::None => "None",
            Singleton::Ellipsis => "Ellipsis",
        }}),
        Expr::Str(text) => json!({ "_": "Str", "value": text }),
        Expr::Bytes(bytes) => json!({ "_": "Bytes", "value": bytes }),
        Expr::FString(parts) => joined(parts),
        Expr::Attribute { value, attr } => {
            json!({ "_": "Attribute", "value": expr(value), "attr": attr })
        }
        Expr::Call { func, args } => {
            let mut positional = Vec::new();
            let mut keywords = Vec::new();
            for arg in args {
                match arg {
                    Arg::Positional(value) => positional.push(expr(value)),
                    Arg::Starred(value) => {
                        positional.push(json!({ "_": "Starred", "value": expr(value) }))
                    }
                    Arg::Keyword(name, value) => {
                        keywords.push(json!({ "_": "keyword", "arg": name, "value": expr(value) }))
                    }
                    Arg::DoubleStarred(value) => {
                        keywords.push(json!({ "_": "keyword", "arg": null, "value": expr(value) }))
                    }
                }
            }
            json!({ "_": "Call", "func": expr(func), "args": positional, "keywords": keywords })
        }
        Expr::Subscript { value, index } => {
            json!({ "_": "Subscript", "value": expr(value), "slice": expr(index) })
        }
        Expr::Slice { lower, upper, step } => json!({
            "_": "Slice", "lower": opt_box(lower), "upper": opt_box(upper), "step": opt_box(step)
        }),
        Expr::BinOp { left, op, right } => json!({
            "_": "BinOp", "left": expr(left), "op": binop(*op), "right": expr(right)
        }),
        Expr::UnaryOp { op, operand } => json!({
            "_": "UnaryOp",
            "op": match op {
                UnaryOp::Not => "Not",
                UnaryOp::Invert => "Invert",
                UnaryOp::UAdd => "UAdd",
                UnaryOp::USub => "USub",
            },
            "operand": expr(operand)
        }),
        Expr::BoolOp { op, values } => json!({
            "_": "BoolOp",
            "op": match op { BoolOp::And => "And", BoolOp::Or => "Or" },
            "values": exprs(values)
        }),
        Expr::Compare { left, rest } => json!({
            "_": "Compare",
            "left": expr(left),
            "ops": rest.iter().map(|(op, _)| cmpop(*op)).collect::<Vec<_>>(),
            "comparators": rest.iter().map(|(_, e)| expr(e)).collect::<Vec<_>>()
        }),
        Expr::IfExp { test, body, orelse } => json!({
            "_": "IfExp", "test": expr(test), "body": expr(body), "orelse": expr(orelse)
        }),
        Expr::Lambda { params, body } => {
            json!({ "_": "Lambda", "args": arguments(params), "body": expr(body) })
        }
        Expr::NamedExpr { target, value } => json!({
            "_": "NamedExpr", "target": { "_": "Name", "id": target }, "value": expr(value)
        }),
        Expr::Tuple(items) => json!({ "_": "Tuple", "elts": exprs(items) }),
        Expr::List(items) => json!({ "_": "List", "elts": exprs(items) }),
        Expr::Set(items) => json!({ "_": "Set", "elts": exprs(items) }),
        Expr::Dict(entries) => json!({
            "_": "Dict",
            "keys": entries.iter().map(|(k, _)| opt(k)).collect::<Vec<_>>(),
            "values": entries.iter().map(|(_, v)| expr(v)).collect::<Vec<_>>()
        }),
        Expr::Starred(inner) => json!({ "_": "Starred", "value": expr(inner) }),
        Expr::ListComp { elt, generators } => {
            json!({ "_": "ListComp", "elt": expr(elt), "generators": comprehensions(generators) })
        }
        Expr::SetComp { elt, generators } => {
            json!({ "_": "SetComp", "elt": expr(elt), "generators": comprehensions(generators) })
        }
        Expr::GeneratorExp { elt, generators } => json!({
            "_": "GeneratorExp", "elt": expr(elt), "generators": comprehensions(generators)
        }),
        Expr::DictComp {
            key,
            value,
            generators,
        } => json!({
            "_": "DictComp", "key": expr(key), "value": expr(value),
            "generators": comprehensions(generators)
        }),
    }
}

fn comprehensions(generators: &[Comprehension]) -> Value {
    Value::Array(
        generators
            .iter()
            .map(|g| {
                json!({
                    "_": "comprehension", "target": expr(&g.target), "iter": expr(&g.iter),
                    "ifs": exprs(&g.ifs), "is_async": 0
                })
            })
            .collect(),
    )
}

/// CPython's `JoinedStr`: text as `Constant`s, fields as `FormattedValue`s whose
/// conversion is the character's code, or -1.
fn joined(parts: &[FPart]) -> Value {
    json!({
        "_": "JoinedStr",
        "values": parts.iter().map(|part| match part {
            FPart::Text(text) => json!({ "_": "Str", "value": text }),
            FPart::Field { value, conversion, spec } => json!({
                "_": "FormattedValue",
                "value": expr(value),
                "conversion": conversion.map_or(-1, |c| c as i64),
                "format_spec": spec.as_ref().map_or(Value::Null, |spec| joined(spec))
            }),
        }).collect::<Vec<_>>()
    })
}

fn binop(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "Add",
        BinOp::Sub => "Sub",
        BinOp::Mult => "Mult",
        BinOp::MatMult => "MatMult",
        BinOp::Div => "Div",
        BinOp::FloorDiv => "FloorDiv",
        BinOp::Mod => "Mod",
        BinOp::Pow => "Pow",
        BinOp::LShift => "LShift",
        BinOp::RShift => "RShift",
        BinOp::BitOr => "BitOr",
        BinOp::BitXor => "BitXor",
        BinOp::BitAnd => "BitAnd",
    }
}

fn cmpop(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "Eq",
        CmpOp::NotEq => "NotEq",
        CmpOp::Lt => "Lt",
        CmpOp::LtE => "LtE",
        CmpOp::Gt => "Gt",
        CmpOp::GtE => "GtE",
        CmpOp::Is => "Is",
        CmpOp::IsNot => "IsNot",
        CmpOp::In => "In",
        CmpOp::NotIn => "NotIn",
    }
}
