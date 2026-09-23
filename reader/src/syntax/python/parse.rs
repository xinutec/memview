//! Tokens to a [`Module`], refusing by name what the tree does not hold.
//!
//! Recursive descent over CPython's own precedence, so a program reads here as it
//! reads there. Anything outside the language the corpus uses is a [`Refusal`]
//! naming the construct, never a guess: those refusals, ranked, are what to build
//! next.

use super::ast::*;
use super::lex::{StrLit, Tok, Token, lex};

/// Why a program was not read, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub reason: Reason,
    /// Byte offset into the text.
    pub at: usize,
}

/// What stopped the parse. **Each names the construct**, so the ranking is the
/// work queue.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    Character(char),
    Tab,
    Indentation,
    UnterminatedString,
    /// A named Unicode escape, `\N{…}`.
    NamedEscape,
    /// `f"{x=}"`, which CPython expands into text and a field.
    DebugField,
    /// `f"{d[\"k\"]}"`: an escaped quote inside a field, which every CPython
    /// refuses — the program never ran. See `python::did_not_run`.
    EscapedQuoteInField,
    Class,
    Async,
    Yield,
    Await,
    Annotation,
    PositionalOnly,
    StarExcept,
    CommentInBrackets,
    ParenthesizedWith,
    /// Something is not assignable where a target must be.
    Target,
    /// Not Python at all, or not this parser's Python: what was expected.
    Expected(&'static str),
}

impl Reason {
    /// The name the report ranks by.
    pub fn label(&self) -> String {
        match self {
            Reason::Character(c) => format!("character {c:?}"),
            Reason::Expected(what) => format!("expected {what}"),
            other => format!("{other:?}"),
        }
    }
}

/// Read a whole program.
pub fn parse(text: &str) -> Result<Module, Refusal> {
    let tokens = lex(text)?;
    let mut parser = Parser { tokens, pos: 0 };
    let body = parser.statements_until(|tok| matches!(tok, Tok::End))?;
    Ok(Module { body })
}

/// Read one expression, as an f-string's field holds it.
fn expression_text(text: &str, at: usize) -> Result<Expr, Refusal> {
    let tokens = lex(text).map_err(|refusal| Refusal {
        at: at + refusal.at,
        ..refusal
    })?;
    let mut parser = Parser { tokens, pos: 0 };
    let expr = parser.star_expressions()?;
    parser.skip_newlines();
    if !matches!(parser.peek(), Tok::End) {
        return Err(Refusal {
            reason: Reason::Expected("the end of the field"),
            at,
        });
    }
    Ok(expr)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

type Parsed<T> = Result<T, Refusal>;

impl Parser {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos.min(self.tokens.len() - 1)].tok
    }

    fn peek_at(&self, ahead: usize) -> &Tok {
        &self.tokens[(self.pos + ahead).min(self.tokens.len() - 1)].tok
    }

    fn at(&self) -> usize {
        self.tokens[self.pos.min(self.tokens.len() - 1)].at
    }

    fn advance(&mut self) -> Tok {
        let tok = self.peek().clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        tok
    }

    fn refuse<T>(&self, reason: Reason) -> Parsed<T> {
        Err(Refusal {
            reason,
            at: self.at(),
        })
    }

    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), Tok::Op(o) if *o == op)
    }

    fn is_name(&self, name: &str) -> bool {
        matches!(self.peek(), Tok::Name(n) if n == name)
    }

    fn eat_op(&mut self, op: &str) -> bool {
        let found = self.is_op(op);
        if found {
            self.advance();
        }
        found
    }

    fn eat_name(&mut self, name: &str) -> bool {
        let found = self.is_name(name);
        if found {
            self.advance();
        }
        found
    }

    fn expect_op(&mut self, op: &'static str) -> Parsed<()> {
        if self.eat_op(op) {
            Ok(())
        } else {
            self.refuse(self.unexpected(op))
        }
    }

    fn expect_name(&mut self, name: &'static str) -> Parsed<()> {
        if self.eat_name(name) {
            Ok(())
        } else {
            self.refuse(self.unexpected(name))
        }
    }

    /// A comment where an expression was expected is its own refusal: it is the
    /// commonest way a bracketed program falls outside this tree.
    fn unexpected(&self, expected: &'static str) -> Reason {
        match self.peek() {
            Tok::Comment { .. } => Reason::CommentInBrackets,
            _ => Reason::Expected(expected),
        }
    }

    fn identifier(&mut self) -> Parsed<String> {
        match self.peek().clone() {
            Tok::Name(name) if !KEYWORDS.contains(&name.as_str()) => {
                self.advance();
                Ok(name)
            }
            _ => self.refuse(self.unexpected("a name")),
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Tok::Newline) {
            self.advance();
        }
    }

    // ---- statements ----

    fn statements_until(&mut self, done: impl Fn(&Tok) -> bool) -> Parsed<Vec<Stmt>> {
        let mut body = Vec::new();
        loop {
            match self.peek().clone() {
                tok if done(&tok) => return Ok(body),
                Tok::Newline => {
                    self.advance();
                }
                Tok::Comment { text, .. } => {
                    self.advance();
                    body.push(Stmt {
                        kind: StmtKind::Comment(text),
                        comment: None,
                    });
                }
                Tok::End | Tok::Dedent => return self.refuse(Reason::Expected("a statement")),
                _ => body.extend(self.statement()?),
            }
        }
    }

    /// One logical line of simple statements, or one compound statement.
    fn statement(&mut self) -> Parsed<Vec<Stmt>> {
        if let Tok::Name(word) = self.peek().clone() {
            match word.as_str() {
                "if" => return self.if_statement().map(|s| vec![s]),
                "for" => return self.for_statement().map(|s| vec![s]),
                "while" => return self.while_statement().map(|s| vec![s]),
                "with" => return self.with_statement().map(|s| vec![s]),
                "def" => return self.function(Vec::new()).map(|s| vec![s]),
                "try" => return self.try_statement().map(|s| vec![s]),
                "class" => return self.refuse(Reason::Class),
                "async" => return self.refuse(Reason::Async),
                _ => {}
            }
        }
        if self.is_op("@") {
            return self.decorated().map(|s| vec![s]);
        }
        self.simple_line()
    }

    fn simple_line(&mut self) -> Parsed<Vec<Stmt>> {
        let mut line = vec![Stmt {
            kind: self.simple()?,
            comment: None,
        }];
        while self.eat_op(";") {
            if matches!(self.peek(), Tok::Newline | Tok::Comment { .. } | Tok::End) {
                break;
            }
            line.push(Stmt {
                kind: self.simple()?,
                comment: None,
            });
        }
        let comment = self.trailing_comment();
        if let Some(last) = line.last_mut() {
            last.comment = comment;
        }
        match self.peek() {
            Tok::Newline => {
                self.advance();
            }
            Tok::End | Tok::Dedent => {}
            _ => return self.refuse(self.unexpected("the end of the line")),
        }
        Ok(line)
    }

    fn trailing_comment(&mut self) -> Option<String> {
        if let Tok::Comment {
            text,
            own_line: false,
        } = self.peek().clone()
        {
            self.advance();
            return Some(text);
        }
        None
    }

    fn simple(&mut self) -> Parsed<StmtKind> {
        if let Tok::Name(word) = self.peek().clone() {
            match word.as_str() {
                "pass" => {
                    self.advance();
                    return Ok(StmtKind::Pass);
                }
                "break" => {
                    self.advance();
                    return Ok(StmtKind::Break);
                }
                "continue" => {
                    self.advance();
                    return Ok(StmtKind::Continue);
                }
                "return" => {
                    self.advance();
                    let value = if self.ends_simple() {
                        None
                    } else {
                        Some(self.star_expressions()?)
                    };
                    return Ok(StmtKind::Return(value));
                }
                "raise" => {
                    self.advance();
                    if self.ends_simple() {
                        return Ok(StmtKind::Raise {
                            exc: None,
                            cause: None,
                        });
                    }
                    let exc = self.expression()?;
                    let cause = if self.eat_name("from") {
                        Some(self.expression()?)
                    } else {
                        None
                    };
                    return Ok(StmtKind::Raise {
                        exc: Some(exc),
                        cause,
                    });
                }
                "assert" => {
                    self.advance();
                    let test = self.expression()?;
                    let msg = if self.eat_op(",") {
                        Some(self.expression()?)
                    } else {
                        None
                    };
                    return Ok(StmtKind::Assert { test, msg });
                }
                "del" => {
                    self.advance();
                    let mut targets = vec![self.target_primary()?];
                    while self.eat_op(",") {
                        if self.ends_simple() {
                            break;
                        }
                        targets.push(self.target_primary()?);
                    }
                    return Ok(StmtKind::Delete(targets));
                }
                "global" | "nonlocal" => {
                    self.advance();
                    let mut names = vec![self.identifier()?];
                    while self.eat_op(",") {
                        names.push(self.identifier()?);
                    }
                    return Ok(if word == "global" {
                        StmtKind::Global(names)
                    } else {
                        StmtKind::Nonlocal(names)
                    });
                }
                "import" => {
                    self.advance();
                    let mut names = vec![self.dotted_alias()?];
                    while self.eat_op(",") {
                        names.push(self.dotted_alias()?);
                    }
                    return Ok(StmtKind::Import(names));
                }
                "from" => return self.import_from(),
                "yield" => return self.refuse(Reason::Yield),
                "await" => return self.refuse(Reason::Await),
                _ => {}
            }
        }
        let first = self.star_expressions()?;
        if self.is_op(":") {
            return self.refuse(Reason::Annotation);
        }
        if let Tok::Op(op) = self.peek().clone()
            && let Some(bin) = augmented(op)
        {
            self.advance();
            check_target(&first, false).map_err(|reason| Refusal {
                reason,
                at: self.at(),
            })?;
            let value = self.star_expressions()?;
            return Ok(StmtKind::AugAssign {
                target: first,
                op: bin,
                value,
            });
        }
        if !self.is_op("=") {
            return Ok(StmtKind::Expr(first));
        }
        let mut targets = vec![first];
        while self.eat_op("=") {
            targets.push(self.star_expressions()?);
        }
        let value = targets.pop().unwrap_or(Expr::Singleton(Singleton::None));
        for target in &targets {
            check_target(target, true).map_err(|reason| Refusal {
                reason,
                at: self.at(),
            })?;
        }
        Ok(StmtKind::Assign { targets, value })
    }

    fn ends_simple(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Newline | Tok::End | Tok::Comment { .. } | Tok::Dedent
        ) || self.is_op(";")
    }

    fn dotted_name(&mut self) -> Parsed<String> {
        let mut name = self.identifier()?;
        while self.eat_op(".") {
            name.push('.');
            name.push_str(&self.identifier()?);
        }
        Ok(name)
    }

    fn dotted_alias(&mut self) -> Parsed<Alias> {
        let name = self.dotted_name()?;
        let asname = if self.eat_name("as") {
            Some(self.identifier()?)
        } else {
            None
        };
        Ok(Alias { name, asname })
    }

    fn import_from(&mut self) -> Parsed<StmtKind> {
        self.expect_name("from")?;
        let mut level = 0;
        loop {
            if self.eat_op(".") {
                level += 1;
            } else if self.eat_op("...") {
                level += 3;
            } else {
                break;
            }
        }
        let module = if self.is_name("import") {
            None
        } else {
            Some(self.dotted_name()?)
        };
        self.expect_name("import")?;
        if self.eat_op("*") {
            return Ok(StmtKind::ImportFrom {
                module,
                level,
                names: vec![Alias {
                    name: "*".to_string(),
                    asname: None,
                }],
            });
        }
        let parenthesised = self.eat_op("(");
        let mut names = Vec::new();
        loop {
            if parenthesised && self.is_op(")") {
                break;
            }
            let name = self.identifier()?;
            let asname = if self.eat_name("as") {
                Some(self.identifier()?)
            } else {
                None
            };
            names.push(Alias { name, asname });
            if !self.eat_op(",") {
                break;
            }
            if !parenthesised && self.ends_simple() {
                return self.refuse(Reason::Expected("a name after `,`"));
            }
        }
        if parenthesised {
            self.expect_op(")")?;
        }
        Ok(StmtKind::ImportFrom {
            module,
            level,
            names,
        })
    }

    /// `:` and then an indented block, or statements on the same line. The comment
    /// after the `:` belongs to the statement that opened the block.
    fn block(&mut self) -> Parsed<(Vec<Stmt>, Option<String>)> {
        self.expect_op(":")?;
        let comment = self.trailing_comment();
        if !matches!(self.peek(), Tok::Newline) {
            return Ok((self.simple_line()?, comment));
        }
        self.advance();
        let mut leading = Vec::new();
        while let Tok::Comment { text, .. } = self.peek().clone() {
            self.advance();
            leading.push(Stmt {
                kind: StmtKind::Comment(text),
                comment: None,
            });
        }
        if !matches!(self.peek(), Tok::Indent) {
            return self.refuse(Reason::Expected("an indented block"));
        }
        self.advance();
        let mut body = self.statements_until(|tok| matches!(tok, Tok::Dedent))?;
        self.advance();
        leading.append(&mut body);
        Ok((leading, comment))
    }

    fn if_statement(&mut self) -> Parsed<Stmt> {
        self.advance();
        let test = self.named_expression()?;
        let (body, comment) = self.block()?;
        let orelse = self.else_branch(true)?;
        Ok(Stmt {
            kind: StmtKind::If { test, body, orelse },
            comment,
        })
    }

    /// What follows an `if` body: `elif` becomes an `else` holding one `if`, as
    /// CPython reads it.
    fn else_branch(&mut self, elif: bool) -> Parsed<Vec<Stmt>> {
        if elif && self.is_name("elif") {
            return self.if_statement().map(|s| vec![s]);
        }
        if self.eat_name("else") {
            let (mut body, comment) = self.block()?;
            if let Some(comment) = comment {
                body.insert(
                    0,
                    Stmt {
                        kind: StmtKind::Comment(comment),
                        comment: None,
                    },
                );
            }
            return Ok(body);
        }
        Ok(Vec::new())
    }

    fn for_statement(&mut self) -> Parsed<Stmt> {
        self.advance();
        let target = self.target_list()?;
        self.expect_name("in")?;
        let iter = self.star_expressions()?;
        let (body, comment) = self.block()?;
        let orelse = self.else_branch(false)?;
        Ok(Stmt {
            kind: StmtKind::For {
                target,
                iter,
                body,
                orelse,
            },
            comment,
        })
    }

    fn while_statement(&mut self) -> Parsed<Stmt> {
        self.advance();
        let test = self.named_expression()?;
        let (body, comment) = self.block()?;
        let orelse = self.else_branch(false)?;
        Ok(Stmt {
            kind: StmtKind::While { test, body, orelse },
            comment,
        })
    }

    fn with_statement(&mut self) -> Parsed<Stmt> {
        self.advance();
        let mut items = Vec::new();
        loop {
            let context = match self.expression() {
                Ok(context) => context,
                Err(refusal) if self.is_name("as") => {
                    return Err(Refusal {
                        reason: Reason::ParenthesizedWith,
                        ..refusal
                    });
                }
                Err(refusal) => return Err(refusal),
            };
            let var = if self.eat_name("as") {
                Some(self.target_primary()?)
            } else {
                None
            };
            items.push(WithItem { context, var });
            if !self.eat_op(",") {
                break;
            }
        }
        let (body, comment) = self.block()?;
        Ok(Stmt {
            kind: StmtKind::With { items, body },
            comment,
        })
    }

    fn decorated(&mut self) -> Parsed<Stmt> {
        let mut decorators = Vec::new();
        while self.eat_op("@") {
            decorators.push(self.named_expression()?);
            if !matches!(self.peek(), Tok::Newline) {
                return self.refuse(Reason::Expected("a newline after a decorator"));
            }
            self.advance();
        }
        match self.peek() {
            Tok::Name(word) if word == "def" => self.function(decorators),
            Tok::Name(word) if word == "class" => self.refuse(Reason::Class),
            Tok::Name(word) if word == "async" => self.refuse(Reason::Async),
            _ => self.refuse(Reason::Expected("a definition after a decorator")),
        }
    }

    fn function(&mut self, decorators: Vec<Expr>) -> Parsed<Stmt> {
        self.expect_name("def")?;
        let name = self.identifier()?;
        self.expect_op("(")?;
        let params = self.params(")")?;
        self.expect_op(")")?;
        if self.is_op("->") {
            return self.refuse(Reason::Annotation);
        }
        let (body, comment) = self.block()?;
        Ok(Stmt {
            kind: StmtKind::FunctionDef {
                decorators,
                name,
                params,
                body,
            },
            comment,
        })
    }

    /// Parameters up to `close`, which is `)` for a `def` and `:` for a lambda.
    fn params(&mut self, close: &'static str) -> Parsed<Params> {
        let mut params = Params::default();
        let mut keyword_only = false;
        while !self.is_op(close) {
            if self.eat_op("**") {
                params.kwarg = Some(self.identifier()?);
            } else if self.eat_op("*") {
                keyword_only = true;
                if !self.is_op(",") && !self.is_op(close) {
                    params.vararg = Some(self.identifier()?);
                }
            } else if self.is_op("/") {
                return self.refuse(Reason::PositionalOnly);
            } else {
                let name = self.identifier()?;
                if close == ")" && self.is_op(":") {
                    return self.refuse(Reason::Annotation);
                }
                let default = if self.eat_op("=") {
                    Some(self.expression()?)
                } else {
                    None
                };
                let param = Param { name, default };
                if keyword_only {
                    params.kwonly.push(param);
                } else {
                    params.args.push(param);
                }
            }
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(params)
    }

    fn try_statement(&mut self) -> Parsed<Stmt> {
        self.advance();
        let (body, comment) = self.block()?;
        let mut handlers = Vec::new();
        while self.eat_name("except") {
            if self.is_op("*") {
                return self.refuse(Reason::StarExcept);
            }
            let (kind, name) = if self.is_op(":") {
                (None, None)
            } else {
                let kind = self.expression()?;
                let name = if self.eat_name("as") {
                    Some(self.identifier()?)
                } else {
                    None
                };
                (Some(kind), name)
            };
            let (mut body, header) = self.block()?;
            if let Some(header) = header {
                body.insert(
                    0,
                    Stmt {
                        kind: StmtKind::Comment(header),
                        comment: None,
                    },
                );
            }
            handlers.push(Handler { kind, name, body });
        }
        let orelse = self.else_branch(false)?;
        let finalbody = if self.eat_name("finally") {
            let (mut body, header) = self.block()?;
            if let Some(header) = header {
                body.insert(
                    0,
                    Stmt {
                        kind: StmtKind::Comment(header),
                        comment: None,
                    },
                );
            }
            body
        } else {
            Vec::new()
        };
        if handlers.is_empty() && finalbody.is_empty() {
            return self.refuse(Reason::Expected("`except` or `finally`"));
        }
        Ok(Stmt {
            kind: StmtKind::Try {
                body,
                handlers,
                orelse,
                finalbody,
            },
            comment,
        })
    }

    // ---- targets ----

    /// `for a, b in …`: targets without parentheses, up to `in`.
    fn target_list(&mut self) -> Parsed<Expr> {
        let first = self.target_primary()?;
        if !self.is_op(",") {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_op(",") {
            if self.is_name("in") || self.is_op("=") {
                break;
            }
            items.push(self.target_primary()?);
        }
        Ok(Expr::Tuple(items))
    }

    fn target_primary(&mut self) -> Parsed<Expr> {
        let target = if self.eat_op("*") {
            Expr::Starred(Box::new(self.bitwise_or()?))
        } else {
            self.bitwise_or()?
        };
        check_target(&target, true).map_err(|reason| Refusal {
            reason,
            at: self.at(),
        })?;
        Ok(target)
    }

    // ---- expressions ----

    /// `a, *b, c` — a tuple when there is a comma.
    fn star_expressions(&mut self) -> Parsed<Expr> {
        let first = self.star_expression()?;
        if !self.is_op(",") {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_op(",") {
            if self.ends_expression_list() {
                break;
            }
            items.push(self.star_expression()?);
        }
        Ok(Expr::Tuple(items))
    }

    fn ends_expression_list(&self) -> bool {
        self.ends_simple()
            || matches!(self.peek(), Tok::Op("=" | ")" | "]" | "}" | ":"))
            || matches!(self.peek(), Tok::Op(op) if augmented(op).is_some())
    }

    fn star_expression(&mut self) -> Parsed<Expr> {
        if self.eat_op("*") {
            return Ok(Expr::Starred(Box::new(self.bitwise_or()?)));
        }
        self.expression()
    }

    fn named_expression(&mut self) -> Parsed<Expr> {
        if let (Tok::Name(name), Tok::Op(":=")) = (self.peek().clone(), self.peek_at(1).clone())
            && !KEYWORDS.contains(&name.as_str())
        {
            self.advance();
            self.advance();
            let value = self.expression()?;
            return Ok(Expr::NamedExpr {
                target: name,
                value: Box::new(value),
            });
        }
        self.expression()
    }

    fn expression(&mut self) -> Parsed<Expr> {
        if self.is_name("lambda") {
            return self.lambda();
        }
        if self.is_name("yield") {
            return self.refuse(Reason::Yield);
        }
        let body = self.disjunction()?;
        if !self.is_name("if") {
            return Ok(body);
        }
        self.advance();
        let test = self.disjunction()?;
        self.expect_name("else")?;
        let orelse = self.expression()?;
        Ok(Expr::IfExp {
            test: Box::new(test),
            body: Box::new(body),
            orelse: Box::new(orelse),
        })
    }

    fn lambda(&mut self) -> Parsed<Expr> {
        self.expect_name("lambda")?;
        let params = self.params(":")?;
        self.expect_op(":")?;
        let body = self.expression()?;
        Ok(Expr::Lambda {
            params,
            body: Box::new(body),
        })
    }

    fn disjunction(&mut self) -> Parsed<Expr> {
        let first = self.conjunction()?;
        if !self.is_name("or") {
            return Ok(first);
        }
        let mut values = vec![first];
        while self.eat_name("or") {
            values.push(self.conjunction()?);
        }
        Ok(Expr::BoolOp {
            op: BoolOp::Or,
            values,
        })
    }

    fn conjunction(&mut self) -> Parsed<Expr> {
        let first = self.inversion()?;
        if !self.is_name("and") {
            return Ok(first);
        }
        let mut values = vec![first];
        while self.eat_name("and") {
            values.push(self.inversion()?);
        }
        Ok(Expr::BoolOp {
            op: BoolOp::And,
            values,
        })
    }

    fn inversion(&mut self) -> Parsed<Expr> {
        if self.eat_name("not") {
            let operand = self.inversion()?;
            return Ok(Expr::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(operand),
            });
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Parsed<Expr> {
        let left = self.bitwise_or()?;
        let mut rest = Vec::new();
        loop {
            let op = match self.peek().clone() {
                Tok::Op("==") => CmpOp::Eq,
                Tok::Op("!=") => CmpOp::NotEq,
                Tok::Op("<") => CmpOp::Lt,
                Tok::Op("<=") => CmpOp::LtE,
                Tok::Op(">") => CmpOp::Gt,
                Tok::Op(">=") => CmpOp::GtE,
                Tok::Name(word) if word == "in" => CmpOp::In,
                Tok::Name(word) if word == "is" => {
                    if matches!(self.peek_at(1), Tok::Name(n) if n == "not") {
                        self.advance();
                        CmpOp::IsNot
                    } else {
                        CmpOp::Is
                    }
                }
                Tok::Name(word)
                    if word == "not" && matches!(self.peek_at(1), Tok::Name(n) if n == "in") =>
                {
                    self.advance();
                    CmpOp::NotIn
                }
                _ => break,
            };
            self.advance();
            rest.push((op, self.bitwise_or()?));
        }
        if rest.is_empty() {
            return Ok(left);
        }
        Ok(Expr::Compare {
            left: Box::new(left),
            rest,
        })
    }

    fn binary(
        &mut self,
        ops: &[(&str, BinOp)],
        next: fn(&mut Self) -> Parsed<Expr>,
    ) -> Parsed<Expr> {
        let mut left = next(self)?;
        'outer: loop {
            for (symbol, op) in ops {
                if self.is_op(symbol) {
                    self.advance();
                    let right = next(self)?;
                    left = Expr::BinOp {
                        left: Box::new(left),
                        op: *op,
                        right: Box::new(right),
                    };
                    continue 'outer;
                }
            }
            return Ok(left);
        }
    }

    fn bitwise_or(&mut self) -> Parsed<Expr> {
        self.binary(&[("|", BinOp::BitOr)], Self::bitwise_xor)
    }

    fn bitwise_xor(&mut self) -> Parsed<Expr> {
        self.binary(&[("^", BinOp::BitXor)], Self::bitwise_and)
    }

    fn bitwise_and(&mut self) -> Parsed<Expr> {
        self.binary(&[("&", BinOp::BitAnd)], Self::shift)
    }

    fn shift(&mut self) -> Parsed<Expr> {
        self.binary(&[("<<", BinOp::LShift), (">>", BinOp::RShift)], Self::sum)
    }

    fn sum(&mut self) -> Parsed<Expr> {
        self.binary(&[("+", BinOp::Add), ("-", BinOp::Sub)], Self::term)
    }

    fn term(&mut self) -> Parsed<Expr> {
        self.binary(
            &[
                ("*", BinOp::Mult),
                ("/", BinOp::Div),
                ("//", BinOp::FloorDiv),
                ("%", BinOp::Mod),
                ("@", BinOp::MatMult),
            ],
            Self::factor,
        )
    }

    fn factor(&mut self) -> Parsed<Expr> {
        let op = match self.peek() {
            Tok::Op("+") => UnaryOp::UAdd,
            Tok::Op("-") => UnaryOp::USub,
            Tok::Op("~") => UnaryOp::Invert,
            _ => return self.power(),
        };
        self.advance();
        let operand = self.factor()?;
        Ok(Expr::UnaryOp {
            op,
            operand: Box::new(operand),
        })
    }

    /// `**` binds tighter than a unary on its left and looser on its right:
    /// `-a ** -b` is `-(a ** (-b))`.
    fn power(&mut self) -> Parsed<Expr> {
        if self.is_name("await") {
            return self.refuse(Reason::Await);
        }
        let base = self.primary()?;
        if !self.eat_op("**") {
            return Ok(base);
        }
        let exponent = self.factor()?;
        Ok(Expr::BinOp {
            left: Box::new(base),
            op: BinOp::Pow,
            right: Box::new(exponent),
        })
    }

    fn primary(&mut self) -> Parsed<Expr> {
        let mut value = self.atom()?;
        loop {
            if self.eat_op(".") {
                let attr = self.identifier()?;
                value = Expr::Attribute {
                    value: Box::new(value),
                    attr,
                };
            } else if self.eat_op("(") {
                let args = self.call_args()?;
                self.expect_op(")")?;
                value = Expr::Call {
                    func: Box::new(value),
                    args,
                };
            } else if self.eat_op("[") {
                let index = self.subscript()?;
                self.expect_op("]")?;
                value = Expr::Subscript {
                    value: Box::new(value),
                    index: Box::new(index),
                };
            } else {
                return Ok(value);
            }
        }
    }

    fn call_args(&mut self) -> Parsed<Vec<Arg>> {
        let mut args = Vec::new();
        while !self.is_op(")") {
            if self.eat_op("**") {
                args.push(Arg::DoubleStarred(self.expression()?));
            } else if self.eat_op("*") {
                args.push(Arg::Starred(self.expression()?));
            } else if let (Tok::Name(name), Tok::Op("=")) =
                (self.peek().clone(), self.peek_at(1).clone())
            {
                self.advance();
                self.advance();
                args.push(Arg::Keyword(name, self.expression()?));
            } else {
                let value = self.named_expression()?;
                // A generator as the only argument needs no brackets of its own.
                if self.is_name("for") {
                    let generators = self.comprehension()?;
                    args.push(Arg::Positional(Expr::GeneratorExp {
                        elt: Box::new(value),
                        generators,
                    }));
                } else {
                    args.push(Arg::Positional(value));
                }
            }
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(args)
    }

    /// What goes between `[` and `]`: an expression, a slice, or several.
    fn subscript(&mut self) -> Parsed<Expr> {
        let first = self.slice()?;
        if !self.is_op(",") {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_op(",") {
            if self.is_op("]") {
                break;
            }
            items.push(self.slice()?);
        }
        Ok(Expr::Tuple(items))
    }

    fn slice(&mut self) -> Parsed<Expr> {
        let lower = if self.is_op(":") {
            None
        } else {
            let value = self.star_expression()?;
            if !self.is_op(":") {
                return Ok(value);
            }
            Some(Box::new(value))
        };
        self.expect_op(":")?;
        let upper = if self.is_op(":") || self.is_op("]") || self.is_op(",") {
            None
        } else {
            Some(Box::new(self.expression()?))
        };
        let step = if self.eat_op(":") && !self.is_op("]") && !self.is_op(",") {
            Some(Box::new(self.expression()?))
        } else {
            None
        };
        Ok(Expr::Slice { lower, upper, step })
    }

    fn comprehension(&mut self) -> Parsed<Vec<Comprehension>> {
        let mut generators = Vec::new();
        while self.is_name("for") || self.is_name("async") {
            if self.is_name("async") {
                return self.refuse(Reason::Async);
            }
            self.advance();
            let target = self.target_list()?;
            self.expect_name("in")?;
            let iter = self.disjunction()?;
            let mut ifs = Vec::new();
            while self.eat_name("if") {
                ifs.push(self.disjunction()?);
            }
            generators.push(Comprehension { target, iter, ifs });
        }
        Ok(generators)
    }

    fn atom(&mut self) -> Parsed<Expr> {
        match self.peek().clone() {
            Tok::Name(word) => match word.as_str() {
                "True" => {
                    self.advance();
                    Ok(Expr::Singleton(Singleton::True))
                }
                "False" => {
                    self.advance();
                    Ok(Expr::Singleton(Singleton::False))
                }
                "None" => {
                    self.advance();
                    Ok(Expr::Singleton(Singleton::None))
                }
                _ => Ok(Expr::Name(self.identifier()?)),
            },
            Tok::Number(number) => {
                self.advance();
                Ok(Expr::Number(number))
            }
            Tok::Str(_) => self.strings(),
            Tok::Op("...") => {
                self.advance();
                Ok(Expr::Singleton(Singleton::Ellipsis))
            }
            Tok::Op("(") => self.parenthesised(),
            Tok::Op("[") => self.list(),
            Tok::Op("{") => self.braces(),
            _ => self.refuse(self.unexpected("an expression")),
        }
    }

    fn parenthesised(&mut self) -> Parsed<Expr> {
        self.expect_op("(")?;
        if self.eat_op(")") {
            return Ok(Expr::Tuple(Vec::new()));
        }
        if self.is_name("yield") {
            return self.refuse(Reason::Yield);
        }
        let first = self.star_named()?;
        if self.is_name("for") {
            let generators = self.comprehension()?;
            self.expect_op(")")?;
            return Ok(Expr::GeneratorExp {
                elt: Box::new(first),
                generators,
            });
        }
        if self.eat_op(")") {
            // Brackets round one thing are that thing.
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_op(",") {
            if self.is_op(")") {
                break;
            }
            items.push(self.star_named()?);
        }
        self.expect_op(")")?;
        Ok(Expr::Tuple(items))
    }

    fn star_named(&mut self) -> Parsed<Expr> {
        if self.eat_op("*") {
            return Ok(Expr::Starred(Box::new(self.bitwise_or()?)));
        }
        self.named_expression()
    }

    fn list(&mut self) -> Parsed<Expr> {
        self.expect_op("[")?;
        if self.eat_op("]") {
            return Ok(Expr::List(Vec::new()));
        }
        let first = self.star_named()?;
        if self.is_name("for") {
            let generators = self.comprehension()?;
            self.expect_op("]")?;
            return Ok(Expr::ListComp {
                elt: Box::new(first),
                generators,
            });
        }
        let mut items = vec![first];
        while self.eat_op(",") {
            if self.is_op("]") {
                break;
            }
            items.push(self.star_named()?);
        }
        self.expect_op("]")?;
        Ok(Expr::List(items))
    }

    fn braces(&mut self) -> Parsed<Expr> {
        self.expect_op("{")?;
        if self.eat_op("}") {
            return Ok(Expr::Dict(Vec::new()));
        }
        // `{**a, …}` is a dict; `{*a, …}` a set.
        let first_key = if self.eat_op("**") {
            None
        } else {
            let key = self.star_named()?;
            if !self.is_op(":") {
                return self.set_rest(key);
            }
            Some(key)
        };
        let first_value = if first_key.is_some() {
            self.expect_op(":")?;
            self.expression()?
        } else {
            self.bitwise_or()?
        };
        if let Some(key) = &first_key
            && self.is_name("for")
        {
            let generators = self.comprehension()?;
            self.expect_op("}")?;
            return Ok(Expr::DictComp {
                key: Box::new(key.clone()),
                value: Box::new(first_value),
                generators,
            });
        }
        let mut entries = vec![(first_key, first_value)];
        while self.eat_op(",") {
            if self.is_op("}") {
                break;
            }
            if self.eat_op("**") {
                entries.push((None, self.bitwise_or()?));
            } else {
                let key = self.expression()?;
                self.expect_op(":")?;
                entries.push((Some(key), self.expression()?));
            }
        }
        self.expect_op("}")?;
        Ok(Expr::Dict(entries))
    }

    fn set_rest(&mut self, first: Expr) -> Parsed<Expr> {
        if self.is_name("for") {
            let generators = self.comprehension()?;
            self.expect_op("}")?;
            return Ok(Expr::SetComp {
                elt: Box::new(first),
                generators,
            });
        }
        let mut items = vec![first];
        while self.eat_op(",") {
            if self.is_op("}") {
                break;
            }
            items.push(self.star_named()?);
        }
        self.expect_op("}")?;
        Ok(Expr::Set(items))
    }

    /// Adjacent literals, merged as CPython merges them.
    fn strings(&mut self) -> Parsed<Expr> {
        let mut parts: Vec<FPart> = Vec::new();
        let mut bytes: Option<Vec<u8>> = None;
        let mut formatted = false;
        let mut any_text = false;
        while let Tok::Str(lit) = self.peek().clone() {
            let at = self.at();
            self.advance();
            if lit.bytes {
                if any_text {
                    return self.refuse(Reason::Expected("bytes and text not mixed"));
                }
                bytes
                    .get_or_insert_with(Vec::new)
                    .extend(decode_bytes(&lit, at)?);
                continue;
            }
            if bytes.is_some() {
                return self.refuse(Reason::Expected("bytes and text not mixed"));
            }
            any_text = true;
            if lit.formatted {
                formatted = true;
                for part in fstring(&lit, at)? {
                    push_part(&mut parts, part);
                }
            } else {
                push_part(&mut parts, FPart::Text(decode_text(&lit, at)?));
            }
        }
        if let Some(bytes) = bytes {
            return Ok(Expr::Bytes(bytes));
        }
        if formatted {
            return Ok(Expr::FString(parts));
        }
        Ok(Expr::Str(match parts.pop() {
            Some(FPart::Text(text)) => text,
            _ => String::new(),
        }))
    }
}

/// Words that are never a name.
const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

fn augmented(op: &str) -> Option<BinOp> {
    Some(match op {
        "+=" => BinOp::Add,
        "-=" => BinOp::Sub,
        "*=" => BinOp::Mult,
        "@=" => BinOp::MatMult,
        "/=" => BinOp::Div,
        "//=" => BinOp::FloorDiv,
        "%=" => BinOp::Mod,
        "**=" => BinOp::Pow,
        "<<=" => BinOp::LShift,
        ">>=" => BinOp::RShift,
        "|=" => BinOp::BitOr,
        "^=" => BinOp::BitXor,
        "&=" => BinOp::BitAnd,
        _ => return None,
    })
}

/// Whether an expression can be assigned to. `several` allows a tuple or list of
/// targets, which an augmented assignment does not.
fn check_target(target: &Expr, several: bool) -> Result<(), Reason> {
    match target {
        Expr::Name(_) | Expr::Attribute { .. } | Expr::Subscript { .. } => Ok(()),
        Expr::Tuple(items) | Expr::List(items) if several => {
            items.iter().try_for_each(|item| check_target(item, true))
        }
        Expr::Starred(inner) if several => check_target(inner, true),
        _ => Err(Reason::Target),
    }
}

/// Add a part, merging text into text so one value has one shape.
fn push_part(parts: &mut Vec<FPart>, part: FPart) {
    if let (FPart::Text(new), Some(FPart::Text(last))) = (&part, parts.last_mut()) {
        last.push_str(new);
        return;
    }
    if let FPart::Text(text) = &part
        && text.is_empty()
    {
        return;
    }
    parts.push(part);
}

/// A string literal's value.
fn decode_text(lit: &StrLit, at: usize) -> Result<String, Refusal> {
    if lit.raw {
        return Ok(lit.body.clone());
    }
    unescape(&lit.body, at)
}

fn unescape(body: &str, at: usize) -> Result<String, Refusal> {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            None => out.push('\\'),
            Some('\n') => {}
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('a') => out.push('\x07'),
            Some('b') => out.push('\x08'),
            Some('f') => out.push('\x0c'),
            Some('v') => out.push('\x0b'),
            Some('x') => {
                let hex: String = (0..2).filter_map(|_| chars.next()).collect();
                let code = u32::from_str_radix(&hex, 16).map_err(|_| Refusal {
                    reason: Reason::Expected("two hex digits after \\x"),
                    at,
                })?;
                out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
            }
            Some(d @ '0'..='7') => {
                let mut code = d.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    match chars.peek() {
                        Some(n @ '0'..='7') => {
                            code = code * 8 + n.to_digit(8).unwrap_or(0);
                            chars.next();
                        }
                        _ => break,
                    }
                }
                out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
            }
            Some(u @ ('u' | 'U')) => {
                let len = if u == 'u' { 4 } else { 8 };
                let hex: String = (0..len).filter_map(|_| chars.next()).collect();
                let code = u32::from_str_radix(&hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or(Refusal {
                        reason: Reason::Expected("a code point after \\u"),
                        at,
                    })?;
                out.push(code);
            }
            Some('N') => {
                return Err(Refusal {
                    reason: Reason::NamedEscape,
                    at,
                });
            }
            // An unknown escape keeps its backslash, as CPython does (with a warning).
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    Ok(out)
}

fn decode_bytes(lit: &StrLit, at: usize) -> Result<Vec<u8>, Refusal> {
    let text = if lit.raw {
        lit.body.clone()
    } else {
        unescape(&lit.body, at)?
    };
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).ok())
        .collect::<Option<Vec<u8>>>()
        .ok_or(Refusal {
            reason: Reason::Expected("bytes holding only ASCII"),
            at,
        })
}

/// An f-string's body as parts: text, and fields holding expressions.
fn fstring(lit: &StrLit, at: usize) -> Result<Vec<FPart>, Refusal> {
    fparts(&lit.body, lit.raw, at)
}

fn fparts(body: &str, raw: bool, at: usize) -> Result<Vec<FPart>, Refusal> {
    let chars: Vec<char> = body.chars().collect();
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' && chars.get(i + 1) == Some(&'{') {
            text.push('{');
            i += 2;
            continue;
        }
        if c == '}' && chars.get(i + 1) == Some(&'}') {
            text.push('}');
            i += 2;
            continue;
        }
        if c == '{' {
            let decoded = if raw {
                text.clone()
            } else {
                unescape(&text, at)?
            };
            push_part(&mut parts, FPart::Text(decoded));
            text.clear();
            let end = field_end(&chars, i + 1).ok_or(Refusal {
                reason: Reason::Expected("a field's closing brace"),
                at,
            })?;
            if chars[i + 1..end]
                .windows(2)
                .any(|pair| pair[0] == '\\' && matches!(pair[1], '\'' | '"'))
            {
                return Err(Refusal {
                    reason: Reason::EscapedQuoteInField,
                    at,
                });
            }
            parts.push(field(&chars[i + 1..end], raw, at)?);
            i = end + 1;
            continue;
        }
        if c == '\\' && !raw && i + 1 < chars.len() {
            text.push(c);
            text.push(chars[i + 1]);
            i += 2;
            continue;
        }
        text.push(c);
        i += 1;
    }
    let decoded = if raw { text } else { unescape(&text, at)? };
    push_part(&mut parts, FPart::Text(decoded));
    Ok(parts)
}

/// The index of the `}` that closes the field opening at `start`.
fn field_end(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '}' if depth == 0 => return Some(i),
            '}' => depth -= 1,
            '\\' => {
                // An escaped character is not the start of a nested literal.
                i += 1;
            }
            quote @ ('\'' | '"') => {
                i += 1;
                while i < chars.len() && chars[i] != quote {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// One field's text: `expression[!conversion][:spec]`.
fn field(chars: &[char], raw: bool, at: usize) -> Result<FPart, Refusal> {
    let mut depth = 0usize;
    let mut i = 0;
    let mut expr_end = chars.len();
    let mut conversion = None;
    let mut spec = None;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            quote @ ('\'' | '"') => {
                i += 1;
                while i < chars.len() && chars[i] != quote {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            '!' if depth == 0 && chars.get(i + 1) != Some(&'=') => {
                expr_end = i;
                conversion = chars.get(i + 1).copied();
                if let Some(colon) = chars.get(i + 2)
                    && *colon == ':'
                {
                    let spec_text: String = chars[i + 3..].iter().collect();
                    spec = Some(fparts(&spec_text, raw, at)?);
                }
                break;
            }
            ':' if depth == 0 => {
                expr_end = i;
                let spec_text: String = chars[i + 1..].iter().collect();
                spec = Some(fparts(&spec_text, raw, at)?);
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let expr_text: String = chars[..expr_end].iter().collect();
    let trimmed = expr_text.trim_end();
    if trimmed.ends_with('=') && !trimmed.ends_with("==") && !trimmed.ends_with("!=") {
        return Err(Refusal {
            reason: Reason::DebugField,
            at,
        });
    }
    let value = expression_text(&expr_text, at)?;
    Ok(FPart::Field {
        value: Box::new(value),
        conversion,
        spec,
    })
}
