// Abstract syntax tree.

use crate::diag::Pos;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    U64,
    /// Narrower unsigned integers (docs/integers.md). Like `u64`, arithmetic
    /// on them is proved to stay in range.
    U8,
    U16,
    U32,
    /// Signed integers, two's complement. Arithmetic is proved to stay
    /// between the type's minimum and maximum.
    I8,
    I16,
    I32,
    I64,
    Bool,
    Unit,
    /// An owned buffer of u64. Linear: must be consumed exactly once.
    Buf,
    /// An owned buffer of bytes. Linear.
    Bytes,
    /// The host operating system: files and command-line arguments. Linear;
    /// hosted programs only, and only `main` receives it.
    Host,
    /// A child process started by `host_spawn`, with pipes to its standard
    /// input and output. Linear: it must be waited for. Hosted only.
    Child,
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
    /// The I/O port space: the right to claim ranges of the 65536 x86 I/O
    /// ports, each at most once. Linear; only `main` receives it. Kernel and
    /// hosted targets (hosted simulates the ports).
    Ports,
    /// A claimed range of I/O ports, owned: the only way to read or write
    /// them. Linear; given back with `port_release`.
    PortIo,
    /// The device window (physical 0xC000_0000 to 4 GiB, mapped uncached by
    /// the platform): the right to claim each byte of it once. Linear; only
    /// `main` receives it. Kernel and hosted targets (hosted simulates it).
    MmioSpace,
    /// A claimed range of memory-mapped device registers, owned: the only
    /// way to read or write them. Linear; given back with `mmio_release`.
    Mmio,
    /// A page lent to devices for DMA: made from a `Frame` with `dma_lend`,
    /// turned back with `dma_take`. While lent it cannot be freed, mapped or
    /// reused, and its physical address (`dma_addr`) can be read. Linear.
    Dma,
    /// A user-defined struct. Linear when any field is linear; otherwise a
    /// plain value that is copied.
    Struct(String),
    /// A user-defined enum (a tagged union). Linear when any variant's
    /// payload is linear.
    Enum(String),
    /// An owning pointer to a heap value. Always linear (it owns the
    /// allocation), and the only way a type may refer to itself.
    Box(Box<Type>),
    /// A growable array of `T` on the heap. Always linear. Every slot below
    /// its length holds a value, so there is no per-slot ownership to track.
    Vec(Box<Type>),
    /// A lock holding a `T`, shared between threads or cores through `&`
    /// borrows. Linear; its contents are reached only inside `locked`.
    Lock(Box<Type>),
    /// A one-word value (an integer, a bool, or a struct with one such
    /// field) read and written atomically through `&` borrows. Linear.
    Atomic(Box<Type>),
    /// `Name[T, ..]` as written: a generic struct or enum applied to type
    /// arguments. Only exists before monomorphization, which replaces it with
    /// the `Struct`/`Enum` instance named by `name()` (e.g. `Option[u64]`).
    Generic(String, Vec<Type>),
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
            Type::U8 => "u8".into(),
            Type::U16 => "u16".into(),
            Type::U32 => "u32".into(),
            Type::I8 => "i8".into(),
            Type::I16 => "i16".into(),
            Type::I32 => "i32".into(),
            Type::I64 => "i64".into(),
            Type::Bool => "bool".into(),
            Type::Unit => "()".into(),
            Type::Buf => "Buf".into(),
            Type::Bytes => "Bytes".into(),
            Type::Host => "Host".into(),
            Type::Child => "Child".into(),
            Type::Serial => "Serial".into(),
            Type::Str => "Str".into(),
            Type::Frame => "Frame".into(),
            Type::PageTable => "PageTable".into(),
            Type::Pool => "Pool".into(),
            Type::Mmu => "Mmu".into(),
            Type::Cpu => "Cpu".into(),
            Type::Ports => "Ports".into(),
            Type::PortIo => "PortIo".into(),
            Type::MmioSpace => "MmioSpace".into(),
            Type::Mmio => "Mmio".into(),
            Type::Dma => "Dma".into(),
            Type::Struct(n) | Type::Enum(n) => n.clone(),
            Type::Box(t) => format!("Box[{}]", t.name()),
            Type::Vec(t) => format!("Vec[{}]", t.name()),
            Type::Lock(t) => format!("Lock[{}]", t.name()),
            Type::Atomic(t) => format!("Atomic[{}]", t.name()),
            Type::Generic(n, args) => format!("{}[{}]", n, args.iter().map(|a| a.name()).collect::<Vec<_>>().join(", ")),
            Type::Ref(t) => format!("&{}", t.name()),
            Type::MutRef(t) => format!("&mut {}", t.name()),
        }
    }
    pub fn is_linear(&self) -> bool {
        match self {
            Type::Buf | Type::Bytes | Type::Host | Type::Child | Type::Serial | Type::Frame | Type::PageTable | Type::Pool | Type::Mmu | Type::Cpu | Type::Ports | Type::PortIo | Type::MmioSpace | Type::Mmio | Type::Dma | Type::Box(_) | Type::Vec(_) | Type::Lock(_) | Type::Atomic(_) => true,
            Type::Struct(n) => struct_def(n).map_or(false, |d| d.fields.iter().any(|f| f.1.is_linear())),
            Type::Enum(n) => enum_def(n).map_or(false, |d| d.variants.iter().any(|v| v.1.as_ref().map_or(false, |t| t.is_linear()))),
            _ => false,
        }
    }
    /// The width in bits of an integer type.
    pub fn int_bits(&self) -> Option<u32> {
        match self {
            Type::U8 => Some(8),
            Type::U16 => Some(16),
            Type::U32 => Some(32),
            Type::U64 => Some(64),
            Type::I8 => Some(8),
            Type::I16 => Some(16),
            Type::I32 => Some(32),
            Type::I64 => Some(64),
            _ => None,
        }
    }
    pub fn is_int(&self) -> bool {
        self.int_bits().is_some()
    }
    pub fn is_signed(&self) -> bool {
        matches!(self, Type::I8 | Type::I16 | Type::I32 | Type::I64)
    }
    /// The largest value of an integer type.
    pub fn int_max(&self) -> i128 {
        let w = self.int_bits().expect("an integer type");
        if self.is_signed() { (1i128 << (w - 1)) - 1 } else { (1i128 << w) - 1 }
    }
    /// The smallest value of an integer type.
    pub fn int_min(&self) -> i128 {
        let w = self.int_bits().expect("an integer type");
        if self.is_signed() { -(1i128 << (w - 1)) } else { 0 }
    }
    /// 2^width: the modulus of wrapping arithmetic.
    pub fn int_modulus(&self) -> i128 {
        1i128 << self.int_bits().expect("an integer type")
    }
    pub fn is_ref(&self) -> bool {
        matches!(self, Type::Ref(_) | Type::MutRef(_))
    }
    pub fn is_mut_ref(&self) -> bool {
        matches!(self, Type::MutRef(_))
    }
    /// Pinned: describes the core or thread that holds it (`Cpu`, `Mmu`), so
    /// it may never reach another one (through `lock_new` or `smp_run`).
    pub fn is_pinned(&self) -> bool {
        match self {
            Type::Mmu | Type::Cpu => true,
            Type::Box(t) | Type::Vec(t) | Type::Lock(t) | Type::Atomic(t) | Type::Ref(t) | Type::MutRef(t) => t.is_pinned(),
            Type::Struct(n) => struct_def(n).map_or(false, |d| d.fields.iter().any(|f| f.1.is_pinned())),
            Type::Enum(n) => enum_def(n).map_or(false, |d| d.variants.iter().any(|v| v.1.as_ref().map_or(false, |t| t.is_pinned()))),
            _ => false,
        }
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
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
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
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
        }
    }

    /// `&`, `|`, `^`, `<<`, `>>`: on unsigned integers only, never in
    /// specifications.
    pub fn is_bitwise(&self) -> bool {
        matches!(self, BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr)
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
    /// `Enum::Variant` or `Enum::Variant(e)`
    Variant(String, String, Option<Box<Expr>>),
    /// `e is Enum::Variant`: does not consume `e`.
    Is(Box<Expr>, String, String),
    Var(String),
    Not(Box<Expr>),
    /// Unary minus, on signed integers only. In front of a literal it is part
    /// of the literal (`-128` is an `i8`).
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    /// `forall x: u64 :: body` (specifications only)
    Forall(String, Box<Expr>),
    /// `name[T, ..]` applied to a call, struct literal or variant: removed by
    /// monomorphization, which renames the inner expression to the instance.
    TypeApp(Vec<Type>, Box<Expr>),
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
    /// `unreachable;`: a point the prover shows no execution reaches. Ends
    /// the path like `return`, with nothing left to consume.
    Unreachable,
    Assert(Expr),
    Expr(Expr),
    /// `match e { Pat => { .. } .. }`
    Match { scrut: Expr, arms: Vec<Arm> },
}

#[derive(Debug, Clone)]
pub enum Pat {
    /// `Enum::Variant`, or `Enum::Variant(x)` / `(mut x)` / `(_)` for a variant
    /// with a payload: (enum, variant, binding, binding is mutable).
    Variant(String, String, Option<String>, bool),
    /// `_`: every variant not named by an earlier arm.
    Wild,
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub pat: Pat,
    pub body: Vec<Stmt>,
    pub pos: Pos,
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
    /// Type parameters (`fn f[T]`); empty after monomorphization.
    pub tparams: Vec<String>,
    pub params: Vec<Param>,
    pub ret: Type,
    pub requires: Vec<Expr>,
    pub ensures: Vec<Expr>,
    pub body: Vec<Stmt>,
    pub pos: Pos,
    /// Filled in by the checker: the integer type of the operands of each
    /// arithmetic operator, comparison, unary minus and wrapping builtin in
    /// the body, by its position. The
    /// verifier and the code generator need it, since literals take their
    /// type from context.
    pub int_ops: std::collections::HashMap<Pos, Type>,
    /// `export fn`: also a C function of the same name, callable from C.
    pub export: bool,
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
    pub tparams: Vec<String>,
    pub fields: Vec<(String, Type)>,
    /// `invariant e` clauses after the fields: facts about the fields that
    /// hold whenever a value is inside a `Lock` (ghost).
    pub invariants: Vec<Expr>,
    pub pos: Pos,
}

impl StructDef {
    pub fn field(&self, f: &str) -> Option<&Type> {
        self.fields.iter().find(|x| x.0 == f).map(|x| &x.1)
    }
}

/// `enum Name { A, B(Type), .. }`
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub tparams: Vec<String>,
    pub variants: Vec<(String, Option<Type>)>,
    pub pos: Pos,
}

impl EnumDef {
    pub fn index(&self, v: &str) -> Option<usize> {
        self.variants.iter().position(|x| x.0 == v)
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    pub fns: Vec<FnDef>,
    pub specs: Vec<SpecFn>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
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

static ENUMS: std::sync::OnceLock<Vec<EnumDef>> = std::sync::OnceLock::new();

pub fn register_enums(defs: Vec<EnumDef>) {
    let _ = ENUMS.set(defs);
}

pub fn enum_def(name: &str) -> Option<&'static EnumDef> {
    ENUMS.get()?.iter().find(|d| d.name == name)
}

/// The parser reads every unknown type name as a struct; turn the names of
/// enums into `Type::Enum` everywhere a type is written.
pub fn resolve_enum_types(p: &mut Program) {
    let names: Vec<String> = p.enums.iter().map(|e| e.name.clone()).collect();
    fn fix(t: &mut Type, names: &[String]) {
        match t {
            Type::Struct(n) if names.contains(n) => *t = Type::Enum(n.clone()),
            Type::Ref(i) | Type::MutRef(i) | Type::Box(i) | Type::Vec(i) => fix(i, names),
            Type::Generic(_, args) => args.iter_mut().for_each(|a| fix(a, names)),
            _ => {}
        }
    }
    // Type arguments that survive monomorphization (`vec_new[T]()`).
    fn fix_expr(e: &mut Expr, names: &[String]) {
        match &mut e.kind {
            ExprKind::TypeApp(ts, inner) => {
                ts.iter_mut().for_each(|t| fix(t, names));
                fix_expr(inner, names);
            }
            ExprKind::Field(x, _) | ExprKind::Is(x, _, _) | ExprKind::Not(x) | ExprKind::Neg(x) | ExprKind::Forall(_, x) => fix_expr(x, names),
            ExprKind::Variant(_, _, Some(x)) => fix_expr(x, names),
            ExprKind::StructLit(_, fs) => fs.iter_mut().for_each(|(_, x)| fix_expr(x, names)),
            ExprKind::Bin(_, a, b) => {
                fix_expr(a, names);
                fix_expr(b, names);
            }
            ExprKind::Call(_, xs) => xs.iter_mut().for_each(|x| fix_expr(x, names)),
            _ => {}
        }
    }
    fn fix_stmts(ss: &mut [Stmt], names: &[String]) {
        for s in ss {
            match &mut s.kind {
                StmtKind::Let { ty, init, .. } => {
                    if let Some(t) = ty {
                        fix(t, names);
                    }
                    fix_expr(init, names);
                }
                StmtKind::Assign { value: x, .. } | StmtKind::Assert(x) | StmtKind::Expr(x) | StmtKind::Return(Some(x)) => fix_expr(x, names),
                StmtKind::If { cond, then_b, else_b } => {
                    fix_expr(cond, names);
                    fix_stmts(then_b, names);
                    fix_stmts(else_b, names);
                }
                StmtKind::While { cond, body, .. } => {
                    fix_expr(cond, names);
                    fix_stmts(body, names);
                }
                StmtKind::Match { scrut, arms } => {
                    fix_expr(scrut, names);
                    for a in arms {
                        fix_stmts(&mut a.body, names);
                    }
                }
                StmtKind::Return(None) | StmtKind::Unreachable => {}
            }
        }
    }
    for f in &mut p.fns {
        for prm in &mut f.params {
            fix(&mut prm.ty, &names);
        }
        fix(&mut f.ret, &names);
        fix_stmts(&mut f.body, &names);
    }
    for sf in &mut p.specs {
        for prm in &mut sf.params {
            fix(&mut prm.ty, &names);
        }
    }
    for d in &mut p.structs {
        for (_, t) in &mut d.fields {
            fix(t, &names);
        }
    }
    for e in &mut p.enums {
        for (_, t) in &mut e.variants {
            if let Some(t) = t {
                fix(t, &names);
            }
        }
    }
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
            ExprKind::Variant(en, v, None) => format!("{}::{}", en, v),
            ExprKind::Variant(en, v, Some(x)) => format!("{}::{}({})", en, v, go(x, true)),
            ExprKind::Is(x, en, v) => {
                let s = format!("{} is {}::{}", go(x, false), en, v);
                if top { s } else { format!("({})", s) }
            }
            ExprKind::TypeApp(ts, x) => {
                // Shown as written: `f[T](..)`; the inner name is still generic.
                let inner = go(x, top);
                let n = match &x.kind {
                    ExprKind::Call(n, _) | ExprKind::StructLit(n, _) | ExprKind::Variant(n, _, _) => n.len(),
                    _ => 0,
                };
                format!("{}[{}]{}", &inner[..n], ts.iter().map(|t| t.name()).collect::<Vec<_>>().join(", "), &inner[n..])
            }
            ExprKind::Not(x) => format!("!{}", go(x, false)),
            ExprKind::Neg(x) => format!("-{}", go(x, false)),
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
