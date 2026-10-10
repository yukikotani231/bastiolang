// Verification-condition generation by symbolic execution.
//
// Every u64 is an SMT integer constrained to [0, 2^64). Every Buf is a pair
// (len: Int, data: Array Int Int). The executor walks each function once,
// keeping a path condition, and at every place where the program could go
// wrong it asks the solver whether the bad case is reachable:
//
//   bounds     get/set index < len
//   overflow   + and * stay below 2^64
//   underflow  a - b with a >= b
//   divzero    divisor != 0
//   alloc      allocation size is sane
//   requires   a callee's precondition holds at the call
//   ensures    our postcondition holds at every return
//   invariant  a loop invariant holds on entry and is preserved
//   assert     an explicit `assert`
//
// Calls are modular: only the callee's requires/ensures are used, never its
// body. Loops are cut with their invariants; variables the body assigns are
// havocked. Small inference keeps annotations down: a buffer that the body
// only ever updates with `x = set(x, ..)` keeps its length across the loop,
// so `len(x)` facts survive without being restated in the invariant.
//
// Linear objects carry ghost state that exists only in the proof:
//   Frame      id (its physical page number, < POOLSIZE <= FRAME_LIMIT)
//   PageTable  id, level, used[i] (which of its 512 slots own something),
//              leases (how many processors may be running on its tree:
//              while it is not 0 the tree only grows, see `mmu_enter`)
//   Pool       has[i] (the page numbers it can still hand out), size
//   Mmu        root (the id of the table in CR3; KROOT for the kernel's own)
//   Ports      free[p] (the I/O ports nobody has claimed yet)
//   MmioSpace  free[a] (the device-window addresses nobody has claimed)
//   Mmio       base, len (the device registers base .. base+len it owns,
//              always inside the device window)
//   Dma        id (the page number of the page it lends to devices)
//   PortIo     base, count (the ports base .. base+count it owns; always
//              within the 65536 ports)
// A struct value is the values of its fields; places like `p.root` read and
// write inside it.
// The builtins' preconditions over this state (see `builtin`) are what make
// page ownership, page-table structure and CR3 switching safe.
//
// `&mut x` arguments: the callee's `requires` see x's value at the call; after
// the call x gets a fresh value, constrained by the callee's `ensures`, where
// `x` means the new value and `old(..)` the value at the call, and by the
// callee's summary (see `summarize`): the parts of x that the callee's body
// provably leaves alone, or changes only in ways that keep their shape, keep
// what is known about them. So a driver function taking `&mut Dev` does not
// have to restate `port_count(d.io) == 8` in its `ensures`, nor its callers'
// loops in their invariants.

use crate::ast::*;
use crate::check::{is_builtin, spec_place};
use crate::diag::{Diag, Pos};
use crate::smt::{SatResult, Solver};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

const U64_MAX: &str = "18446744073709551615";
/// Largest element count `alloc` accepts (2^60): keeps `n * 8` far from overflow.
const ALLOC_MAX: &str = "1152921504606846976";
/// Page numbers are below this: 2^16 pages, 256 MiB of physical memory.
const FRAME_LIMIT: &str = "65536";
/// `mmu_root` while CR3 holds the kernel's own tables (no Frame has this id).
const KROOT: &str = "65536";
/// There is one pool per program, so its size is one global ghost constant.
/// Every page number is below it: pages only ever come from `pool_take`.
const POOLSIZE: &str = "POOLSIZE";
/// The number of x86 I/O ports: a PortIo's ports are all below it.
const PORT_LIMIT: &str = "65536";
/// The device window a `Mmio` lies in: physical 0x8000_0000 up to 2^46
/// (64 TiB, where firmware puts 64-bit PCI BARs), which the platform maps
/// uncached (and which holds no pool page, since pages stay below
/// FRAME_LIMIT).
const MMIO_LO: &str = "2147483648";
const MMIO_HI: &str = "70368744177664";
const NO_BOOL: &str = "((as const (Array Int Bool)) false)";

#[derive(Clone, Debug, PartialEq)]
enum Val {
    U(String),
    B(String),
    Buf { len: String, data: String },
    /// A string literal: its bytes as an SMT array, so proofs know them exactly.
    Str { len: String, data: String },
    /// A linear object; its ghost fields follow `ghost_fields(type)`.
    Obj(Type, Vec<String>),
    /// A struct: its fields' values in declaration order.
    Struct(String, Vec<Val>),
    /// An enum: its tag (the variant's index) and one payload slot per
    /// variant (`Unit` for variants without one). Only the slot the tag
    /// names means anything.
    Enum(String, String, Vec<Val>),
    /// A box: its contents, or (beyond the depth the verifier unfolds
    /// recursive data to) only their type.
    Box(BoxVal),
    /// A vector: its length, and its elements "lifted": a value shaped like
    /// one element whose every term is an SMT array from index to that term
    /// (see `leaves`). Boxes inside the elements are opaque.
    Vec { elem: Type, len: String, elems: Box<Val> },
    Unit,
}

#[derive(Clone, Debug, PartialEq)]
enum BoxVal {
    Known(Box<Val>),
    /// Contents nothing is known about yet; they are given fresh values the
    /// first time code looks inside.
    Opaque(Type),
}

/// How many boxes deep a fresh value is unfolded.
const BOX_DEPTH: usize = 2;

/// Looks through boxes: the value a box holds (boxes on a place's path are
/// opened by `open_path` before they are read).
fn unbox_known(mut v: &Val) -> &Val {
    while let Val::Box(BoxVal::Known(i)) = v {
        v = i;
    }
    v
}

/// The value at a dotted path.
fn get_path(env: &HashMap<String, Val>, path: &str) -> Val {
    let mut parts = path.split('.');
    let mut v = &env[parts.next().unwrap()];
    for f in parts {
        let Val::Struct(n, fs) = unbox_known(v) else { unreachable!("checked: a field of a struct (boxes opened)") };
        let k = struct_def(n).unwrap().fields.iter().position(|x| x.0 == f).expect("checked field");
        v = &fs[k];
    }
    v.clone()
}

/// Replaces the value at a dotted path.
fn set_path(env: &mut HashMap<String, Val>, path: &str, nv: Val) {
    fn go(v: &mut Val, rest: &[&str], nv: Val) {
        if rest.is_empty() {
            *v = nv;
            return;
        }
        let mut v = v;
        while let Val::Box(BoxVal::Known(i)) = v {
            v = &mut **i;
        }
        let Val::Struct(n, fs) = v else { unreachable!("checked: a field of a struct (boxes opened)") };
        let k = struct_def(n).unwrap().fields.iter().position(|x| x.0 == rest[0]).expect("checked field");
        go(&mut fs[k], &rest[1..], nv);
    }
    let parts: Vec<&str> = path.split('.').collect();
    go(env.get_mut(parts[0]).expect("known variable"), &parts[1..], nv);
}

/// `if c then a else b`, field by field.
fn merge_val(c: &str, a: &Val, b: &Val) -> Val {
    match (a, b) {
        (Val::U(x), Val::U(y)) => Val::U(ite(c, x, y)),
        (Val::B(x), Val::B(y)) => Val::B(ite(c, x, y)),
        (Val::Buf { len: l1, data: d1 }, Val::Buf { len: l2, data: d2 }) => Val::Buf { len: ite(c, l1, l2), data: ite(c, d1, d2) },
        (Val::Str { len: l1, data: d1 }, Val::Str { len: l2, data: d2 }) => Val::Str { len: ite(c, l1, l2), data: ite(c, d1, d2) },
        (Val::Obj(ty, f1), Val::Obj(_, f2)) => Val::Obj(ty.clone(), f1.iter().zip(f2).map(|(x, y)| ite(c, x, y)).collect()),
        (Val::Struct(n, f1), Val::Struct(_, f2)) => Val::Struct(n.clone(), f1.iter().zip(f2).map(|(x, y)| merge_val(c, x, y)).collect()),
        (Val::Enum(n, t1, p1), Val::Enum(_, t2, p2)) => Val::Enum(n.clone(), ite(c, t1, t2), p1.iter().zip(p2).map(|(x, y)| merge_val(c, x, y)).collect()),
        (Val::Vec { elem, len: l1, elems: e1 }, Val::Vec { len: l2, elems: e2, .. }) => Val::Vec { elem: elem.clone(), len: ite(c, l1, l2), elems: Box::new(merge_val(c, e1, e2)) },
        (Val::Box(BoxVal::Known(x)), Val::Box(BoxVal::Known(y))) => Val::Box(BoxVal::Known(Box::new(merge_val(c, x, y)))),
        // Forgetting what one side knew is sound.
        (Val::Box(BoxVal::Opaque(t)), Val::Box(_)) | (Val::Box(_), Val::Box(BoxVal::Opaque(t))) => Val::Box(BoxVal::Opaque(t.clone())),
        (Val::Unit, Val::Unit) => Val::Unit,
        _ => unreachable!("type checker guarantees matching kinds"),
    }
}

/// The SMT terms a value is made of, with their sorts, in a fixed order that
/// depends only on its type. Box contents are left out. A vector's element
/// terms are arrays one level deeper than the element's own sorts.
fn leaves(v: &Val, out: &mut Vec<(String, String)>) {
    match v {
        Val::U(t) => out.push((t.clone(), "Int".into())),
        Val::B(t) => out.push((t.clone(), "Bool".into())),
        Val::Buf { len, data } | Val::Str { len, data } => {
            out.push((len.clone(), "Int".into()));
            out.push((data.clone(), "(Array Int Int)".into()));
        }
        Val::Obj(t, fs) => {
            for ((_, arr, _), f) in ghost_fields(t).iter().zip(fs) {
                out.push((f.clone(), if *arr { "(Array Int Bool)" } else { "Int" }.into()));
            }
        }
        Val::Struct(_, fs) => fs.iter().for_each(|f| leaves(f, out)),
        Val::Enum(_, tag, ps) => {
            out.push((tag.clone(), "Int".into()));
            ps.iter().for_each(|p| leaves(p, out));
        }
        Val::Vec { len, elems, .. } => {
            out.push((len.clone(), "Int".into()));
            let mut inner = Vec::new();
            leaves(elems, &mut inner);
            out.extend(inner.into_iter().map(|(t, s)| (t, format!("(Array Int {})", s))));
        }
        Val::Box(_) | Val::Unit => {}
    }
}

/// The value shaped like `v` whose terms are `f` applied to `v`'s terms
/// (in the order of `leaves`).
fn map_leaves(v: &Val, f: &mut dyn FnMut(&str) -> String) -> Val {
    match v {
        Val::U(t) => Val::U(f(t)),
        Val::B(t) => Val::B(f(t)),
        Val::Buf { len, data } => Val::Buf { len: f(len), data: f(data) },
        Val::Str { len, data } => Val::Str { len: f(len), data: f(data) },
        Val::Obj(t, fs) => Val::Obj(t.clone(), fs.iter().map(|x| f(x)).collect()),
        Val::Struct(n, fs) => Val::Struct(n.clone(), fs.iter().map(|x| map_leaves(x, f)).collect()),
        Val::Enum(n, tag, ps) => {
            let tag = f(tag);
            Val::Enum(n.clone(), tag, ps.iter().map(|x| map_leaves(x, f)).collect())
        }
        Val::Vec { elem, len, elems } => {
            let len = f(len);
            Val::Vec { elem: elem.clone(), len, elems: Box::new(map_leaves(elems, f)) }
        }
        Val::Box(b) => Val::Box(b.clone()),
        Val::Unit => Val::Unit,
    }
}

/// `elems` with `x` stored at index `i`.
fn vec_write(elems: &Val, i: &str, x: &Val) -> Val {
    let mut xs = Vec::new();
    leaves(x, &mut xs);
    let mut it = xs.into_iter();
    map_leaves(elems, &mut |arr| format!("(store {} {} {})", arr, i, it.next().unwrap().0))
}

/// Makes every box in `v` (of type `ty`) opaque: a lifted template must not
/// claim the same box contents for every index.
fn opaque_boxes(v: &Val, ty: &Type) -> Val {
    match (v, ty) {
        (Val::Struct(n, fs), _) => Val::Struct(n.clone(), fs.iter().zip(&struct_def(n).unwrap().fields).map(|(f, (_, ft))| opaque_boxes(f, ft)).collect()),
        (Val::Enum(n, tag, ps), _) => Val::Enum(
            n.clone(),
            tag.clone(),
            ps.iter().zip(&enum_def(n).unwrap().variants).map(|(p, (_, pt))| match pt {
                Some(pt) => opaque_boxes(p, pt),
                None => Val::Unit,
            }).collect(),
        ),
        (Val::Box(_), Type::Box(t)) => Val::Box(BoxVal::Opaque((**t).clone())),
        (v, _) => v.clone(),
    }
}

/// The type of a value, for `lock_new`. A `Bytes` reads as `Buf`: the
/// lock then forgets that every byte is <= 255, which loses facts but never
/// assumes a false one.
fn val_type(v: &Val) -> Type {
    match v {
        Val::U(_) => Type::U64,
        Val::B(_) => Type::Bool,
        Val::Buf { .. } => Type::Buf,
        Val::Str { .. } => Type::Str,
        Val::Obj(t, _) => t.clone(),
        Val::Struct(n, _) => Type::Struct(n.clone()),
        Val::Enum(n, _, _) => Type::Enum(n.clone()),
        Val::Box(BoxVal::Known(i)) => Type::Box(Box::new(val_type(i))),
        Val::Box(BoxVal::Opaque(t)) => Type::Box(Box::new(t.clone())),
        Val::Vec { elem, .. } => Type::Vec(Box::new(elem.clone())),
        Val::Unit => Type::Unit,
    }
}

impl Val {
    fn term(&self) -> &str {
        match self {
            Val::U(t) | Val::B(t) => t,
            _ => panic!("not a scalar"),
        }
    }
    fn field(&self, name: &str) -> String {
        match self {
            Val::Obj(t, fs) => fs[ghost_fields(t).iter().position(|f| f.0 == name).expect("known ghost field")].clone(),
            _ => panic!("not an object"),
        }
    }
}

/// Ghost fields of each object type: (name, is an array, accessor shown in counterexamples).
fn ghost_fields(t: &Type) -> &'static [(&'static str, bool, &'static str)] {
    match t {
        Type::Frame => &[("id", false, "frame_id")],
        Type::PageTable => &[("id", false, "pt_id"), ("level", false, "pt_level"), ("used", true, ""), ("leases", false, "pt_leases")],
        Type::Pool => &[("has", true, ""), ("size", false, "pool_size")],
        Type::Mmu => &[("root", false, "mmu_root")],
        Type::Ports => &[("free", true, "")],
        Type::PortIo => &[("base", false, "port_base"), ("count", false, "port_count")],
        Type::MmioSpace => &[("free", true, "")],
        Type::Mmio => &[("base", false, "mmio_base"), ("len", false, "mmio_len")],
        Type::Dma => &[("id", false, "dma_id")],
        _ => &[],
    }
}

/// Ghost fields a loop keeps when the object is changed only by builtins
/// that never change them (a page table keeps its id and level).
fn kept_by_builtins(t: &Type, field: &str) -> bool {
    matches!((t, field), (Type::Frame, "id") | (Type::PageTable, "id") | (Type::PageTable, "level") | (Type::PageTable, "leases") | (Type::Pool, "size") | (Type::PortIo, "base") | (Type::PortIo, "count") | (Type::Mmio, "base") | (Type::Mmio, "len") | (Type::Dma, "id"))
}

/// Builtins whose `&mut` argument keeps the fields above.
const SHAPE_KEEPING: &[&str] =
    &["bset", "vec_set", "vec_swap", "pt_put", "pt_take", "pt_put_frame", "pt_take_frame", "pt_borrow", "pt_borrow_frame", "pt_lease_count", "pool_take", "pool_give", "frame_write", "frame_write8", "cpu_set", "cpu_run", "cpu_idle", "cpu_fpu_save", "cpu_fpu_load",
      "pio_in8", "pio_in16", "pio_in32", "pio_out8", "pio_out16", "pio_out32",
      "mmio_read8", "mmio_read16", "mmio_read32", "mmio_read64", "mmio_write8", "mmio_write16", "mmio_write32", "mmio_write64",
      "dma_read8", "dma_read16", "dma_read32", "dma_read64", "dma_write8", "dma_write16", "dma_write32", "dma_write64"];

/// What a call may do to one `&mut` argument: the places under it that may
/// change, as paths relative to the parameter (`""` is the parameter itself,
/// `".f.g"` a field of it), each with the `changed_vars` flag (true: changed
/// only in ways that keep the shape). The parameter itself is always listed.
type Summary = Vec<(String, bool)>;

/// Each function's summary for each of its parameters (`None` for those
/// that are not `&mut`).
type Summaries = HashMap<String, Vec<Option<Summary>>>;

/// Places deeper than this are summarized by their prefix of this depth,
/// marked as changed in any way. Recursion through boxes could otherwise
/// grow paths forever.
const SUMMARY_DEPTH: usize = 8;

/// Computes every function's summaries from its body: what `changed_vars`
/// finds under each `&mut` parameter, where lending a place to another user
/// function counts as that function's summary applied to the place.
///
/// This is a greatest fixed point: it starts from "every parameter keeps its
/// shape" and weakens summaries until none changes. Sound for recursion too:
/// a change that is not shape-keeping happens at some assignment or builtin
/// call, which is recorded there and then propagates to every caller.
fn summarize(p: &Program) -> Summaries {
    let mut sums: Summaries = HashMap::new();
    for f in &p.fns {
        let ps = f.params.iter().map(|q| q.ty.is_mut_ref().then(|| vec![(String::new(), true)])).collect();
        sums.insert(f.name.clone(), ps);
    }
    loop {
        let mut changed = false;
        for f in &p.fns {
            let mut m = HashMap::new();
            changed_vars(&f.body, &mut m, &sums);
            for (i, q) in f.params.iter().enumerate() {
                if !q.ty.is_mut_ref() {
                    continue;
                }
                let mut s: HashMap<String, bool> = HashMap::new();
                s.insert(String::new(), true);
                for (path, keep) in &m {
                    if path_root(path) == q.name {
                        let x = s.entry(path[q.name.len()..].to_string()).or_insert(true);
                        *x = *x && *keep;
                    }
                }
                let mut s: Summary = s.into_iter().collect();
                s.sort_by_key(|(r, _)| (r.matches('.').count(), r.clone()));
                let slot = &mut sums.get_mut(&f.name).unwrap()[i];
                if slot.as_ref() != Some(&s) {
                    *slot = Some(s);
                    changed = true;
                }
            }
        }
        if !changed {
            return sums;
        }
    }
}

#[derive(Clone)]
struct State {
    pc: Vec<String>,
    env: HashMap<String, Val>,
}

struct FnCtx<'a> {
    f: &'a FnDef,
    entry: HashMap<String, Val>,
}

/// What a specification can see.
#[derive(Clone, Copy)]
struct Scope<'e> {
    env: &'e HashMap<String, Val>,
    /// The environment `old(..)` reads (callee entry, or the call site).
    old: Option<&'e HashMap<String, Val>>,
    result: Option<&'e Val>,
}

pub struct Verifier<'p> {
    prog: &'p Program,
    solver: Solver,
    counter: usize,
    pub obligations: usize,
    /// Boxes being unfolded by `fresh_of` right now.
    box_depth: usize,
    /// The integer type of each arithmetic operator in the function being
    /// verified (`FnDef::int_ops`).
    int_ops: HashMap<Pos, Type>,
    sums: &'p Summaries,
    /// The functions whose proofs can mention `vcount` (see `counting_fns`).
    counting: &'p HashSet<String>,
    /// Whether the function being verified is one of them: only then are
    /// the counting function and its lemmas declared, so programs that
    /// never count get the same queries as before counting existed.
    counts: bool,
}

/// Does `e` call `vcount`, or name one of `names` (spec fns that count,
/// functions whose contracts count)?
fn mentions_count(e: &Expr, names: &HashSet<String>) -> bool {
    match &e.kind {
        ExprKind::Call(n, xs) => n == "vcount" || names.contains(n) || xs.iter().any(|x| mentions_count(x, names)),
        ExprKind::Var(n) => names.contains(n),
        ExprKind::Field(x, _) | ExprKind::Is(x, _, _) | ExprKind::Not(x) | ExprKind::Neg(x) | ExprKind::Forall(_, x) | ExprKind::TypeApp(_, x) => mentions_count(x, names),
        ExprKind::Variant(_, _, Some(x)) => mentions_count(x, names),
        ExprKind::StructLit(_, fs) => fs.iter().any(|(_, x)| mentions_count(x, names)),
        ExprKind::Bin(_, a, b) => mentions_count(a, names) || mentions_count(b, names),
        _ => false,
    }
}

fn stmts_mention_count(ss: &[Stmt], names: &HashSet<String>) -> bool {
    ss.iter().any(|st| match &st.kind {
        StmtKind::Let { init: e, .. } | StmtKind::Assign { value: e, .. } | StmtKind::Assert(e) | StmtKind::Expr(e) | StmtKind::Return(Some(e)) => mentions_count(e, names),
        StmtKind::If { cond, then_b, else_b } => mentions_count(cond, names) || stmts_mention_count(then_b, names) || stmts_mention_count(else_b, names),
        StmtKind::While { cond, invariants, body } => mentions_count(cond, names) || invariants.iter().any(|i| mentions_count(i, names)) || stmts_mention_count(body, names),
        StmtKind::Match { scrut, arms } => mentions_count(scrut, names) || arms.iter().any(|a| stmts_mention_count(&a.body, names)),
        StmtKind::Return(None) | StmtKind::Unreachable => false,
    })
}

/// The functions whose verification needs the counting lemmas: those whose
/// own contract, loop invariants or assertions use `vcount` (directly or
/// through spec fns), or that call a function whose contract does. When a
/// struct invariant counts, every function does.
fn counting_fns(p: &Program) -> HashSet<String> {
    let mut names = HashSet::new();
    for sf in &p.specs {
        if mentions_count(&sf.body, &names) {
            names.insert(sf.name.clone());
        }
    }
    let all = p.structs.iter().any(|d| d.invariants.iter().any(|i| mentions_count(i, &names)));
    for f in &p.fns {
        if f.requires.iter().chain(&f.ensures).any(|e| mentions_count(e, &names)) {
            names.insert(f.name.clone());
        }
    }
    p.fns.iter().filter(|f| all || names.contains(&f.name) || stmts_mention_count(&f.body, &names)).map(|f| f.name.clone()).collect()
}

/// `vcount(v, x)` is `(cnt a n x)` over the vector's element array `a` and
/// length `n`. These facts about it hold for every array and length; each is
/// a theorem about finite sequences, proved by induction on `n` outside the
/// solver, so they are part of the trusted base (docs/trusted-base.md).
/// `cnt_w` names an index where a counted value is.
const COUNT_LEMMAS: &[&str] = &[
    "(declare-fun cnt ((Array Int Int) Int Int) Int)",
    "(declare-fun cnt_w ((Array Int Int) Int Int) Int)",
    // A0: a count is between 0 and the length.
    "(assert (forall ((a (Array Int Int)) (n Int) (x Int)) (! (=> (<= 0 n) (and (<= 0 (cnt a n x)) (<= (cnt a n x) n))) :pattern ((cnt a n x)))))",
    // A1: an element is counted.
    "(assert (forall ((a (Array Int Int)) (n Int) (i Int)) (! (=> (and (<= 0 i) (< i n)) (<= 1 (cnt a n (select a i)))) :pattern ((cnt a n (select a i))))))",
    // A2: a counted value is an element.
    "(assert (forall ((a (Array Int Int)) (n Int) (x Int)) (! (=> (<= 1 (cnt a n x)) (and (<= 0 (cnt_w a n x)) (< (cnt_w a n x) n) (= (select a (cnt_w a n x)) x))) :pattern ((cnt a n x)))))",
];

/// How a vector operation changes every count: `(cnt a2 n2 x)` is
/// `(cnt a n x)` plus `delta` (a term over `x`). Ground in everything but
/// `x`, and triggered by either count.
fn count_step(a2: &str, n2: &str, a: &str, n: &str, delta: &str) -> String {
    format!("(forall ((x Int)) (! (= (cnt {a2} {n2} x) (+ (cnt {a} {n} x) {delta})) :pattern ((cnt {a2} {n2} x)) :pattern ((cnt {a} {n} x))))")
}

fn sanitize(n: &str) -> String {
    n.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect()
}

fn and_all(ts: &[String]) -> String {
    match ts.len() {
        0 => "true".into(),
        1 => ts[0].clone(),
        _ => format!("(and {})", ts.join(" ")),
    }
}

/// Two values of one struct or enum type (made of numbers) are equal: every
/// field is, and for an enum the tags are and so is the payload the tag
/// names.
fn eq_term(a: &Val, b: &Val) -> String {
    match (unbox_known(a), unbox_known(b)) {
        (Val::U(x), Val::U(y)) | (Val::B(x), Val::B(y)) => format!("(= {} {})", x, y),
        (Val::Struct(_, xs), Val::Struct(_, ys)) => {
            let ts: Vec<String> = xs.iter().zip(ys).map(|(x, y)| eq_term(x, y)).filter(|t| t != "true").collect();
            and_all(&ts)
        }
        (Val::Enum(_, tx, xs), Val::Enum(_, ty, ys)) => {
            let mut ts = vec![format!("(= {} {})", tx, ty)];
            for (k, (x, y)) in xs.iter().zip(ys).enumerate() {
                let t = eq_term(x, y);
                if t != "true" {
                    ts.push(format!("(=> (= {} {}) {})", tx, k, t));
                }
            }
            and_all(&ts)
        }
        _ => "true".into(),
    }
}

fn or_all(ts: &[String]) -> String {
    match ts.len() {
        0 => "false".into(),
        1 => ts[0].clone(),
        _ => format!("(or {})", ts.join(" ")),
    }
}

/// An integer as an SMT-LIB term (negative numbers are written `(- n)`).
fn num(v: i128) -> String {
    if v < 0 { format!("(- {})", -v) } else { v.to_string() }
}

/// `x` reduced to the range of the integer type `t`, as a C conversion to
/// `t` does: modulo 2^width, two's complement for a signed type.
fn wrap_term(t: &Type, x: &str) -> String {
    if t.is_signed() {
        let h = t.int_modulus() / 2;
        format!("(- (mod (+ {} {}) {}) {})", x, h, t.int_modulus(), h)
    } else {
        format!("(mod {} {})", x, t.int_modulus())
    }
}

/// C's division on signed operands, which truncates toward zero (SMT-LIB's
/// `div` rounds so that the remainder is never negative).
fn c_div(a: &str, b: &str) -> String {
    let abs = |x: &str| format!("(ite (>= {x} 0) {x} (- {x}))");
    let q = format!("(div {} {})", abs(a), abs(b));
    format!("(ite (= (>= {} 0) (>= {} 0)) {q} (- {q}))", a, b)
}

/// 2^k for a shift amount `k` already proved less than `w`.
fn pow2(k: &str, w: u32) -> String {
    let mut t = "1".to_string();
    for i in (1..w).rev() {
        t = format!("(ite (= {} {}) {} {})", k, i, 1u128 << i, t);
    }
    t
}

fn ite(c: &str, a: &str, b: &str) -> String {
    if a == b { a.to_string() } else { format!("(ite {} {} {})", c, a, b) }
}

pub fn verify_program(p: &Program, one_session: bool) -> Result<(usize, Vec<String>), Diag> {
    // Each function gets its own solver scope: what is declared and assumed
    // while verifying one (some of it quantified, such as "every byte of a
    // Bytes is <= 255") must not burden the queries for all later ones.
    // Because the scopes are independent, the functions are shared out
    // among several solvers running at once (BASTIO_JOBS, default: the
    // number of CPUs, at most 8). The error reported is the one sequential
    // verification would report: that of the first failing function in
    // program order. BASTIO_TIME=1 prints the time each function took.
    // `one_session` (for --dump-smt, which stage1/difftest.sh compares with
    // stage1's verifier) keeps everything on one solver, in program order.
    let sums = summarize(p);
    let counting = counting_fns(p);
    let n = p.fns.len();
    let jobs = std::env::var("BASTIO_JOBS")
        .ok()
        .and_then(|j| j.parse::<usize>().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |c| c.get()).min(8))
        .clamp(1, if one_session { 1 } else { n.max(1) });
    let next = AtomicUsize::new(0);
    // The lowest index of a function that failed so far: no worker starts
    // a function after it, since its result could not be reported anyway.
    let failed = AtomicUsize::new(usize::MAX);
    let workers: Vec<Worker> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..jobs).map(|_| s.spawn(|| verify_worker(p, &sums, &counting, &next, &failed))).collect();
        hs.into_iter().map(|h| h.join().expect("a verifier thread panicked")).collect()
    });
    let mut obligations = 0;
    let mut log = Vec::new();
    let mut times = Vec::new();
    let mut err: Option<(usize, Diag)> = None;
    for w in workers {
        obligations += w.obligations;
        log.extend(w.log);
        times.extend(w.times);
        if let Some((i, d)) = w.err {
            if err.as_ref().map_or(true, |(j, _)| i < *j) {
                err = Some((i, d));
            }
        }
    }
    if let Some((_, d)) = err {
        return Err(d);
    }
    if std::env::var("BASTIO_TIME").is_ok() {
        times.sort_by_key(|(i, _)| *i);
        for (i, ms) in times {
            eprintln!("{:8.1} ms {}", ms, p.fns[i].name);
        }
    }
    Ok((obligations, log))
}

/// What one solver did: its obligation count, its session (for
/// `--dump-smt`, which uses a single worker), the
/// time per function, and the first function it could not verify.
struct Worker {
    obligations: usize,
    log: Vec<String>,
    times: Vec<(usize, f64)>,
    err: Option<(usize, Diag)>,
}

/// Verifies functions, taking the next unverified index each time, on a
/// solver of its own.
fn verify_worker(p: &Program, sums: &Summaries, counting: &HashSet<String>, next: &AtomicUsize, failed: &AtomicUsize) -> Worker {
    let mut w = Worker { obligations: 0, log: Vec::new(), times: Vec::new(), err: None };
    let solver = match Solver::start() {
        Ok(s) => s,
        Err(e) => {
            w.err = Some((0, Diag::new("E-SOLVER", Pos::default(), e)));
            return w;
        }
    };
    let mut v = Verifier { prog: p, solver, counter: 0, obligations: 0, box_depth: 0, int_ops: HashMap::new(), sums, counting, counts: false };
    let mut run = || -> Result<(), (usize, Diag)> {
        v.smt(&format!("(declare-const {} Int)", POOLSIZE)).map_err(|d| (0, d))?;
        v.smt(&format!("(assert (and (<= 0 {p}) (<= {p} {})))", FRAME_LIMIT, p = POOLSIZE)).map_err(|d| (0, d))?;
        loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            if i >= p.fns.len() || i > failed.load(Ordering::SeqCst) {
                return Ok(());
            }
            let f = &p.fns[i];
            let t0 = std::time::Instant::now();
            let kept = if f.export { buffers_kept(f, sums) } else { Ok(()) };
            let r = kept
                .and_then(|_| v.smt("(push 1)"))
                .and_then(|_| v.verify_fn(f).map_err(|d| crate::check::in_copy(d, &f.name)))
                .and_then(|_| v.smt("(pop 1)"));
            if let Err(d) = r {
                failed.fetch_min(i, Ordering::SeqCst);
                return Err((i, d));
            }
            w.times.push((i, t0.elapsed().as_secs_f64() * 1000.0));
        }
    };
    let r = run();
    w.err = r.err();
    w.obligations = v.obligations;
    w.log = std::mem::take(&mut v.solver.log);
    w
}

/// A `&mut Bytes` that C lends to an `export fn` is the caller's memory:
/// the function may write its bytes but must hand back the same buffer,
/// never free or replace it. Its summary shows that: the parameter is
/// changed only by shape-keeping builtins (`bset`), directly or through
/// other functions' summaries, so it is the same allocation of the same
/// length on return.
fn buffers_kept(f: &FnDef, sums: &Summaries) -> Result<(), Diag> {
    for (i, q) in f.params.iter().enumerate() {
        if !matches!(&q.ty, Type::MutRef(t) if **t == Type::Bytes) {
            continue;
        }
        let kept = sums[&f.name][i].as_ref().map_or(false, |s| s.iter().all(|(_, keep)| *keep));
        if !kept {
            return Err(Diag::new("E-EXPORT", q.pos, format!("`{}` belongs to the C caller, but `{}` may free or replace it", q.name, f.name))
                .note("a buffer lent by C may only be written in place (`bset`, or lent to functions that do only that)"));
        }
    }
    Ok(())
}

impl<'p> Verifier<'p> {
    fn smt(&mut self, cmd: &str) -> Result<(), Diag> {
        self.solver.send(cmd).map_err(|e| Diag::new("E-SOLVER", Pos::default(), e))
    }

    fn fresh_name(&mut self, hint: &str) -> String {
        self.counter += 1;
        format!("{}!{}", sanitize(hint), self.counter)
    }

    fn fresh_int(&mut self, hint: &str, lo: &str, hi_incl: &str) -> Result<String, Diag> {
        let n = self.fresh_name(hint);
        self.smt(&format!("(declare-const {} Int)", n))?;
        self.smt(&format!("(assert (and (<= {lo} {n}) (<= {n} {hi_incl})))"))?;
        Ok(n)
    }

    fn fresh_u64(&mut self, hint: &str) -> Result<Val, Diag> {
        Ok(Val::U(self.fresh_int(hint, "0", U64_MAX)?))
    }

    fn fresh_bool(&mut self, hint: &str) -> Result<Val, Diag> {
        let n = self.fresh_name(hint);
        self.smt(&format!("(declare-const {} Bool)", n))?;
        Ok(Val::B(n))
    }

    fn fresh_len(&mut self, hint: &str) -> Result<String, Diag> {
        self.fresh_int(&format!("len_{}", hint), "0", ALLOC_MAX)
    }

    fn fresh_array(&mut self, hint: &str, elem: &str) -> Result<String, Diag> {
        let n = self.fresh_name(&format!("data_{}", hint));
        self.smt(&format!("(declare-const {} (Array Int {}))", n, elem))?;
        Ok(n)
    }

    fn fresh_field(&mut self, t: &Type, field: &str, hint: &str) -> Result<String, Diag> {
        let h = format!("{}_{}", hint, field);
        match (t, field) {
            (_, "used") | (_, "has") | (_, "free") => self.fresh_array(&h, "Bool"),
            (Type::PortIo, "base") => self.fresh_int(&h, "0", PORT_LIMIT),
            (Type::PortIo, "count") => self.fresh_int(&h, "0", PORT_LIMIT),
            (Type::Mmio, "base") => self.fresh_int(&h, MMIO_LO, MMIO_HI),
            (Type::Mmio, "len") => self.fresh_int(&h, "0", MMIO_HI),
            (Type::Frame, "id") | (Type::PageTable, "id") | (Type::Dma, "id") => {
                let n = self.fresh_int(&h, "0", FRAME_LIMIT)?;
                self.smt(&format!("(assert (< {} {}))", n, POOLSIZE))?;
                Ok(n)
            }
            (Type::PageTable, "level") => self.fresh_int(&h, "1", "5"),
            (Type::PageTable, "leases") => self.fresh_int(&h, "0", U64_MAX),
            (Type::Pool, "size") => Ok(POOLSIZE.to_string()),
            (Type::Mmu, "root") => self.fresh_int(&h, "0", KROOT),
            _ => unreachable!("unknown ghost field"),
        }
    }

    fn fresh_of(&mut self, ty: &Type, hint: &str) -> Result<Val, Diag> {
        match ty {
            Type::U64 => self.fresh_u64(hint),
            Type::U8 | Type::U16 | Type::U32 | Type::I8 | Type::I16 | Type::I32 | Type::I64 => Ok(Val::U(self.fresh_int(hint, &num(ty.int_min()), &num(ty.int_max()))?)),
            Type::Bool => self.fresh_bool(hint),
            Type::Buf => Ok(Val::Buf { len: self.fresh_len(hint)?, data: self.fresh_array(hint, "Int")? }),
            Type::Bytes => {
                // Every byte of a Bytes value is <= 255.
                let data = self.fresh_array(hint, "Int")?;
                self.smt(&format!("(assert (forall ((k Int)) (and (<= 0 (select {d} k)) (<= (select {d} k) 255))))", d = data))?;
                Ok(Val::Buf { len: self.fresh_len(hint)?, data })
            }
            Type::Str => Ok(Val::Str { len: self.fresh_len(hint)?, data: self.fresh_array(hint, "Int")? }),
            // A borrowed table may be one `pt_borrow` lent, with its tree's
            // leases at any level.
            Type::Ref(t) | Type::MutRef(t) if **t == Type::PageTable => self.fresh_table(hint),
            Type::Ref(t) | Type::MutRef(t) => {
                let t = (**t).clone();
                self.fresh_of(&t, hint)
            }
            Type::Unit => Ok(Val::Unit),
            Type::Struct(n) => {
                let mut fs = Vec::new();
                for (f, ft) in &struct_def(n).unwrap().fields {
                    fs.push(self.fresh_of(ft, &format!("{}_{}", hint, f))?);
                }
                Ok(Val::Struct(n.clone(), fs))
            }
            Type::Enum(n) => {
                let d = enum_def(n).unwrap();
                let tag = self.fresh_int(&format!("{}_tag", hint), "0", &(d.variants.len() - 1).to_string())?;
                let mut ps = Vec::new();
                for (v, pt) in &d.variants {
                    ps.push(match pt {
                        Some(pt) => self.fresh_of(pt, &format!("{}_{}", hint, v))?,
                        None => Val::Unit,
                    });
                }
                Ok(Val::Enum(n.clone(), tag, ps))
            }
            Type::Vec(t) => {
                let elems = self.fresh_lifted(t, hint)?;
                Ok(Val::Vec { elem: (**t).clone(), len: self.fresh_len(hint)?, elems: Box::new(elems) })
            }
            Type::Box(t) => {
                if self.box_depth >= BOX_DEPTH {
                    return Ok(Val::Box(BoxVal::Opaque((**t).clone())));
                }
                self.box_depth += 1;
                let inner = self.fresh_of(t, hint);
                self.box_depth -= 1;
                Ok(Val::Box(BoxVal::Known(Box::new(inner?))))
            }
            t => {
                let mut fs = Vec::new();
                for (f, _, _) in ghost_fields(t) {
                    fs.push(self.fresh_field(t, f, hint)?);
                }
                if *t == Type::PortIo {
                    self.smt(&format!("(assert (<= (+ {} {}) {}))", fs[0], fs[1], PORT_LIMIT))?;
                }
                if *t == Type::Mmio {
                    self.smt(&format!("(assert (<= (+ {} {}) {}))", fs[0], fs[1], MMIO_HI))?;
                }
                // An owned table below the top is never live: it was taken
                // out of a tree no processor ran on, or never in one (only
                // `pt_borrow` lends a live one, and only as `&mut`).
                if *t == Type::PageTable {
                    self.smt(&format!("(assert (=> (< {} 4) (= {} 0)))", fs[1], fs[3]))?;
                }
                Ok(Val::Obj(t.clone(), fs))
            }
        }
    }

    /// A fresh table that may be live at any level (a borrowed one).
    fn fresh_table(&mut self, hint: &str) -> Result<Val, Diag> {
        let mut fs = Vec::new();
        for (f, _, _) in ghost_fields(&Type::PageTable) {
            fs.push(self.fresh_field(&Type::PageTable, f, hint)?);
        }
        Ok(Val::Obj(Type::PageTable, fs))
    }

    /// Fresh contents for a vector of `ty`: every term of an element becomes
    /// an unconstrained array indexed by position.
    fn fresh_lifted(&mut self, ty: &Type, hint: &str) -> Result<Val, Diag> {
        let tmpl = self.fresh_of(ty, hint)?;
        let tmpl = opaque_boxes(&tmpl, ty);
        let mut ls = Vec::new();
        leaves(&tmpl, &mut ls);
        let mut arrs = Vec::new();
        for (_, sort) in &ls {
            arrs.push(self.fresh_array(&format!("{}_elems", hint), sort)?);
        }
        let mut it = arrs.into_iter();
        Ok(map_leaves(&tmpl, &mut |_| it.next().unwrap()))
    }

    /// The element at index `i`: a fresh value (with every fact a value of
    /// its type satisfies) equal, term by term, to what the vector stores.
    fn vec_read(&mut self, elem: &Type, elems: &Val, i: &str, st: &mut State) -> Result<Val, Diag> {
        let v = self.fresh_of(elem, "elem")?;
        let (mut a, mut b) = (Vec::new(), Vec::new());
        leaves(&v, &mut a);
        leaves(elems, &mut b);
        for ((x, _), (arr, _)) in a.iter().zip(&b) {
            st.pc.push(format!("(= {} (select {} {}))", x, arr, i));
        }
        Ok(v)
    }

    /// Proves `goal` under the path condition, or reports a diagnostic with a
    /// counterexample drawn from the variables in scope.
    fn prove(&mut self, st: &State, goal: &str, pos: Pos, code: &'static str, what: String) -> Result<(), Diag> {
        self.obligations += 1;
        self.smt("(push 1)")?;
        for c in &st.pc {
            self.smt(&format!("(assert {})", c))?;
        }
        self.smt(&format!("(assert (not {}))", goal))?;
        let r = self.solver.check_sat().map_err(|e| Diag::new("E-SOLVER", pos, e))?;
        let res = match r {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => {
                let mut d = Diag::new(code, pos, format!("could not prove {}", what));
                let mut names: Vec<_> = st.env.keys().cloned().collect();
                names.sort();
                fn show(n: &str, v: &Val, shown: &mut Vec<(String, String)>) {
                    match v {
                        Val::U(t) | Val::B(t) => shown.push((n.to_string(), t.clone())),
                        Val::Buf { len, .. } => shown.push((format!("len({})", n), len.clone())),
                        Val::Str { len, .. } => shown.push((format!("slen({})", n), len.clone())),
                        Val::Vec { len, .. } => shown.push((format!("vlen({})", n), len.clone())),
                        Val::Obj(t, fs) => {
                            for ((_, arr, acc), term) in ghost_fields(t).iter().zip(fs) {
                                if !arr {
                                    shown.push((format!("{}({})", acc, n), term.clone()));
                                }
                            }
                        }
                        Val::Struct(sn, fs) => {
                            for ((f, _), fv) in struct_def(sn).unwrap().fields.iter().zip(fs) {
                                show(&format!("{}.{}", n, f), fv, shown);
                            }
                        }
                        Val::Enum(en, tag, ps) => {
                            shown.push((format!("tag({})", n), tag.clone()));
                            for ((v, _), pv) in enum_def(en).unwrap().variants.iter().zip(ps) {
                                show(&format!("{}.{}", n, v), pv, shown);
                            }
                        }
                        Val::Box(BoxVal::Known(i)) => show(n, i, shown),
                        Val::Box(BoxVal::Opaque(_)) | Val::Unit => {}
                    }
                }
                let mut shown: Vec<(String, String)> = Vec::new();
                for n in names {
                    show(&n, &st.env[&n], &mut shown);
                }
                let mut cex = Vec::new();
                for (label, term) in shown {
                    let v = self.solver.get_value(&term).map_err(|e| Diag::new("E-SOLVER", pos, e))?;
                    cex.push(format!("{} = {}", label, v));
                }
                if !cex.is_empty() {
                    d = d.note(format!("counterexample: {}", cex.join(", ")));
                }
                Err(d)
            }
            SatResult::Unknown => Err(Diag::new(code, pos, format!("could not prove {}", what))
                .note("the solver gave up (its work limit, or quantifiers); add an `assert` or a stronger invariant as a hint")),
        };
        self.smt("(pop 1)")?;
        res
    }

    fn verify_fn(&mut self, f: &FnDef) -> Result<(), Diag> {
        self.int_ops = f.int_ops.clone();
        self.counts = self.counting.contains(&f.name);
        if self.counts {
            for c in COUNT_LEMMAS {
                self.smt(c)?;
            }
        }
        let mut env = HashMap::new();
        for p in &f.params {
            let v = self.fresh_of(&p.ty, &p.name)?;
            env.insert(p.name.clone(), v);
        }
        let mut st = State { pc: Vec::new(), env: env.clone() };
        if f.name == "main" {
            // What the runtime hands to `main`: a full pool and the kernel's own tables in CR3.
            for p in &f.params {
                match (p.ty.base(), &env[&p.name]) {
                    (Type::Pool, v) => st.pc.push(format!(
                        "(forall ((k Int)) (= (select {} k) (and (<= 0 k) (< k {}))))",
                        v.field("has"),
                        v.field("size")
                    )),
                    (Type::Mmu, v) => st.pc.push(format!("(= {} {})", v.field("root"), KROOT)),
                    (Type::Ports, v) => st.pc.push(format!(
                        "(forall ((k Int)) (= (select {} k) (and (<= 0 k) (< k {}))))",
                        v.field("free"),
                        PORT_LIMIT
                    )),
                    (Type::MmioSpace, v) => st.pc.push(format!(
                        "(forall ((k Int)) (= (select {} k) (and (<= {} k) (< k {}))))",
                        v.field("free"),
                        MMIO_LO,
                        MMIO_HI
                    )),
                    _ => {}
                }
            }
        }
        let cx = FnCtx { f, entry: env };
        for r in &f.requires {
            let sc = Scope { env: &cx.entry, old: None, result: None };
            let t = self.spec(r, sc, &mut Vec::new())?;
            st.pc.push(t);
        }
        if let Some(end) = self.block(&f.body, st, &cx)? {
            // Only unit functions can fall through (checked earlier).
            self.check_ensures(&end, &cx, &Val::Unit, f.pos)?;
        }
        Ok(())
    }

    fn check_ensures(&mut self, st: &State, cx: &FnCtx, result: &Val, pos: Pos) -> Result<(), Diag> {
        // Parameters mean their entry values, except `&mut` ones, which mean
        // the value handed back; `old(..)` reads the entry values.
        let mut post = cx.entry.clone();
        for p in &cx.f.params {
            if p.ty.is_mut_ref() {
                post.insert(p.name.clone(), st.env[&p.name].clone());
            }
        }
        for e in &cx.f.ensures {
            let sc = Scope { env: &post, old: Some(&cx.entry), result: Some(result) };
            let t = self.spec(e, sc, &mut Vec::new())?;
            self.prove(st, &t, pos, "E-VERIFY-ENSURES", format!("the postcondition `{}` of `{}` at this return", render(e), cx.f.name))?;
        }
        Ok(())
    }

    /// Executes statements; `None` means every path returned.
    fn block(&mut self, stmts: &[Stmt], mut st: State, cx: &FnCtx) -> Result<Option<State>, Diag> {
        for s in stmts {
            match self.stmt(s, st, cx)? {
                Some(next) => st = next,
                None => return Ok(None),
            }
        }
        Ok(Some(st))
    }

    fn here(&mut self, e: &Expr, st: &State) -> Result<String, Diag> {
        let sc = Scope { env: &st.env, old: None, result: None };
        self.spec(e, sc, &mut Vec::new())
    }

    /// A loop invariant read in `st`; `old(..)` reads the function's entry.
    fn here_inv(&mut self, e: &Expr, st: &State, cx: &FnCtx) -> Result<String, Diag> {
        let sc = Scope { env: &st.env, old: Some(&cx.entry), result: None };
        self.spec(e, sc, &mut Vec::new())
    }

    fn stmt(&mut self, s: &Stmt, mut st: State, cx: &FnCtx) -> Result<Option<State>, Diag> {
        match &s.kind {
            StmtKind::Let { name, init, .. } => {
                let v = self.eval(init, &mut st)?;
                st.env.insert(name.clone(), v);
                Ok(Some(st))
            }
            StmtKind::Assign { name, value } => {
                let v = self.eval(value, &mut st)?;
                self.open_path(&mut st.env, name)?;
                set_path(&mut st.env, name, v);
                Ok(Some(st))
            }
            StmtKind::Expr(e) => {
                self.eval(e, &mut st)?;
                Ok(Some(st))
            }
            StmtKind::Assert(e) => {
                let t = self.here(e, &st)?;
                self.prove(&st, &t, s.pos, "E-VERIFY-ASSERT", format!("`{}`", render(e)))?;
                st.pc.push(t);
                Ok(Some(st))
            }
            StmtKind::Unreachable => {
                self.prove(&st, "false", s.pos, "E-VERIFY-UNREACHABLE", "that this point is unreachable".into())?;
                Ok(None)
            }
            StmtKind::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(e, &mut st)?,
                    None => Val::Unit,
                };
                self.check_ensures(&st, cx, &v, s.pos)?;
                Ok(None)
            }
            StmtKind::If { cond, then_b, else_b } => {
                let c = self.eval(cond, &mut st)?.term().to_string();
                let mut ts = st.clone();
                ts.pc.push(c.clone());
                let mut es = st.clone();
                es.pc.push(format!("(not {})", c));
                let t_out = self.block(then_b, ts, cx)?;
                let e_out = self.block(else_b, es, cx)?;
                match (t_out, e_out) {
                    (None, None) => Ok(None),
                    (Some(t), None) => Ok(Some(t)),
                    (None, Some(e)) => Ok(Some(e)),
                    (Some(t), Some(e)) => {
                        let mut env = HashMap::new();
                        for (name, _) in st.env.iter() {
                            env.insert(name.clone(), merge_val(&c, &t.env[name], &e.env[name]));
                        }
                        // Facts learned inside either arm are kept as implications.
                        let mut pc = st.pc.clone();
                        let extra_t: Vec<_> = t.pc[st.pc.len() + 1..].to_vec();
                        let extra_e: Vec<_> = e.pc[st.pc.len() + 1..].to_vec();
                        if !extra_t.is_empty() {
                            pc.push(format!("(=> {} {})", c, and_all(&extra_t)));
                        }
                        if !extra_e.is_empty() {
                            pc.push(format!("(=> (not {}) {})", c, and_all(&extra_e)));
                        }
                        Ok(Some(State { pc, env }))
                    }
                }
            }
            StmtKind::Match { scrut, arms } => {
                let Val::Enum(en, tag, ps) = self.eval(scrut, &mut st)? else { unreachable!("checked: an enum") };
                let d = enum_def(&en).unwrap();
                let mut covered = Vec::new();
                let mut outs: Vec<(String, State)> = Vec::new();
                for a in arms {
                    let mut ast = st.clone();
                    let c = match &a.pat {
                        Pat::Variant(_, v, b, _) => {
                            let k = d.index(v).unwrap();
                            covered.push(format!("(= {} {})", tag, k));
                            if let Some(b) = b {
                                if b != "_" {
                                    ast.env.insert(b.clone(), ps[k].clone());
                                }
                            }
                            format!("(= {} {})", tag, k)
                        }
                        Pat::Wild => format!("(not {})", or_all(&covered)),
                    };
                    ast.pc.push(c.clone());
                    if let Some(out) = self.block(&a.body, ast, cx)? {
                        outs.push((c, out));
                    }
                }
                if outs.len() <= 1 {
                    return Ok(outs.pop().map(|(_, o)| o));
                }
                // Arms are disjoint, so the merged value is a chain of ites
                // ending in the last arm that falls through.
                let (_, last) = outs.last().unwrap().clone();
                let mut env = HashMap::new();
                for name in st.env.keys() {
                    let mut v = last.env[name].clone();
                    for (c, o) in outs[..outs.len() - 1].iter().rev() {
                        v = merge_val(c, &o.env[name], &v);
                    }
                    env.insert(name.clone(), v);
                }
                let mut pc = st.pc.clone();
                if outs.len() < arms.len() {
                    let conds: Vec<String> = outs.iter().map(|(c, _)| c.clone()).collect();
                    pc.push(or_all(&conds));
                }
                for (c, o) in &outs {
                    let extra: Vec<_> = o.pc[st.pc.len() + 1..].to_vec();
                    if !extra.is_empty() {
                        pc.push(format!("(=> {} {})", c, and_all(&extra)));
                    }
                }
                Ok(Some(State { pc, env }))
            }
            StmtKind::While { cond, invariants, body } => {
                // 1. invariants hold on entry
                for inv in invariants {
                    let t = self.here_inv(inv, &st, cx)?;
                    self.prove(&st, &t, inv.pos, "E-VERIFY-INVARIANT", format!("the loop invariant `{}` on entry to the loop", render(inv)))?;
                }
                // 2. havoc what the body (or the condition) changes
                let mut changed = HashMap::new();
                changed_vars(body, &mut changed, self.sums);
                changed_in_expr(cond, &mut changed, self.sums);
                let mut lst = st.clone();
                // Enclosing places first, so a field's own havoc lands inside its struct's.
                let mut changed: Vec<(String, bool)> = changed.into_iter().collect();
                changed.sort_by_key(|(n, _)| (n.matches('.').count(), n.clone()));
                for (name, keep) in changed {
                    if !st.env.contains_key(path_root(&name)) {
                        continue; // declared inside the loop
                    }
                    self.open_path(&mut lst.env, &name)?;
                    let old = get_path(&lst.env, &name);
                    let nv = self.havoc(&old, keep, &name)?;
                    set_path(&mut lst.env, &name, nv);
                }
                for inv in invariants {
                    let t = self.here_inv(inv, &lst, cx)?;
                    lst.pc.push(t);
                }
                // 3. one arbitrary iteration preserves the invariants
                let c = self.eval(cond, &mut lst)?.term().to_string();
                let mut bst = lst.clone();
                bst.pc.push(c.clone());
                if let Some(end) = self.block(body, bst, cx)? {
                    for inv in invariants {
                        let t = self.here_inv(inv, &end, cx)?;
                        self.prove(&end, &t, inv.pos, "E-VERIFY-INVARIANT", format!("that the loop body preserves the invariant `{}`", render(inv)))?;
                    }
                }
                // 4. after the loop: invariants and the negated condition
                lst.pc.push(format!("(not {})", c));
                Ok(Some(lst))
            }
        }
    }

    /// Gives every opaque box on `path` (before its last component) fresh
    /// contents, so the path can be read and written. Sound: nothing was
    /// known about them, and a box has one owner, so no other copy exists.
    fn open_path(&mut self, env: &mut HashMap<String, Val>, path: &str) -> Result<(), Diag> {
        let parts: Vec<&str> = path.split('.').collect();
        for k in 1..parts.len() {
            let prefix = parts[..k].join(".");
            let v = get_path(env, &prefix);
            let opened = self.open_val(v.clone(), &sanitize(&prefix))?;
            if opened != v {
                set_path(env, &prefix, opened);
            }
        }
        Ok(())
    }

    fn open_val(&mut self, v: Val, hint: &str) -> Result<Val, Diag> {
        Ok(match v {
            Val::Box(BoxVal::Opaque(t)) => {
                let inner = self.fresh_of(&t, hint)?;
                Val::Box(BoxVal::Known(Box::new(self.open_val(inner, hint)?)))
            }
            Val::Box(BoxVal::Known(i)) => Val::Box(BoxVal::Known(Box::new(self.open_val(*i, hint)?))),
            v => v,
        })
    }

    /// What a specification sees inside a box. An opaque box reads as fresh
    /// values each time: only incomplete (a fact about it cannot be used
    /// later), never unsound.
    fn look_inside(&mut self, v: Val) -> Result<Val, Diag> {
        match v {
            Val::Box(BoxVal::Known(i)) => self.look_inside(*i),
            Val::Box(BoxVal::Opaque(t)) => {
                let inner = self.fresh_of(&t, "opaque")?;
                self.look_inside(inner)
            }
            v => Ok(v),
        }
    }

    /// A fresh value of the same type as `old`. With `keep` (the value was
    /// changed only in shape-keeping ways, see `changed_vars`), a Buf or a
    /// vector keeps its length, an object the ghost fields builtins never
    /// change, and scalars, enum tags and the fields of structs stay as they
    /// were: no shape-keeping builtin changes them, and any other change to a
    /// place inside is havocked separately, after this.
    fn havoc(&mut self, old: &Val, keep: bool, name: &str) -> Result<Val, Diag> {
        Ok(match old {
            Val::U(_) | Val::B(_) | Val::Str { .. } if keep => old.clone(),
            Val::Buf { len, .. } => {
                let len = if keep { len.clone() } else { self.fresh_len(name)? };
                Val::Buf { len, data: self.fresh_array(name, "Int")? }
            }
            Val::Str { .. } => Val::Str { len: self.fresh_len(name)?, data: self.fresh_array(name, "Int")? },
            Val::U(_) => self.fresh_u64(name)?,
            Val::B(_) => self.fresh_bool(name)?,
            Val::Obj(t, fs) => {
                let mut nf = Vec::new();
                for ((fname, _, _), old_term) in ghost_fields(t).iter().zip(fs) {
                    if keep && kept_by_builtins(t, fname) {
                        nf.push(old_term.clone());
                    } else {
                        nf.push(self.fresh_field(t, fname, name)?);
                    }
                }
                Val::Obj(t.clone(), nf)
            }
            Val::Struct(n, fs) => {
                let mut nf = Vec::new();
                for ((f, _), fv) in struct_def(n).unwrap().fields.iter().zip(fs) {
                    nf.push(self.havoc(fv, keep, &format!("{}_{}", name, f))?);
                }
                Val::Struct(n.clone(), nf)
            }
            Val::Enum(n, tag, ps) => {
                let d = enum_def(n).unwrap();
                let tag = if keep { tag.clone() } else { self.fresh_int(&format!("{}_tag", name), "0", &(d.variants.len() - 1).to_string())? };
                let mut np = Vec::new();
                for ((v, _), pv) in d.variants.iter().zip(ps) {
                    np.push(self.havoc(pv, keep, &format!("{}_{}", name, v))?);
                }
                Val::Enum(n.clone(), tag, np)
            }
            Val::Vec { elem, len, .. } => {
                let len = if keep { len.clone() } else { self.fresh_len(name)? };
                Val::Vec { elem: elem.clone(), len, elems: Box::new(self.fresh_lifted(elem, name)?) }
            }
            Val::Box(BoxVal::Known(i)) => Val::Box(BoxVal::Known(Box::new(self.havoc(i, keep, name)?))),
            Val::Box(BoxVal::Opaque(t)) => Val::Box(BoxVal::Opaque(t.clone())),
            Val::Unit => Val::Unit,
        })
    }

    /// Evaluates a code expression, proving its safety obligations on the way.
    fn eval(&mut self, e: &Expr, st: &mut State) -> Result<Val, Diag> {
        match &e.kind {
            ExprKind::TypeApp(targs, inner) => match &inner.kind {
                ExprKind::Call(n, _) if n == "vec_new" => {
                    Ok(Val::Vec { elem: targs[0].clone(), len: "0".into(), elems: Box::new(self.fresh_lifted(&targs[0], "vec")?) })
                }
                // `cast[T](x)` keeps the value and proves it fits; `wrap[T](x)`
                // reduces it modulo 2^width.
                ExprKind::Call(n, args) if n == "cast" || n == "wrap" => {
                    let x = self.eval(&args[0], st)?.term().to_string();
                    let t = &targs[0];
                    if n == "wrap" {
                        return Ok(Val::U(wrap_term(t, &x)));
                    }
                    let from = self.int_ops.get(&e.pos).cloned().unwrap_or(Type::U64);
                    let max = num(t.int_max());
                    if !from.is_signed() && !t.is_signed() {
                        self.prove(st, &format!("(<= {} {})", x, max), e.pos, "E-VERIFY-CAST", format!("that `{}` fits in {} (`<= {}`)", render(&args[0]), t.name(), max))?;
                    } else {
                        self.prove(st, &format!("(and (<= {} {}) (<= {} {}))", num(t.int_min()), x, x, max), e.pos, "E-VERIFY-CAST", format!("that `{}` fits in {} ({} to {})", render(&args[0]), t.name(), t.int_min(), t.int_max()))?;
                    }
                    Ok(Val::U(x))
                }
                _ => unreachable!("removed by monomorphization"),
            },
            ExprKind::Int(v) => Ok(Val::U(v.to_string())),
            ExprKind::Bool(b) => Ok(Val::B(b.to_string())),
            ExprKind::Str(bytes) => Ok(str_val(bytes)),
            ExprKind::Blob(bytes) => Ok(Val::Str { len: bytes.len().to_string(), data: self.fresh_array("blob", "Int")? }),
            ExprKind::Var(n) => Ok(st.env[n].clone()),
            ExprKind::Field(..) => {
                let p = place(e).expect("checked: a place");
                self.open_path(&mut st.env, &p)?;
                Ok(get_path(&st.env, &p))
            }
            ExprKind::StructLit(n, fs) => {
                let mut vals = HashMap::new();
                for (f, x) in fs {
                    vals.insert(f.clone(), self.eval(x, st)?);
                }
                let fields = struct_def(n).unwrap().fields.iter().map(|(f, _)| vals.remove(f).unwrap()).collect();
                Ok(Val::Struct(n.clone(), fields))
            }
            ExprKind::Variant(n, v, payload) => {
                let d = enum_def(n).unwrap();
                let k = d.index(v).unwrap();
                let mut ps = Vec::new();
                for (j, (vn, pt)) in d.variants.iter().enumerate() {
                    ps.push(match (pt, payload) {
                        (Some(_), Some(x)) if j == k => self.eval(x, st)?,
                        (Some(pt), _) => self.fresh_of(pt, &format!("{}_{}", n, vn))?,
                        (None, _) => Val::Unit,
                    });
                }
                Ok(Val::Enum(n.clone(), k.to_string(), ps))
            }
            ExprKind::Is(x, _, v) => match self.eval(x, st)? {
                Val::Enum(n, tag, _) => Ok(Val::B(format!("(= {} {})", tag, enum_def(&n).unwrap().index(v).unwrap()))),
                _ => unreachable!("checked: an enum"),
            },
            ExprKind::MutBorrow(_) => unreachable!("only a call argument"),
            ExprKind::Not(x) => {
                let v = self.eval(x, st)?;
                // Bitwise not on an unsigned integer: max - x.
                if let Some(ity) = self.int_ops.get(&e.pos) {
                    return Ok(Val::U(format!("(- {} {})", ity.int_max(), v.term())));
                }
                Ok(Val::B(format!("(not {})", v.term())))
            }
            ExprKind::Neg(x) => {
                if let ExprKind::Int(v) = x.kind {
                    return Ok(Val::U(num(-(v as i128))));
                }
                let v = self.eval(x, st)?.term().to_string();
                let ity = self.int_ops[&e.pos].clone();
                self.prove(st, &format!("(not (= {} {}))", v, num(ity.int_min())), e.pos, "E-VERIFY-OVERFLOW", format!("that `-{}` does not overflow {} (`{}` is not {})", render(x), ity.name(), render(x), ity.int_min()))?;
                Ok(Val::U(format!("(- {})", v)))
            }
            ExprKind::Forall(..) => unreachable!("rejected by the checker"),
            ExprKind::Bin(op, a, b) => {
                let va = self.eval(a, st)?;
                let ta = va.term().to_string();
                // && and || short-circuit: the right side runs only when needed.
                if matches!(op, BinOp::And | BinOp::Or) {
                    let guard = if *op == BinOp::And { ta.clone() } else { format!("(not {})", ta) };
                    let mut sub = st.clone();
                    sub.pc.push(guard.clone());
                    let vb = self.eval(b, &mut sub)?;
                    // Facts learned on the right side hold whenever it ran.
                    let extra: Vec<_> = sub.pc[st.pc.len() + 1..].to_vec();
                    if !extra.is_empty() {
                        st.pc.push(format!("(=> {} {})", guard, and_all(&extra)));
                    }
                    // So do the new values of anything it lent as `&mut`.
                    let names: Vec<String> = st.env.keys().cloned().collect();
                    for n in names {
                        if sub.env[&n] != st.env[&n] {
                            let m = merge_val(&guard, &sub.env[&n], &st.env[&n]);
                            st.env.insert(n, m);
                        }
                    }
                    let sop = if *op == BinOp::And { "and" } else { "or" };
                    return Ok(Val::B(format!("({} {} {})", sop, ta, vb.term())));
                }
                let vb = self.eval(b, st)?;
                let tb = vb.term().to_string();
                let (ra, rb) = (render(a), render(b));
                let ity = self.int_ops.get(&e.pos).cloned().unwrap_or(Type::U64);
                let max = ity.int_max().to_string();
                if ity.is_signed() && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem) {
                    return self.signed_arith(*op, &ta, &tb, &ra, &rb, &ity, e.pos, st);
                }
                if op.is_bitwise() {
                    return self.bitwise(*op, a, b, &ta, &tb, &ity, e.pos, st);
                }
                let v = match op {
                    BinOp::Add => {
                        let t = format!("(+ {} {})", ta, tb);
                        self.prove(st, &format!("(<= {} {})", t, max), e.pos, "E-VERIFY-OVERFLOW", format!("that `{} + {}` does not overflow {}", ra, rb, ity.name()))?;
                        Val::U(t)
                    }
                    BinOp::Sub => {
                        self.prove(st, &format!("(>= {} {})", ta, tb), e.pos, "E-VERIFY-UNDERFLOW", format!("that `{} - {}` does not go below zero", ra, rb))?;
                        Val::U(format!("(- {} {})", ta, tb))
                    }
                    BinOp::Mul => {
                        let t = format!("(* {} {})", ta, tb);
                        self.prove(st, &format!("(<= {} {})", t, max), e.pos, "E-VERIFY-OVERFLOW", format!("that `{} * {}` does not overflow {}", ra, rb, ity.name()))?;
                        Val::U(t)
                    }
                    BinOp::Div | BinOp::Rem => {
                        self.prove(st, &format!("(not (= {} 0))", tb), e.pos, "E-VERIFY-DIVZERO", format!("that the divisor `{}` is not zero", rb))?;
                        let f = if *op == BinOp::Div { "div" } else { "mod" };
                        Val::U(format!("({} {} {})", f, ta, tb))
                    }
                    BinOp::Eq => Val::B(format!("(= {} {})", ta, tb)),
                    BinOp::Ne => Val::B(format!("(not (= {} {}))", ta, tb)),
                    BinOp::Lt => Val::B(format!("(< {} {})", ta, tb)),
                    BinOp::Le => Val::B(format!("(<= {} {})", ta, tb)),
                    BinOp::Gt => Val::B(format!("(> {} {})", ta, tb)),
                    BinOp::Ge => Val::B(format!("(>= {} {})", ta, tb)),
                _ => unreachable!(),
                };
                Ok(v)
            }
            ExprKind::Call(name, args) => self.call(name, args, e.pos, st),
        }
    }

    /// Arithmetic on a signed type: the result must stay between the type's
    /// minimum (E-VERIFY-UNDERFLOW) and maximum (E-VERIFY-OVERFLOW); `/` and
    /// `%` truncate toward zero, as in C, and `MIN / -1` overflows.
    #[allow(clippy::too_many_arguments)]
    fn signed_arith(&mut self, op: BinOp, ta: &str, tb: &str, ra: &str, rb: &str, ity: &Type, pos: Pos, st: &mut State) -> Result<Val, Diag> {
        let (min, max) = (num(ity.int_min()), num(ity.int_max()));
        let t = match op {
            BinOp::Add => format!("(+ {} {})", ta, tb),
            BinOp::Sub => format!("(- {} {})", ta, tb),
            BinOp::Mul => format!("(* {} {})", ta, tb),
            _ => {
                self.prove(st, &format!("(not (= {} 0))", tb), pos, "E-VERIFY-DIVZERO", format!("that the divisor `{}` is not zero", rb))?;
                self.prove(st, &format!("(not (and (= {} {}) (= {} (- 1))))", ta, min, tb), pos, "E-VERIFY-OVERFLOW", format!("that `{} {} {}` is not {} {} -1, which overflows {}", ra, op.text(), rb, ity.int_min(), op.text(), ity.name()))?;
                let q = c_div(ta, tb);
                return Ok(Val::U(if op == BinOp::Div { q } else { format!("(- {} (* {} {}))", ta, tb, q) }));
            }
        };
        let text = format!("{} {} {}", ra, op.text(), rb);
        self.prove(st, &format!("(<= {} {})", t, max), pos, "E-VERIFY-OVERFLOW", format!("that `{}` does not overflow {} (`<= {}`)", text, ity.name(), ity.int_max()))?;
        self.prove(st, &format!("(<= {} {})", min, t), pos, "E-VERIFY-UNDERFLOW", format!("that `{}` does not go below the minimum of {} (`>= {}`)", text, ity.name(), ity.int_min()))?;
        Ok(Val::U(t))
    }

    /// Bitwise operators on an unsigned type. Z3's integers have no bitwise
    /// operations and its bit-vector bridge is far too slow at 64 bits, so
    /// the kernel cases are exact arithmetic (a low mask is `mod`, a shift
    /// is a multiplication or division by a power of two) and the rest is a
    /// fresh value with facts that always hold (`x & y <= x`, ...).
    #[allow(clippy::too_many_arguments)]
    fn bitwise(&mut self, op: BinOp, a: &Expr, b: &Expr, ta: &str, tb: &str, ity: &Type, pos: Pos, st: &mut State) -> Result<Val, Diag> {
        let (ra, rb) = (render(a), render(b));
        if matches!(op, BinOp::Shl | BinOp::Shr) {
            let w = ity.int_bits().expect("checked: an integer");
            self.prove(st, &format!("(< {} {})", tb, w), pos, "E-VERIFY-SHIFT", format!("that the shift amount `{}` is less than {}, the width of {}", rb, w, ity.name()))?;
            let p = match b.kind {
                ExprKind::Int(k) => (1u128 << k.min(127)).to_string(),
                _ => pow2(tb, w),
            };
            if op == BinOp::Shr {
                return Ok(Val::U(format!("(div {} {})", ta, p)));
            }
            let t = format!("(* {} {})", ta, p);
            self.prove(st, &format!("(<= {} {})", t, ity.int_max()), pos, "E-VERIFY-OVERFLOW", format!("that `{} << {}` loses no bits of {}", ra, rb, ity.name()))?;
            return Ok(Val::U(t));
        }
        // `x & (2^k - 1)` keeps the low k bits.
        if op == BinOp::BitAnd {
            for (lit, other) in [(b, ta), (a, tb)] {
                if let ExprKind::Int(m) = lit.kind {
                    if (m + 1).is_power_of_two() {
                        return Ok(Val::U(format!("(mod {} {})", other, m + 1)));
                    }
                }
            }
        }
        let r = self.fresh_int("bits", "0", &ity.int_max().to_string())?;
        st.pc.push(match op {
            BinOp::BitAnd => format!("(and (<= {r} {ta}) (<= {r} {tb}))"),
            BinOp::BitOr => format!("(and (<= {ta} {r}) (<= {tb} {r}) (<= {r} (+ {ta} {tb})))"),
            _ => format!("(<= {r} (+ {ta} {tb}))"),
        });
        Ok(Val::U(r))
    }

    fn call(&mut self, name: &str, args: &[Expr], pos: Pos, st: &mut State) -> Result<Val, Diag> {
        if name == "locked" {
            return self.locked(args, pos, st);
        }
        if name == "smp_run" {
            return self.smp_run(args, pos, st);
        }
        if name == "pt_borrow" || name == "pt_borrow_frame" {
            return self.pt_borrow(name, args, pos, st);
        }
        for a in args {
            if let Some(p) = match &a.kind {
                ExprKind::MutBorrow(n) => Some(n.clone()),
                _ => place(a),
            } {
                self.open_path(&mut st.env, &p)?;
            }
        }
        // Argument order mirrors the checker: scalars first, then linear values.
        let is_lin = |a: &Expr, st: &State| match &a.kind {
            ExprKind::MutBorrow(_) => true,
            ExprKind::Var(_) | ExprKind::Field(..) => {
                !matches!(get_path(&st.env, &place(a).unwrap()), Val::U(_) | Val::B(_) | Val::Str { .. })
            }
            _ => false,
        };
        let mut vals: Vec<Option<Val>> = vec![None; args.len()];
        for (i, a) in args.iter().enumerate() {
            if !is_lin(a, st) {
                vals[i] = Some(self.eval(a, st)?);
            }
        }
        for (i, a) in args.iter().enumerate() {
            if vals[i].is_none() {
                vals[i] = Some(match &a.kind {
                    ExprKind::MutBorrow(n) => get_path(&st.env, n),
                    _ => self.eval(a, st)?,
                });
            }
        }
        let vals: Vec<Val> = vals.into_iter().map(|v| v.unwrap()).collect();
        if is_builtin(name) {
            return self.builtin(name, args, &vals, pos, st);
        }
        let f = self.prog.fns.iter().find(|f| f.name == name).expect("checked");
        let mut pre = HashMap::new();
        for (p, v) in f.params.iter().zip(&vals) {
            pre.insert(p.name.clone(), v.clone());
        }
        for r in &f.requires {
            let t = self.spec(r, Scope { env: &pre, old: None, result: None }, &mut Vec::new())?;
            self.prove(st, &t, pos, "E-VERIFY-REQUIRES", format!("the precondition `{}` of `{}`", render(r), name))?;
        }
        let res = self.fresh_of(&f.ret, &format!("{}_ret", name))?;
        let mut post = pre.clone();
        for (i, (p, a)) in f.params.iter().zip(args).enumerate() {
            if let (Type::MutRef(_), ExprKind::MutBorrow(var)) = (&p.ty, &a.kind) {
                let sum = self.sums[name][i].clone().expect("a &mut parameter has a summary");
                if sum[0] == (String::new(), true) {
                    // The callee keeps the argument's shape: havoc only what
                    // it may change, as a loop would (shallowest first).
                    for (rel, keep) in &sum {
                        let path = format!("{}{}", var, rel);
                        self.open_path(&mut st.env, &path)?;
                        let old = get_path(&st.env, &path);
                        let nv = self.havoc(&old, *keep, &path.replace('.', "_"))?;
                        set_path(&mut st.env, &path, nv);
                    }
                } else {
                    let nv = self.fresh_of(&p.ty, &var.replace('.', "_"))?;
                    set_path(&mut st.env, var, nv);
                }
                post.insert(p.name.clone(), get_path(&st.env, var));
            }
        }
        for en in &f.ensures {
            let t = self.spec(en, Scope { env: &post, old: Some(&pre), result: Some(&res) }, &mut Vec::new())?;
            st.pc.push(t);
        }
        Ok(res)
    }

    /// The facts a value's struct invariants state, with how to name each
    /// one in a message. Structs held directly as fields count too; ones
    /// behind a box, in a vector or in an enum payload do not (yet).
    fn invariant_terms(&mut self, v: &Val, out: &mut Vec<(String, String)>) -> Result<(), Diag> {
        if let Val::Struct(n, fs) = v {
            let d = struct_def(n).unwrap();
            let env: HashMap<String, Val> = d.fields.iter().map(|(f, _)| f.clone()).zip(fs.iter().cloned()).collect();
            for i in &d.invariants {
                let t = self.spec(i, Scope { env: &env, old: None, result: None }, &mut Vec::new())?;
                out.push((format!("`{}` of {}", render(i), n), t));
            }
            for f in fs {
                self.invariant_terms(f, out)?;
            }
        }
        Ok(())
    }

    fn invariants_of(&mut self, v: &Val) -> Result<Vec<(String, String)>, Diag> {
        let mut out = Vec::new();
        self.invariant_terms(v, &mut out)?;
        Ok(out)
    }

    /// A value read from an atomic: fresh, with its invariants.
    fn atomic_read(&mut self, t: &Type, st: &mut State) -> Result<Val, Diag> {
        let v = self.fresh_of(t, "atomic")?;
        for (_, i) in self.invariants_of(&v)? {
            st.pc.push(i);
        }
        Ok(v)
    }

    /// `locked(l, f, args..)`: the contents are a fresh value about which
    /// only the invariants are known (another processor may have changed
    /// them since this one last looked). `f` gets them as its `&mut` first
    /// argument and must hand them back with the invariants holding again.
    fn locked(&mut self, args: &[Expr], pos: Pos, st: &mut State) -> Result<Val, Diag> {
        let lp = place(&args[0]).expect("checked: a place");
        self.open_path(&mut st.env, &lp)?;
        let Val::Obj(Type::Lock(t), _) = get_path(&st.env, &lp) else { unreachable!("checked: a lock") };
        let ExprKind::Var(f) = &args[1].kind else { unreachable!("checked: a function name") };
        let inner = self.fresh_of(&t, &format!("{}_contents", sanitize(&lp)))?;
        for (_, i) in self.invariants_of(&inner)? {
            st.pc.push(i);
        }
        // The contents live in a variable no program can name, for the call.
        self.counter += 1;
        let key = format!("locked*{}", self.counter);
        st.env.insert(key.clone(), inner);
        let mut cargs = vec![Expr { kind: ExprKind::MutBorrow(key.clone()), pos: args[0].pos }];
        cargs.extend(args[2..].iter().cloned());
        let res = self.call(f, &cargs, pos, st)?;
        let after = st.env.remove(&key).expect("still there");
        for (what, i) in self.invariants_of(&after)? {
            self.prove(st, &i, pos, "E-VERIFY-LOCKINV", format!("that `{}` keeps the invariant {} before the lock `{}` is released", f, what, lp))?;
        }
        Ok(res)
    }

    /// `pt_borrow(&mut t, i, f, args..)`: `f` gets the table in slot `i` of
    /// `t` (`pt_borrow_frame`: the page) as its `&mut` first argument, left
    /// in place. Nothing is known about what is under it, except that it is
    /// as live as `t`: it has `t`'s leases, so `f` cannot take anything out
    /// of it while a processor may be running on the tree. `f` must hand
    /// back the same table (id, level and leases unchanged) or page.
    fn pt_borrow(&mut self, name: &str, args: &[Expr], pos: Pos, st: &mut State) -> Result<Val, Diag> {
        let ra = |i: usize| render(&args[i]).trim_start_matches("&mut ").to_string();
        let ExprKind::MutBorrow(tp) = &args[0].kind else { unreachable!("checked: a &mut argument") };
        self.open_path(&mut st.env, tp)?;
        let t = get_path(&st.env, tp);
        let i = self.eval(&args[1], st)?.term().to_string();
        let ExprKind::Var(f) = &args[2].kind else { unreachable!("checked: a function name") };
        let (lvl, used) = (t.field("level"), t.field("used"));
        self.need(st, format!("(< {} 512)", i), pos, "E-VERIFY-BOUNDS", format!("that the index `{}` is within a page table (512 slots) (`{} < 512`)", ra(1), ra(1)))?;
        let child = if name == "pt_borrow" {
            self.need(st, format!("(>= {} 2)", lvl), pos, "E-VERIFY-PT", format!("that `{}` is not a last-level table: its slots hold tables, not pages", ra(0)))?;
            self.need(st, format!("(=> (= {} 4) (and (<= 1 {i}) (< {i} 256)))", lvl), pos, "E-VERIFY-PT",
                format!("that slot `{}` of a top-level table is a user slot (1 to 255; slot 0 maps the kernel)", ra(1)))?;
            let mut c = self.fresh_of(&Type::PageTable, "pt_borrow")?;
            if let Val::Obj(_, fs) = &mut c {
                fs[1] = format!("(- {} 1)", lvl);
                fs[3] = t.field("leases");
            }
            c
        } else {
            self.need(st, format!("(= {} 1)", lvl), pos, "E-VERIFY-PT", format!("that `{}` is a last-level table (level 1), which maps pages", ra(0)))?;
            self.fresh_of(&Type::Frame, "pt_borrow_frame")?
        };
        self.need(st, format!("(select {} {})", used, i), pos, "E-VERIFY-OWN", format!("that slot `{}` of `{}` holds something (`pt_used({}, {})`)", ra(1), ra(0), ra(0), ra(1)))?;
        self.counter += 1;
        let key = format!("borrowed*{}", self.counter);
        st.env.insert(key.clone(), child.clone());
        let mut cargs = vec![Expr { kind: ExprKind::MutBorrow(key.clone()), pos: args[0].pos }];
        cargs.extend(args[3..].iter().cloned());
        let res = self.call(f, &cargs, pos, st)?;
        let after = st.env.remove(&key).expect("still there");
        let same = match &child {
            Val::Obj(Type::PageTable, _) => format!("(and (= {} {}) (= {} {}) (= {} {}))", after.field("id"), child.field("id"), after.field("level"), child.field("level"), after.field("leases"), child.field("leases")),
            _ => format!("(= {} {})", after.field("id"), child.field("id")),
        };
        self.need(st, same, pos, "E-VERIFY-OWN", format!("that `{}` hands back what `{}` lent it, unchanged but for its contents", f, name))?;
        Ok(res)
    }

    /// `smp_run(f, args..)`: this processor's call is an ordinary call. The
    /// other processors' calls get the same shared arguments and their own
    /// `Cpu` and `Mmu`, with the kernel's tables in CR3, so `f`'s
    /// precondition must hold for those too. They change nothing this
    /// processor can see: everything shared is behind `&` (locks hold their
    /// contents outside the proof's view).
    fn smp_run(&mut self, args: &[Expr], pos: Pos, st: &mut State) -> Result<Val, Diag> {
        let ExprKind::Var(name) = &args[0].kind else { unreachable!("checked: a function name") };
        let f = self.prog.fns.iter().find(|f| &f.name == name).expect("checked");
        if !f.requires.is_empty() {
            let mut pre = HashMap::new();
            for (p, a) in f.params.iter().zip(&args[1..]) {
                let v = match p.ty.base() {
                    Type::Cpu => Val::Obj(Type::Cpu, vec![]),
                    Type::Mmu => Val::Obj(Type::Mmu, vec![KROOT.into()]),
                    _ => self.eval(a, st)?,
                };
                pre.insert(p.name.clone(), v);
            }
            for r in &f.requires {
                let t = self.spec(r, Scope { env: &pre, old: None, result: None }, &mut Vec::new())?;
                self.prove(st, &t, pos, "E-VERIFY-REQUIRES", format!("the precondition `{}` of `{}` on the other processors, which start with the kernel's own tables", render(r), name))?;
            }
        }
        self.call(name, &args[1..], pos, st)
    }

    /// Writes the new value of a `&mut` argument back to its variable.
    fn put_back(&mut self, arg: &Expr, v: Val, st: &mut State) {
        let ExprKind::MutBorrow(n) = &arg.kind else { unreachable!("checked: a &mut argument") };
        set_path(&mut st.env, n, v);
    }

    fn need(&mut self, st: &State, goal: String, pos: Pos, code: &'static str, what: String) -> Result<(), Diag> {
        self.prove(st, &goal, pos, code, what)
    }

    fn builtin(&mut self, name: &str, args: &[Expr], vals: &[Val], pos: Pos, st: &mut State) -> Result<Val, Diag> {
        // `&mut x` reads as plain `x` in messages, matching specification syntax.
        let ra = |i: usize| render(&args[i]).trim_start_matches("&mut ").to_string();
        let index_ok = |this: &mut Self, st: &State, i: &str, lim: &str, k: usize, of: &str| -> Result<(), Diag> {
            this.need(st, format!("(< {} {})", i, lim), pos, "E-VERIFY-BOUNDS", format!("that the index `{}` is within {} (`{} < {}`)", render(&args[k]), of, render(&args[k]), lim))
        };
        match name {
            "box" => Ok(Val::Box(BoxVal::Known(Box::new(vals[0].clone())))),
            "wrap_add" | "wrap_sub" | "wrap_mul" => {
                let ity = self.int_ops.get(&pos).cloned().expect("typed by the checker");
                let op = match name { "wrap_add" => "+", "wrap_sub" => "-", _ => "*" };
                Ok(Val::U(wrap_term(&ity, &format!("({} {} {})", op, vals[0].term(), vals[1].term()))))
            }
            "lock_new" => {
                for (what, i) in self.invariants_of(&vals[0])? {
                    self.need(st, i, pos, "E-VERIFY-LOCKINV", format!("that the value put in the lock satisfies the invariant {}", what))?;
                }
                Ok(Val::Obj(Type::Lock(Box::new(val_type(&vals[0]))), vec![]))
            }
            // An atomic's value may change at any moment, so like a lock's
            // contents only its invariants are known when it is read, and
            // they are proved of every value written.
            "atomic_new" => {
                for (what, i) in self.invariants_of(&vals[0])? {
                    self.need(st, i, pos, "E-VERIFY-LOCKINV", format!("that the value put in the atomic satisfies the invariant {}", what))?;
                }
                let t = self.int_ops.get(&pos).cloned().unwrap_or_else(|| val_type(&vals[0]));
                Ok(Val::Obj(Type::Atomic(Box::new(t)), vec![]))
            }
            "atomic_store" | "atomic_swap" | "atomic_cas" => {
                let Val::Obj(Type::Atomic(t), _) = &vals[0] else { unreachable!("checked: an atomic") };
                let new = if name == "atomic_cas" { &vals[2] } else { &vals[1] };
                for (what, i) in self.invariants_of(new)? {
                    self.need(st, i, pos, "E-VERIFY-LOCKINV", format!("that the value stored in the atomic `{}` satisfies the invariant {}", ra(0), what))?;
                }
                match name {
                    "atomic_store" => Ok(Val::Unit),
                    "atomic_cas" => self.fresh_of(&Type::Bool, "cas"),
                    _ => self.atomic_read(t, st),
                }
            }
            "atomic_end" | "atomic_load" | "atomic_wrap_add" => {
                let Val::Obj(Type::Atomic(t), _) = &vals[0] else { unreachable!("checked: an atomic") };
                self.atomic_read(t, st)
            }
            "lock_end" => {
                let Val::Obj(Type::Lock(t), _) = &vals[0] else { unreachable!("checked: a lock") };
                let v = self.fresh_of(t, "unlocked")?;
                for (_, i) in self.invariants_of(&v)? {
                    st.pc.push(i);
                }
                Ok(v)
            }
            "unbox" => match &vals[0] {
                Val::Box(BoxVal::Known(v)) => Ok((**v).clone()),
                Val::Box(BoxVal::Opaque(t)) => self.fresh_of(t, "unboxed"),
                _ => unreachable!("checked: a box"),
            },
            "alloc" => {
                let n = vals[0].term().to_string();
                self.need(st, format!("(<= {} {})", n, ALLOC_MAX), pos, "E-VERIFY-ALLOC", format!("that the allocation size `{}` is at most 2^60 elements", ra(0)))?;
                // alloc zero-fills.
                Ok(Val::Buf { len: n, data: "((as const (Array Int Int)) 0)".into() })
            }
            "len" => match &vals[0] {
                Val::Buf { len, .. } => Ok(Val::U(len.clone())),
                _ => unreachable!(),
            },
            "get" => match &vals[0] {
                Val::Buf { len, data } => {
                    let i = vals[1].term();
                    self.need(st, format!("(< {} {})", i, len), pos, "E-VERIFY-BOUNDS", format!("that the index `{}` is within `{}` (`{} < len({})`)", ra(1), ra(0), ra(1), ra(0)))?;
                    let sel = format!("(select {} {})", data, i);
                    // Every stored element is a u64 (set only stores u64 values).
                    st.pc.push(format!("(and (<= 0 {s}) (<= {s} {}))", U64_MAX, s = sel));
                    Ok(Val::U(sel))
                }
                _ => unreachable!(),
            },
            "set" => match &vals[0] {
                Val::Buf { len, data } => {
                    let i = vals[1].term();
                    self.need(st, format!("(< {} {})", i, len), pos, "E-VERIFY-BOUNDS", format!("that the index `{}` is within `{}` (`{} < len({})`)", ra(1), ra(0), ra(1), ra(0)))?;
                    Ok(Val::Buf { len: len.clone(), data: format!("(store {} {} {})", data, i, vals[2].term()) })
                }
                _ => unreachable!(),
            },
            "slen" => match &vals[0] {
                Val::Str { len, .. } => Ok(Val::U(len.clone())),
                _ => unreachable!(),
            },
            "sget" => match &vals[0] {
                Val::Str { len, data } => {
                    let i = vals[1].term();
                    self.need(st, format!("(< {} {})", i, len), pos, "E-VERIFY-BOUNDS", format!("that the index `{}` is within the string `{}` (`{} < slen({})`)", ra(1), ra(0), ra(1), ra(0)))?;
                    let sel = format!("(select {} {})", data, i);
                    st.pc.push(format!("(and (<= 0 {s}) (<= {s} 255))", s = sel));
                    Ok(Val::U(sel))
                }
                _ => unreachable!(),
            },
            // A u8 always fits in a byte: the type says what E-VERIFY-BYTE used to prove.
            "serial_write" => Ok(Val::Unit),
            "serial_read" | "serial_poll" => {
                // serial_poll returns 256 when no byte is waiting.
                let v = self.fresh_u64(name)?;
                st.pc.push(format!("(<= {} {})", v.term(), if name == "serial_read" { 255 } else { 256 }));
                Ok(v)
            }
            "free" | "print" | "bfree" => Ok(Val::Unit),

            // ---- vectors ----
            "vec_len" => match &vals[0] {
                Val::Vec { len, .. } => Ok(Val::U(len.clone())),
                _ => unreachable!(),
            },
            "vec_push" => {
                let Val::Vec { elem, len, elems } = vals[0].clone() else { unreachable!() };
                // A push that cannot grow the vector aborts (out of memory),
                // so after it the length stayed below 2^60.
                st.pc.push(format!("(< {} {})", len, ALLOC_MAX));
                let elems2 = vec_write(&elems, &len, &vals[1]);
                if let (true, Val::U(a), Val::U(a2)) = (self.counts, &*elems, &elems2) {
                    st.pc.push(count_step(a2, &format!("(+ {} 1)", len), a, &len, &format!("(ite (= {} x) 1 0)", vals[1].term())));
                }
                let elems = elems2;
                self.put_back(&args[0], Val::Vec { elem, len: format!("(+ {} 1)", len), elems: Box::new(elems) }, st);
                Ok(Val::Unit)
            }
            "vec_pop" => {
                let Val::Vec { elem, len, elems } = vals[0].clone() else { unreachable!() };
                self.need(st, format!("(> {} 0)", len), pos, "E-VERIFY-BOUNDS", format!("that `{}` is not empty (`vlen({}) > 0`)", ra(0), ra(0)))?;
                let last = format!("(- {} 1)", len);
                if let (true, Val::U(a)) = (self.counts, &*elems) {
                    st.pc.push(count_step(a, &last, a, &len, &format!("(- (ite (= (select {} {}) x) 1 0))", a, last)));
                }
                let v = self.vec_read(&elem, &elems, &last, st)?;
                self.put_back(&args[0], Val::Vec { elem, len: last, elems }, st);
                Ok(v)
            }
            "vec_get" | "vec_set" | "vec_swap" => {
                let Val::Vec { elem, len, elems } = vals[0].clone() else { unreachable!() };
                let i = vals[1].term().to_string();
                self.need(st, format!("(< {} {})", i, len), pos, "E-VERIFY-BOUNDS", format!("that the index `{}` is within `{}` (`{} < vlen({})`)", ra(1), ra(0), ra(1), ra(0)))?;
                let old = if name == "vec_set" { Val::Unit } else { self.vec_read(&elem, &elems, &i, st)? };
                if name != "vec_get" {
                    let elems2 = vec_write(&elems, &i, &vals[2]);
                    if let (true, Val::U(a), Val::U(a2)) = (self.counts, &*elems, &elems2) {
                        st.pc.push(count_step(a2, &len, a, &len, &format!("(- (ite (= {} x) 1 0) (ite (= (select {} {}) x) 1 0))", vals[2].term(), a, i)));
                    }
                    let elems = elems2;
                    self.put_back(&args[0], Val::Vec { elem, len, elems: Box::new(elems) }, st);
                }
                Ok(old)
            }
            "vec_free" => {
                let Val::Vec { elem, len, .. } = &vals[0] else { unreachable!() };
                if elem.is_linear() {
                    // Freeing the storage would drop the elements still in it.
                    self.need(st, format!("(= {} 0)", len), pos, "E-VERIFY-LEAK", format!("that `{}` is empty (`vlen({}) == 0`): its {} elements are linear and would be dropped", ra(0), ra(0), elem.name()))?;
                }
                Ok(Val::Unit)
            }

            // ---- bytes ----
            "balloc" => {
                let n = vals[0].term().to_string();
                self.need(st, format!("(<= {} {})", n, ALLOC_MAX), pos, "E-VERIFY-ALLOC", format!("that the allocation size `{}` is at most 2^60 bytes", ra(0)))?;
                Ok(Val::Buf { len: n, data: "((as const (Array Int Int)) 0)".into() })
            }
            "blen" => match &vals[0] {
                Val::Buf { len, .. } => Ok(Val::U(len.clone())),
                _ => unreachable!(),
            },
            "bget" => match &vals[0] {
                Val::Buf { len, data } => {
                    let i = vals[1].term();
                    self.need(st, format!("(< {} {})", i, len), pos, "E-VERIFY-BOUNDS", format!("that the index `{}` is within `{}` (`{} < blen({})`)", ra(1), ra(0), ra(1), ra(0)))?;
                    let sel = format!("(select {} {})", data, i);
                    st.pc.push(format!("(and (<= 0 {s}) (<= {s} 255))", s = sel));
                    Ok(Val::U(sel))
                }
                _ => unreachable!(),
            },
            "bset" => match &vals[0] {
                Val::Buf { len, data } => {
                    let (i, v) = (vals[1].term(), vals[2].term());
                    self.need(st, format!("(< {} {})", i, len), pos, "E-VERIFY-BOUNDS", format!("that the index `{}` is within `{}` (`{} < blen({})`)", ra(1), ra(0), ra(1), ra(0)))?;
                    let nv = Val::Buf { len: len.clone(), data: format!("(store {} {} {})", data, i, v) };
                    self.put_back(&args[0], nv, st);
                    Ok(Val::Unit)
                }
                _ => unreachable!(),
            },
            "bfrom" => match &vals[0] {
                Val::Str { len, data } => Ok(Val::Buf { len: len.clone(), data: data.clone() }),
                _ => unreachable!(),
            },
            "host_argc" => self.fresh_u64("argc"),
            "host_arg" => self.fresh_of(&Type::Bytes, "arg"),
            "host_read" => {
                let nv = self.fresh_of(&Type::Bytes, "file")?;
                self.put_back(&args[2], nv, st);
                self.fresh_bool("read_ok")
            }
            "host_write" => self.fresh_bool("write_ok"),
            "host_eprint" => Ok(Val::Unit),
            "host_env" => {
                let nv = self.fresh_of(&Type::Bytes, "env")?;
                self.put_back(&args[2], nv, st);
                self.fresh_bool("env_set")
            }
            "host_cpus" => self.fresh_u64("cpus"),
            "host_spawn" => self.fresh_of(&Type::Child, "child"),
            "child_write" => self.fresh_bool("child_write_ok"),
            "child_close" => Ok(Val::Unit),
            "child_read_line" => {
                let nv = self.fresh_of(&Type::Bytes, "line")?;
                self.put_back(&args[1], nv, st);
                self.fresh_bool("line_ok")
            }
            // An exit status, or 128 + a signal number.
            "child_wait" => Ok(Val::U(self.fresh_int("status", "0", "255")?)),

            // ---- physical pages ----
            "pool_size" => Ok(Val::U(vals[0].field("size"))),
            "pool_take" => {
                let (p, i) = (&vals[0], vals[1].term());
                self.need(st, format!("(< {} {})", i, p.field("size")), pos, "E-VERIFY-BOUNDS", format!("that page `{}` is within the pool (`{} < pool_size({})`)", ra(1), ra(1), ra(0)))?;
                self.need(st, format!("(select {} {})", p.field("has"), i), pos, "E-VERIFY-OWN", format!("that the pool still has page `{}` (`pool_has({}, {})`): it may already have been handed out", ra(1), ra(0), ra(1)))?;
                let np = Val::Obj(Type::Pool, vec![format!("(store {} {} false)", p.field("has"), i), p.field("size")]);
                self.put_back(&args[0], np, st);
                Ok(Val::Obj(Type::Frame, vec![i.to_string()]))
            }
            "pool_give" => {
                let (p, f) = (&vals[0], &vals[1]);
                let np = Val::Obj(Type::Pool, vec![format!("(store {} {} true)", p.field("has"), f.field("id")), p.field("size")]);
                self.put_back(&args[0], np, st);
                Ok(Val::Unit)
            }
            "frame_id" => Ok(Val::U(vals[0].field("id"))),
            "frame_read" | "frame_write" => {
                let w = vals[1].term().to_string();
                index_ok(self, st, &w, "512", 1, "a page (512 words)")?;
                if name == "frame_write" {
                    return Ok(Val::Unit);
                }
                self.fresh_u64("frame_read")
            }
            "frame_read8" | "frame_write8" => {
                let w = vals[1].term().to_string();
                index_ok(self, st, &w, "4096", 1, "a page (4096 bytes)")?;
                if name == "frame_write8" {
                    return Ok(Val::Unit);
                }
                let v = self.fresh_u64("frame_read8")?;
                st.pc.push(format!("(<= {} 255)", v.term()));
                Ok(v)
            }

            // ---- page tables ----
            "pt_new" => {
                let (f, lvl) = (&vals[0], vals[1].term());
                self.need(st, format!("(and (<= 1 {l}) (<= {l} 5))", l = lvl), pos, "E-VERIFY-PT", format!("that the level `{}` is between 1 and 5", ra(1)))?;
                Ok(Val::Obj(Type::PageTable, vec![f.field("id"), lvl.to_string(), NO_BOOL.into(), "0".into()]))
            }
            "pt_free" | "pt_free_root" => {
                let k = if name == "pt_free" { 0 } else { 1 };
                let t = &vals[k];
                let (lvl, used) = (t.field("level"), t.field("used"));
                if name == "pt_free" {
                    self.need(st, format!("(not (= {} 4))", lvl), pos, "E-VERIFY-PT", format!("that `{}` is not a top-level table (free those with `pt_free_root`)", ra(k)))?;
                    self.need(st, format!("(forall ((k Int)) (=> (and (<= 0 k) (< k 512)) (not (select {} k))))", used), pos, "E-VERIFY-OWN",
                        format!("that `{}` is empty: freeing a table that still owns pages would lose them", ra(k)))?;
                } else {
                    self.need(st, format!("(= {} 4)", lvl), pos, "E-VERIFY-PT", format!("that `{}` is a top-level table (level 4)", ra(k)))?;
                    self.need(st, format!("(forall ((k Int)) (=> (and (<= 1 k) (< k 256)) (not (select {} k))))", used), pos, "E-VERIFY-OWN",
                        format!("that `{}` is empty: freeing a table that still owns pages would lose them", ra(k)))?;
                    self.need(st, format!("(not (= {} {}))", t.field("id"), vals[0].field("root")), pos, "E-VERIFY-MMU",
                        format!("that `{}` is not the address space the MMU is using (`pt_id({}) != mmu_root({})`)", ra(k), ra(k), ra(0)))?;
                    self.need(st, format!("(= {} 0)", t.field("leases")), pos, "E-VERIFY-LEASE",
                        format!("that no processor holds a lease on `{}` (`pt_leases({}) == 0`): one may be running on it", ra(k), ra(k)))?;
                }
                Ok(Val::Obj(Type::Frame, vec![t.field("id")]))
            }
            "pt_has" => {
                let i = vals[1].term().to_string();
                index_ok(self, st, &i, "512", 1, "a page table (512 slots)")?;
                // Slot 0 of a top-level table maps the kernel on the kernel
                // target: the hardware entry is present although the table
                // does not own it, so what pt_has says there is unknown.
                let k = self.fresh_bool("pt_has_kernel")?;
                Ok(Val::B(format!("(ite (and (= {} 4) (= {} 0)) {} (select {} {}))", vals[0].field("level"), i, k.term(), vals[0].field("used"), i)))
            }
            "pt_writable" => {
                // Whether the page in slot i of a last-level table may be
                // written. Nothing the proof tracks: the answer is unknown.
                let i = vals[1].term().to_string();
                index_ok(self, st, &i, "512", 1, "a page table (512 slots)")?;
                self.need(st, format!("(= {} 1)", vals[0].field("level")), pos, "E-VERIFY-PT", format!("that `{}` is a last-level table (level 1), which maps pages", ra(0)))?;
                let w = self.fresh_bool("pt_writable")?;
                Ok(w)
            }
            "pt_put" | "pt_take" | "pt_put_frame" | "pt_take_frame" => {
                let t = &vals[0];
                let i = vals[1].term().to_string();
                let (lvl, used) = (t.field("level"), t.field("used"));
                index_ok(self, st, &i, "512", 1, "a page table (512 slots)")?;
                let leaf = name.ends_with("_frame");
                if leaf {
                    self.need(st, format!("(= {} 1)", lvl), pos, "E-VERIFY-PT", format!("that `{}` is a last-level table (level 1), which maps pages", ra(0)))?;
                } else {
                    self.need(st, format!("(>= {} 2)", lvl), pos, "E-VERIFY-PT", format!("that `{}` is not a last-level table: its slots hold tables, not pages", ra(0)))?;
                    // Slot 0 of a top-level table maps the kernel; slots 256.. are never used.
                    self.need(st, format!("(=> (= {} 4) (and (<= 1 {i}) (< {i} 256)))", lvl), pos, "E-VERIFY-PT",
                        format!("that slot `{}` of a top-level table is a user slot (1 to 255; slot 0 maps the kernel)", ra(1)))?;
                }
                let putting = name.starts_with("pt_put");
                if putting {
                    self.need(st, format!("(not (select {} {}))", used, i), pos, "E-VERIFY-OWN", format!("that slot `{}` of `{}` is empty (`!pt_used({}, {})`): it would lose what is there", ra(1), ra(0), ra(0), ra(1)))?;
                    if !leaf {
                        self.need(st, format!("(= (+ {} 1) {})", vals[2].field("level"), lvl), pos, "E-VERIFY-PT",
                            format!("that `{}` is one level below `{}` (`pt_level({}) + 1 == pt_level({})`)", ra(2), ra(0), ra(2), ra(0)))?;
                    }
                } else {
                    self.need(st, format!("(select {} {})", used, i), pos, "E-VERIFY-OWN", format!("that slot `{}` of `{}` holds something (`pt_used({}, {})`)", ra(1), ra(0), ra(0), ra(1)))?;
                    // Taking unmaps: another processor running on the tree
                    // could still reach what is taken, through its TLB or a
                    // walk under way. Putting only adds.
                    self.need(st, format!("(= {} 0)", t.field("leases")), pos, "E-VERIFY-LEASE",
                        format!("that no processor may be running on `{}` (`pt_leases({}) == 0`): taking would unmap under it", ra(0), ra(0)))?;
                }
                let nt = Val::Obj(Type::PageTable, vec![t.field("id"), lvl.clone(), format!("(store {} {} {})", used, i, putting), t.field("leases")]);
                self.put_back(&args[0], nt, st);
                match name {
                    "pt_take" => {
                        let mut c = self.fresh_of(&Type::PageTable, "pt_take")?;
                        if let Val::Obj(_, fs) = &mut c {
                            fs[1] = format!("(- {} 1)", lvl);
                            fs[3] = "0".into();
                        }
                        Ok(c)
                    }
                    "pt_take_frame" => self.fresh_of(&Type::Frame, "pt_take_frame"),
                    _ => Ok(Val::Unit),
                }
            }

            // ---- I/O ports ----
            "port_claim" => {
                let (p, b, c) = (&vals[0], vals[1].term(), vals[2].term());
                let inr = format!("(and (<= {b} k) (< k (+ {b} {c})))");
                self.need(st, format!("(<= (+ {} {}) {})", b, c, PORT_LIMIT), pos, "E-VERIFY-BOUNDS",
                    format!("that the ports `{}` .. `{} + {}` exist (`{} + {} <= 65536`)", ra(1), ra(1), ra(2), ra(1), ra(2)))?;
                self.need(st, format!("(forall ((k Int)) (=> {} (select {} k)))", inr, p.field("free")), pos, "E-VERIFY-OWN",
                    format!("that nobody has claimed any of these ports (`port_free({}, k)` for each): two owners could drive one device", ra(0)))?;
                // A lambda, not a fresh array with a quantified definition:
                // reads of it reduce directly, so chains of claims stay
                // quantifier-free.
                let nf = format!("(lambda ((k Int)) (and (select {} k) (not {})))", p.field("free"), inr);
                self.put_back(&args[0], Val::Obj(Type::Ports, vec![nf]), st);
                Ok(Val::Obj(Type::PortIo, vec![b.to_string(), c.to_string()]))
            }
            "port_release" => {
                let (p, io) = (&vals[0], &vals[1]);
                let inr = format!("(and (<= {b} k) (< k (+ {b} {c})))", b = io.field("base"), c = io.field("count"));
                let nf = format!("(lambda ((k Int)) (or (select {} k) {}))", p.field("free"), inr);
                self.put_back(&args[0], Val::Obj(Type::Ports, vec![nf]), st);
                Ok(Val::Unit)
            }
            // ---- memory-mapped device registers ----
            "mmio_claim" => {
                let (p, b, l) = (&vals[0], vals[1].term(), vals[2].term());
                let inr = format!("(and (<= {b} k) (< k (+ {b} {l})))");
                self.need(st, format!("(and (<= {} {}) (<= (+ {} {}) {}))", MMIO_LO, b, b, l, MMIO_HI), pos, "E-VERIFY-BOUNDS",
                    format!("that `{}` .. `{} + {}` lies in the device window (0x80000000 to 2^46)", ra(1), ra(1), ra(2)))?;
                self.need(st, format!("(forall ((k Int)) (=> {} (select {} k)))", inr, p.field("free")), pos, "E-VERIFY-OWN",
                    format!("that nobody has claimed any of these addresses (`mmio_free({}, a)` for each): two owners could drive one device", ra(0)))?;
                let nf = format!("(lambda ((k Int)) (and (select {} k) (not {})))", p.field("free"), inr);
                self.put_back(&args[0], Val::Obj(Type::MmioSpace, vec![nf]), st);
                Ok(Val::Obj(Type::Mmio, vec![b.to_string(), l.to_string()]))
            }
            "mmio_release" => {
                let (p, io) = (&vals[0], &vals[1]);
                let inr = format!("(and (<= {b} k) (< k (+ {b} {l})))", b = io.field("base"), l = io.field("len"));
                let nf = format!("(lambda ((k Int)) (or (select {} k) {}))", p.field("free"), inr);
                self.put_back(&args[0], Val::Obj(Type::MmioSpace, vec![nf]), st);
                Ok(Val::Unit)
            }
            "mmio_base" => Ok(Val::U(vals[0].field("base"))),
            "mmio_len" => Ok(Val::U(vals[0].field("len"))),
            "mmio_read8" | "mmio_read16" | "mmio_read32" | "mmio_read64" | "mmio_write8" | "mmio_write16" | "mmio_write32" | "mmio_write64" => {
                let (bytes, max) = match name.trim_start_matches("mmio_read").trim_start_matches("mmio_write") {
                    "8" => (1, "255"),
                    "16" => (2, "65535"),
                    "32" => (4, "4294967295"),
                    _ => (8, U64_MAX),
                };
                let (io, off) = (&vals[0], vals[1].term());
                self.need(st, format!("(<= (+ {} {}) {})", off, bytes, io.field("len")), pos, "E-VERIFY-BOUNDS",
                    format!("that register offset `{}` is within `{}` (`{} + {} <= mmio_len({})`)", ra(1), ra(0), ra(1), bytes, ra(0)))?;
                // Devices want naturally aligned accesses (and an unaligned
                // one could straddle into the next register).
                self.need(st, format!("(= (mod (+ {} {}) {}) 0)", io.field("base"), off, bytes), pos, "E-VERIFY-BOUNDS",
                    format!("that the access at offset `{}` of `{}` is aligned to {} bytes", ra(1), ra(0), bytes))?;
                if name.starts_with("mmio_write") {
                    return Ok(Val::Unit);
                }
                let v = self.fresh_u64(name)?;
                st.pc.push(format!("(<= {} {})", v.term(), max));
                Ok(v)
            }
            // ---- pages lent to devices (DMA) ----
            // Lending and taking back only change the type: the page keeps
            // its number, and while it is a `Dma` nothing can free, map or
            // reuse it. That the device has stopped using it by `dma_take`
            // is the driver's word (devices are outside the proof).
            "dma_lend" => Ok(Val::Obj(Type::Dma, vec![vals[0].field("id")])),
            "dma_take" => Ok(Val::Obj(Type::Frame, vec![vals[0].field("id")])),
            "dma_id" => Ok(Val::U(vals[0].field("id"))),
            "dma_addr" => self.fresh_u64("dma_addr"),
            "dma_read8" | "dma_read16" | "dma_read32" | "dma_read64" | "dma_write8" | "dma_write16" | "dma_write32" | "dma_write64" => {
                let (bytes, max) = match name.trim_start_matches("dma_read").trim_start_matches("dma_write") {
                    "8" => (1, "255"),
                    "16" => (2, "65535"),
                    "32" => (4, "4294967295"),
                    _ => (8, U64_MAX),
                };
                let off = vals[1].term();
                self.need(st, format!("(<= (+ {} {}) 4096)", off, bytes), pos, "E-VERIFY-BOUNDS",
                    format!("that offset `{}` is within the page (`{} + {} <= 4096`)", ra(1), ra(1), bytes))?;
                // The page is page-aligned, so an aligned offset is an
                // aligned address: devices see whole fields, never torn ones.
                self.need(st, format!("(= (mod {} {}) 0)", off, bytes), pos, "E-VERIFY-BOUNDS",
                    format!("that offset `{}` is aligned to {} bytes", ra(1), bytes))?;
                if name.starts_with("dma_write") {
                    return Ok(Val::Unit);
                }
                let v = self.fresh_u64(name)?;
                st.pc.push(format!("(<= {} {})", v.term(), max));
                Ok(v)
            }
            "port_base" => Ok(Val::U(vals[0].field("base"))),
            "port_count" => Ok(Val::U(vals[0].field("count"))),
            "pio_in8" | "pio_in16" | "pio_in32" | "pio_out8" | "pio_out16" | "pio_out32" => {
                let (bytes, max) = match &name[name.len() - 1..] {
                    "8" => (1, "255"),
                    "6" => (2, "65535"),
                    _ => (4, "4294967295"),
                };
                let (io, off) = (&vals[0], vals[1].term());
                self.need(st, format!("(<= (+ {} {}) {})", off, bytes, io.field("count")), pos, "E-VERIFY-BOUNDS",
                    format!("that port offset `{}` is within `{}` (`{} + {} <= port_count({})`)", ra(1), ra(0), ra(1), bytes, ra(0)))?;
                if name.starts_with("pio_out") {
                    return Ok(Val::Unit);
                }
                let v = self.fresh_u64(name)?;
                st.pc.push(format!("(<= {} {})", v.term(), max));
                Ok(v)
            }

            // ---- MMU and CPU ----
            "mmu_kernel" => {
                self.put_back(&args[0], Val::Obj(Type::Mmu, vec![KROOT.into()]), st);
                Ok(Val::Unit)
            }
            // A lease counts this processor among those that may be running
            // on the table, so nothing is taken out of its tree until the
            // lease is given back.
            "mmu_enter" => {
                let t = &vals[1];
                self.need(st, format!("(= {} 4)", t.field("level")), pos, "E-VERIFY-PT", format!("that `{}` is a top-level table (level 4)", ra(1)))?;
                self.need(st, format!("(= {} {})", vals[0].field("root"), KROOT), pos, "E-VERIFY-MMU",
                    format!("that `{}` has the kernel's own tables loaded (`mmu_root({}) == {}`), not another lease's", ra(0), ra(0), KROOT))?;
                let id = t.field("id");
                self.put_back(&args[0], Val::Obj(Type::Mmu, vec![id.clone()]), st);
                self.put_back(&args[1], Val::Obj(Type::PageTable, vec![id.clone(), t.field("level"), t.field("used"), format!("(+ {} 1)", t.field("leases"))]), st);
                Ok(Val::Obj(Type::Lease, vec![]))
            }
            // Loads the kernel's tables and gives the lease back. No table's
            // count in the proof drops (none is named): they only ever
            // overcount, which is safe. `pt_lease_count` reads the true one.
            "mmu_exit" => {
                self.put_back(&args[0], Val::Obj(Type::Mmu, vec![KROOT.into()]), st);
                Ok(Val::Unit)
            }
            // The number of leases outstanding on the table, which only its
            // holder (this `&mut`) could raise: from here on that is what the
            // proof knows.
            "pt_lease_count" => {
                let t = &vals[0];
                let n = self.fresh_u64("pt_lease_count")?;
                self.put_back(&args[0], Val::Obj(Type::PageTable, vec![t.field("id"), t.field("level"), t.field("used"), n.term().to_string()]), st);
                Ok(n)
            }
            // The page's contents are not tracked, so neither changes anything
            // the proof knows about.
            "cpu_fpu_save" | "cpu_fpu_load" => Ok(Val::Unit),
            "cpu_run" | "cpu_idle" => {
                let v = self.fresh_u64("vector")?;
                st.pc.push(format!("(<= {} 255)", v.term()));
                Ok(v)
            }
            "boot_fb" => {
                // The prelude reports a framebuffer only if it ends at or
                // below 4 GiB, so every field fits in 32 bits.
                let x = self.fresh_u64("fb")?;
                st.pc.push(format!("(<= {} 4294967295)", x.term()));
                Ok(x)
            }
            "cpu_count" => {
                let n = self.fresh_u64("cpus")?;
                // The platform's promise: at least this processor, and no
                // more than 4096 (so per-processor tables can be allocated).
                st.pc.push(format!("(and (>= {0} 1) (<= {0} 4096))", n.term()));
                Ok(n)
            }
            // Any number: the proof knows nothing about the counter, not
            // even that it goes up.
            "cpu_tsc" => self.fresh_u64("tsc"),
            "cpu_get" => {
                let r = vals[1].term().to_string();
                index_ok(self, st, &r, "21", 1, "the saved registers")?;
                self.fresh_u64("reg")
            }
            "cpu_set" => {
                // Registers 0-17 and the FS base (20); the error code and
                // fault address (18, 19) are only reported by a trap.
                let r = vals[1].term().to_string();
                self.need(st, format!("(and (<= 0 {r}) (or (< {r} 18) (= {r} 20)))"), pos, "E-VERIFY-BOUNDS",
                    format!("that register `{}` can be set (0-17 or 20)", ra(1)))?;
                Ok(Val::Unit)
            }

            // ---- user programs ----
            "sys_spawn" => {
                let (from, to) = (vals[1].term(), vals[2].term());
                let Val::Buf { len, .. } = &vals[0] else { unreachable!() };
                self.need(st, format!("(and (<= {} {}) (<= {} {}))", from, to, to, len), pos, "E-VERIFY-BOUNDS",
                    format!("that `{}..{}` is a range within `{}`", ra(1), ra(2), ra(0)))?;
                self.fresh_u64("spawn")
            }
            "sys_wait" | "sys_kill" | "sys_getpid" | "sys_info" | "sys_call" => self.fresh_u64(name),
            "sys_send" => {
                let n = vals[3].term();
                let Val::Buf { len, .. } = &vals[2] else { unreachable!() };
                self.need(st, format!("(<= {} {})", n, len), pos, "E-VERIFY-BOUNDS",
                    format!("that `{}` bytes fit in `{}` (`{} <= blen({})`)", ra(3), ra(2), ra(3), ra(2)))?;
                self.fresh_u64(name)
            }
            // The kernel may overwrite every byte of the buffer, but not its length.
            "sys_recv" => {
                let Val::Buf { len, .. } = &vals[2] else { unreachable!() };
                let len = len.clone();
                let Val::Buf { data, .. } = self.fresh_of(&Type::Bytes, "recv")? else { unreachable!() };
                self.put_back(&args[2], Val::Buf { len, data }, st);
                self.fresh_u64(name)
            }
            "sys_yield" | "sys_ps" | "sys_poweroff" => Ok(Val::Unit),
            _ => unreachable!("unknown builtin {}", name),
        }
    }

    /// The value a specification names: a variable, `result`, `old(x)` or a literal.
    fn spec_val(&mut self, e: &Expr, sc: Scope, bound: &mut Vec<(String, String)>) -> Result<Val, Diag> {
        match &e.kind {
            ExprKind::Var(n) if n == "result" => Ok(sc.result.expect("checked").clone()),
            ExprKind::Var(n) => match bound.iter().rev().find(|b| &b.0 == n) {
                Some(b) => Ok(Val::U(b.1.clone())),
                None => Ok(sc.env[n].clone()),
            },
            ExprKind::Call(n, xs) if n == "old" => {
                let old = sc.old.expect("checked: old() only in ensures and loop invariants");
                self.spec_val(&xs[0], Scope { env: old, old: None, result: None }, bound)
            }
            ExprKind::Str(bytes) => Ok(str_val(bytes)),
            ExprKind::Call(n, xs) if n == "vget" => {
                let Val::Vec { elems, .. } = self.spec_val(&xs[0], sc, bound)? else { unreachable!("checked: a Vec") };
                let i = self.spec(&xs[1], sc, bound)?;
                Ok(map_leaves(&elems, &mut |arr| format!("(select {} {})", arr, i)))
            }
            ExprKind::Field(b, f) => match self.spec_val(b, sc, bound).and_then(|v| self.look_inside(v))? {
                Val::Struct(n, fs) => {
                    let k = struct_def(&n).unwrap().fields.iter().position(|x| &x.0 == f).expect("checked field");
                    Ok(fs[k].clone())
                }
                // `o.V`: the payload slot of variant V.
                Val::Enum(n, _, ps) => Ok(ps[enum_def(&n).unwrap().index(f).expect("checked variant")].clone()),
                _ => unreachable!("checked: a field of a struct"),
            },
            _ => Err(Diag::new("E-TYPE", e.pos, "this argument must be a variable, a field of one, `result` or `old(x)`")),
        }
    }

    /// Translates a ghost specification expression. No obligations.
    fn spec(&mut self, e: &Expr, sc: Scope, bound: &mut Vec<(String, String)>) -> Result<String, Diag> {
        Ok(match &e.kind {
            ExprKind::TypeApp(..) => unreachable!("removed by monomorphization"),
            ExprKind::Int(v) => v.to_string(),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(_) | ExprKind::Blob(_) => {
                return Err(Diag::new("E-TYPE", e.pos, "a string literal can only appear as the argument of `slen` or `sget` in a specification"))
            }
            ExprKind::MutBorrow(_) | ExprKind::StructLit(..) | ExprKind::Variant(..) => unreachable!("rejected by the checker"),
            ExprKind::Is(x, _, v) => match self.spec_val(x, sc, bound)? {
                Val::Enum(n, tag, _) => format!("(= {} {})", tag, enum_def(&n).unwrap().index(v).unwrap()),
                _ => unreachable!("checked: an enum"),
            },
            ExprKind::Var(_) | ExprKind::Field(..) => self.spec_val(e, sc, bound)?.term().to_string(),
            ExprKind::Not(x) => format!("(not {})", self.spec(x, sc, bound)?),
            ExprKind::Neg(x) => format!("(- {})", self.spec(x, sc, bound)?),
            ExprKind::Bin(op, a, b) => {
                // `==` on a value that names a struct or enum compares it
                // part by part (the checker allows it only between such
                // values); on a number it is the same `(= a b)` as below.
                if matches!(op, BinOp::Eq | BinOp::Ne) && spec_place(a) {
                    let va = self.spec_val(a, sc, bound)?;
                    let eq = match va {
                        Val::U(_) | Val::B(_) => format!("(= {} {})", va.term(), self.spec(b, sc, bound)?),
                        _ => {
                            let vb = self.spec_val(b, sc, bound)?;
                            eq_term(&va, &vb)
                        }
                    };
                    return Ok(if *op == BinOp::Ne { format!("(not {})", eq) } else { eq });
                }
                let ta = self.spec(a, sc, bound)?;
                let tb = self.spec(b, sc, bound)?;
                let f = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "div",
                    BinOp::Rem => "mod",
                    BinOp::Eq => "=",
                    BinOp::Ne => return Ok(format!("(not (= {} {}))", ta, tb)),
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    BinOp::And => "and",
                    BinOp::Or => "or",
                    BinOp::Implies => "=>",
                    _ => unreachable!("rejected by the checker"),
                };
                format!("({} {} {})", f, ta, tb)
            }
            ExprKind::Forall(v, body) => {
                // A fresh SMT name per quantifier, so expanding a spec fn can never capture.
                let q = self.fresh_name(&format!("q_{}", v));
                bound.push((v.clone(), q.clone()));
                let tb = self.spec(body, sc, bound)?;
                bound.pop();
                format!("(forall (({q} Int)) (=> (and (<= 0 {q}) (<= {q} {})) {}))", U64_MAX, tb)
            }
            ExprKind::Call(name, args) => {
                if name == "old" {
                    let old = sc.old.expect("checked: old() only in ensures and loop invariants");
                    return self.spec(&args[0], Scope { env: old, old: None, result: None }, bound);
                }
                if let Some(sf) = self.prog.specs.iter().find(|s| &s.name == name).cloned() {
                    let mut env = HashMap::new();
                    for (p, a) in sf.params.iter().zip(args) {
                        let v = match p.ty.base() {
                            t if t.is_int() => Val::U(self.spec(a, sc, bound)?),
                            Type::Bool => Val::B(self.spec(a, sc, bound)?),
                            _ => self.spec_val(a, sc, bound)?,
                        };
                        env.insert(p.name.clone(), v);
                    }
                    return self.spec(&sf.body, Scope { env: &env, old: None, result: None }, &mut Vec::new());
                }
                if name == "vget" {
                    return Ok(self.spec_val(e, sc, bound)?.term().to_string());
                }
                if name == "vcount" {
                    let Val::Vec { len, elems, .. } = self.spec_val(&args[0], sc, bound)? else { unreachable!("checked: a Vec") };
                    let x = self.spec(&args[1], sc, bound)?;
                    return Ok(format!("(cnt {} {} {})", elems.term(), len, x));
                }
                let v = self.spec_val(&args[0], sc, bound)?;
                match name.as_str() {
                    "vlen" => match v {
                        Val::Vec { len, .. } => len,
                        _ => unreachable!(),
                    },
                    "len" | "slen" | "blen" => match v {
                        Val::Buf { len, .. } | Val::Str { len, .. } => len,
                        _ => unreachable!(),
                    },
                    "get" | "sget" | "bget" => match v {
                        Val::Buf { data, .. } | Val::Str { data, .. } => format!("(select {} {})", data, self.spec(&args[1], sc, bound)?),
                        _ => unreachable!(),
                    },
                    "frame_id" | "pt_id" | "dma_id" => v.field("id"),
                    "pt_level" => v.field("level"),
                    "pt_leases" => v.field("leases"),
                    "pool_size" => v.field("size"),
                    "mmu_root" => v.field("root"),
                    "port_base" => v.field("base"),
                    "port_count" => v.field("count"),
                    "port_free" | "mmio_free" => format!("(select {} {})", v.field("free"), self.spec(&args[1], sc, bound)?),
                    "mmio_base" => v.field("base"),
                    "mmio_len" => v.field("len"),
                    "pt_used" => format!("(select {} {})", v.field("used"), self.spec(&args[1], sc, bound)?),
                    "pool_has" => format!("(select {} {})", v.field("has"), self.spec(&args[1], sc, bound)?),
                    _ => unreachable!("checked: {}", name),
                }
            }
        })
    }
}

/// Places a loop body (or a function body) may change, with a flag that is
/// true when every change keeps the value's shape: `x = set(x, ..)` keeps a
/// Buf's length, the builtins in SHAPE_KEEPING keep an object's id, level
/// and size, and a user function changes what its summary says.
fn changed_vars(stmts: &[Stmt], out: &mut HashMap<String, bool>, sums: &Summaries) {
    for s in stmts {
        match &s.kind {
            StmtKind::Assign { name, value } => {
                let only_set = matches!(&value.kind, ExprKind::Call(f, args) if f == "set" && place(&args[0]).as_deref() == Some(name.as_str()));
                let e = out.entry(name.clone()).or_insert(true);
                *e = *e && only_set;
                changed_in_expr(value, out, sums);
            }
            StmtKind::Let { init, .. } => changed_in_expr(init, out, sums),
            StmtKind::Expr(e) | StmtKind::Return(Some(e)) => changed_in_expr(e, out, sums),
            StmtKind::If { cond, then_b, else_b } => {
                changed_in_expr(cond, out, sums);
                changed_vars(then_b, out, sums);
                changed_vars(else_b, out, sums);
            }
            StmtKind::While { cond, body, .. } => {
                changed_in_expr(cond, out, sums);
                changed_vars(body, out, sums);
            }
            StmtKind::Match { scrut, arms } => {
                changed_in_expr(scrut, out, sums);
                for a in arms {
                    changed_vars(&a.body, out, sums);
                }
            }
            StmtKind::Return(None) | StmtKind::Assert(_) | StmtKind::Unreachable => {}
        }
    }
}

/// Records that `path` may change (`keep`: only in shape-keeping ways).
fn note(out: &mut HashMap<String, bool>, path: String, keep: bool) {
    let parts: Vec<&str> = path.split('.').collect();
    let (path, keep) = if parts.len() > SUMMARY_DEPTH { (parts[..SUMMARY_DEPTH].join("."), false) } else { (path, keep) };
    let x = out.entry(path).or_insert(true);
    *x = *x && keep;
}

/// Places lent as `&mut` anywhere in `e`, and what the callee may change
/// under them.
fn changed_in_expr(e: &Expr, out: &mut HashMap<String, bool>, sums: &Summaries) {
    match &e.kind {
        ExprKind::Call(f, args) => {
            // `locked(l, f, ..)` and `smp_run(f, ..)` lend their trailing
            // arguments to `f`: shift them onto its parameters.
            // `pt_borrow(t, i, f, ..)` keeps `t` and lends the rest to `f`.
            let (f, skip, shift) = match (f.as_str(), args.get(1), args.first(), args.get(2)) {
                ("locked", Some(Expr { kind: ExprKind::Var(g), .. }), _, _) => (g, 2, 1),
                ("smp_run", _, Some(Expr { kind: ExprKind::Var(g), .. }), _) => (g, 1, 1),
                ("pt_borrow" | "pt_borrow_frame", _, Some(Expr { kind: ExprKind::MutBorrow(t), .. }), Some(Expr { kind: ExprKind::Var(g), .. })) => {
                    note(out, t.clone(), true);
                    changed_in_expr(&args[1], out, sums);
                    (g, 3, 2)
                }
                _ => (f, 0, 0),
            };
            for (i, a) in args.iter().enumerate().skip(skip) {
                let i = i - shift;
                if let ExprKind::MutBorrow(n) = &a.kind {
                    if is_builtin(f) {
                        note(out, n.clone(), SHAPE_KEEPING.contains(&f.as_str()));
                    } else {
                        match sums.get(f).and_then(|ps| ps.get(i).cloned()).flatten() {
                            Some(sum) => sum.into_iter().for_each(|(rel, keep)| note(out, format!("{}{}", n, rel), keep)),
                            None => note(out, n.clone(), false),
                        }
                    }
                } else {
                    changed_in_expr(a, out, sums);
                }
            }
        }
        ExprKind::Not(x) | ExprKind::Neg(x) => changed_in_expr(x, out, sums),
        ExprKind::Bin(_, a, b) => {
            changed_in_expr(a, out, sums);
            changed_in_expr(b, out, sums);
        }
        ExprKind::StructLit(_, fs) => {
            for (_, x) in fs {
                changed_in_expr(x, out, sums);
            }
        }
        ExprKind::Variant(_, _, Some(x)) | ExprKind::Is(x, _, _) => changed_in_expr(x, out, sums),
        _ => {}
    }
}

/// The SMT value of a string literal: exact length and exact bytes.
fn str_val(bytes: &[u8]) -> Val {
    let mut data = "((as const (Array Int Int)) 0)".to_string();
    for (i, b) in bytes.iter().enumerate() {
        data = format!("(store {} {} {})", data, i, b);
    }
    Val::Str { len: bytes.len().to_string(), data }
}
