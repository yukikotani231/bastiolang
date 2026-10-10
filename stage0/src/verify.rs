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
//   PageTable  id, level, used[i] (which of its 512 slots own something)
//   Pool       has[i] (the page numbers it can still hand out), size
//   Mmu        root (the id of the table in CR3; KROOT for the kernel's own)
// A struct value is the values of its fields; places like `p.root` read and
// write inside it.
// The builtins' preconditions over this state (see `builtin`) are what make
// page ownership, page-table structure and CR3 switching safe.
//
// `&mut x` arguments: the callee's `requires` see x's value at the call; after
// the call x gets a fresh value, constrained only by the callee's `ensures`,
// where `x` means the new value and `old(..)` the value at the call.

use crate::ast::*;
use crate::check::is_builtin;
use crate::diag::{Diag, Pos};
use crate::smt::{SatResult, Solver};
use std::collections::HashMap;

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
        Type::PageTable => &[("id", false, "pt_id"), ("level", false, "pt_level"), ("used", true, "")],
        Type::Pool => &[("has", true, ""), ("size", false, "pool_size")],
        Type::Mmu => &[("root", false, "mmu_root")],
        _ => &[],
    }
}

/// Ghost fields a loop keeps when the object is changed only by builtins
/// that never change them (a page table keeps its id and level).
fn kept_by_builtins(t: &Type, field: &str) -> bool {
    matches!((t, field), (Type::Frame, "id") | (Type::PageTable, "id") | (Type::PageTable, "level") | (Type::Pool, "size"))
}

/// Builtins whose `&mut` argument keeps the fields above.
const SHAPE_KEEPING: &[&str] =
    &["bset", "vec_set", "vec_swap", "pt_put", "pt_take", "pt_put_frame", "pt_take_frame", "pool_take", "pool_give", "frame_write", "frame_write8", "cpu_set", "cpu_run", "cpu_idle"];

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

fn or_all(ts: &[String]) -> String {
    match ts.len() {
        0 => "false".into(),
        1 => ts[0].clone(),
        _ => format!("(or {})", ts.join(" ")),
    }
}

fn ite(c: &str, a: &str, b: &str) -> String {
    if a == b { a.to_string() } else { format!("(ite {} {} {})", c, a, b) }
}

pub fn verify_program(p: &Program) -> Result<(usize, Vec<String>), Diag> {
    let solver = Solver::start().map_err(|e| Diag::new("E-SOLVER", Pos::default(), e))?;
    let mut v = Verifier { prog: p, solver, counter: 0, obligations: 0, box_depth: 0, int_ops: HashMap::new() };
    v.smt(&format!("(declare-const {} Int)", POOLSIZE))?;
    v.smt(&format!("(assert (and (<= 0 {p}) (<= {p} {})))", FRAME_LIMIT, p = POOLSIZE))?;
    for f in &p.fns {
        v.verify_fn(f).map_err(|d| crate::check::in_copy(d, &f.name))?;
    }
    let log = std::mem::take(&mut v.solver.log);
    Ok((v.obligations, log))
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
            (_, "used") | (_, "has") => self.fresh_array(&h, "Bool"),
            (Type::Frame, "id") | (Type::PageTable, "id") => {
                let n = self.fresh_int(&h, "0", FRAME_LIMIT)?;
                self.smt(&format!("(assert (< {} {}))", n, POOLSIZE))?;
                Ok(n)
            }
            (Type::PageTable, "level") => self.fresh_int(&h, "1", "5"),
            (Type::Pool, "size") => Ok(POOLSIZE.to_string()),
            (Type::Mmu, "root") => self.fresh_int(&h, "0", KROOT),
            _ => unreachable!("unknown ghost field"),
        }
    }

    fn fresh_of(&mut self, ty: &Type, hint: &str) -> Result<Val, Diag> {
        match ty {
            Type::U64 => self.fresh_u64(hint),
            Type::U8 | Type::U16 | Type::U32 => Ok(Val::U(self.fresh_int(hint, "0", &ty.int_max().to_string())?)),
            Type::Bool => self.fresh_bool(hint),
            Type::Buf => Ok(Val::Buf { len: self.fresh_len(hint)?, data: self.fresh_array(hint, "Int")? }),
            Type::Bytes => {
                // Every byte of a Bytes value is <= 255.
                let data = self.fresh_array(hint, "Int")?;
                self.smt(&format!("(assert (forall ((k Int)) (and (<= 0 (select {d} k)) (<= (select {d} k) 255))))", d = data))?;
                Ok(Val::Buf { len: self.fresh_len(hint)?, data })
            }
            Type::Str => Ok(Val::Str { len: self.fresh_len(hint)?, data: self.fresh_array(hint, "Int")? }),
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
                Ok(Val::Obj(t.clone(), fs))
            }
        }
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
                .note("the solver gave up (timeout or quantifiers); add an `assert` or a stronger invariant as a hint")),
        };
        self.smt("(pop 1)")?;
        res
    }

    fn verify_fn(&mut self, f: &FnDef) -> Result<(), Diag> {
        self.int_ops = f.int_ops.clone();
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
                    let t = self.here(inv, &st)?;
                    self.prove(&st, &t, inv.pos, "E-VERIFY-INVARIANT", format!("the loop invariant `{}` on entry to the loop", render(inv)))?;
                }
                // 2. havoc what the body (or the condition) changes
                let mut changed = HashMap::new();
                changed_vars(body, &mut changed);
                changed_in_expr(cond, &mut changed);
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
                    let t = self.here(inv, &lst)?;
                    lst.pc.push(t);
                }
                // 3. one arbitrary iteration preserves the invariants
                let c = self.eval(cond, &mut lst)?.term().to_string();
                let mut bst = lst.clone();
                bst.pc.push(c.clone());
                if let Some(end) = self.block(body, bst, cx)? {
                    for inv in invariants {
                        let t = self.here(inv, &end)?;
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

    /// A fresh value of the same type as `old`. With `keep`, a Buf keeps its
    /// length and an object the ghost fields builtins never change.
    fn havoc(&mut self, old: &Val, keep: bool, name: &str) -> Result<Val, Diag> {
        Ok(match old {
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
                    nf.push(self.havoc(fv, false, &format!("{}_{}", name, f))?);
                }
                Val::Struct(n.clone(), nf)
            }
            Val::Enum(n, _, ps) => {
                let d = enum_def(n).unwrap();
                let tag = self.fresh_int(&format!("{}_tag", name), "0", &(d.variants.len() - 1).to_string())?;
                let mut np = Vec::new();
                for ((v, _), pv) in d.variants.iter().zip(ps) {
                    np.push(self.havoc(pv, false, &format!("{}_{}", name, v))?);
                }
                Val::Enum(n.clone(), tag, np)
            }
            Val::Vec { elem, len, .. } => {
                let len = if keep { len.clone() } else { self.fresh_len(name)? };
                Val::Vec { elem: elem.clone(), len, elems: Box::new(self.fresh_lifted(elem, name)?) }
            }
            Val::Box(BoxVal::Known(i)) => Val::Box(BoxVal::Known(Box::new(self.havoc(i, false, name)?))),
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
                        return Ok(Val::U(format!("(mod {} {})", x, t.int_max() + 1)));
                    }
                    let max = t.int_max().to_string();
                    self.prove(st, &format!("(<= {} {})", x, max), e.pos, "E-VERIFY-CAST", format!("that `{}` fits in {} (`<= {}`)", render(&args[0]), t.name(), max))?;
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
                Ok(Val::B(format!("(not {})", v.term())))
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
                    BinOp::And | BinOp::Or | BinOp::Implies => unreachable!(),
                };
                Ok(v)
            }
            ExprKind::Call(name, args) => self.call(name, args, e.pos, st),
        }
    }

    fn call(&mut self, name: &str, args: &[Expr], pos: Pos, st: &mut State) -> Result<Val, Diag> {
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
        for (p, a) in f.params.iter().zip(args) {
            if let (Type::MutRef(t), ExprKind::MutBorrow(var)) = (&p.ty, &a.kind) {
                let nv = self.fresh_of(t, &var.replace('.', "_"))?;
                post.insert(p.name.clone(), nv.clone());
                set_path(&mut st.env, var, nv);
            }
        }
        for en in &f.ensures {
            let t = self.spec(en, Scope { env: &post, old: Some(&pre), result: Some(&res) }, &mut Vec::new())?;
            st.pc.push(t);
        }
        Ok(res)
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
                Ok(Val::U(format!("(mod ({} {} {}) {})", op, vals[0].term(), vals[1].term(), ity.int_max() + 1)))
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
            "serial_write" => {
                let b = vals[1].term();
                self.need(st, format!("(<= {} 255)", b), pos, "E-VERIFY-BYTE", format!("that `{}` fits in a byte (`<= 255`)", ra(1)))?;
                Ok(Val::Unit)
            }
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
                let elems = vec_write(&elems, &len, &vals[1]);
                self.put_back(&args[0], Val::Vec { elem, len: format!("(+ {} 1)", len), elems: Box::new(elems) }, st);
                Ok(Val::Unit)
            }
            "vec_pop" => {
                let Val::Vec { elem, len, elems } = vals[0].clone() else { unreachable!() };
                self.need(st, format!("(> {} 0)", len), pos, "E-VERIFY-BOUNDS", format!("that `{}` is not empty (`vlen({}) > 0`)", ra(0), ra(0)))?;
                let last = format!("(- {} 1)", len);
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
                    let elems = vec_write(&elems, &i, &vals[2]);
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
                    self.need(st, format!("(<= {} 255)", v), pos, "E-VERIFY-BYTE", format!("that `{}` fits in a byte (`<= 255`)", ra(2)))?;
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
                    let b = vals[2].term();
                    self.need(st, format!("(<= {} 255)", b), pos, "E-VERIFY-BYTE", format!("that `{}` fits in a byte (`<= 255`)", ra(2)))?;
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
                Ok(Val::Obj(Type::PageTable, vec![f.field("id"), lvl.to_string(), NO_BOOL.into()]))
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
                }
                Ok(Val::Obj(Type::Frame, vec![t.field("id")]))
            }
            "pt_has" => {
                let i = vals[1].term().to_string();
                index_ok(self, st, &i, "512", 1, "a page table (512 slots)")?;
                Ok(Val::B(format!("(select {} {})", vals[0].field("used"), i)))
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
                }
                let nt = Val::Obj(Type::PageTable, vec![t.field("id"), lvl.clone(), format!("(store {} {} {})", used, i, putting)]);
                self.put_back(&args[0], nt, st);
                match name {
                    "pt_take" => {
                        let mut c = self.fresh_of(&Type::PageTable, "pt_take")?;
                        if let Val::Obj(_, fs) = &mut c {
                            fs[1] = format!("(- {} 1)", lvl);
                        }
                        Ok(c)
                    }
                    "pt_take_frame" => self.fresh_of(&Type::Frame, "pt_take_frame"),
                    _ => Ok(Val::Unit),
                }
            }

            // ---- MMU and CPU ----
            "mmu_switch" => {
                let t = &vals[1];
                self.need(st, format!("(= {} 4)", t.field("level")), pos, "E-VERIFY-PT", format!("that `{}` is a top-level table (level 4)", ra(1)))?;
                self.put_back(&args[0], Val::Obj(Type::Mmu, vec![t.field("id")]), st);
                Ok(Val::Unit)
            }
            "mmu_kernel" => {
                self.put_back(&args[0], Val::Obj(Type::Mmu, vec![KROOT.into()]), st);
                Ok(Val::Unit)
            }
            "cpu_run" | "cpu_idle" => {
                let v = self.fresh_u64("vector")?;
                st.pc.push(format!("(<= {} 255)", v.term()));
                Ok(v)
            }
            "cpu_get" | "cpu_set" => {
                let r = vals[1].term().to_string();
                let lim = if name == "cpu_get" { "20" } else { "18" };
                index_ok(self, st, &r, lim, 1, "the saved registers")?;
                if name == "cpu_set" {
                    return Ok(Val::Unit);
                }
                self.fresh_u64("reg")
            }

            // ---- user programs ----
            "sys_spawn" => {
                let (from, to) = (vals[1].term(), vals[2].term());
                let Val::Buf { len, .. } = &vals[0] else { unreachable!() };
                self.need(st, format!("(and (<= {} {}) (<= {} {}))", from, to, to, len), pos, "E-VERIFY-BOUNDS",
                    format!("that `{}..{}` is a range within `{}`", ra(1), ra(2), ra(0)))?;
                self.fresh_u64("spawn")
            }
            "sys_wait" | "sys_kill" | "sys_getpid" | "sys_info" => self.fresh_u64(name),
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
                let old = sc.old.expect("checked: old() only in ensures");
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
            ExprKind::Bin(op, a, b) => {
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
                    let old = sc.old.expect("checked: old() only in ensures");
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
                    "frame_id" | "pt_id" => v.field("id"),
                    "pt_level" => v.field("level"),
                    "pool_size" => v.field("size"),
                    "mmu_root" => v.field("root"),
                    "pt_used" => format!("(select {} {})", v.field("used"), self.spec(&args[1], sc, bound)?),
                    "pool_has" => format!("(select {} {})", v.field("has"), self.spec(&args[1], sc, bound)?),
                    _ => unreachable!("checked: {}", name),
                }
            }
        })
    }
}

/// Variables a loop body may change, with a flag that is true when every
/// change keeps the value's shape: `x = set(x, ..)` keeps a Buf's length,
/// and the builtins in SHAPE_KEEPING keep an object's id, level and size.
fn changed_vars(stmts: &[Stmt], out: &mut HashMap<String, bool>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Assign { name, value } => {
                let only_set = matches!(&value.kind, ExprKind::Call(f, args) if f == "set" && place(&args[0]).as_deref() == Some(name.as_str()));
                let e = out.entry(name.clone()).or_insert(true);
                *e = *e && only_set;
                changed_in_expr(value, out);
            }
            StmtKind::Let { init, .. } => changed_in_expr(init, out),
            StmtKind::Expr(e) | StmtKind::Return(Some(e)) => changed_in_expr(e, out),
            StmtKind::If { cond, then_b, else_b } => {
                changed_in_expr(cond, out);
                changed_vars(then_b, out);
                changed_vars(else_b, out);
            }
            StmtKind::While { cond, body, .. } => {
                changed_in_expr(cond, out);
                changed_vars(body, out);
            }
            StmtKind::Match { scrut, arms } => {
                changed_in_expr(scrut, out);
                for a in arms {
                    changed_vars(&a.body, out);
                }
            }
            StmtKind::Return(None) | StmtKind::Assert(_) | StmtKind::Unreachable => {}
        }
    }
}

/// Variables lent as `&mut` anywhere in `e`.
fn changed_in_expr(e: &Expr, out: &mut HashMap<String, bool>) {
    match &e.kind {
        ExprKind::Call(f, args) => {
            for a in args {
                if let ExprKind::MutBorrow(n) = &a.kind {
                    let keep = SHAPE_KEEPING.contains(&f.as_str());
                    let x = out.entry(n.clone()).or_insert(true);
                    *x = *x && keep;
                } else {
                    changed_in_expr(a, out);
                }
            }
        }
        ExprKind::Not(x) => changed_in_expr(x, out),
        ExprKind::Bin(_, a, b) => {
            changed_in_expr(a, out);
            changed_in_expr(b, out);
        }
        ExprKind::StructLit(_, fs) => {
            for (_, x) in fs {
                changed_in_expr(x, out);
            }
        }
        ExprKind::Variant(_, _, Some(x)) | ExprKind::Is(x, _, _) => changed_in_expr(x, out),
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
