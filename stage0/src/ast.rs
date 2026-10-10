// Abstract syntax tree.

use crate::diag::Pos;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    U64,
    Bool,
    Unit,
    /// An owned buffer of u64. Linear: must be consumed exactly once.
    Buf,
    /// An owned buffer of bytes. Linear.
    Bytes,
    /// The host operating system: files and command-line arguments. Linear;
    /// hosted programs only, and only `main` receives it.
    Host,
    /// The console capability. Linear: whoever owns it may talk to the
    /// serial line, and nobody else can. Only `main` receives one.
    Serial,
    /// An immutable string literal. Freely copyable.
    Str,
    /// One physical 4 KiB page, owned. Linear. Kernel and hosted targets.
    Frame,
    /// A page-table page (levels 1-4 are x86-64 tables; level 5 is a kernel-side
    /// directory of address spaces that the MMU never walks). It owns every
    /// table and frame it points to. Linear.
    PageTable,
    /// The physical page pool: the right to turn page number i into a Frame,
    /// once, while the pool still has it. Linear; only `main` receives it.
    Pool,
    /// The MMU: the right to load CR3. Linear; kernel target only.
    Mmu,
    /// The CPU's user-mode context: the right to enter user mode and the
    /// registers saved by the last trap. Linear; kernel target only.
    Cpu,
    /// A user-defined struct. Linear when any field is linear; otherwise a
    /// plain value that is copied.
    Struct(String),
    /// A read-only borrow of a linear value. Only allowed as a parameter type.
    Ref(Box<Type>),
    /// A mutable borrow of a linear value. Only allowed as a parameter type;
    /// the callee must hand the value back (still owned) when it returns.
    MutRef(Box<Type>),
}

impl Type {
    pub fn name(&self) -> String {
        match self {
            Type::U64 => "u64".into(),
            Type::Bool => "bool".into(),
            Type::Unit => "()".into(),
            Type::Buf => "Buf".into(),
            Type::Bytes => "Bytes".into(),
            Type::Host => "Host".into(),
            Type::Serial => "Serial".into(),
            Type::Str => "Str".into(),
            Type::Frame => "Frame".into(),
            Type::PageTable => "PageTable".into(),
            Type::Pool => "Pool".into(),
            Type::Mmu => "Mmu".into(),
            Type::Cpu => "Cpu".into(),
            Type::Struct(n) => n.clone(),
            Type::Ref(t) => format!("&{}", t.name()),
            Type::MutRef(t) => format!("&mut {}", t.name()),
        }
    }
    pub fn is_linear(&self) -> bool {
        match self {
            Type::Buf | Type::Bytes | Type::Host | Type::Serial | Type::Frame | Type::PageTable | Type::Pool | Type::Mmu | Type::Cpu => true,
            Type::Struct(n) => struct_def(n).map_or(false, |d| d.fields.iter().any(|f| f.1.is_linear())),
            _ => false,
        }
    }
    pub fn is_ref(&self) -> bool {
        matches!(self, Type::Ref(_) | Type::MutRef(_))
    }
    pub fn is_mut_ref(&self) -> bool {
        matches!(self, Type::MutRef(_))
    }
    /// The type with any borrow removed.
    pub fn base(&self) -> &Type {
        match self {
            Type::Ref(t) | Type::MutRef(t) => t,
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
    /// `include_bytes("path")`: a file's bytes as a Str. The prover only knows
    /// its length and that every byte is <= 255, not the bytes themselves.
    Blob(Vec<u8>),
    /// `&mut x` or `&mut x.f.g`, only as a call argument. Holds the place as
    /// a dotted path.
    MutBorrow(String),
    /// `e.f`. In code, `e` must be a place (a variable or a field of one).
    Field(Box<Expr>, String),
    /// `Name { f: e, .. }`
    StructLit(String, Vec<(String, Expr)>),
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
    /// `x = e;` or `x.f.g = e;`; `name` is the dotted path.
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

/// `spec fn name(params) -> bool { expr }`: a named specification, expanded
/// in place wherever a specification calls it. Ghost: generates no code.
#[derive(Debug, Clone)]
pub struct SpecFn {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Type,
    pub body: Expr,
    pub pos: Pos,
}

/// `struct Name { field: Type, .. }`
#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<(String, Type)>,
    pub pos: Pos,
}

impl StructDef {
    pub fn field(&self, f: &str) -> Option<&Type> {
        self.fields.iter().find(|x| x.0 == f).map(|x| &x.1)
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    pub fns: Vec<FnDef>,
    pub specs: Vec<SpecFn>,
    pub structs: Vec<StructDef>,
}

/// The program's struct definitions, registered once after parsing so that
/// `Type::is_linear` and the later passes can look them up.
static STRUCTS: std::sync::OnceLock<Vec<StructDef>> = std::sync::OnceLock::new();

pub fn register_structs(defs: Vec<StructDef>) {
    let _ = STRUCTS.set(defs);
}

pub fn struct_def(name: &str) -> Option<&'static StructDef> {
    STRUCTS.get()?.iter().find(|d| d.name == name)
}

/// The dotted path of a place expression (`x`, `x.f`, `x.f.g`).
pub fn place(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Var(n) => Some(n.clone()),
        ExprKind::Field(b, f) => place(b).map(|p| format!("{}.{}", p, f)),
        _ => None,
    }
}

/// The variable a dotted path starts from.
pub fn path_root(path: &str) -> &str {
    path.split('.').next().unwrap()
}

/// Source-like rendering of an expression, used in diagnostics.
pub fn render(e: &Expr) -> String {
    fn go(e: &Expr, top: bool) -> String {
        match &e.kind {
            ExprKind::Int(v) => v.to_string(),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(b) => format!("{:?}", String::from_utf8_lossy(b)),
            ExprKind::Blob(b) => format!("include_bytes(<{} bytes>)", b.len()),
            ExprKind::MutBorrow(n) => format!("&mut {}", n),
            ExprKind::Var(n) => n.clone(),
            ExprKind::Field(b, f) => format!("{}.{}", go(b, false), f),
            ExprKind::StructLit(n, fs) => {
                format!("{} {{ {} }}", n, fs.iter().map(|(f, x)| format!("{}: {}", f, go(x, true))).collect::<Vec<_>>().join(", "))
            }
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
