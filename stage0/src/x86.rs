// Native x86-64 backend (experimental, `--backend native`, hosted target
// only): Bastio straight to GNU assembler text, without generating C.
//
// Why: the C backend puts the C compiler in the trusted base (ROADMAP v3,
// "native x86-64 backend"). This is a prototype in stage0 so that the gap to
// the C backend can be measured early (ROADMAP invariant 5) and so the
// self-hosted compiler has a reference to test its own native backend
// against. It is deliberately simple, so it ports to Bastio line by line:
// one pass over the checked syntax tree, no intermediate representation.
//
// Integers narrower than 64 bits are kept zero-extended (unsigned) or
// sign-extended (signed) to a full word. Arithmetic is proved to stay in
// range, so plain 64-bit instructions keep that form; only `wrap` and the
// `wrap_*` builtins truncate.
//
// Zero overhead holds here as in the C backend: specifications, asserts and
// capability tokens' ghost state generate nothing, and there are no bounds
// or overflow checks (the verifier proved them unnecessary).
//
// Data layout: every value is a sequence of 8-byte words.
//   integers, bool, capability tokens, Box (a pointer), &mut T (a pointer): 1 word
//   Buf, Bytes, Str: { len, data }       Vec[T]: { len, cap, data }
//   Frame, PageTable: a page's address   Pool: { base, size }
//   struct: its fields in order          enum: a tag word, then the payload
//   ()  : no words                       &T  : a copy of the T
//
// Calling convention (internal; only `main` follows the System V ABI): the
// caller reserves room for a result of more than one word, then pushes the
// arguments left to right (each argument's word 0 at its lowest address),
// calls, and pops the arguments, which leaves a multi-word result on top of
// the stack. A one-word result comes back in %rax. Small functions are
// mostly not called but inlined (`Gen::inline`). Variables live in
// registers when `assign_regs` gives them some (%rbx, %r12-%r15, and
// %r8-%r11, which are saved around calls); everything else lives in the
// frame. Expression code works like a stack machine with %rax
// as the accumulator: `scalar` leaves a one-word value in %rax, `push_val`
// pushes a value of any size. %rcx holds the address of a place being read
// or written; %rdx, %rsi and %rdi are scratch.
//
// The runtime (`RUNTIME` below) is assembly over the C library: printing,
// the console, and the allocator. It is part of the trusted base, listed by
// `--trusted-base` like the C preludes.

use crate::ast::*;
use crate::check::Sig;
use std::collections::{HashMap, HashSet};

/// Size of a value of type `t`, in 8-byte words.
fn words(t: &Type) -> Result<usize, String> {
    Ok(match t {
        Type::Unit => 0,
        Type::U64 | Type::U8 | Type::U16 | Type::U32 | Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Bool | Type::Serial | Type::Host => 1,
        Type::Frame | Type::PageTable | Type::Box(_) | Type::MutRef(_) => 1,
        Type::Buf | Type::Bytes | Type::Str | Type::Pool => 2,
        Type::Vec(_) => 3,
        Type::Ref(t) => words(t)?,
        Type::Struct(n) => {
            let d = struct_def(n).ok_or_else(|| format!("unknown struct {}", n))?;
            let mut s = 0;
            for f in &d.fields {
                s += words(&f.1)?;
            }
            s
        }
        Type::Enum(n) => {
            let d = enum_def(n).ok_or_else(|| format!("unknown enum {}", n))?;
            let mut m = 0;
            for v in &d.variants {
                if let Some(t) = &v.1 {
                    m = m.max(words(t)?);
                }
            }
            1 + m
        }
        t => return Err(format!("the native backend does not support {} yet", t.name())),
    })
}

/// Word offset and type of field `f` of struct `s`.
fn field(s: &Type, f: &str) -> (usize, Type) {
    let d = struct_def(s.name().as_str()).expect("checked: a struct");
    let mut off = 0;
    for (n, t) in &d.fields {
        if n == f {
            return (off, t.clone());
        }
        off += words(t).expect("sized when the struct was");
    }
    unreachable!("checked: a field")
}

/// An assembler symbol for a Bastio function (generic instances such as
/// `last_or[u64]` contain brackets).
fn sym(n: &str) -> String {
    let mut s = String::from("f_");
    for c in n.chars() {
        match c {
            c if c.is_ascii_alphanumeric() || c == '_' => s.push(c),
            '[' => s.push_str("_L"),
            ']' => s.push_str("_R"),
            ',' => s.push_str("_C"),
            ' ' => {}
            c => s.push_str(&format!("_{:x}", c as u32)),
        }
    }
    s
}

/// Where a variable lives.
#[derive(Clone, Copy, Debug)]
enum Store {
    /// In the frame, word 0 at this offset from %rbp.
    Slot(i64),
    /// A `&mut` parameter: the frame slot at this offset holds its address.
    Ptr(i64),
    /// A scalar kept in a callee-saved register.
    Reg(&'static str),
    /// A buffer or vector value that never changes, one register per word
    /// ("" for a word that is never read).
    Regs([&'static str; 3]),
}

#[derive(Clone, Debug)]
struct Var {
    st: Store,
    ty: Type,
}

/// Where `val_into` puts a value.
#[derive(Clone, Copy)]
enum Dst {
    /// The frame, word 0 at this offset from %rbp.
    Frame(i64),
    /// The stack area reserved when the depth was `.0`, from its word `.1`.
    Stack(i64, usize),
    /// The memory a pointer points to, from its word `.1`; the pointer is
    /// the stack word pushed when the depth became `.0`.
    Heap(i64, usize),
}

impl Dst {
    fn at(self, k: usize) -> Dst {
        match self {
            Dst::Frame(o) => Dst::Frame(o + 8 * k as i64),
            Dst::Stack(m, o) => Dst::Stack(m, o + k),
            Dst::Heap(m, o) => Dst::Heap(m, o + k),
        }
    }
}

/// A place being read or written: memory at an offset from a base register
/// (%rbp, or %rcx after following pointers), or a register.
#[derive(Clone, Copy)]
enum Loc {
    Mem(&'static str, i64),
    Reg(&'static str),
    /// One register per word.
    Regs([&'static str; 3]),
}

impl Loc {
    /// The operand for word `i` of the place.
    fn op(&self, i: usize) -> String {
        match self {
            Loc::Mem(b, o) => format!("{}({})", o + 8 * i as i64, b),
            Loc::Reg(r) => {
                assert_eq!(i, 0);
                r.to_string()
            }
            Loc::Regs(rs) => {
                assert!(!rs[i].is_empty(), "a word read is in a register");
                rs[i].to_string()
            }
        }
    }
}

/// The length builtins: word 0 of their argument.
const LENS: &[&str] = &["len", "blen", "slen", "vec_len"];

/// The size suffix an instruction needs for operand `o` (a memory operand
/// with an immediate or no other register would be ambiguous).
fn q(o: &str) -> &'static str {
    if o.starts_with('%') { "" } else { "q" }
}

const SAVED: [&str; 5] = ["%rbx", "%r12", "%r13", "%r14", "%r15"];

/// Caller-saved registers that only variables of leaf functions use.
const LEAF: [&str; 4] = ["%r8", "%r9", "%r10", "%r11"];

/// Builtins generated inline, with no call to the runtime.
const INLINE_BUILTINS: &[&str] = &[
    "len", "get", "set", "blen", "bget", "bset", "slen", "sget", "vec_len", "vec_get", "vec_set", "vec_swap", "vec_pop", "wrap_add", "wrap_sub", "wrap_mul",
    "cast", "wrap", "vec_new", "pool_size", "pool_take", "pool_give", "pt_new", "pt_free", "frame_id", "frame_read", "frame_read8", "frame_write", "frame_write8",
    "pt_has", "pt_writable", "pt_put", "pt_put_frame", "pt_take", "pt_take_frame", "bcopy", "bmove", "frame_to_bytes", "bytes_to_frame",
];

/// How deep inlining goes: the function being generated, then up to two
/// levels of inlined bodies; deeper, only the tiniest functions.
const INLINE_DEPTH: usize = 3;
const INLINE_TINY: usize = 10;

/// The body to inline for a call to `name`, given the functions being
/// generated or inlined at this point (outermost first), if it is inlined:
/// never into itself, and not too deep.
fn will_inline<'i>(inl: &'i HashMap<String, FnDef>, name: &str, stack: &[String]) -> Option<&'i FnDef> {
    inl.get(name).filter(|f| (stack.len() < INLINE_DEPTH || body_size(f) <= INLINE_TINY) && !stack.iter().any(|s| s == name))
}

/// Does code `ss` call nothing: no function that is not inlined, and no
/// builtin that calls the runtime? `stack` as for `will_inline`.
fn is_leaf(ss: &[Stmt], inl: &HashMap<String, FnDef>, stack: &mut Vec<String>) -> bool {
    fn expr(e: &Expr, inl: &HashMap<String, FnDef>, stack: &mut Vec<String>) -> bool {
        match &e.kind {
            ExprKind::Call(n, xs) => {
                let ok = INLINE_BUILTINS.contains(&n.as_str())
                    || match will_inline(inl, n, stack) {
                        Some(f) => {
                            stack.push(n.clone());
                            let l = is_leaf(&f.body, inl, stack);
                            stack.pop();
                            l
                        }
                        None => false,
                    };
                ok && xs.iter().all(|x| expr(x, inl, stack))
            }
            ExprKind::Field(x, _) | ExprKind::Is(x, _, _) | ExprKind::Not(x) | ExprKind::Neg(x) | ExprKind::TypeApp(_, x) => expr(x, inl, stack),
            ExprKind::Variant(_, _, Some(x)) => expr(x, inl, stack),
            ExprKind::StructLit(_, fs) => fs.iter().all(|(_, x)| expr(x, inl, stack)),
            ExprKind::Bin(_, a, b) => expr(a, inl, stack) && expr(b, inl, stack),
            _ => true,
        }
    }
    ss.iter().all(|s| match &s.kind {
        StmtKind::Let { init: x, .. } | StmtKind::Assign { value: x, .. } | StmtKind::Expr(x) | StmtKind::Return(Some(x)) => expr(x, inl, stack),
        StmtKind::If { cond, then_b, else_b } => expr(cond, inl, stack) && is_leaf(then_b, inl, stack) && is_leaf(else_b, inl, stack),
        StmtKind::While { cond, body, .. } => expr(cond, inl, stack) && is_leaf(body, inl, stack),
        StmtKind::Match { scrut, arms } => expr(scrut, inl, stack) && arms.iter().all(|a| is_leaf(&a.body, inl, stack)),
        _ => true,
    })
}

struct Gen<'a> {
    out: String,
    sigs: &'a HashMap<String, Sig>,
    /// The integer type of each operator, by position (`FnDef::int_ops`).
    int_ops: &'a HashMap<crate::diag::Pos, Type>,
    /// The functions that can be inlined, renamed (see `renamed`).
    inl: &'a HashMap<String, FnDef>,
    /// The function being generated, then the functions being inlined at
    /// this point of it (see `will_inline`).
    stack: Vec<String>,
    vars: HashMap<String, Var>,
    /// Frame words used so far by locals.
    frame: i64,
    /// Words pushed on the stack beyond the frame at this point of the code.
    depth: i64,
    labels: &'a mut usize,
    strings: &'a mut Vec<Vec<u8>>,
    ret_label: String,
    ret_words: usize,
    /// Offset from %rbp of the caller's room for a multi-word result.
    ret_off: i64,
    /// Where an inlined body puts a multi-word result.
    ret_dst: Option<Dst>,
    /// The variables kept in registers, one per word (see `assign_regs`).
    regs: HashMap<String, Vec<&'static str>>,
    /// Callee-saved registers this function uses (saved in its frame).
    used_regs: Vec<&'static str>,
    /// Where `emit` saves %r8-%r11 around calls.
    call_saves: Vec<(&'static str, i64)>,
    /// Variables whose address is taken (`&mut x`): never in a register.
    borrowed: HashSet<String>,
}

type R<T> = Result<T, String>;

/// `--backend native`: writes the assembly (`--emit-asm`), or assembles it
/// with `as` and links it against the C library with `cc` (used only as the
/// linker driver; no C is compiled).
pub fn build(p: &Program, target: crate::codegen::Target, emit_asm: bool, out: Option<String>) {
    use std::process::{exit, Command};
    if target != crate::codegen::Target::Hosted {
        eprintln!("bastioc0: the native backend supports only the hosted target so far");
        exit(2);
    }
    let asm = emit(p).unwrap_or_else(|m| {
        eprintln!("bastioc0: {}", m);
        exit(4)
    });
    let write = |path: &str, s: &str| {
        std::fs::write(path, s).unwrap_or_else(|e| {
            eprintln!("bastioc0: cannot write {}: {}", path, e);
            exit(2)
        })
    };
    if emit_asm {
        write(&out.unwrap_or_else(|| "out.s".into()), &asm);
        return;
    }
    let exe = out.unwrap_or_else(|| "a.out".into());
    let (sfile, ofile) = (format!("{}.s", exe), format!("{}.o", exe));
    write(&sfile, &asm);
    let run = |cmd: &mut Command, what: &str| match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(_) => {
            eprintln!("bastioc0: {} failed (this is a compiler bug)", what);
            exit(3);
        }
        Err(e) => {
            eprintln!("bastioc0: cannot run {}: {}", what, e);
            exit(3);
        }
    };
    run(Command::new("as").args(["--64", "-o", &ofile, &sfile]), "the assembler");
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    run(Command::new(&cc).args(["-o", &exe, &ofile]), "the linker");
    let _ = std::fs::remove_file(&sfile);
    let _ = std::fs::remove_file(&ofile);
}

pub fn emit(p: &Program) -> R<String> {
    let sigs = crate::check::signatures(p, crate::codegen::Target::Hosted).expect("checked");
    let mut labels = 0;
    let mut strings = Vec::new();
    // `export fn` only matters to C callers; a program is still built
    // around its `main` (a library without one is not supported yet).
    if !p.fns.iter().any(|f| f.name == "main") {
        return Err("the native backend does not support libraries (`export fn` without `main`) yet".into());
    }
    let inl: HashMap<String, FnDef> = p.fns.iter().filter(|f| inlinable(f)).map(|f| (f.name.clone(), renamed(f))).collect();
    let mut out = String::from("# generated by bastioc0 --backend native -- do not edit\n\t.text\n");
    for f in &p.fns {
        out.push_str(&function(f, &inl, &sigs, &mut labels, &mut strings)?);
    }
    out.push_str(&entry(p)?);
    out.push_str(RUNTIME);
    out.push_str("\t.section .rodata\n");
    for (i, s) in strings.iter().enumerate() {
        out.push_str(&format!(".Lstr{}:\n", i));
        for chunk in s.chunks(24) {
            out.push_str(&format!("\t.byte {}\n", chunk.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",")));
        }
    }
    out.push_str("\t.section .note.GNU-stack,\"\",@progbits\n");
    Ok(out)
}

/// `main` with the System V ABI: builds the capability tokens `main` asks
/// for, calls it, and returns its result as the exit status.
fn entry(p: &Program) -> R<String> {
    let main = p.fns.iter().find(|f| f.name == "main").expect("checked");
    let mut s = String::from("\t.globl main\nmain:\n\tpush %rbp\n\tmov %rsp, %rbp\n\tpush %rbx\n\tsub $72, %rsp\n");
    s.push_str("\tmov %edi, bst_rt_argc(%rip)\n\tmov %rsi, bst_rt_argv(%rip)\n");
    // Each capability lives in a frame slot below the saved %rbx; `main`
    // gets it by value or by address.
    let mut off = 8;
    let mut pool = None;
    for prm in &main.params {
        let t = prm.ty.base();
        let n = match t {
            Type::Serial | Type::Host => 1,
            Type::Pool => 2,
            t => return Err(format!("the native backend does not support a `main` parameter of type {} yet", t.name())),
        };
        off += 8 * n;
        if *t == Type::Pool {
            s.push_str(&format!("\tcall bst_rt_pool_init\n\tmov %rax, -{}(%rbp)\n\tmov %rdx, -{}(%rbp)\n", off, off - 8));
            pool = Some(off);
        } else {
            s.push_str(&format!("\tmovq $0, -{}(%rbp)\n", off));
        }
        if prm.ty.is_mut_ref() {
            s.push_str(&format!("\tlea -{}(%rbp), %rax\n\tpush %rax\n", off));
        } else {
            for i in (0..n).rev() {
                s.push_str(&format!("\tpush -{}(%rbp)\n", off - 8 * i));
            }
        }
    }
    if off > 64 {
        return Err("the native backend does not support so many `main` parameters yet".into());
    }
    s.push_str(&format!("\tcall {}\n", sym("main")));
    if main.ret != Type::U64 {
        s.push_str("\txor %eax, %eax\n");
    }
    if let Some(o) = pool {
        s.push_str(&format!("\tmov %rax, %rbx\n\tmov -{}(%rbp), %rdi\n\tcall bst_rt_free\n\tmov %rbx, %rax\n", o));
    }
    s.push_str("\tmov -8(%rbp), %rbx\n\tleave\n\tret\n\n");
    Ok(s)
}

fn function<'a>(f: &'a FnDef, inl: &'a HashMap<String, FnDef>, sigs: &'a HashMap<String, Sig>, labels: &mut usize, strings: &mut Vec<Vec<u8>>) -> R<String> {
    let mut borrowed = HashSet::new();
    stmts_borrows(&f.body, &mut borrowed);
    for g in inl.values() {
        stmts_borrows(&g.body, &mut borrowed);
    }
    let mut g = Gen {
        out: String::new(),
        sigs,
        int_ops: &f.int_ops,
        inl,
        stack: vec![f.name.clone()],
        vars: HashMap::new(),
        frame: 0,
        depth: 0,
        labels,
        strings,
        ret_label: String::new(),
        ret_words: words(&f.ret)?,
        ret_off: 0,
        ret_dst: None,
        regs: assign_regs(f, inl),
        used_regs: Vec::new(),
        call_saves: Vec::new(),
        borrowed,
    };
    g.ret_label = g.label();
    // Parameters: the last one is nearest to the return address.
    let mut off = 16i64;
    let mut sizes = Vec::new();
    for prm in &f.params {
        sizes.push(words(&prm.ty)? as i64);
    }
    for (prm, n) in f.params.iter().zip(&sizes).rev() {
        let st = if prm.ty.is_mut_ref() { Store::Ptr(off) } else { Store::Slot(off) };
        g.vars.insert(prm.name.clone(), Var { st, ty: prm.ty.base().clone() });
        off += 8 * n;
    }
    g.ret_off = off;
    // Scalar parameters move to registers, when there are registers left.
    for prm in &f.params {
        if let Store::Slot(o) = g.vars[&prm.name].st {
            if let Some(r) = g.reg_for(&prm.name, &prm.ty) {
                g.emit(&format!("mov {}(%rbp), {}", o, r));
                g.vars.get_mut(&prm.name).unwrap().st = Store::Reg(r);
            } else if let Some(rs) = g.regs_for(&prm.name) {
                for (i, r) in rs.iter().enumerate().filter(|(_, r)| !r.is_empty()) {
                    g.emit(&format!("mov {}(%rbp), {}", o + 8 * i as i64, r));
                }
                g.vars.get_mut(&prm.name).unwrap().st = Store::Regs(rs);
            }
        }
    }
    g.block(&f.body)?;
    // Save the callee-saved registers the body used, in the frame.
    let mut saves = String::new();
    let mut restores = String::new();
    for r in g.used_regs.iter().filter(|r| !LEAF.contains(r)) {
        g.frame += 1;
        saves.push_str(&format!("\tmov {}, {}(%rbp)\n", r, -8 * g.frame));
        restores.push_str(&format!("\tmov {}(%rbp), {}\n", -8 * g.frame, r));
    }
    let body = peephole(&g.out);
    let frame = (g.frame + 1) & !1; // keep %rsp 16-byte aligned
    let name = sym(&f.name);
    Ok(format!(
        "\t.type {name}, @function\n{name}:\n\tpush %rbp\n\tmov %rsp, %rbp\n\tsub ${}, %rsp\n{saves}{}{}:\n{restores}\tleave\n\tret\n\t.size {name}, .-{name}\n\n",
        8 * frame,
        body,
        g.ret_label
    ))
}

/// `push X` directly followed by `pop Y` is `mov X, Y` (nothing is
/// removed when X is Y). The stack-machine code above produces many.
fn peephole(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        if let (Some(x), Some(y)) = (lines[i].strip_prefix("\tpush "), lines.get(i + 1).and_then(|l| l.strip_prefix("\tpop "))) {
            if x != y {
                out.push_str(&format!("\tmov{} {}, {}\n", if x.starts_with('$') { "q" } else { "" }, x, y));
            }
            i += 2;
            continue;
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

/// Chooses the variables that live in registers, by use: a use inside a
/// loop counts eight times as much per level of nesting. Scalars whose
/// address is never taken qualify, and so do buffers, strings and vectors
/// (and shared references to them) that are never assigned, borrowed or
/// matched, one register per word. The language has no shadowing, so
/// variables of the same name never live at the same time and can share
/// registers; a name qualifies only if every variable of that name does,
/// with the same size.
fn assign_regs(f: &FnDef, inl: &HashMap<String, FnDef>) -> HashMap<String, Vec<&'static str>> {
    /// Words of a variable of type `t` kept in registers, if it may be.
    fn size(t: &Type) -> Option<usize> {
        match t {
            t if t.is_int() || *t == Type::Bool => Some(1),
            Type::Buf | Type::Bytes | Type::Str => Some(2),
            Type::Vec(_) => Some(3),
            Type::Ref(i) if size(i)? > 1 => size(i),
            _ => None,
        }
    }
    /// Counts the uses in `e`. A buffer or vector named only by the builtins
    /// that read one word of it (its length or its data pointer) needs only
    /// those words in registers; any other use needs them all (`whole`).
    fn expr(e: &Expr, w: u64, a: &mut Acc) {
        match &e.kind {
            ExprKind::Var(n) => {
                let n = a.resolve(n);
                *a.uses.entry(n.clone()).or_insert(0) += w;
                a.whole.insert(n);
            }
            ExprKind::Field(x, _) | ExprKind::Is(x, _, _) | ExprKind::Not(x) | ExprKind::Neg(x) | ExprKind::TypeApp(_, x) => expr(x, w, a),
            ExprKind::Variant(_, _, Some(x)) => expr(x, w, a),
            ExprKind::StructLit(_, fs) => fs.iter().for_each(|(_, x)| expr(x, w, a)),
            ExprKind::Bin(_, l, r) => {
                expr(l, w, a);
                expr(r, w, a);
            }
            ExprKind::Call(f, xs) => {
                // An inlined body counts as part of the caller.
                if let Some(g) = will_inline(a.inl, f, &a.stack) {
                    a.stack.push(f.clone());
                    let outer = a.enter();
                    let mut changed = HashSet::new();
                    stmts_borrows(&g.body, &mut changed);
                    assigned_roots(&g.body, &mut changed);
                    for (p, x) in g.params.iter().zip(xs) {
                        // As in `Gen::bind_params`: the caller's variable.
                        if let (ExprKind::Var(v), false, false) = (&x.kind, p.ty.is_mut_ref(), changed.contains(&p.name)) {
                            let v = a.resolve(v);
                            a.alias.insert(p.name.clone(), v);
                            continue;
                        }
                        a.alias.remove(&p.name);
                        a.declare(&p.name);
                        if p.ty.is_mut_ref() {
                            a.bad.insert(p.name.clone());
                        } else {
                            a.bind(&p.name, Some(&p.ty));
                        }
                        *a.uses.entry(p.name.clone()).or_insert(0) += w;
                    }
                    stmts(&g.body, w, a);
                    for p in &g.params {
                        a.alias.remove(&p.name);
                    }
                    a.cur = outer;
                    a.stack.pop();
                }
                let word = match f.as_str() {
                    f if LENS.contains(&f) => Some(0),
                    "get" | "bget" | "sget" => Some(1),
                    "vec_get" => Some(2),
                    _ => None,
                };
                for (i, x) in xs.iter().enumerate() {
                    match (&x.kind, word) {
                        (ExprKind::Var(n), Some(k)) if i == 0 => {
                            let n = a.resolve(n);
                            *a.uses.entry(n.clone()).or_insert(0) += w;
                            a.read.entry(n).or_default().insert(k);
                        }
                        _ => expr(x, w, a),
                    }
                }
            }
            _ => {}
        }
    }
    struct Acc<'i> {
        inl: &'i HashMap<String, FnDef>,
        stack: Vec<String>,
        uses: HashMap<String, u64>,
        whole: HashSet<String>,
        read: HashMap<String, std::collections::BTreeSet<usize>>,
        sizes: HashMap<String, usize>,
        bad: HashSet<String>,
        assigned: HashSet<String>,
        /// The scopes (blocks and inlined bodies): each one's parent.
        parent: Vec<usize>,
        cur: usize,
        /// The scopes where each name is declared.
        decl: HashMap<String, Vec<usize>>,
        /// Parameters of inlined bodies that are a caller's variable.
        alias: HashMap<String, String>,
    }
    impl Acc<'_> {
        fn resolve(&self, n: &str) -> String {
            self.alias.get(n).cloned().unwrap_or_else(|| n.to_string())
        }
        /// Enters a new scope inside the current one; returns the current.
        fn enter(&mut self) -> usize {
            self.parent.push(self.cur);
            std::mem::replace(&mut self.cur, self.parent.len() - 1)
        }
        fn declare(&mut self, name: &str) {
            self.decl.entry(name.to_string()).or_default().push(self.cur);
        }
        /// Is scope `a` scope `b` or one that encloses it?
        fn encloses(&self, a: usize, mut b: usize) -> bool {
            loop {
                if a == b {
                    return true;
                }
                if b == 0 {
                    return false;
                }
                b = self.parent[b];
            }
        }
        /// Can variables `x` and `y` be alive at the same time? Not when
        /// they are declared in disjoint scopes.
        fn overlap(&self, x: &str, y: &str) -> bool {
            let (dx, dy) = (&self.decl[x], &self.decl[y]);
            dx.iter().any(|&a| dy.iter().any(|&b| self.encloses(a, b) || self.encloses(b, a)))
        }
        fn bind(&mut self, name: &str, t: Option<&Type>) {
            match (t.and_then(size), self.sizes.get(name)) {
                (Some(n), None) => {
                    self.sizes.insert(name.to_string(), n);
                }
                (Some(n), Some(m)) if n == *m => {}
                _ => {
                    self.bad.insert(name.to_string());
                }
            }
        }
    }
    fn stmts(ss: &[Stmt], w: u64, a: &mut Acc) {
        for s in ss {
            match &s.kind {
                StmtKind::Let { name, ty, init, .. } => {
                    a.bind(name, ty.as_ref());
                    a.declare(name);
                    *a.uses.entry(name.clone()).or_insert(0) += w;
                    expr(init, w, a);
                }
                StmtKind::Assign { name, value } => {
                    let root = path_root(name).to_string();
                    *a.uses.entry(root.clone()).or_insert(0) += w;
                    a.assigned.insert(root);
                    expr(value, w, a);
                }
                StmtKind::Expr(x) | StmtKind::Return(Some(x)) => expr(x, w, a),
                StmtKind::If { cond, then_b, else_b } => {
                    expr(cond, w, a);
                    let outer = a.enter();
                    stmts(then_b, w, a);
                    a.cur = outer;
                    let outer = a.enter();
                    stmts(else_b, w, a);
                    a.cur = outer;
                }
                StmtKind::While { cond, body, .. } => {
                    let w = w.saturating_mul(8);
                    expr(cond, w, a);
                    let outer = a.enter();
                    stmts(body, w, a);
                    a.cur = outer;
                }
                StmtKind::Match { scrut, arms } => {
                    expr(scrut, w, a);
                    for arm in arms {
                        // A payload is bound in place, never in a register.
                        if let Pat::Variant(_, _, Some(b), _) = &arm.pat {
                            a.bad.insert(b.clone());
                        }
                        let outer = a.enter();
                        stmts(&arm.body, w, a);
                        a.cur = outer;
                    }
                }
                _ => {}
            }
        }
    }
    let mut a = Acc { inl, stack: vec![f.name.clone()], uses: HashMap::new(), whole: HashSet::new(), read: HashMap::new(), sizes: HashMap::new(), bad: HashSet::new(), assigned: HashSet::new(), parent: vec![0], cur: 0, decl: HashMap::new(), alias: HashMap::new() };
    stmts_borrows(&f.body, &mut a.bad);
    for g in inl.values() {
        stmts_borrows(&g.body, &mut a.bad);
    }
    for p in &f.params {
        a.declare(&p.name);
        if p.ty.is_mut_ref() {
            a.bad.insert(p.name.clone());
        } else {
            a.bind(&p.name, Some(&p.ty));
        }
    }
    stmts(&f.body, 1, &mut a);
    // Each candidate: its name, weight, and the words it needs registers
    // for (`None` for a word that is never read).
    let mut v: Vec<(String, u64, Vec<bool>)> = a
        .uses
        .iter()
        .filter_map(|(n, &w)| {
            let k = *a.sizes.get(n)?;
            if a.bad.contains(n) || (k > 1 && a.assigned.contains(n)) {
                return None;
            }
            let need = (0..k).map(|i| k == 1 || a.whole.contains(n) || a.read.get(n).map_or(false, |r| r.contains(&i))).collect();
            Some((n.clone(), w, need))
        })
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    // %r8-%r11 cost nothing to save in a function that calls nothing
    // (inlined calls aside): it uses them first. Elsewhere they are saved
    // around each call (see `Gen::emit`), so they come last.
    let regs: Vec<&'static str> = if is_leaf(&f.body, inl, &mut vec![f.name.clone()]) { LEAF.iter().chain(SAVED.iter()).copied().collect() } else { SAVED.iter().chain(LEAF.iter()).copied().collect() };
    // Most used first, each takes the first registers that no variable
    // which may be alive at the same time already has.
    let mut out: HashMap<String, Vec<&'static str>> = HashMap::new();
    for (n, _, need) in v {
        let taken: HashSet<&'static str> = out.iter().filter(|(m, _)| a.overlap(&n, m)).flat_map(|(_, rs)| rs.iter().copied()).collect();
        let mut free = regs.iter().filter(|r| !taken.contains(*r));
        let rs: Vec<&'static str> = need.iter().map_while(|b| if *b { free.next().copied() } else { Some("") }).collect();
        if rs.len() == need.len() {
            out.insert(n, rs);
        }
    }
    out
}

/// Largest body (in statements and expression nodes) that is inlined.
const INLINE_MAX: usize = 160;

/// Can calls to `f` be inlined (except into `f` itself)? Small functions
/// can: their body is generated in the caller, one level deep (calls in
/// an inlined body are calls).
fn inlinable(f: &FnDef) -> bool {
    f.name != "main" && body_size(f) <= INLINE_MAX
}

/// The number of statements and expression nodes in `f`'s body.
fn body_size(f: &FnDef) -> usize {
    fn expr(e: &Expr) -> usize {
        1 + match &e.kind {
            ExprKind::Field(x, _) | ExprKind::Is(x, _, _) | ExprKind::Not(x) | ExprKind::Neg(x) | ExprKind::TypeApp(_, x) => expr(x),
            ExprKind::Variant(_, _, Some(x)) => expr(x),
            ExprKind::StructLit(_, fs) => fs.iter().map(|(_, x)| expr(x)).sum(),
            ExprKind::Bin(_, a, b) => expr(a) + expr(b),
            ExprKind::Call(_, xs) => xs.iter().map(expr).sum(),
            _ => 0,
        }
    }
    fn stmts(ss: &[Stmt]) -> usize {
        ss.iter()
            .map(|s| {
                1 + match &s.kind {
                    StmtKind::Let { init: x, .. } | StmtKind::Assign { value: x, .. } | StmtKind::Expr(x) | StmtKind::Return(Some(x)) => expr(x),
                    StmtKind::If { cond, then_b, else_b } => expr(cond) + stmts(then_b) + stmts(else_b),
                    StmtKind::While { cond, body, .. } => expr(cond) + stmts(body),
                    StmtKind::Match { scrut, arms } => expr(scrut) + arms.iter().map(|a| 1 + stmts(&a.body)).sum::<usize>(),
                    _ => 0,
                }
            })
            .sum()
    }
    stmts(&f.body)
}

/// `f` with every variable renamed to `f'x`, which no variable of a caller
/// can be named: the body that `Gen::inline` generates in callers.
fn renamed(f: &FnDef) -> FnDef {
    let pre = format!("{}'", f.name);
    let rn = |n: &str| format!("{}{}", pre, n);
    let path = |p: &str| match p.split_once('.') {
        Some((r, rest)) => format!("{}{}.{}", pre, r, rest),
        None => rn(p),
    };
    fn expr(e: &Expr, path: &dyn Fn(&str) -> String) -> Expr {
        let k = match &e.kind {
            ExprKind::Var(n) => ExprKind::Var(path(n)),
            ExprKind::MutBorrow(p) => ExprKind::MutBorrow(path(p)),
            ExprKind::Field(x, f) => ExprKind::Field(Box::new(expr(x, path)), f.clone()),
            ExprKind::Is(x, a, b) => ExprKind::Is(Box::new(expr(x, path)), a.clone(), b.clone()),
            ExprKind::Not(x) => ExprKind::Not(Box::new(expr(x, path))),
            ExprKind::Neg(x) => ExprKind::Neg(Box::new(expr(x, path))),
            ExprKind::TypeApp(t, x) => ExprKind::TypeApp(t.clone(), Box::new(expr(x, path))),
            ExprKind::Variant(a, b, x) => ExprKind::Variant(a.clone(), b.clone(), x.as_ref().map(|x| Box::new(expr(x, path)))),
            ExprKind::StructLit(n, fs) => ExprKind::StructLit(n.clone(), fs.iter().map(|(f, x)| (f.clone(), expr(x, path))).collect()),
            ExprKind::Bin(op, a, b) => ExprKind::Bin(*op, Box::new(expr(a, path)), Box::new(expr(b, path))),
            ExprKind::Call(n, xs) => ExprKind::Call(n.clone(), xs.iter().map(|x| expr(x, path)).collect()),
            k => k.clone(),
        };
        Expr { kind: k, pos: e.pos }
    }
    fn stmts(ss: &[Stmt], path: &dyn Fn(&str) -> String) -> Vec<Stmt> {
        ss.iter()
            .map(|s| {
                let k = match &s.kind {
                    StmtKind::Let { name, mutable, ty, init } => StmtKind::Let { name: path(name), mutable: *mutable, ty: ty.clone(), init: expr(init, path) },
                    StmtKind::Assign { name, value } => StmtKind::Assign { name: path(name), value: expr(value, path) },
                    StmtKind::If { cond, then_b, else_b } => StmtKind::If { cond: expr(cond, path), then_b: stmts(then_b, path), else_b: stmts(else_b, path) },
                    // Invariants and asserts generate no code.
                    StmtKind::While { cond, body, .. } => StmtKind::While { cond: expr(cond, path), invariants: Vec::new(), body: stmts(body, path) },
                    StmtKind::Return(x) => StmtKind::Return(x.as_ref().map(|x| expr(x, path))),
                    StmtKind::Assert(_) => StmtKind::Assert(Expr { kind: ExprKind::Bool(true), pos: s.pos }),
                    StmtKind::Expr(x) => StmtKind::Expr(expr(x, path)),
                    StmtKind::Match { scrut, arms } => StmtKind::Match {
                        scrut: expr(scrut, path),
                        arms: arms
                            .iter()
                            .map(|a| Arm {
                                pat: match &a.pat {
                                    Pat::Variant(e, v, Some(b), m) if b != "_" => Pat::Variant(e.clone(), v.clone(), Some(path(b)), *m),
                                    p => p.clone(),
                                },
                                body: stmts(&a.body, path),
                                pos: a.pos,
                            })
                            .collect(),
                    },
                    k => k.clone(),
                };
                Stmt { kind: k, pos: s.pos }
            })
            .collect()
    }
    let mut g = f.clone();
    g.params = f.params.iter().map(|p| Param { name: rn(&p.name), ..p.clone() }).collect();
    g.body = stmts(&f.body, &path);
    g.requires.clear();
    g.ensures.clear();
    g
}

/// The variables assigned (wholly or in part) somewhere in `ss`.
fn assigned_roots(ss: &[Stmt], out: &mut HashSet<String>) {
    for s in ss {
        match &s.kind {
            StmtKind::Assign { name, .. } => {
                out.insert(path_root(name).to_string());
            }
            StmtKind::If { then_b, else_b, .. } => {
                assigned_roots(then_b, out);
                assigned_roots(else_b, out);
            }
            StmtKind::While { body, .. } => assigned_roots(body, out),
            StmtKind::Match { arms, .. } => arms.iter().for_each(|a| assigned_roots(&a.body, out)),
            _ => {}
        }
    }
}

/// The variables whose address is taken (`&mut x`) somewhere in `ss`.
fn stmts_borrows(ss: &[Stmt], out: &mut HashSet<String>) {
    fn expr(e: &Expr, out: &mut HashSet<String>) {
        match &e.kind {
            ExprKind::MutBorrow(p) => {
                out.insert(path_root(p).to_string());
            }
            ExprKind::Field(x, _) | ExprKind::Is(x, _, _) | ExprKind::Not(x) | ExprKind::Neg(x) | ExprKind::TypeApp(_, x) => expr(x, out),
            ExprKind::Variant(_, _, Some(x)) => expr(x, out),
            ExprKind::StructLit(_, fs) => fs.iter().for_each(|(_, x)| expr(x, out)),
            ExprKind::Bin(_, a, b) => {
                expr(a, out);
                expr(b, out);
            }
            ExprKind::Call(_, xs) => xs.iter().for_each(|x| expr(x, out)),
            _ => {}
        }
    }
    for s in ss {
        match &s.kind {
            StmtKind::Let { init: x, .. } | StmtKind::Assign { value: x, .. } | StmtKind::Expr(x) | StmtKind::Return(Some(x)) => expr(x, out),
            StmtKind::If { cond, then_b, else_b } => {
                expr(cond, out);
                stmts_borrows(then_b, out);
                stmts_borrows(else_b, out);
            }
            StmtKind::While { cond, body, .. } => {
                expr(cond, out);
                stmts_borrows(body, out);
            }
            StmtKind::Match { scrut, arms } => {
                expr(scrut, out);
                for a in arms {
                    stmts_borrows(&a.body, out);
                }
            }
            _ => {}
        }
    }
}

/// The value of `e` if it is a constant that fits a 32-bit immediate.
fn small_int(e: &Expr) -> Option<u128> {
    match e.kind {
        ExprKind::Int(v) if v <= i32::MAX as u128 => Some(v),
        _ => None,
    }
}

/// Does `e` lend anything mutably (and so possibly change a place)?
fn borrows(e: &Expr) -> bool {
    let mut s = HashSet::new();
    let st = Stmt { kind: StmtKind::Expr(e.clone()), pos: e.pos };
    stmts_borrows(std::slice::from_ref(&st), &mut s);
    !s.is_empty()
}

/// Condition codes for comparisons: (jump/set when true, when false).
fn cc(op: BinOp, signed: bool) -> Option<(&'static str, &'static str)> {
    Some(match (op, signed) {
        (BinOp::Eq, _) => ("e", "ne"),
        (BinOp::Ne, _) => ("ne", "e"),
        (BinOp::Lt, false) => ("b", "ae"),
        (BinOp::Le, false) => ("be", "a"),
        (BinOp::Gt, false) => ("a", "be"),
        (BinOp::Ge, false) => ("ae", "b"),
        (BinOp::Lt, true) => ("l", "ge"),
        (BinOp::Le, true) => ("le", "g"),
        (BinOp::Gt, true) => ("g", "le"),
        (BinOp::Ge, true) => ("ge", "l"),
        _ => return None,
    })
}

/// Truncates %rax to integer type `t` and extends it back to a full word.
fn normalize(t: &Type) -> &'static str {
    match t {
        Type::U8 => "movzbl %al, %eax",
        Type::U16 => "movzwl %ax, %eax",
        Type::U32 => "mov %eax, %eax",
        Type::I8 => "movsbq %al, %rax",
        Type::I16 => "movswq %ax, %rax",
        Type::I32 => "movslq %eax, %rax",
        _ => "",
    }
}

impl<'a> Gen<'a> {
    fn emit(&mut self, s: &str) {
        // A call may change %r8-%r11: the variables there so far are saved
        // in the frame around it.
        let saves: Vec<(&str, i64)> = if s.starts_with("call ") {
            let leaf: Vec<&'static str> = self.used_regs.iter().copied().filter(|r| LEAF.contains(r)).collect();
            leaf.into_iter().map(|r| (r, self.save_slot(r))).collect()
        } else {
            Vec::new()
        };
        for (r, o) in &saves {
            self.out.push_str(&format!("\tmov {}, {}(%rbp)\n", r, o));
        }
        self.out.push('\t');
        self.out.push_str(s);
        self.out.push('\n');
        for (r, o) in &saves {
            self.out.push_str(&format!("\tmov {}(%rbp), {}\n", o, r));
        }
    }

    /// The frame slot where `emit` saves caller-saved register `r`.
    fn save_slot(&mut self, r: &'static str) -> i64 {
        if let Some(&(_, o)) = self.call_saves.iter().find(|(x, _)| *x == r) {
            return o;
        }
        let o = self.slot(1);
        self.call_saves.push((r, o));
        o
    }
    fn label(&mut self) -> String {
        *self.labels += 1;
        format!(".L{}", self.labels)
    }
    fn place_label(&mut self, l: &str) {
        self.out.push_str(l);
        self.out.push_str(":\n");
    }
    fn push(&mut self, op: &str) {
        self.emit(&format!("push {}", op));
        self.depth += 1;
    }
    fn pop(&mut self, op: &str) {
        self.emit(&format!("pop {}", op));
        self.depth -= 1;
    }
    fn reserve(&mut self, n: usize) {
        if n > 0 {
            self.emit(&format!("sub ${}, %rsp", 8 * n));
            self.depth += n as i64;
        }
    }
    /// Pops `n` words without looking at them (`lea` keeps the flags).
    fn drop_words(&mut self, n: usize) {
        if n > 0 {
            self.emit(&format!("lea {}(%rsp), %rsp", 8 * n));
            self.depth -= n as i64;
        }
    }
    /// A frame slot of `n` words; returns the offset of word 0.
    fn slot(&mut self, n: usize) -> i64 {
        self.frame += n as i64;
        -8 * self.frame
    }
    /// A callee-saved register for a scalar variable, if one is free.
    fn reg_for(&mut self, name: &str, t: &Type) -> Option<&'static str> {
        if !(t.is_int() || *t == Type::Bool) || self.borrowed.contains(name) {
            return None;
        }
        let r = *self.regs.get(name)?.first()?;
        self.use_reg(r);
        Some(r)
    }

    /// The registers of a multi-word variable that never changes, if
    /// `assign_regs` gave it some.
    fn regs_for(&mut self, name: &str) -> Option<[&'static str; 3]> {
        let rs = self.regs.get(name).filter(|rs| rs.len() > 1)?.clone();
        let mut a = [""; 3];
        for (i, r) in rs.into_iter().enumerate() {
            if !r.is_empty() {
                self.use_reg(r);
            }
            a[i] = r;
        }
        Some(a)
    }

    /// The register of `e` if it is a variable kept in one.
    fn var_reg(&self, e: &Expr) -> Option<&'static str> {
        match &e.kind {
            ExprKind::Var(n) => match self.vars.get(n)?.st {
                Store::Reg(r) => Some(r),
                _ => None,
            },
            _ => None,
        }
    }

    fn use_reg(&mut self, r: &'static str) {
        if !self.used_regs.contains(&r) {
            self.used_regs.push(r);
        }
    }
    /// Pops `n` words into the place at `loc`.
    fn pop_to(&mut self, loc: Loc, n: usize) {
        for i in 0..n {
            self.pop("%rax");
            self.emit(&format!("mov %rax, {}", loc.op(i)));
        }
    }
    /// Pushes the `n` words at `loc` (word 0 ends on top).
    fn push_from(&mut self, loc: Loc, n: usize) {
        for i in (0..n).rev() {
            self.push(&loc.op(i));
        }
    }

    // ---- places ----

    /// Computes the location of a dotted place (using %rcx to follow pointers).
    fn place(&mut self, path: &str) -> (Loc, Type) {
        let mut parts = path.split('.');
        let v = self.vars[parts.next().unwrap()].clone();
        let mut t = v.ty;
        let mut loc = match v.st {
            Store::Slot(o) => Loc::Mem("%rbp", o),
            Store::Reg(r) => Loc::Reg(r),
            Store::Regs(rs) => Loc::Regs(rs),
            Store::Ptr(o) => {
                self.emit(&format!("mov {}(%rbp), %rcx", o));
                Loc::Mem("%rcx", 0)
            }
        };
        for f in parts {
            while let Type::Box(i) = t {
                self.emit(&format!("mov {}, %rcx", loc.op(0)));
                loc = Loc::Mem("%rcx", 0);
                t = *i;
            }
            let (o, ft) = field(&t, f);
            let Loc::Mem(b, base) = loc else { unreachable!("a register holds no struct") };
            loc = Loc::Mem(b, base + 8 * o as i64);
            t = ft;
        }
        (loc, t)
    }

    fn type_of(&self, e: &Expr) -> Type {
        match &e.kind {
            ExprKind::TypeApp(targs, inner) => match &inner.kind {
                ExprKind::Call(n, _) if n == "vec_new" => Type::Vec(Box::new(targs[0].clone())),
                _ => targs[0].clone(), // cast, wrap
            },
            ExprKind::Neg(_) => self.int_ops.get(&e.pos).cloned().unwrap_or(Type::I64),
            ExprKind::Call(n, _) if crate::check::WRAP_BUILTINS.contains(&n.as_str()) => self.int_ops[&e.pos].clone(),
            ExprKind::Int(_) => Type::U64,
            ExprKind::Bool(_) | ExprKind::Is(..) => Type::Bool,
            ExprKind::Not(_) => self.int_ops.get(&e.pos).cloned().unwrap_or(Type::Bool),
            ExprKind::Str(_) | ExprKind::Blob(_) => Type::Str,
            ExprKind::Var(n) => self.vars[n].ty.clone(),
            ExprKind::Field(b, f) => {
                let mut t = self.type_of(b);
                while let Type::Box(i) = t {
                    t = *i;
                }
                field(&t, f).1
            }
            ExprKind::StructLit(n, _) => Type::Struct(n.clone()),
            ExprKind::Variant(n, ..) => Type::Enum(n.clone()),
            ExprKind::Bin(op, ..) => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => self.int_ops.get(&e.pos).cloned().unwrap_or(Type::U64),
                op if op.is_bitwise() => self.int_ops[&e.pos].clone(),
                _ => Type::Bool,
            },
            ExprKind::Call(n, args) if n == "box" => Type::Box(Box::new(self.type_of(&args[0]))),
            ExprKind::Call(n, args) if n == "unbox" => match self.type_of(&args[0]) {
                Type::Box(i) => *i,
                _ => unreachable!("checked"),
            },
            ExprKind::Call(n, args) if crate::check::VEC_BUILTINS.contains(&n.as_str()) => match n.as_str() {
                "vec_len" => Type::U64,
                "vec_push" | "vec_set" | "vec_free" => Type::Unit,
                _ => self.vec_elem(&args[0]),
            },
            ExprKind::Call(n, args) if crate::check::PT_BORROW_BUILTINS.contains(&n.as_str()) => match &args[2].kind {
                ExprKind::Var(f) => self.sigs[f].ret.clone(),
                _ => unreachable!("checked"),
            },
            ExprKind::Call(n, _) => self.sigs[n].ret.clone(),
            ExprKind::MutBorrow(p) => Type::MutRef(Box::new(self.place_type(p))),
            ExprKind::Forall(..) => unreachable!("not a value"),
        }
    }

    fn place_type(&self, path: &str) -> Type {
        let mut parts = path.split('.');
        let mut t = self.vars[parts.next().unwrap()].ty.clone();
        for f in parts {
            while let Type::Box(i) = t {
                t = *i;
            }
            t = field(&t, f).1;
        }
        t
    }

    fn vec_elem(&self, a: &Expr) -> Type {
        let t = match &a.kind {
            ExprKind::MutBorrow(n) => self.place_type(n),
            _ => self.type_of(a),
        };
        let Type::Vec(el) = t else { unreachable!("checked: a Vec") };
        *el
    }

    // ---- statements ----

    /// A block: variables declared inside go out of scope at its end (their
    /// registers are reused; frame slots are not).
    fn block(&mut self, ss: &[Stmt]) -> R<()> {
        let saved = self.vars.clone();
        for s in ss {
            self.stmt(s)?;
        }
        self.vars = saved;
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> R<()> {
        match &s.kind {
            StmtKind::Let { name, ty, init, .. } => {
                let t = ty.clone().expect("typed by checker");
                let n = words(&t)?;
                if n > 1 {
                    if let Some(rs) = self.regs_for(name) {
                        self.push_val(init)?;
                        for r in &rs[..n] {
                            if r.is_empty() {
                                self.drop_words(1);
                            } else {
                                self.pop(r);
                            }
                        }
                        self.vars.insert(name.clone(), Var { st: Store::Regs(rs), ty: t });
                        return Ok(());
                    }
                    let o = self.slot(n);
                    self.val_into(init, Dst::Frame(o))?;
                    self.vars.insert(name.clone(), Var { st: Store::Slot(o), ty: t });
                    return Ok(());
                }
                if n == 1 {
                    let (dst, st) = match self.reg_for(name, &t) {
                        Some(r) => (r.to_string(), Store::Reg(r)),
                        None => {
                            let o = self.slot(1);
                            (format!("{}(%rbp)", o), Store::Slot(o))
                        }
                    };
                    self.scalar_into(init, &dst)?;
                    self.vars.insert(name.clone(), Var { st, ty: t });
                    return Ok(());
                }
                // A value of no words (`()`).
                self.push_val(init)?;
                let o = self.slot(0);
                self.vars.insert(name.clone(), Var { st: Store::Slot(o), ty: t });
            }
            StmtKind::Assign { name, value } => {
                // `b = set(b, i, v)` stores one word in place.
                if let ExprKind::Call(f, args) = &value.kind {
                    if f == "set" && place(&args[0]).as_deref() == Some(name.as_str()) {
                        if let Some(v) = small_int(&args[2]) {
                            self.scalar(&args[1])?;
                            let (loc, _) = self.place(name);
                            self.emit(&format!("mov {}, %rcx", loc.op(1)));
                            self.emit(&format!("movq ${}, (%rcx,%rax,8)", v));
                            return Ok(());
                        }
                        if self.simple(&args[1]) {
                            self.scalar(&args[2])?;
                            let o = self.operand(&args[1]).unwrap();
                            self.emit(&format!("mov {}, %rdx", o));
                        } else {
                            self.scalar(&args[1])?;
                            self.push("%rax");
                            self.scalar(&args[2])?;
                            self.pop("%rdx");
                        }
                        let (loc, _) = self.place(name);
                        self.emit(&format!("mov {}, %rcx", loc.op(1)));
                        self.emit("mov %rax, (%rcx,%rdx,8)");
                        return Ok(());
                    }
                }
                // `x = x + e` / `x = x - e` on a register variable, `e` simple.
                if let (Some(Store::Reg(r)), ExprKind::Bin(op @ (BinOp::Add | BinOp::Sub), a, b)) = (self.vars.get(name).map(|v| v.st), &value.kind) {
                    let is_x = |e: &Expr| matches!(&e.kind, ExprKind::Var(v) if v == name);
                    let other = if is_x(a) { Some(b) } else if *op == BinOp::Add && is_x(b) { Some(a) } else { None };
                    if let Some(o) = other.filter(|o| !borrows(o)) {
                        let o = if self.simple(o) {
                            self.operand(o).unwrap()
                        } else {
                            self.scalar(o)?;
                            "%rax".to_string()
                        };
                        self.emit(&format!("{} {}, {}", if *op == BinOp::Add { "add" } else { "sub" }, o, r));
                        return Ok(());
                    }
                }
                let n = words(&self.place_type(name))?;
                // Straight into a register or frame slot.
                let direct = match self.vars[path_root(name)].st {
                    Store::Reg(r) => Some(r.to_string()),
                    Store::Slot(_) => match self.store_of(name) {
                        Some(Store::Slot(o)) => Some(format!("{}(%rbp)", o)),
                        _ => None,
                    },
                    _ => None,
                };
                if let (1, Some(d)) = (n, &direct) {
                    self.scalar_into(value, d)?;
                } else if n == 1 {
                    self.scalar(value)?;
                    let (loc, _) = self.place(name);
                    self.emit(&format!("mov %rax, {}", loc.op(0)));
                } else {
                    self.push_val(value)?;
                    let (loc, _) = self.place(name);
                    self.pop_to(loc, n);
                }
            }
            StmtKind::If { cond, then_b, else_b } => {
                let (l_else, l_end) = (self.label(), self.label());
                self.branch(cond, &l_else, false)?;
                self.block(then_b)?;
                if else_b.is_empty() {
                    self.place_label(&l_else);
                } else {
                    self.emit(&format!("jmp {}", l_end));
                    self.place_label(&l_else);
                    self.block(else_b)?;
                    self.place_label(&l_end);
                }
            }
            StmtKind::While { cond, body, .. } => {
                let (l_top, l_cond) = (self.label(), self.label());
                self.emit(&format!("jmp {}", l_cond));
                self.out.push_str("\t.p2align 4\n");
                self.place_label(&l_top);
                self.block(body)?;
                self.place_label(&l_cond);
                self.branch(cond, &l_top, true)?;
            }
            StmtKind::Match { scrut, arms } => {
                // The scrutinee is copied, as in the C backend, so an arm may
                // assign the matched variable while its payload is bound.
                let t = self.type_of(scrut);
                let en = enum_def(&t.name()).unwrap();
                let n = words(&t)?;
                // A variable in the frame that no arm changes is matched
                // where it is.
                let in_place = place(scrut).and_then(|p| {
                    let mut changed = HashSet::new();
                    for a in arms.iter() {
                        stmts_borrows(&a.body, &mut changed);
                        assigned_roots(&a.body, &mut changed);
                    }
                    // Nor its payload, through a binding.
                    let payload_changed = arms.iter().any(|a| matches!(&a.pat, Pat::Variant(_, _, Some(b), _) if changed.contains(b)));
                    match self.store_of(&p) {
                        Some(Store::Slot(o)) if !changed.contains(path_root(&p)) && !payload_changed => Some(o),
                        _ => None,
                    }
                });
                let o = match in_place {
                    Some(o) => o,
                    None => {
                        let o = self.slot(n);
                        self.val_into(scrut, Dst::Frame(o))?;
                        o
                    }
                };
                let l_end = self.label();
                for (i, a) in arms.iter().enumerate() {
                    let last = i + 1 == arms.len();
                    let l_next = self.label();
                    let saved = self.vars.clone();
                    if let Pat::Variant(_, v, b, _) = &a.pat {
                        let k = en.index(v).unwrap();
                        if !last {
                            self.emit(&format!("cmpq ${}, {}(%rbp)", k, o));
                            self.emit(&format!("jne {}", l_next));
                        }
                        if let Some(b) = b.as_ref().filter(|b| *b != "_") {
                            let pt = en.variants[k].1.clone().unwrap();
                            // The payload is bound where it lies in the copy.
                            self.vars.insert(b.clone(), Var { st: Store::Slot(o + 8), ty: pt });
                        }
                    }
                    self.block(&a.body)?;
                    self.vars = saved;
                    if !last {
                        self.emit(&format!("jmp {}", l_end));
                    }
                    self.place_label(&l_next);
                }
                self.place_label(&l_end);
            }
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    if self.ret_words == 1 {
                        self.scalar(e)?;
                    } else {
                        let d = self.ret_dst.unwrap_or(Dst::Frame(self.ret_off));
                        self.val_into(e, d)?;
                    }
                }
                let l = self.ret_label.clone();
                self.emit(&format!("jmp {}", l));
            }
            StmtKind::Assert(_) => {}
            StmtKind::Unreachable => self.emit("ud2"),
            StmtKind::Expr(e) => {
                let n = words(&self.type_of(e))?;
                if n == 1 {
                    self.scalar(e)?;
                } else {
                    self.push_val(e)?;
                    self.drop_words(n);
                }
            }
        }
        Ok(())
    }

    // ---- expressions ----

    /// Jumps to `target` when `e` evaluates to `when`; falls through otherwise.
    fn branch(&mut self, e: &Expr, target: &str, when: bool) -> R<()> {
        match &e.kind {
            ExprKind::Bool(b) => {
                if *b == when {
                    self.emit(&format!("jmp {}", target));
                }
            }
            ExprKind::Not(x) if !self.int_ops.contains_key(&e.pos) => self.branch(x, target, !when)?,
            ExprKind::Call(n, args) if will_inline(self.inl, n, &self.stack).map_or(false, |f| matches!(f.body.as_slice(), [Stmt { kind: StmtKind::Return(Some(_)), .. }])) => {
                let f = will_inline(self.inl, n, &self.stack).unwrap();
                self.branch_inline(f, args, target, when)?;
            }
            ExprKind::Bin(op @ (BinOp::And | BinOp::Or | BinOp::Implies), a, b) => {
                // `a` decides the whole when it is `stop`: false for `&&`,
                // true for `||`, false for `==>` (which is `!a || b`); the
                // whole is then `decided`.
                let (stop, decided) = match op {
                    BinOp::And => (false, false),
                    BinOp::Or => (true, true),
                    _ => (false, true),
                };
                if decided == when {
                    self.branch(a, target, stop)?;
                    self.branch(b, target, when)?;
                } else {
                    let skip = self.label();
                    self.branch(a, &skip, stop)?;
                    self.branch(b, target, when)?;
                    self.place_label(&skip);
                }
            }
            ExprKind::Bin(op, a, b) if cc(*op, false).is_some() => {
                let (t, f) = self.compare(*op, self.signed(e), a, b)?;
                self.emit(&format!("j{} {}", if when { t } else { f }, target));
            }
            _ => {
                self.scalar(e)?;
                self.emit("test %rax, %rax");
                self.emit(&format!("j{} {}", if when { "nz" } else { "z" }, target));
            }
        }
        Ok(())
    }

    /// An operand for `e` that needs no register but %rcx: a small constant,
    /// a place, or the length of a place. Code computing the place's address
    /// is emitted. None (and no code) when `simple(e)` is false.
    fn operand(&mut self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Int(v) if *v <= i32::MAX as u128 => Some(format!("${}", v)),
            ExprKind::Bool(b) => Some(format!("${}", *b as u8)),
            ExprKind::Var(_) | ExprKind::Field(..) => {
                let p = place(e)?;
                let (loc, _) = self.place(&p);
                Some(loc.op(0))
            }
            ExprKind::Call(n, args) if LENS.contains(&n.as_str()) => {
                let p = place(&args[0])?;
                let (loc, _) = self.place(&p);
                Some(loc.op(0))
            }
            _ => None,
        }
    }

    /// Can `operand` handle `e`?
    fn simple(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Int(v) => *v <= i32::MAX as u128,
            ExprKind::Bool(_) => true,
            ExprKind::Call(n, args) if LENS.contains(&n.as_str()) => place(&args[0]).is_some(),
            _ => place(e).is_some(),
        }
    }

    /// Puts the one-word value of `e` in `dst` (a register, or memory not
    /// addressed through %rcx). Through %rax unless one move does it, or,
    /// for `a + b` / `a - b` into a register, two instructions on it.
    fn scalar_into(&mut self, e: &Expr, dst: &str) -> R<()> {
        let reg = dst.starts_with('%');
        if self.simple(e) && !dst.contains("%rcx") {
            let op = self.operand(e).unwrap();
            if reg || op.starts_with('%') || op.starts_with('$') {
                if op != dst {
                    self.emit(&format!("mov{} {}, {}", if op.starts_with('$') && !reg { "q" } else { "" }, op, dst));
                }
                return Ok(());
            }
            self.emit(&format!("mov {}, %rax", op));
        } else if let (true, ExprKind::Bin(op @ (BinOp::Add | BinOp::Sub), a, b)) = (reg, &e.kind) {
            // `dst` is written before `b` is read: `b` must not be in it.
            if self.simple(a) && self.simple(b) && !self.mentions(b, dst) {
                let x = self.operand(a).unwrap();
                if x != dst {
                    self.emit(&format!("mov {}, {}", x, dst));
                }
                let y = self.operand(b).unwrap();
                self.emit(&format!("{} {}, {}", if *op == BinOp::Add { "add" } else { "sub" }, y, dst));
                return Ok(());
            }
            self.scalar(e)?;
        } else {
            self.scalar(e)?;
        }
        self.emit(&format!("mov %rax, {}", dst));
        Ok(())
    }

    /// Might reading simple expression `e` read register `r`?
    fn mentions(&self, e: &Expr, r: &str) -> bool {
        let p = match &e.kind {
            ExprKind::Call(_, args) => place(&args[0]),
            _ => place(e),
        };
        p.map_or(false, |p| match self.vars.get(path_root(&p)).map(|v| v.st) {
            Some(Store::Reg(x)) => x == r,
            Some(Store::Regs(xs)) => xs.contains(&r),
            _ => false,
        })
    }

    /// One operand in %rax and the other as a direct operand or in %rcx.
    /// When `a` is simple and `b` is not and the operation is commutative
    /// (`swap_ok`), `b` goes in %rax instead: reading `a` has no effect, so
    /// the order does not matter. Returns the operand and whether it swapped.
    fn two(&mut self, a: &Expr, b: &Expr, swap_ok: bool) -> R<(String, bool)> {
        if self.simple(b) {
            self.scalar(a)?;
            return Ok((self.operand(b).unwrap(), false));
        }
        if swap_ok && self.simple(a) && !borrows(b) {
            self.scalar(b)?;
            return Ok((self.operand(a).unwrap(), true));
        }
        self.scalar(a)?;
        self.push("%rax");
        self.scalar(b)?;
        self.emit("mov %rax, %rcx");
        self.pop("%rax");
        Ok(("%rcx".into(), false))
    }

    /// Compares `a` with `b`; returns the condition codes of `op` for the
    /// flags set (reversed when the operands were swapped).
    /// Are the operands of operator `e` signed integers?
    fn signed(&self, e: &Expr) -> bool {
        self.int_ops.get(&e.pos).map_or(false, |t| t.is_signed())
    }

    fn compare(&mut self, op: BinOp, signed: bool, a: &Expr, b: &Expr) -> R<(&'static str, &'static str)> {
        // A register variable against a simple operand needs no %rax.
        if let ExprKind::Var(v) = &a.kind {
            if let (Store::Reg(r), true) = (self.vars[v].st, self.simple(b)) {
                let o = self.operand(b).unwrap();
                if o == "$0" && matches!(op, BinOp::Eq | BinOp::Ne) {
                    self.emit(&format!("test {}, {}", r, r));
                } else {
                    self.emit(&format!("cmp{} {}, {}", q(&o), o, r));
                }
                return Ok(cc(op, signed).unwrap());
            }
        }
        let (o, swapped) = self.two(a, b, true)?;
        if o == "$0" && matches!(op, BinOp::Eq | BinOp::Ne) {
            self.emit("test %rax, %rax");
        } else {
            self.emit(&format!("cmp{} {}, %rax", q(&o), o));
        }
        let op = if !swapped {
            op
        } else {
            match op {
                BinOp::Lt => BinOp::Gt,
                BinOp::Gt => BinOp::Lt,
                BinOp::Le => BinOp::Ge,
                BinOp::Ge => BinOp::Le,
                op => op,
            }
        };
        Ok(cc(op, signed).unwrap())
    }

    /// Evaluates a one-word expression into %rax.
    fn scalar(&mut self, e: &Expr) -> R<()> {
        match &e.kind {
            ExprKind::Int(v) => {
                if *v == 0 {
                    self.emit("xor %eax, %eax");
                } else if *v <= u32::MAX as u128 {
                    self.emit(&format!("mov ${}, %eax", v));
                } else {
                    self.emit(&format!("movabs ${}, %rax", *v as u64));
                }
            }
            ExprKind::Bool(b) => self.emit(&format!("mov ${}, %eax", *b as u8)),
            ExprKind::Var(_) | ExprKind::Field(..) => {
                let p = place(e).expect("checked: a place");
                let (loc, _) = self.place(&p);
                self.emit(&format!("mov {}, %rax", loc.op(0)));
            }
            ExprKind::MutBorrow(p) => {
                let v = &self.vars[path_root(p)];
                if let (Store::Ptr(o), false) = (v.st, p.contains('.')) {
                    self.emit(&format!("mov {}(%rbp), %rax", o));
                } else {
                    let (loc, _) = self.place(p);
                    let Loc::Mem(..) = loc else { unreachable!("a borrowed variable is in memory") };
                    self.emit(&format!("lea {}, %rax", loc.op(0)));
                }
            }
            // `!` on an unsigned integer flips its bits within its width.
            ExprKind::Not(x) => {
                self.scalar(x)?;
                match self.int_ops.get(&e.pos).cloned() {
                    Some(t) => {
                        self.emit("not %rax");
                        let n = normalize(&t);
                        if !n.is_empty() {
                            self.emit(n);
                        }
                    }
                    None => self.emit("xor $1, %eax"),
                }
            }
            ExprKind::Neg(x) => match x.kind {
                ExprKind::Int(v) => self.emit(&format!("movabs ${}, %rax", (v as i128).wrapping_neg() as i64)),
                _ => {
                    self.scalar(x)?;
                    self.emit("neg %rax");
                }
            },
            // `cast[T](x)` is proved to fit, so the word is unchanged;
            // `wrap[T](x)` keeps the low bits.
            ExprKind::TypeApp(ts, inner) if matches!(&inner.kind, ExprKind::Call(n, _) if n == "cast" || n == "wrap") => {
                let ExprKind::Call(n, args) = &inner.kind else { unreachable!() };
                self.scalar(&args[0])?;
                if n == "wrap" {
                    let ins = normalize(&ts[0]);
                    if !ins.is_empty() {
                        self.emit(ins);
                    }
                }
            }
            ExprKind::Is(x, _, v) => {
                let t = self.type_of(x);
                let k = enum_def(&t.name()).unwrap().index(v).unwrap();
                if let Some(p) = place(x) {
                    let (loc, _) = self.place(&p);
                    self.emit(&format!("cmpq ${}, {}", k, loc.op(0)));
                } else {
                    let n = words(&t)?;
                    self.push_val(x)?;
                    self.emit(&format!("cmpq ${}, (%rsp)", k));
                    self.drop_words(n);
                }
                self.emit("sete %al");
                self.emit("movzbl %al, %eax");
            }
            ExprKind::Bin(op, a, b) => match op {
                BinOp::Mul if small_int(b).map_or(false, |k| k.is_power_of_two()) => {
                    self.scalar(a)?;
                    let k = small_int(b).unwrap().trailing_zeros();
                    if k > 0 {
                        self.emit(&format!("shl ${}, %rax", k));
                    }
                }
                BinOp::Add | BinOp::Sub | BinOp::Mul => {
                    let (o, _) = self.two(a, b, *op != BinOp::Sub)?;
                    let ins = match op {
                        BinOp::Add => "add",
                        BinOp::Sub => "sub",
                        _ => "imul",
                    };
                    self.emit(&format!("{}{} {}, %rax", ins, q(&o), o));
                }
                // Unsigned only. A left shift is proved to lose no bits, and
                // the amount to be less than the width.
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                    let (o, _) = self.two(a, b, true)?;
                    let ins = match op {
                        BinOp::BitAnd => "and",
                        BinOp::BitOr => "or",
                        _ => "xor",
                    };
                    self.emit(&format!("{}{} {}, %rax", ins, q(&o), o));
                }
                BinOp::Shl | BinOp::Shr => {
                    let ins = if *op == BinOp::Shl { "shl" } else { "shr" };
                    if let Some(k) = small_int(b) {
                        self.scalar(a)?;
                        self.emit(&format!("{} ${}, %rax", ins, k));
                    } else {
                        let (o, _) = self.two(a, b, false)?;
                        if o != "%rcx" {
                            self.emit(&format!("mov {}, %rcx", o));
                        }
                        self.emit(&format!("{} %cl, %rax", ins));
                    }
                }
                BinOp::Div | BinOp::Rem if self.signed(e) => {
                    // Truncating division, as C's: what `idiv` does.
                    let (o, _) = self.two(a, b, false)?;
                    if o != "%rcx" {
                        self.emit(&format!("mov {}, %rcx", o));
                    }
                    self.emit("cqo");
                    self.emit("idiv %rcx");
                    if *op == BinOp::Rem {
                        self.emit("mov %rdx, %rax");
                    }
                }
                BinOp::Div | BinOp::Rem => {
                    if let ExprKind::Int(d) = b.kind {
                        if d > 0 && d < 1 << 63 {
                            self.scalar(a)?;
                            self.div_const(d as u64, *op == BinOp::Rem);
                            return Ok(());
                        }
                    }
                    let (o, _) = self.two(a, b, false)?;
                    if o != "%rcx" {
                        self.emit(&format!("mov {}, %rcx", o));
                    }
                    self.emit("xor %edx, %edx");
                    self.emit("div %rcx");
                    if *op == BinOp::Rem {
                        self.emit("mov %rdx, %rax");
                    }
                }
                _ if cc(*op, false).is_some() => {
                    let (t, _) = self.compare(*op, self.signed(e), a, b)?;
                    self.emit(&format!("set{} %al", t));
                    self.emit("movzbl %al, %eax");
                }
                _ => {
                    let (f, end) = (self.label(), self.label());
                    self.branch(e, &f, false)?;
                    self.emit("mov $1, %eax");
                    self.emit(&format!("jmp {}", end));
                    self.place_label(&f);
                    self.emit("xor %eax, %eax");
                    self.place_label(&end);
                }
            },
            // Modular arithmetic: the 64-bit result, truncated to the type.
            ExprKind::Call(name, args) if crate::check::WRAP_BUILTINS.contains(&name.as_str()) => {
                let t = self.int_ops[&e.pos].clone();
                let (o, _) = self.two(&args[0], &args[1], name != "wrap_sub")?;
                let ins = match name.as_str() {
                    "wrap_add" => "add",
                    "wrap_sub" => "sub",
                    _ => "imul",
                };
                self.emit(&format!("{}{} {}, %rax", ins, q(&o), o));
                let n = normalize(&t);
                if !n.is_empty() {
                    self.emit(n);
                }
            }
            ExprKind::Call(name, args) => {
                let t = self.type_of(e);
                if words(&t)? != 1 {
                    return Err(format!("internal: `{}` used as a scalar", name));
                }
                self.call(name, args, &t)?;
            }
            ExprKind::StructLit(..) | ExprKind::Variant(..) => {
                // A one-word struct or a payload-free enum.
                self.push_val(e)?;
                self.pop("%rax");
            }
            _ => return Err(format!("internal: not a scalar: {}", render(e))),
        }
        Ok(())
    }

    /// %rax = %rax / d (or % d), d a constant, without `div`: a power of two
    /// is a shift or a mask; anything else multiplies by a fixed-point
    /// reciprocal (Granlund and Montgomery, "Division by invariant integers
    /// using multiplication", 1994), as C compilers do.
    fn div_const(&mut self, d: u64, rem: bool) {
        if d.is_power_of_two() {
            let k = d.trailing_zeros();
            if rem {
                if d - 1 <= i32::MAX as u64 {
                    self.emit(&format!("and ${}, %rax", d - 1));
                } else {
                    self.emit(&format!("movabs ${}, %rcx", d - 1));
                    self.emit("and %rcx, %rax");
                }
            } else if k > 0 {
                self.emit(&format!("shr ${}, %rax", k));
            }
            return;
        }
        // An even divisor: shift first, then n < 2^(64 - k) often lets a
        // 64-bit multiplier be exact: q = mulhi(n >> k, m) >> s.
        let k = d.trailing_zeros();
        let (dd, bits) = (d >> k, 64 - k);
        for sh in 0..64u32 {
            let m = ((1u128 << (64 + sh)) + dd as u128 - 1) / dd as u128;
            if m >= 1u128 << 64 {
                break;
            }
            let err = m * dd as u128 - (1u128 << (64 + sh));
            if 64 + sh >= bits && err <= 1u128 << (64 + sh - bits) {
                if rem {
                    self.emit("mov %rax, %rsi");
                }
                if k > 0 {
                    self.emit(&format!("shr ${}, %rax", k));
                }
                self.emit(&format!("movabs ${}, %rdx", m as u64));
                self.emit("mul %rdx");
                self.emit("mov %rdx, %rax");
                if sh > 0 {
                    self.emit(&format!("shr ${}, %rax", sh));
                }
                if rem {
                    if d <= i32::MAX as u64 {
                        self.emit(&format!("imul ${}, %rax, %rax", d));
                    } else {
                        self.emit(&format!("movabs ${}, %rcx", d));
                        self.emit("imul %rcx, %rax");
                    }
                    self.emit("sub %rax, %rsi");
                    self.emit("mov %rsi, %rax");
                }
                return;
            }
        }
        // Otherwise q = floor(n * m / 2^(64 + l)) with a 65-bit m.
        if rem {
            self.emit("mov %rax, %rsi");
        }
        let l = 64 - (d - 1).leading_zeros(); // ceil(log2 d)
        let m = ((1u128 << (64 + l)) / d as u128 + 1) as u128; // in [2^64, 2^65)
        let mlow = (m - (1u128 << 64)) as u64;
        // With the 65-bit multiplier: t = mulhi(n, mlow); q = (t + ((n - t) >> 1)) >> (l - 1).
        self.emit("mov %rax, %rdi");
        self.emit(&format!("movabs ${}, %rdx", mlow));
        self.emit("mul %rdx");
        self.emit("mov %rdi, %rax");
        self.emit("sub %rdx, %rax");
        self.emit("shr $1, %rax");
        self.emit("add %rdx, %rax");
        if l > 1 {
            self.emit(&format!("shr ${}, %rax", l - 1));
        }
        if rem {
            if d <= i32::MAX as u64 {
                self.emit(&format!("imul ${}, %rax, %rax", d));
            } else {
                self.emit(&format!("movabs ${}, %rcx", d));
                self.emit("imul %rcx, %rax");
            }
            self.emit("sub %rax, %rsi");
            self.emit("mov %rsi, %rax");
        }
    }

    /// Pushes the value of `e`, whatever its size (word 0 ends on top).
    fn push_val(&mut self, e: &Expr) -> R<()> {
        let t = self.type_of(e);
        let n = words(&t)?;
        match &e.kind {
            ExprKind::Str(b) | ExprKind::Blob(b) => {
                self.strings.push(b.clone());
                let i = self.strings.len() - 1;
                self.emit(&format!("lea .Lstr{}(%rip), %rax", i));
                self.push("%rax");
                self.push(&format!("${}", b.len()));
            }
            ExprKind::Var(_) | ExprKind::Field(..) if n != 1 => {
                let (loc, _) = self.place(&place(e).expect("checked: a place"));
                self.push_from(loc, n);
            }
            ExprKind::StructLit(..) | ExprKind::Variant(..) => {
                self.reserve(n);
                let mark = self.depth;
                self.val_into(e, Dst::Stack(mark, 0))?;
            }
            ExprKind::TypeApp(_, inner) if matches!(&inner.kind, ExprKind::Call(n, _) if n == "vec_new") => {
                for _ in 0..3 {
                    self.push("$0");
                }
            }
            ExprKind::Call(name, args) if n != 1 => self.call(name, args, &t)?,
            _ => {
                if n != 1 {
                    return Err(format!("internal: cannot push {}", render(e)));
                }
                self.scalar(e)?;
                self.push("%rax");
            }
        }
        Ok(())
    }

    /// Evaluates `e` straight into `d`, building struct literals and variants
    /// field by field where they will stay instead of on the stack first.
    /// `d` must not be read by `e` (a fresh slot or reserved area).
    fn val_into(&mut self, e: &Expr, d: Dst) -> R<()> {
        match &e.kind {
            ExprKind::StructLit(sn, fs) => {
                for (f, x) in fs {
                    let (o, _) = field(&Type::Struct(sn.clone()), f);
                    self.val_into(x, d.at(o))?;
                }
            }
            ExprKind::Variant(en, v, payload) => {
                let k = enum_def(en).unwrap().index(v).unwrap();
                let at = self.dst_op(d);
                self.emit(&format!("movq ${}, {}", k, at));
                if let Some(x) = payload {
                    self.val_into(x, d.at(1))?;
                }
            }
            // An inlined body returns straight into `d`, which is storage
            // no variable names, so the body cannot see it change.
            ExprKind::Call(name, args) if words(&self.type_of(e))? > 1 && will_inline(self.inl, name, &self.stack).is_some() => {
                let f = will_inline(self.inl, name, &self.stack).unwrap();
                self.inline(f, args, Some(d))?;
            }
            _ => {
                let n = words(&self.type_of(e))?;
                if let (1, Dst::Frame(o)) = (n, d) {
                    self.scalar_into(e, &format!("{}(%rbp)", o))?;
                } else if n == 1 {
                    self.scalar(e)?;
                    let at = self.dst_op(d);
                    self.emit(&format!("mov %rax, {}", at));
                } else {
                    self.push_val(e)?;
                    for i in 0..n {
                        self.pop("%rax");
                        let at = self.dst_op(d.at(i));
                        self.emit(&format!("mov %rax, {}", at));
                    }
                }
            }
        }
        Ok(())
    }

    /// The operand for word 0 of `d` at this point of the code (it may load
    /// a pointer into %rcx).
    fn dst_op(&mut self, d: Dst) -> String {
        match d {
            Dst::Frame(o) => format!("{}(%rbp)", o),
            Dst::Stack(mark, off) => format!("{}(%rsp)", 8 * (self.depth - mark + off as i64)),
            Dst::Heap(mark, off) => {
                self.emit(&format!("mov {}(%rsp), %rcx", 8 * (self.depth - mark)));
                format!("{}(%rcx)", 8 * off)
            }
        }
    }

    /// Pushes a call argument: the address for `&mut`, else the value.
    fn arg(&mut self, a: &Expr) -> R<()> {
        if let ExprKind::MutBorrow(_) = a.kind {
            self.scalar(a)?;
            self.push("%rax");
            return Ok(());
        }
        self.push_val(a)
    }

    /// A call. A one-word result ends in %rax, a larger one on the stack.
    fn call(&mut self, name: &str, args: &[Expr], ret: &Type) -> R<()> {
        let rn = words(ret)?;
        if crate::check::is_builtin(name) {
            return self.builtin(name, args);
        }
        if let Some(f) = will_inline(self.inl, name, &self.stack) {
            return self.inline(f, args, None);
        }
        if rn > 1 {
            self.reserve(rn);
        }
        let mut aw = 0;
        for a in args {
            aw += match a.kind {
                ExprKind::MutBorrow(_) => 1,
                _ => words(&self.type_of(a))?,
            };
            self.arg(a)?;
        }
        self.emit(&format!("call {}", sym(name)));
        self.drop_words(aw);
        Ok(())
    }

    /// Inlines a call to `f` (renamed, see `inlinable`), with the result
    /// convention of `call`. The arguments are evaluated first, all of them
    /// (one may inline `f` too), then moved to `f`'s parameters, which live
    /// in registers or frame slots of the caller like its own variables; a
    /// `return` jumps to the end of the inlined body.
    /// A multi-word result goes to `dst` if given.
    fn inline(&mut self, f: &'a FnDef, args: &[Expr], dst: Option<Dst>) -> R<()> {
        let rn = words(&f.ret)?;
        if rn > 1 && dst.is_none() {
            self.reserve(rn);
        }
        let mark = self.depth;
        let dst = dst.unwrap_or(Dst::Stack(mark, 0));
        let saved_vars = self.bind_params(f, args)?;
        let end = self.label();
        let saved = (
            self.int_ops,
            std::mem::replace(&mut self.ret_label, end.clone()),
            std::mem::replace(&mut self.ret_words, rn),
            std::mem::replace(&mut self.ret_dst, Some(dst)),
        );
        self.int_ops = &f.int_ops;
        self.stack.push(f.name.clone());
        let r = self.block(&f.body);
        self.stack.pop();
        self.place_label(&end);
        (self.int_ops, self.ret_label, self.ret_words, self.ret_dst) = saved;
        self.vars = saved_vars;
        assert_eq!(self.depth, mark, "an inlined body leaves the stack as it found it");
        r
    }

    /// Branches on a call of a function inlined whose body is one
    /// `return e;`, as on `e` (no boolean result is materialized).
    fn branch_inline(&mut self, f: &'a FnDef, args: &[Expr], target: &str, when: bool) -> R<()> {
        let [Stmt { kind: StmtKind::Return(Some(e)), .. }] = f.body.as_slice() else { unreachable!("checked by the caller") };
        let saved_vars = self.bind_params(f, args)?;
        let saved = self.int_ops;
        self.int_ops = &f.int_ops;
        self.stack.push(f.name.clone());
        let r = self.branch(e, target, when);
        self.stack.pop();
        self.int_ops = saved;
        self.vars = saved_vars;
        r
    }

    /// Evaluates the arguments of an inlined call to `f` and binds `f`'s
    /// parameters to them, as the only variables; returns the caller's.
    fn bind_params(&mut self, f: &'a FnDef, args: &[Expr]) -> R<HashMap<String, Var>> {
        // A parameter that `f` never changes, given a variable of the caller
        // (or `&mut` of one), is that variable: nothing else can change it
        // while `f` runs, since only `&mut` arguments are changed.
        let mut changed = HashSet::new();
        stmts_borrows(&f.body, &mut changed);
        assigned_roots(&f.body, &mut changed);
        let mut alias = Vec::new();
        for (prm, a) in f.params.iter().zip(args) {
            let st = match &a.kind {
                _ if changed.contains(&prm.name) => None,
                ExprKind::Var(v) if !prm.ty.is_mut_ref() => Some(self.vars[v].st),
                ExprKind::MutBorrow(p) if prm.ty.is_mut_ref() => self.store_of(p),
                _ => None,
            };
            alias.push(st);
            if st.is_none() {
                self.arg(a)?;
            }
        }
        let saved_vars = std::mem::take(&mut self.vars);
        for (prm, alias) in f.params.iter().zip(alias).rev() {
            let t = prm.ty.base().clone();
            let st = if let Some(st) = alias {
                st
            } else if prm.ty.is_mut_ref() {
                let o = self.slot(1);
                self.pop_to(Loc::Mem("%rbp", o), 1);
                Store::Ptr(o)
            } else if let Some(r) = self.reg_for(&prm.name, &prm.ty) {
                self.pop(r);
                Store::Reg(r)
            } else if let Some(rs) = self.regs_for(&prm.name) {
                for r in &rs[..words(&t)?] {
                    if r.is_empty() {
                        self.drop_words(1);
                    } else {
                        self.pop(r);
                    }
                }
                Store::Regs(rs)
            } else {
                let n = words(&t)?;
                let o = self.slot(n);
                self.pop_to(Loc::Mem("%rbp", o), n);
                Store::Slot(o)
            };
            self.vars.insert(prm.name.clone(), Var { st, ty: t });
        }
        Ok(saved_vars)
    }

    /// The storage of place `p` as a variable's, if it has one: a frame
    /// slot, or the pointer of a `&mut` parameter.
    fn store_of(&self, p: &str) -> Option<Store> {
        let mut parts = p.split('.');
        let v = &self.vars[parts.next().unwrap()];
        match v.st {
            Store::Ptr(_) if !p.contains('.') => Some(v.st),
            Store::Slot(mut o) => {
                let mut t = v.ty.clone();
                for f in parts {
                    let Type::Struct(_) = t else { return None };
                    let (k, ft) = field(&t, f);
                    o += 8 * k as i64;
                    t = ft;
                }
                Some(Store::Slot(o))
            }
            _ => None,
        }
    }

    /// Pops a Buf/Bytes/Str value: %rax = len, %rdx = data.
    fn pop2(&mut self) {
        self.pop("%rax");
        self.pop("%rdx");
    }

    /// A builtin, with the result convention of `call`.
    fn builtin(&mut self, name: &str, args: &[Expr]) -> R<()> {
        match name {
            "print" => {
                self.scalar(&args[0])?;
                self.emit("mov %rax, %rdi");
                self.emit("call bst_rt_print");
            }
            "alloc" | "balloc" => {
                self.scalar(&args[0])?;
                self.push("%rax");
                self.emit("mov %rax, %rdi");
                self.emit(&format!("call bst_rt_{}", name));
                self.pop("%rcx");
                self.push("%rax");
                self.push("%rcx");
            }
            "free" | "bfree" => {
                self.load_data(&args[0], 1)?;
                self.emit("mov %rax, %rdi");
                self.emit("call bst_rt_free");
            }
            "len" | "blen" | "slen" | "vec_len" => {
                if let Some(p) = place(&args[0]) {
                    let (loc, _) = self.place(&p);
                    self.emit(&format!("mov {}, %rax", loc.op(0)));
                } else {
                    let n = words(&self.type_of(&args[0]))?;
                    self.push_val(&args[0])?;
                    self.emit("mov (%rsp), %rax");
                    self.drop_words(n);
                }
            }
            "get" | "bget" | "sget" => {
                let byte = name != "get";
                // The data pointer and the index, from their registers when
                // they live in some.
                let (mut base, mut index) = ("%rcx".to_string(), "%rax".to_string());
                if let Some(p) = place(&args[0]) {
                    match self.var_reg(&args[1]) {
                        Some(r) => index = r.to_string(),
                        None => self.scalar(&args[1])?,
                    }
                    let (loc, _) = self.place(&p);
                    match loc {
                        Loc::Regs(rs) => base = rs[1].to_string(),
                        _ => self.emit(&format!("mov {}, %rcx", loc.op(1))),
                    }
                } else {
                    self.push_val(&args[0])?;
                    self.scalar(&args[1])?;
                    self.emit("mov 8(%rsp), %rcx");
                    self.drop_words(2);
                }
                if byte {
                    self.emit(&format!("movzbl ({},{}), %eax", base, index));
                } else {
                    self.emit(&format!("mov ({},{},8), %rax", base, index));
                }
            }
            "set" => {
                self.push_val(&args[0])?;
                self.scalar(&args[1])?;
                self.push("%rax");
                self.scalar(&args[2])?;
                self.pop("%rcx");
                self.emit("mov 8(%rsp), %rdx");
                self.emit("mov %rax, (%rdx,%rcx,8)");
            }
            "bset" => {
                self.arg(&args[0])?;
                self.scalar(&args[1])?;
                self.push("%rax");
                self.scalar(&args[2])?;
                self.pop("%rcx");
                self.pop("%rdx");
                self.emit("mov 8(%rdx), %rdx");
                self.emit("mov %al, (%rdx,%rcx)");
            }
            "bfrom" => {
                self.push_val(&args[0])?;
                self.pop2();
                self.emit("mov %rax, %rdi");
                self.emit("mov %rdx, %rsi");
                self.emit("call bst_rt_bfrom");
                self.push("%rax");
                self.push("%rdx");
            }
            "serial_write" => {
                self.scalar(&args[1])?;
                self.emit("mov %rax, %rdi");
                self.emit("call bst_rt_putc");
            }
            "serial_read" => self.emit("call bst_rt_getc"),
            // Physical pages, simulated with heap memory (as in the C
            // prelude): a Frame or PageTable is the page's address.
            // The Host capability: command-line arguments and whole files.
            "host_argc" => {
                self.to_regs(args, &["%rdx"])?;
                self.emit("movslq bst_rt_argc(%rip), %rax");
            }
            "host_arg" => {
                self.to_regs(args, &["%rdx", "%rdi"])?;
                self.emit("call bst_rt_host_arg");
                self.push("%rax");
                self.push("%rdx");
            }
            "host_read" => {
                self.to_regs(args, &["%rcx", "%rdi", "%rsi", "%rdx"])?;
                self.emit("call bst_rt_host_read");
            }
            "host_write" => {
                self.to_regs(args, &["%rax", "%rdi", "%rsi", "%rdx", "%rcx"])?;
                self.emit("call bst_rt_host_write");
            }
            "pool_size" => {
                let p = place(&args[0]).expect("checked: a pool variable");
                let (loc, _) = self.place(&p);
                self.emit(&format!("mov {}, %rax", loc.op(1)));
            }
            "pool_take" | "pt_new" => {
                // A page is zeroed whenever it is handed out, so it never
                // leaks what it held before.
                if name == "pool_take" {
                    self.to_regs(args, &["%rdx", "%rax"])?;
                    self.emit("shl $12, %rax");
                    self.emit("add (%rdx), %rax");
                } else {
                    self.to_regs(args, &["%rax", "%rdx"])?;
                }
                self.emit("mov %rax, %rdx");
                self.emit("mov %rax, %rdi");
                self.emit("mov $512, %ecx");
                self.emit("xor %eax, %eax");
                self.emit("rep stosq");
                self.emit("mov %rdx, %rax");
            }
            "pool_give" => self.to_regs(args, &["%rdx", "%rax"])?,
            "pt_free" => self.to_regs(args, &["%rax"])?,
            "frame_id" => {
                self.to_regs(args, &["%rax"])?;
                self.emit("sub bst_rt_pool_base(%rip), %rax");
                self.emit("shr $12, %rax");
            }
            "frame_read" | "frame_read8" => {
                self.to_regs(args, &["%rdx", "%rax"])?;
                self.emit(if name == "frame_read" { "mov (%rdx,%rax,8), %rax" } else { "movzbl (%rdx,%rax), %eax" });
            }
            "frame_write" | "frame_write8" => {
                self.to_regs(args, &["%rdx", "%rcx", "%rax"])?;
                self.emit("mov (%rdx), %rdx");
                self.emit(if name == "frame_write" { "mov %rax, (%rdx,%rcx,8)" } else { "mov %al, (%rdx,%rcx)" });
            }
            // Bulk copies: %rdi the destination, %rsi the source, %rcx the
            // count. The prover has shown both ranges in bounds.
            "bcopy" | "bytes_to_frame" => {
                // The source is a Bytes value: its data pointer is word 1.
                for a in args {
                    self.arg(a)?;
                }
                self.pop("%rcx");
                self.pop("%rax");
                self.drop_words(1);
                self.pop("%rsi");
                self.emit("add %rax, %rsi");
                self.pop("%rax");
                self.pop("%rdi");
                self.emit(if name == "bcopy" { "mov 8(%rdi), %rdi" } else { "mov (%rdi), %rdi" });
                self.emit("add %rax, %rdi");
                self.copy_fwd();
            }
            "frame_to_bytes" => {
                self.to_regs(args, &["%rdi", "%rax", "%rsi", "%rdx", "%rcx"])?;
                self.emit("mov 8(%rdi), %rdi");
                self.emit("add %rax, %rdi");
                self.emit("add %rdx, %rsi");
                self.copy_fwd();
            }
            "bmove" => {
                // Backward, a byte at a time, when the destination starts
                // inside the source (memmove semantics).
                self.to_regs(args, &["%rdx", "%rdi", "%rsi", "%rcx"])?;
                self.emit("mov 8(%rdx), %rdx");
                self.emit("add %rdx, %rdi");
                self.emit("add %rdx, %rsi");
                let (back, done) = (self.label(), self.label());
                self.emit("mov %rdi, %rax");
                self.emit("sub %rsi, %rax");
                self.emit("cmp %rcx, %rax");
                self.emit(&format!("jb {}", back));
                self.copy_fwd();
                self.emit(&format!("jmp {}", done));
                self.place_label(&back);
                self.emit("dec %rcx");
                self.emit("movzbl (%rsi,%rcx), %eax");
                self.emit("mov %al, (%rdi,%rcx)");
                self.emit(&format!("jnz {}", back));
                self.place_label(&done);
            }
            "pt_has" => {
                self.to_regs(args, &["%rdx", "%rax"])?;
                self.emit("mov (%rdx,%rax,8), %rax");
                self.emit("and $1, %eax");
            }
            "pt_writable" => {
                self.to_regs(args, &["%rdx", "%rax"])?;
                self.emit("mov (%rdx,%rax,8), %rax");
                self.emit("shr $1, %eax");
                self.emit("and $1, %eax");
            }
            "pt_put" | "pt_put_frame" => {
                // Present, user, and writable (a frame only when asked).
                if name == "pt_put" {
                    self.to_regs(args, &["%rdx", "%rcx", "%rax"])?;
                    self.emit("or $7, %rax");
                } else {
                    self.to_regs(args, &["%rdx", "%rcx", "%rax", "%rsi"])?;
                    self.emit("shl $1, %rsi");
                    self.emit("or %rsi, %rax");
                    self.emit("or $5, %rax");
                }
                self.emit("mov (%rdx), %rdx");
                self.emit("mov %rax, (%rdx,%rcx,8)");
            }
            "pt_take" | "pt_take_frame" => {
                self.to_regs(args, &["%rdx", "%rcx"])?;
                self.emit("mov (%rdx), %rdx");
                self.emit("lea (%rdx,%rcx,8), %rdx");
                self.emit("mov (%rdx), %rax");
                self.emit("movq $0, (%rdx)");
                self.emit("movabs $0x000FFFFFFFFFF000, %rcx");
                self.emit("and %rcx, %rax");
            }

            "box" => {
                let t = self.type_of(&args[0]);
                let n = words(&t)?;
                // Allocate before evaluating the contents, as the C backend
                // does, so a tree is laid out parent first.
                self.emit(&format!("mov ${}, %edi", n));
                self.emit("call bst_rt_alloc");
                self.push("%rax");
                let mark = self.depth;
                self.val_into(&args[0], Dst::Heap(mark, 0))?;
                self.pop("%rax");
            }
            "unbox" => {
                let Type::Box(inner) = self.type_of(&args[0]) else { unreachable!("checked") };
                let n = words(&inner)?;
                self.scalar(&args[0])?;
                self.reserve(n);
                for i in 0..n {
                    self.emit(&format!("mov {}(%rax), %rdx", 8 * i));
                    self.emit(&format!("mov %rdx, {}(%rsp)", 8 * i));
                }
                self.emit("mov %rax, %rdi");
                self.emit("call bst_rt_free");
                if n == 1 {
                    self.pop("%rax");
                }
            }
            "vec_push" if words(&self.vec_elem(&args[0]))? == 1 && matches!(args[0].kind, ExprKind::MutBorrow(_)) => {
                // The value cannot change the vector (it is lent), so it
                // goes first, into %rdx, and the vector's address into %rdi.
                let l = self.label();
                self.scalar(&args[1])?;
                self.emit("mov %rax, %rdx");
                self.scalar(&args[0])?;
                self.emit("mov %rax, %rdi");
                self.emit("mov (%rdi), %rax");
                self.emit("cmp 8(%rdi), %rax");
                self.emit(&format!("jne {}", l));
                self.push("%rdx");
                self.push("%rdi");
                self.emit("mov $8, %esi");
                self.emit("call bst_rt_vec_grow");
                self.pop("%rdi");
                self.pop("%rdx");
                self.emit("mov (%rdi), %rax");
                self.place_label(&l);
                self.emit("mov 16(%rdi), %rcx");
                self.emit("mov %rdx, (%rcx,%rax,8)");
                self.emit("incq (%rdi)");
            }
            "vec_push" => {
                let n = words(&self.vec_elem(&args[0]))?;
                self.arg(&args[0])?;
                self.push_val(&args[1])?;
                let l = self.label();
                self.emit(&format!("mov {}(%rsp), %rdi", 8 * n));
                self.emit("mov (%rdi), %rax");
                self.emit("cmp 8(%rdi), %rax");
                self.emit(&format!("jne {}", l));
                self.emit(&format!("mov ${}, %esi", 8 * n));
                self.emit("call bst_rt_vec_grow");
                self.emit(&format!("mov {}(%rsp), %rdi", 8 * n));
                self.emit("mov (%rdi), %rax");
                self.place_label(&l);
                self.elem_addr(n);
                self.pop_to(Loc::Mem("%rcx", 0), n);
                self.emit("incq (%rdi)");
                self.drop_words(1);
            }
            "vec_pop" => {
                let n = words(&self.vec_elem(&args[0]))?;
                self.scalar(&args[0])?;
                self.emit("mov %rax, %rdi");
                self.emit("decq (%rdi)");
                self.emit("mov (%rdi), %rax");
                self.elem_addr(n);
                if n == 1 {
                    self.emit("mov (%rcx), %rax");
                } else {
                    self.push_from(Loc::Mem("%rcx", 0), n);
                }
            }
            "vec_get" => {
                let n = words(&self.vec_elem(&args[0]))?;
                let p = place(&args[0]).expect("checked: names a vector");
                let index = match self.var_reg(&args[1]) {
                    Some(r) if n == 1 => r,
                    _ => {
                        self.scalar(&args[1])?;
                        "%rax"
                    }
                };
                let (loc, _) = self.place(&p);
                if n == 1 {
                    let base = match loc {
                        Loc::Regs(rs) => rs[2].to_string(),
                        _ => {
                            self.emit(&format!("mov {}, %rcx", loc.op(2)));
                            "%rcx".to_string()
                        }
                    };
                    self.emit(&format!("mov ({},{},8), %rax", base, index));
                    return Ok(());
                }
                self.emit(&format!("mov {}, %rcx", loc.op(2)));
                if n == 1 {
                    self.emit("mov (%rcx,%rax,8), %rax");
                } else {
                    self.emit(&format!("imul ${}, %rax, %rax", 8 * n));
                    self.emit("add %rax, %rcx");
                    self.push_from(Loc::Mem("%rcx", 0), n);
                }
            }
            "vec_set" | "vec_swap" => {
                let n = words(&self.vec_elem(&args[0]))?;
                self.arg(&args[0])?;
                self.scalar(&args[1])?;
                self.push("%rax");
                self.push_val(&args[2])?;
                self.emit(&format!("mov {}(%rsp), %rax", 8 * n));
                self.emit(&format!("mov {}(%rsp), %rdi", 8 * n + 8));
                self.elem_addr(n);
                if name == "vec_set" {
                    self.pop_to(Loc::Mem("%rcx", 0), n);
                    self.drop_words(2);
                } else {
                    // Exchange the new value on the stack with the old one,
                    // then slide it over the vector and index.
                    for i in 0..n {
                        self.emit(&format!("mov {}(%rcx), %rax", 8 * i));
                        self.emit(&format!("mov {}(%rsp), %rdx", 8 * i));
                        self.emit(&format!("mov %rdx, {}(%rcx)", 8 * i));
                        self.emit(&format!("mov %rax, {}(%rsp)", 8 * i));
                    }
                    for i in (0..n).rev() {
                        self.emit(&format!("mov {}(%rsp), %rax", 8 * i));
                        self.emit(&format!("mov %rax, {}(%rsp)", 8 * i + 16));
                    }
                    self.drop_words(2);
                    if n == 1 {
                        self.pop("%rax");
                    }
                }
            }
            "vec_free" => {
                self.load_data(&args[0], 2)?;
                self.emit("mov %rax, %rdi");
                self.emit("call bst_rt_free");
            }
            _ => return Err(format!("the native backend does not support `{}` yet", name)),
        }
        Ok(())
    }

    /// Evaluates the arguments left to right, then loads their words into
    /// `regs`, in order (a two-word argument takes its data word, then its
    /// length).
    fn to_regs(&mut self, args: &[Expr], regs: &[&str]) -> R<()> {
        for a in args {
            self.arg(a)?;
        }
        for r in regs.iter().rev() {
            self.pop(r);
        }
        Ok(())
    }

    /// Copies %rcx bytes from (%rsi) to (%rdi), lowest first: whole words
    /// with rep movsq, then the rest with rep movsb (the C prelude's
    /// bst_copy_fwd).
    fn copy_fwd(&mut self) {
        self.emit("mov %rcx, %rax");
        self.emit("shr $3, %rcx");
        self.emit("rep movsq");
        self.emit("mov %eax, %ecx");
        self.emit("and $7, %ecx");
        self.emit("rep movsb");
    }

    /// %rcx = the address of element %rax of the vector at (%rdi), for
    /// elements of `n` words.
    fn elem_addr(&mut self, n: usize) {
        self.emit("mov 16(%rdi), %rcx");
        if n == 1 {
            self.emit("lea (%rcx,%rax,8), %rcx");
        } else {
            self.emit(&format!("imul ${}, %rax, %rax", 8 * n));
            self.emit("add %rax, %rcx");
        }
    }

    /// %rax = word `w` of a buffer or vector value (its data pointer).
    fn load_data(&mut self, e: &Expr, w: usize) -> R<()> {
        if let Some(p) = place(e) {
            let (loc, _) = self.place(&p);
            self.emit(&format!("mov {}, %rax", loc.op(w)));
        } else {
            let n = words(&self.type_of(e))?;
            self.push_val(e)?;
            self.emit(&format!("mov {}(%rsp), %rax", 8 * w));
            self.drop_words(n);
        }
        Ok(())
    }
}

/// The runtime of the native backend on the hosted target: assembly over
/// the C library. Each routine keeps %rbx and %r12-%r15 (the C library does
/// too) and aligns the stack itself, since Bastio code does not keep it
/// aligned for C calls.
pub const RUNTIME: &str = r#"
# ---- runtime (trusted base) ----
bst_rt_print:                   # print(%rdi) as a decimal line
	push %rbp
	mov %rsp, %rbp
	and $-16, %rsp
	mov %rdi, %rsi
	lea .Lrt_fmt(%rip), %rdi
	xor %eax, %eax
	call printf@PLT
	leave
	ret
bst_rt_putc:                    # write byte %rdi to stdout; flush at a newline
	push %rbp
	mov %rsp, %rbp
	push %rbx
	push %rbx
	mov %rdi, %rbx
	movzbl %dil, %edi
	call putchar@PLT
	cmp $10, %rbx
	jne 1f
	mov stdout@GOTPCREL(%rip), %rax
	mov (%rax), %rdi
	call fflush@PLT
1:	mov -8(%rbp), %rbx
	leave
	ret
bst_rt_getc:                    # %rax = next byte of stdin, 4 at the end
	push %rbp
	mov %rsp, %rbp
	and $-16, %rsp
	mov stdout@GOTPCREL(%rip), %rax
	mov (%rax), %rdi
	call fflush@PLT
	call getchar@PLT
	cmp $-1, %eax
	jne 1f
	mov $4, %eax
1:	cltq
	leave
	ret
bst_rt_alloc:                   # %rax = %rdi zeroed words (at least one)
	mov $8, %esi
	jmp 1f
bst_rt_balloc:                  # %rax = %rdi zeroed bytes (at least one)
	mov $1, %esi
1:	push %rbp
	mov %rsp, %rbp
	and $-16, %rsp
	test %rdi, %rdi
	jnz 2f
	mov $1, %edi
2:	call calloc@PLT
	test %rax, %rax
	jz bst_rt_oom
	leave
	ret
bst_rt_free:                    # free(%rdi)
	push %rbp
	mov %rsp, %rbp
	and $-16, %rsp
	call free@PLT
	leave
	ret
bst_rt_vec_grow:                # grow the Vec at (%rdi), elements of %rsi bytes
	push %rbp
	mov %rsp, %rbp
	push %rbx
	push %r12
	mov %rdi, %rbx
	mov 8(%rbx), %r12           # new capacity: double, or 4 when empty
	add %r12, %r12
	jnz 1f
	mov $4, %r12d
1:	mov %r12, %rax
	mul %rsi                    # bytes; past 64 bits the request cannot be met
	jo bst_rt_oom
	mov %rax, %rsi
	mov 16(%rbx), %rdi
	call realloc@PLT
	test %rax, %rax
	jz bst_rt_oom
	mov %rax, 16(%rbx)
	mov %r12, 8(%rbx)
	mov -8(%rbp), %rbx
	mov -16(%rbp), %r12
	leave
	ret
bst_rt_bfrom:                   # Bytes copied from the %rdi bytes at %rsi: %rax = data, %rdx = len
	push %rbp
	mov %rsp, %rbp
	push %rbx
	push %r12
	mov %rdi, %rbx
	mov %rsi, %r12
	call bst_rt_balloc
	mov %rax, %rdi
	mov %r12, %rsi
	mov %rbx, %rcx
	rep movsb
	mov %rbx, %rdx
	mov -8(%rbp), %rbx
	mov -16(%rbp), %r12
	leave
	ret
bst_rt_host_arg:                # argument %rdi as Bytes: %rax = data, %rdx = len
	push %rbp
	mov %rsp, %rbp
	push %rbx
	push %r12
	and $-16, %rsp
	movslq bst_rt_argc(%rip), %rax
	cmp %rax, %rdi
	jb 1f
	xor %edi, %edi              # past the end: empty
	call bst_rt_balloc
	xor %edx, %edx
	jmp 2f
1:	mov bst_rt_argv(%rip), %rax
	mov (%rax,%rdi,8), %rbx
	mov %rbx, %rdi
	call strlen@PLT
	mov %rax, %r12
	mov %rax, %rdi
	call bst_rt_balloc
	mov %rax, %rdi
	mov %rbx, %rsi
	mov %r12, %rcx
	rep movsb
	mov %r12, %rdx
2:	mov -8(%rbp), %rbx
	mov -16(%rbp), %r12
	leave
	ret
bst_rt_cpath:                   # NUL-terminated copy of the %rsi bytes at %rdi (a NUL inside ends the path)
	push %rbp
	mov %rsp, %rbp
	push %rbx
	push %r12
	and $-16, %rsp
	mov %rdi, %rbx
	mov %rsi, %r12
	lea 1(%rsi), %rdi
	call malloc@PLT
	test %rax, %rax
	jz bst_rt_oom
	mov %rax, %rdi
	mov %rbx, %rsi
	mov %r12, %rcx
	rep movsb
	movb $0, (%rdi)
	mov -8(%rbp), %rbx
	mov -16(%rbp), %r12
	leave
	ret
bst_rt_host_read:               # read the file named by %rdi/%rsi into the Bytes at (%rdx); %rax = 1 if it worked
	push %rbp
	mov %rsp, %rbp
	push %rbx
	push %r12
	push %r13
	push %r14
	push %r15
	and $-16, %rsp
	mov %rdx, %rbx
	call bst_rt_cpath
	mov %rax, %r13
	mov %rax, %rdi
	xor %esi, %esi              # O_RDONLY
	xor %eax, %eax
	call open@PLT
	movslq %eax, %r12
	mov %r13, %rdi
	call free@PLT
	test %r12, %r12
	js 8f
	mov $4096, %r15d            # capacity, doubled when full
	mov %r15, %rdi
	call malloc@PLT
	test %rax, %rax
	jz bst_rt_oom
	mov %rax, %r13
	xor %r14d, %r14d            # bytes read
1:	mov %r12, %rdi
	lea (%r13,%r14), %rsi
	mov %r15, %rdx
	sub %r14, %rdx
	call read@PLT
	test %rax, %rax
	js 3f
	jz 4f
	add %rax, %r14
	cmp %r15, %r14
	jne 1b
	add %r15, %r15
	mov %r13, %rdi
	mov %r15, %rsi
	call realloc@PLT
	test %rax, %rax
	jz bst_rt_oom
	mov %rax, %r13
	jmp 1b
3:	mov %r12, %rdi              # a read error: nothing changes
	call close@PLT
	mov %r13, %rdi
	call free@PLT
	jmp 8f
4:	mov %r12, %rdi
	call close@PLT
	mov 8(%rbx), %rdi           # replace the old contents
	call free@PLT
	mov %r14, (%rbx)
	mov %r13, 8(%rbx)
	mov $1, %eax
	jmp 9f
8:	xor %eax, %eax
9:	mov -8(%rbp), %rbx
	mov -16(%rbp), %r12
	mov -24(%rbp), %r13
	mov -32(%rbp), %r14
	mov -40(%rbp), %r15
	leave
	ret
bst_rt_host_write:              # write the %rcx bytes at %rdx to the file named by %rdi/%rsi; %rax = 1 if it worked
	push %rbp
	mov %rsp, %rbp
	push %rbx
	push %r12
	push %r13
	push %r14
	push %r15
	and $-16, %rsp
	mov %rdx, %rbx
	mov %rcx, %r14
	call bst_rt_cpath
	mov %rax, %r13
	mov %rax, %rdi
	mov $0x241, %esi            # O_WRONLY | O_CREAT | O_TRUNC
	mov $0666, %edx
	xor %eax, %eax
	call open@PLT
	movslq %eax, %r12
	mov %r13, %rdi
	call free@PLT
	test %r12, %r12
	js 8f
	xor %r15d, %r15d            # bytes written
1:	cmp %r14, %r15
	jae 4f
	mov %r12, %rdi
	lea (%rbx,%r15), %rsi
	mov %r14, %rdx
	sub %r15, %rdx
	call write@PLT
	test %rax, %rax
	jle 3f
	add %rax, %r15
	jmp 1b
3:	mov %r12, %rdi
	call close@PLT
	jmp 8f
4:	mov %r12, %rdi
	call close@PLT
	test %eax, %eax
	jnz 8f
	mov $1, %eax
	jmp 9f
8:	xor %eax, %eax
9:	mov -8(%rbp), %rbx
	mov -16(%rbp), %r12
	mov -24(%rbp), %r13
	mov -32(%rbp), %r14
	mov -40(%rbp), %r15
	leave
	ret
bst_rt_pool_init:               # the simulated pool: %rax = base, %rdx = pages
	push %rbp
	mov %rsp, %rbp
	and $-16, %rsp
	mov $4096, %edi
	mov $1048576, %esi          # 256 pages
	call aligned_alloc@PLT
	test %rax, %rax
	jz bst_rt_oom
	mov %rax, bst_rt_pool_base(%rip)
	mov $256, %edx
	leave
	ret
bst_rt_oom:
	and $-16, %rsp
	mov $2, %edi
	lea .Lrt_oom(%rip), %rsi
	mov $22, %edx
	call write@PLT
	call abort@PLT
	.bss
	.p2align 3
bst_rt_argc:	.quad 0
bst_rt_argv:	.quad 0
bst_rt_pool_base:	.quad 0
	.section .rodata
.Lrt_fmt:
	.string "%llu\n"
.Lrt_oom:
	.string "Bastio: out of memory\n"
	.text
"#;
