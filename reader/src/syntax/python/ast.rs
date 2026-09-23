//! The tree for the Python the fleet runs.
//!
//! Shaped after CPython's own `ast`, so the second gate can compare the two node
//! for node, and **normalised the way CPython normalises**: an `elif` is an `else`
//! holding one `if`, adjacent string literals are one literal, and a string's
//! quoting is gone — `'a'`, `"a"` and `'''a'''` are one value. What CPython keeps
//! apart, this keeps apart.
//!
//! Comments are nodes, as they are in the shell's tree: a comment on a line of
//! its own is a statement, and one after code on the same line rides on that
//! statement. CPython drops both, so only the round-trip law checks them.

/// A whole program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub body: Vec<Stmt>,
}

/// One statement, with the comment written after it on its last line, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StmtKind {
    /// A comment on a line of its own, without the `#`.
    Comment(String),
    Expr(Expr),
    /// `a = b = value`: every target, left to right.
    Assign {
        targets: Vec<Expr>,
        value: Expr,
    },
    /// `a += 1`.
    AugAssign {
        target: Expr,
        op: BinOp,
        value: Expr,
    },
    Import(Vec<Alias>),
    /// `from ..pkg import a as b`. `level` counts the dots; `names` holding one
    /// alias named `*` is `import *`.
    ImportFrom {
        module: Option<String>,
        level: u32,
        names: Vec<Alias>,
    },
    Assert {
        test: Expr,
        msg: Option<Expr>,
    },
    Delete(Vec<Expr>),
    Pass,
    Break,
    Continue,
    Return(Option<Expr>),
    Raise {
        exc: Option<Expr>,
        cause: Option<Expr>,
    },
    Global(Vec<String>),
    Nonlocal(Vec<String>),
    /// No `elif`: CPython unfolds one into an `orelse` holding one `If`, and so
    /// does this, or one program would be two trees.
    If {
        test: Expr,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    For {
        target: Expr,
        iter: Expr,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    While {
        test: Expr,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    With {
        items: Vec<WithItem>,
        body: Vec<Stmt>,
    },
    FunctionDef {
        decorators: Vec<Expr>,
        name: String,
        params: Params,
        body: Vec<Stmt>,
    },
    Try {
        body: Vec<Stmt>,
        handlers: Vec<Handler>,
        orelse: Vec<Stmt>,
        finalbody: Vec<Stmt>,
    },
}

/// `import a.b as c`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    pub name: String,
    pub asname: Option<String>,
}

/// `with open(p) as f`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithItem {
    pub context: Expr,
    pub var: Option<Expr>,
}

/// `except (A, B) as e:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handler {
    pub kind: Option<Expr>,
    pub name: Option<String>,
    pub body: Vec<Stmt>,
}

/// A function's parameters. Positional-only parameters (`/`) are refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params {
    pub args: Vec<Param>,
    pub vararg: Option<String>,
    /// After `*` or `*args`.
    pub kwonly: Vec<Param>,
    pub kwarg: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Name(String),
    /// A number, as written: CPython's value of it is the oracle's to compute.
    Number(String),
    /// `True`, `False`, `None`, `...`.
    Singleton(Singleton),
    /// A string's value, quoting and escapes resolved.
    Str(String),
    /// A `b'…'` literal's value.
    Bytes(Vec<u8>),
    /// An f-string, adjacent literals merged into it.
    FString(Vec<FPart>),
    Attribute {
        value: Box<Expr>,
        attr: String,
    },
    Call {
        func: Box<Expr>,
        args: Vec<Arg>,
    },
    Subscript {
        value: Box<Expr>,
        index: Box<Expr>,
    },
    /// `a:b:c`, only ever an index or an element of one.
    Slice {
        lower: Option<Box<Expr>>,
        upper: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
    },
    BinOp {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    /// `a and b and c`, flattened as CPython flattens it.
    BoolOp {
        op: BoolOp,
        values: Vec<Expr>,
    },
    /// `a < b <= c`: one comparison, not two.
    Compare {
        left: Box<Expr>,
        rest: Vec<(CmpOp, Expr)>,
    },
    IfExp {
        test: Box<Expr>,
        body: Box<Expr>,
        orelse: Box<Expr>,
    },
    Lambda {
        params: Params,
        body: Box<Expr>,
    },
    /// `name := value`.
    NamedExpr {
        target: String,
        value: Box<Expr>,
    },
    Tuple(Vec<Expr>),
    List(Vec<Expr>),
    Set(Vec<Expr>),
    /// `None` as a key is `**mapping`.
    Dict(Vec<(Option<Expr>, Expr)>),
    Starred(Box<Expr>),
    ListComp {
        elt: Box<Expr>,
        generators: Vec<Comprehension>,
    },
    SetComp {
        elt: Box<Expr>,
        generators: Vec<Comprehension>,
    },
    GeneratorExp {
        elt: Box<Expr>,
        generators: Vec<Comprehension>,
    },
    DictComp {
        key: Box<Expr>,
        value: Box<Expr>,
        generators: Vec<Comprehension>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Singleton {
    True,
    False,
    None,
    Ellipsis,
}

/// One part of an f-string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FPart {
    Text(String),
    /// `{value!r:spec}`. `{x=}` is refused: CPython expands it into text and a
    /// field, which would make one written form two trees.
    Field {
        value: Box<Expr>,
        conversion: Option<char>,
        spec: Option<Vec<FPart>>,
    },
}

/// One argument of a call, in the order written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    Positional(Expr),
    /// `*args`.
    Starred(Expr),
    /// `name=value`.
    Keyword(String, Expr),
    /// `**kwargs`.
    DoubleStarred(Expr),
}

/// `for target in iter if cond …` inside a comprehension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comprehension {
    pub target: Expr,
    pub iter: Expr,
    pub ifs: Vec<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mult,
    MatMult,
    Div,
    FloorDiv,
    Mod,
    Pow,
    LShift,
    RShift,
    BitOr,
    BitXor,
    BitAnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Invert,
    UAdd,
    USub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
    Is,
    IsNot,
    In,
    NotIn,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mult => "*",
            BinOp::MatMult => "@",
            BinOp::Div => "/",
            BinOp::FloorDiv => "//",
            BinOp::Mod => "%",
            BinOp::Pow => "**",
            BinOp::LShift => "<<",
            BinOp::RShift => ">>",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::BitAnd => "&",
        }
    }
}

impl CmpOp {
    pub fn symbol(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::NotEq => "!=",
            CmpOp::Lt => "<",
            CmpOp::LtE => "<=",
            CmpOp::Gt => ">",
            CmpOp::GtE => ">=",
            CmpOp::Is => "is",
            CmpOp::IsNot => "is not",
            CmpOp::In => "in",
            CmpOp::NotIn => "not in",
        }
    }
}
