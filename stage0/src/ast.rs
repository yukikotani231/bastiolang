// Abstract syntax tree.

use crate::diag::Pos;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    U64,
    Bool,
    Unit,
    /// An owned buffer of u64. Linear: must be consumed exactly once.
    Buf,
    /// The console capability. Linear: whoever owns it may talk to the
    /// serial line, and nobody else can. Only `main` receives one.
    Serial,
    /// An immutable string literal. Freely copyable.
    Str,
    /// A read-only borrow of a linear value. Only allowed as a parameter type.
    Ref(Box<Type>),
}

impl Type {
    pub fn name(&self) -> String {
        match self {
            Type::U64 => "u64".into(),
            Type::Bool => "bool".into(),
            Type::Unit => "()".into(),
            Type::Buf => "Buf".into(),
            Type::Serial => "Serial".into(),
            Type::Str => "Str".into(),
            Type::Ref(t) => format!("&{}", t.name()),
        }
    }
    pub fn is_linear(&self) -> bool {
        matches!(self, Type::Buf | Type::Serial)
    }
    pub fn is_ref(&self) -> bool {
        matches!(self, Type::Ref(_))
    }
    /// The type with any borrow removed.
    pub fn base(&self) -> &Type {
        match self {
            Type::Ref(t) => t,
            t => t,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Implies,
}

impl BinOp {
    pub fn text(&self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::Implies => "==>",
        }
    }
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(u128),
    Bool(bool),
    /// A string literal (already unescaped).
    Str(Vec<u8>),
    Var(String),
    Not(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    /// `forall x: u64 :: body` (specifications only)
    Forall(String, Box<Expr>),
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Let { name: String, mutable: bool, ty: Option<Type>, init: Expr },
    Assign { name: String, value: Expr },
    If { cond: Expr, then_b: Vec<Stmt>, else_b: Vec<Stmt> },
    While { cond: Expr, invariants: Vec<Expr>, body: Vec<Stmt> },
    Return(Option<Expr>),
    Assert(Expr),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub mutable: bool,
    pub ty: Type,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Type,
    pub requires: Vec<Expr>,
    pub ensures: Vec<Expr>,
    pub body: Vec<Stmt>,
    pub pos: Pos,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub fns: Vec<FnDef>,
}

/// Source-like rendering of an expression, used in diagnostics.
pub fn render(e: &Expr) -> String {
    fn go(e: &Expr, top: bool) -> String {
        match &e.kind {
            ExprKind::Int(v) => v.to_string(),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(b) => format!("{:?}", String::from_utf8_lossy(b)),
            ExprKind::Var(n) => n.clone(),
            ExprKind::Not(x) => format!("!{}", go(x, false)),
            ExprKind::Bin(op, a, b) => {
                let s = format!("{} {} {}", go(a, false), op.text(), go(b, false));
                if top { s } else { format!("({})", s) }
            }
            ExprKind::Call(n, args) => {
                format!("{}({})", n, args.iter().map(|a| go(a, true)).collect::<Vec<_>>().join(", "))
            }
            ExprKind::Forall(v, b) => {
                let s = format!("forall {}: u64 :: {}", v, go(b, true));
                if top { s } else { format!("({})", s) }
            }
        }
    }
    go(e, true)
}
