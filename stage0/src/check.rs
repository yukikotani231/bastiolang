// Type and linearity checking.
//
// Linearity rules for `Buf` (owned buffer):
//   * every Buf value is consumed exactly once: moved into a `Buf` parameter,
//     returned, or passed to `free`;
//   * a moved variable cannot be used again (E-LINEAR-USE);
//   * a Buf still live at the end of its scope or at a `return` is a leak
//     (E-LINEAR-LEAK) -- leaks are compile errors, unlike Rust's `mem::forget`;
//   * both arms of an `if` must consume the same Buf variables, and a loop
//     body must leave the outer Buf variables as it found them
//     (E-LINEAR-BRANCH);
//   * `&Buf` is a read-only borrow; it is only a parameter type, so a borrow
//     can never outlive the call that created it. A single call may not both
//     borrow and move the same variable (no aliasing between `&Buf` and `Buf`).
//
// Specification expressions (requires / ensures / invariant / assert) are
// ghost: they may mention any variable, moved or not, and never consume.

use crate::ast::*;
use crate::codegen::Target;
use crate::diag::{Diag, Pos};
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub struct Sig {
    pub params: Vec<Type>,
    pub ret: Type,
}

/// Which builtins exist depends on what the program runs on.
#[derive(Clone, Copy, PartialEq)]
enum Avail {
    All,
    /// Hosted and kernel: physical pages and page tables (hosted simulates them).
    Mem,
    Kernel,
    User,
}

fn r(t: Type) -> Type {
    Type::Ref(Box::new(t))
}
fn m(t: Type) -> Type {
    Type::MutRef(Box::new(t))
}

fn builtin_table() -> Vec<(&'static str, Avail, Vec<Type>, Type)> {
    use Type::*;
    vec![
        ("alloc", Avail::All, vec![U64], Buf),
        ("len", Avail::All, vec![r(Buf)], U64),
        ("get", Avail::All, vec![r(Buf), U64], U64),
        ("set", Avail::All, vec![Buf, U64, U64], Buf),
        ("free", Avail::All, vec![Buf], Unit),
        ("print", Avail::All, vec![U64], Unit),
        ("slen", Avail::All, vec![Str], U64),
        ("sget", Avail::All, vec![Str, U64], U64),
        ("serial_write", Avail::All, vec![r(Serial), U64], Unit),
        ("serial_read", Avail::All, vec![r(Serial)], U64),
        ("serial_poll", Avail::Kernel, vec![r(Serial)], U64),
        ("pool_size", Avail::Mem, vec![r(Pool)], U64),
        ("pool_take", Avail::Mem, vec![m(Pool), U64], Frame),
        ("pool_give", Avail::Mem, vec![m(Pool), Frame], Unit),
        ("frame_id", Avail::Mem, vec![r(Frame)], U64),
        ("frame_read", Avail::Mem, vec![r(Frame), U64], U64),
        ("frame_write", Avail::Mem, vec![m(Frame), U64, U64], Unit),
        ("frame_read8", Avail::Mem, vec![r(Frame), U64], U64),
        ("frame_write8", Avail::Mem, vec![m(Frame), U64, U64], Unit),
        ("pt_new", Avail::Mem, vec![Frame, U64], PageTable),
        ("pt_free", Avail::Mem, vec![PageTable], Frame),
        ("pt_free_root", Avail::Kernel, vec![r(Mmu), PageTable], Frame),
        ("pt_has", Avail::Mem, vec![r(PageTable), U64], Bool),
        ("pt_put", Avail::Mem, vec![m(PageTable), U64, PageTable], Unit),
        ("pt_take", Avail::Mem, vec![m(PageTable), U64], PageTable),
        ("pt_put_frame", Avail::Mem, vec![m(PageTable), U64, Frame, Bool], Unit),
        ("pt_take_frame", Avail::Mem, vec![m(PageTable), U64], Frame),
        ("mmu_switch", Avail::Kernel, vec![m(Mmu), r(PageTable)], Unit),
        ("mmu_kernel", Avail::Kernel, vec![m(Mmu)], Unit),
        ("cpu_run", Avail::Kernel, vec![m(Cpu), r(Mmu)], U64),
        ("cpu_idle", Avail::Kernel, vec![m(Cpu)], U64),
        ("cpu_get", Avail::Kernel, vec![r(Cpu), U64], U64),
        ("cpu_set", Avail::Kernel, vec![m(Cpu), U64, U64], Unit),
        ("sys_spawn", Avail::User, vec![r(Buf), U64, U64], U64),
        ("sys_wait", Avail::User, vec![U64], U64),
        ("sys_kill", Avail::User, vec![U64], U64),
        ("sys_yield", Avail::User, vec![], Unit),
        ("sys_getpid", Avail::User, vec![], U64),
        ("sys_info", Avail::User, vec![U64], U64),
        ("sys_ps", Avail::User, vec![], Unit),
        ("sys_poweroff", Avail::User, vec![], Unit),
    ]
}

/// Ghost accessors, usable only in specifications: (name, argument type, has an index, result).
pub const GHOSTS: &[(&str, Type, bool, Type)] = &[
    ("frame_id", Type::Frame, false, Type::U64),
    ("pt_id", Type::PageTable, false, Type::U64),
    ("pt_level", Type::PageTable, false, Type::U64),
    ("pt_used", Type::PageTable, true, Type::Bool),
    ("pool_has", Type::Pool, true, Type::Bool),
    ("pool_size", Type::Pool, false, Type::U64),
    ("mmu_root", Type::Mmu, false, Type::U64),
];

pub fn is_ghost(name: &str) -> bool {
    GHOSTS.iter().any(|g| g.0 == name)
}

pub fn builtin_sigs() -> HashMap<String, Sig> {
    builtin_table().into_iter().map(|(n, _, params, ret)| (n.to_string(), Sig { params, ret })).collect()
}

pub fn is_builtin(name: &str) -> bool {
    builtin_table().iter().any(|b| b.0 == name)
}

fn available(name: &str, target: Target) -> bool {
    let Some(b) = builtin_table().into_iter().find(|b| b.0 == name) else { return true };
    match (b.1, target) {
        (Avail::All, _) => true,
        (Avail::Mem, Target::Hosted | Target::Kernel) => true,
        (Avail::Kernel, Target::Kernel) => true,
        (Avail::User, Target::User) => true,
        _ => false,
    }
}

/// The capabilities `main` may ask for on each target.
fn main_caps(target: Target) -> &'static [Type] {
    match target {
        Target::Hosted => &[Type::Serial, Type::Pool],
        Target::Kernel => &[Type::Serial, Type::Pool, Type::Mmu, Type::Cpu],
        Target::User => &[Type::Serial],
    }
}

pub fn signatures(p: &Program, target: Target) -> Result<HashMap<String, Sig>, Diag> {
    let mut sigs = builtin_sigs();
    for f in &p.fns {
        if is_builtin(&f.name) || is_ghost(&f.name) || f.name == "old" {
            return Err(Diag::new("E-TYPE", f.pos, format!("`{}` is a builtin and cannot be redefined", f.name)));
        }
        if sigs.contains_key(&f.name) || p.specs.iter().any(|s| s.name == f.name) {
            return Err(Diag::new("E-TYPE", f.pos, format!("function `{}` is defined twice", f.name)));
        }
        if f.ret.is_ref() {
            return Err(Diag::new("E-TYPE", f.pos, "a function cannot return a borrow"));
        }
        sigs.insert(f.name.clone(), Sig { params: f.params.iter().map(|p| p.ty.clone()).collect(), ret: f.ret.clone() });
    }
    let main = p.fns.iter().find(|f| f.name == "main");
    let Some(main) = main else { return Err(Diag::new("E-TYPE", Pos::default(), "no `main` function")) };
    let legacy = main.params.len() == 1 && main.params[0].ty == Type::Serial && main.ret == Type::Serial;
    let caps = main_caps(target);
    let mut seen = Vec::new();
    let caps_ok = (main.ret == Type::U64 || main.ret == Type::Unit)
        && main.params.iter().all(|p| match &p.ty {
            Type::MutRef(t) if caps.contains(t) && !seen.contains(&**t) => {
                seen.push((**t).clone());
                true
            }
            _ => false,
        });
    if !(legacy && target != Target::User) && !caps_ok {
        let names: Vec<String> = caps.iter().map(|c| format!("&mut {}", c.name())).collect();
        return Err(Diag::new("E-TYPE", main.pos, format!("on this target `main` must take some of {} (each at most once) and return u64 or nothing", names.join(", ")))
            .note("`fn main() -> u64` and, outside user programs, `fn main(con: Serial) -> Serial` are also accepted"));
    }
    if target == Target::User && main.ret != Type::U64 {
        return Err(Diag::new("E-TYPE", main.pos, "a user program's `main` returns its exit code (u64)"));
    }
    Ok(sigs)
}

#[derive(Clone)]
struct VarInfo {
    ty: Type,
    mutable: bool,
}

struct Ctx<'a> {
    sigs: &'a HashMap<String, Sig>,
    specs: &'a [SpecFn],
    target: Target,
    ret: Type,
    scopes: Vec<Vec<String>>,
    vars: HashMap<String, VarInfo>,
    /// Linear variables that have been consumed (or, for `&mut` parameters,
    /// moved out of and not yet put back).
    moved: HashSet<String>,
    all_names: HashSet<String>,
}

pub fn check_program(p: &mut Program, target: Target) -> Result<HashMap<String, Sig>, Diag> {
    let sigs = signatures(p, target)?;
    let specs = p.specs.clone();
    for (i, sf) in specs.iter().enumerate() {
        check_spec_fn(sf, &specs[..i], &sigs, target)?;
    }
    for f in &mut p.fns {
        check_fn(f, &sigs, &specs, target)?;
    }
    Ok(sigs)
}

fn new_ctx<'a>(sigs: &'a HashMap<String, Sig>, specs: &'a [SpecFn], target: Target, ret: Type) -> Ctx<'a> {
    Ctx { sigs, specs, target, ret, scopes: vec![Vec::new()], vars: HashMap::new(), moved: HashSet::new(), all_names: HashSet::new() }
}

/// A spec fn may only call spec fns defined before it, so expansion terminates.
fn check_spec_fn(sf: &SpecFn, earlier: &[SpecFn], sigs: &HashMap<String, Sig>, target: Target) -> Result<(), Diag> {
    if sigs.contains_key(&sf.name) || is_ghost(&sf.name) || earlier.iter().any(|e| e.name == sf.name) {
        return Err(Diag::new("E-TYPE", sf.pos, format!("`{}` is already defined", sf.name)));
    }
    if !matches!(sf.ret, Type::Bool | Type::U64) {
        return Err(Diag::new("E-TYPE", sf.pos, "a spec fn returns bool or u64"));
    }
    let mut cx = new_ctx(sigs, earlier, target, Type::Unit);
    for p in &sf.params {
        if p.ty.is_linear() || p.ty.is_mut_ref() {
            return Err(Diag::new("E-TYPE", p.pos, "spec fn parameters are values (`u64`, `bool`) or shared borrows (`&T`)"));
        }
        cx.declare(&p.name, p.ty.clone(), false, p.pos)?;
    }
    let t = cx.spec_expr(&sf.body, false, &mut Vec::new())?;
    expect_ty(&t, &sf.ret, sf.body.pos, &format!("the body of spec fn `{}`", sf.name))
}

fn check_fn(f: &mut FnDef, sigs: &HashMap<String, Sig>, specs: &[SpecFn], target: Target) -> Result<(), Diag> {
    let mut cx = new_ctx(sigs, specs, target, f.ret.clone());
    for p in &f.params {
        if p.ty.is_ref() && p.mutable {
            return Err(Diag::new("E-TYPE", p.pos, "a borrow cannot be `mut` (a `&mut` parameter is already assignable)"));
        }
        let mutable = p.mutable || p.ty.is_mut_ref();
        cx.declare(&p.name, p.ty.clone(), mutable, p.pos)?;
    }
    for r in &f.requires {
        let t = cx.spec_expr(r, false, &mut Vec::new())?;
        expect_ty(&t, &Type::Bool, r.pos, "a `requires` clause")?;
    }
    for e in &f.ensures {
        let t = cx.spec_expr(e, true, &mut Vec::new())?;
        expect_ty(&t, &Type::Bool, e.pos, "an `ensures` clause")?;
    }
    let diverges = cx.block(&mut f.body)?;
    if !diverges {
        if f.ret != Type::Unit {
            return Err(Diag::new("E-TYPE", f.pos, format!("function `{}` can reach its end without returning a {}", f.name, f.ret.name())));
        }
        cx.leak_check_all(f.pos, "at the end of the function")?;
    }
    Ok(())
}

fn expect_ty(got: &Type, want: &Type, pos: Pos, what: &str) -> Result<(), Diag> {
    if got != want {
        return Err(Diag::new("E-TYPE", pos, format!("{} must be {}, found {}", what, want.name(), got.name())));
    }
    Ok(())
}

impl<'a> Ctx<'a> {
    fn declare(&mut self, name: &str, ty: Type, mutable: bool, pos: Pos) -> Result<(), Diag> {
        if name == "result" || name == "old" {
            return Err(Diag::new("E-TYPE", pos, format!("`{}` is reserved for specifications", name)));
        }
        if self.all_names.contains(name) || self.sigs.contains_key(name) || is_ghost(name) || self.specs.iter().any(|s| s.name == name) {
            return Err(Diag::new("E-TYPE", pos, format!("`{}` is already defined (shadowing is not allowed)", name)));
        }
        self.all_names.insert(name.to_string());
        self.vars.insert(name.to_string(), VarInfo { ty, mutable });
        self.scopes.last_mut().unwrap().push(name.to_string());
        Ok(())
    }

    /// Owned linear variables still holding a value (`&mut` parameters excluded).
    fn live_owned(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .scopes
            .iter()
            .flatten()
            .filter(|n| self.vars[*n].ty.is_linear() && !self.moved.contains(*n))
            .cloned()
            .collect();
        v.sort();
        v
    }

    /// `&mut` parameters whose value has been moved out and not put back.
    fn missing_mut_refs(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .scopes
            .iter()
            .flatten()
            .filter(|n| self.vars[*n].ty.is_mut_ref() && self.moved.contains(*n))
            .cloned()
            .collect();
        v.sort();
        v
    }

    fn leak_check_all(&self, pos: Pos, when: &str) -> Result<(), Diag> {
        let live = self.live_owned();
        if !live.is_empty() {
            return Err(Diag::new("E-LINEAR-LEAK", pos, format!("`{}` is still owned {}", live.join("`, `"), when))
                .note("every linear value must be consumed exactly once: return it, pass it to a function that takes it by value, or `free` it"));
        }
        let missing = self.missing_mut_refs();
        if !missing.is_empty() {
            return Err(Diag::new("E-LINEAR-USE", pos, format!("the borrowed `{}` must hold a value again {}", missing.join("`, `"), when))
                .note("a `&mut` parameter can be moved out of, but the caller gets it back, so assign it a new value before returning"));
        }
        Ok(())
    }

    /// Returns true when the block always diverges (returns).
    fn block(&mut self, stmts: &mut [Stmt]) -> Result<bool, Diag> {
        self.scopes.push(Vec::new());
        let mut diverges = false;
        for s in stmts.iter_mut() {
            if diverges {
                return Err(Diag::new("E-TYPE", s.pos, "unreachable statement after `return`"));
            }
            diverges = self.stmt(s)?;
        }
        let scope = self.scopes.pop().unwrap();
        if !diverges {
            for n in &scope {
                if self.vars[n].ty.is_linear() && !self.moved.contains(n) {
                    return Err(Diag::new("E-LINEAR-LEAK", stmts.last().map(|s| s.pos).unwrap_or_default(), format!("`{}` goes out of scope while still owned", n))
                        .note("every linear value must be consumed exactly once: return it, pass it to a function that takes it by value, or `free` it"));
                }
            }
        }
        for n in &scope {
            self.vars.remove(n);
            self.moved.remove(n);
        }
        Ok(diverges)
    }

    fn stmt(&mut self, s: &mut Stmt) -> Result<bool, Diag> {
        let pos = s.pos;
        match &mut s.kind {
            StmtKind::Let { name, mutable, ty, init } => {
                let t = self.expr(init)?;
                if let Some(want) = ty {
                    if want.is_ref() {
                        return Err(Diag::new("E-TYPE", pos, "a local cannot hold a borrow"));
                    }
                    expect_ty(&t, want, init.pos, &format!("the initializer of `{}`", name))?;
                }
                if t == Type::Unit {
                    return Err(Diag::new("E-TYPE", init.pos, "cannot bind a value of type ()"));
                }
                *ty = Some(t.clone());
                let (n, m) = (name.clone(), *mutable);
                self.declare(&n, t, m, pos)?;
                Ok(false)
            }
            StmtKind::Assign { name, value } => {
                let info = self.vars.get(name.as_str()).cloned().ok_or_else(|| Diag::new("E-TYPE", pos, format!("unknown variable `{}`", name)))?;
                if !info.mutable {
                    return Err(Diag::new("E-TYPE", pos, format!("`{}` is not mutable (declare it with `mut`)", name)));
                }
                let t = self.expr(value)?;
                expect_ty(&t, info.ty.base(), value.pos, &format!("the value assigned to `{}`", name))?;
                if info.ty.base().is_linear() {
                    if !self.moved.contains(name.as_str()) {
                        return Err(Diag::new("E-LINEAR-LEAK", pos, format!("assigning to `{}` would drop the value it still owns", name))
                            .note("consume the old value first, e.g. `b = set(b, i, v);`"));
                    }
                    self.moved.remove(name.as_str());
                }
                Ok(false)
            }
            StmtKind::If { cond, then_b, else_b } => {
                let t = self.expr(cond)?;
                expect_ty(&t, &Type::Bool, cond.pos, "an `if` condition")?;
                let before = self.moved.clone();
                let d1 = self.block(then_b)?;
                let after_then = std::mem::replace(&mut self.moved, before);
                let d2 = self.block(else_b)?;
                let after_else = self.moved.clone();
                match (d1, d2) {
                    (true, true) => Ok(true),
                    (true, false) => Ok(false),
                    (false, true) => {
                        self.moved = after_then;
                        Ok(false)
                    }
                    (false, false) => {
                        if after_then != after_else {
                            let mut diff: Vec<_> = after_then.symmetric_difference(&after_else).cloned().collect();
                            diff.sort();
                            return Err(Diag::new("E-LINEAR-BRANCH", pos, format!("`{}` is consumed in only one branch of this `if`", diff.join("`, `"))));
                        }
                        Ok(false)
                    }
                }
            }
            StmtKind::While { cond, invariants, body } => {
                let before_cond = self.moved.clone();
                let t = self.expr(cond)?;
                if self.moved != before_cond {
                    return Err(Diag::new("E-LINEAR-USE", cond.pos, "a `while` condition runs many times, so it cannot consume a linear value"));
                }
                expect_ty(&t, &Type::Bool, cond.pos, "a `while` condition")?;
                for inv in invariants.iter() {
                    let t = self.spec_expr(inv, false, &mut Vec::new())?;
                    expect_ty(&t, &Type::Bool, inv.pos, "a loop invariant")?;
                }
                let before = self.moved.clone();
                let d = self.block(body)?;
                if !d && self.moved != before {
                    let mut diff: Vec<_> = self.moved.symmetric_difference(&before).cloned().collect();
                    diff.sort();
                    return Err(Diag::new("E-LINEAR-BRANCH", pos, format!("the loop body changes the ownership of `{}`", diff.join("`, `")))
                        .note("a loop body must leave every outer linear value owned exactly as it found it"));
                }
                self.moved = before;
                Ok(false)
            }
            StmtKind::Return(e) => {
                let t = match e {
                    Some(e) => self.expr(e)?,
                    None => Type::Unit,
                };
                let ret = self.ret.clone();
                expect_ty(&t, &ret, pos, "the returned value")?;
                self.leak_check_all(pos, "at this `return`")?;
                Ok(true)
            }
            StmtKind::Assert(e) => {
                let t = self.spec_expr(e, false, &mut Vec::new())?;
                expect_ty(&t, &Type::Bool, e.pos, "an `assert`")?;
                Ok(false)
            }
            StmtKind::Expr(e) => {
                let t = self.expr(e)?;
                if t.is_linear() {
                    return Err(Diag::new("E-LINEAR-LEAK", pos, "this linear value is created and immediately dropped"));
                }
                Ok(false)
            }
        }
    }

    fn var_info(&self, name: &str, pos: Pos) -> Result<VarInfo, Diag> {
        self.vars.get(name).cloned().ok_or_else(|| Diag::new("E-TYPE", pos, format!("unknown variable `{}`", name)))
    }

    /// Code expression: linear variables in value position are moved.
    fn expr(&mut self, e: &Expr) -> Result<Type, Diag> {
        match &e.kind {
            ExprKind::Int(_) => Ok(Type::U64),
            ExprKind::Bool(_) => Ok(Type::Bool),
            ExprKind::Str(_) | ExprKind::Blob(_) => Ok(Type::Str),
            ExprKind::MutBorrow(_) => Err(Diag::new("E-TYPE", e.pos, "`&mut x` is only allowed as a call argument")),
            ExprKind::Var(n) => {
                let info = self.var_info(n, e.pos)?;
                match info.ty {
                    Type::Ref(_) => Err(Diag::new("E-LINEAR-USE", e.pos, format!("cannot move out of the shared borrow `{}`", n))),
                    t if t.base().is_linear() => {
                        if self.moved.contains(n.as_str()) {
                            return Err(Diag::new("E-LINEAR-USE", e.pos, format!("`{}` is used after it was consumed", n)));
                        }
                        self.moved.insert(n.clone());
                        Ok(t.base().clone())
                    }
                    t => Ok(t),
                }
            }
            ExprKind::Not(x) => {
                let t = self.expr(x)?;
                expect_ty(&t, &Type::Bool, x.pos, "the operand of `!`")?;
                Ok(Type::Bool)
            }
            ExprKind::Bin(op, a, b) => {
                if *op == BinOp::Implies {
                    return Err(Diag::new("E-TYPE", e.pos, "`==>` is only allowed in specifications"));
                }
                let ta = self.expr(a)?;
                let before_rhs = self.moved.clone();
                let tb = self.expr(b)?;
                if matches!(op, BinOp::And | BinOp::Or) && self.moved != before_rhs {
                    return Err(Diag::new("E-LINEAR-USE", b.pos, format!("the right side of `{}` may not run, so it cannot consume a linear value", op.text())));
                }
                bin_type(*op, &ta, &tb, e.pos)
            }
            ExprKind::Forall(..) => Err(Diag::new("E-TYPE", e.pos, "`forall` is only allowed in specifications")),
            ExprKind::Call(name, args) => {
                if name == "result" || name == "old" || (is_ghost(name) && !self.sigs.contains_key(name)) || self.specs.iter().any(|s| &s.name == name) {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`{}` is only allowed in specifications", name)));
                }
                let sig = self.sigs.get(name).cloned().ok_or_else(|| Diag::new("E-TYPE", e.pos, format!("unknown function `{}`", name)))?;
                if !available(name, self.target) {
                    return Err(Diag::new("E-TARGET", e.pos, format!("`{}` is not available on this target", name)));
                }
                if sig.params.len() != args.len() {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`{}` takes {} argument(s), {} given", name, sig.params.len(), args.len())));
                }
                // Scalars first, then borrows, then moves: a scalar argument may
                // read a value that a later argument consumes (`set(b, i, get(b, j))`).
                for (pt, a) in sig.params.iter().zip(args) {
                    if !pt.is_linear() && !pt.is_ref() {
                        let t = self.expr(a)?;
                        expect_ty(&t, pt, a.pos, &format!("argument of `{}`", name))?;
                    }
                }
                let mut borrowed: HashSet<String> = HashSet::new();
                for (pt, a) in sig.params.iter().zip(args) {
                    if !pt.is_ref() {
                        continue;
                    }
                    let n = match (&a.kind, pt) {
                        (ExprKind::MutBorrow(n), Type::MutRef(_)) => n,
                        (ExprKind::Var(n), Type::Ref(_)) => n,
                        (_, Type::MutRef(_)) => return Err(Diag::new("E-TYPE", a.pos, format!("argument of `{}` must be `&mut x` for a variable `x`", name))),
                        _ => return Err(Diag::new("E-TYPE", a.pos, format!("argument of `{}` must name a value to borrow", name))),
                    };
                    let info = self.var_info(n, a.pos)?;
                    if info.ty.base() != pt.base() {
                        return Err(Diag::new("E-TYPE", a.pos, format!("argument of `{}` must be {}, found {}", name, pt.base().name(), info.ty.name())));
                    }
                    if pt.is_mut_ref() {
                        if matches!(info.ty, Type::Ref(_)) {
                            return Err(Diag::new("E-LINEAR-USE", a.pos, format!("`{}` is a shared borrow and cannot be lent as `&mut`", n)));
                        }
                        if !info.mutable {
                            return Err(Diag::new("E-TYPE", a.pos, format!("`{}` is not mutable (declare it with `mut`)", n)));
                        }
                    }
                    if self.moved.contains(n.as_str()) {
                        return Err(Diag::new("E-LINEAR-USE", a.pos, format!("`{}` is borrowed after it was consumed", n)));
                    }
                    if !borrowed.insert(n.clone()) && pt.is_mut_ref() {
                        return Err(Diag::new("E-LINEAR-USE", a.pos, format!("`{}` is lent as `&mut` while also borrowed in this call", n)));
                    }
                    if pt.is_mut_ref() {
                        borrowed.insert(format!("{}#mut", n));
                    }
                }
                // A shared borrow next to a `&mut` of the same variable is aliasing too.
                for (pt, a) in sig.params.iter().zip(args) {
                    if let (Type::Ref(_), ExprKind::Var(n)) = (pt, &a.kind) {
                        if borrowed.contains(&format!("{}#mut", n)) {
                            return Err(Diag::new("E-LINEAR-USE", a.pos, format!("`{}` is lent as `&mut` while also borrowed in this call", n)));
                        }
                    }
                }
                for (pt, a) in sig.params.iter().zip(args) {
                    if pt.is_linear() {
                        if let ExprKind::Var(n) = &a.kind {
                            if borrowed.contains(n) {
                                return Err(Diag::new("E-LINEAR-USE", a.pos, format!("`{}` is both borrowed and moved in this call", n)));
                            }
                        }
                        let t = self.expr(a)?;
                        expect_ty(&t, pt, a.pos, &format!("argument of `{}`", name))?;
                    }
                }
                Ok(sig.ret)
            }
        }
    }

    /// Specification expression: ghost, never consumes.
    fn spec_expr(&self, e: &Expr, in_ensures: bool, bound: &mut Vec<String>) -> Result<Type, Diag> {
        match &e.kind {
            ExprKind::Int(_) => Ok(Type::U64),
            ExprKind::Bool(_) => Ok(Type::Bool),
            ExprKind::Str(_) | ExprKind::Blob(_) => Ok(Type::Str),
            ExprKind::MutBorrow(_) => Err(Diag::new("E-TYPE", e.pos, "`&mut` has no meaning in a specification")),
            ExprKind::Var(n) => {
                if bound.contains(n) {
                    return Ok(Type::U64);
                }
                if n == "result" {
                    if !in_ensures {
                        return Err(Diag::new("E-TYPE", e.pos, "`result` is only allowed in `ensures`"));
                    }
                    if self.ret == Type::Unit {
                        return Err(Diag::new("E-TYPE", e.pos, "this function returns nothing, so `result` does not exist"));
                    }
                    return Ok(self.ret.clone());
                }
                Ok(self.var_info(n, e.pos)?.ty.base().clone())
            }
            ExprKind::Not(x) => {
                let t = self.spec_expr(x, in_ensures, bound)?;
                expect_ty(&t, &Type::Bool, x.pos, "the operand of `!`")?;
                Ok(Type::Bool)
            }
            ExprKind::Bin(op, a, b) => {
                let ta = self.spec_expr(a, in_ensures, bound)?;
                let tb = self.spec_expr(b, in_ensures, bound)?;
                bin_type(*op, &ta, &tb, e.pos)
            }
            ExprKind::Forall(v, body) => {
                if self.vars.contains_key(v) || bound.contains(v) {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`{}` is already defined (shadowing is not allowed)", v)));
                }
                bound.push(v.clone());
                let t = self.spec_expr(body, in_ensures, bound)?;
                bound.pop();
                expect_ty(&t, &Type::Bool, body.pos, "the body of `forall`")?;
                Ok(Type::Bool)
            }
            ExprKind::Call(name, args) => {
                let arity = |n: usize| -> Result<(), Diag> {
                    if args.len() != n {
                        return Err(Diag::new("E-TYPE", e.pos, format!("`{}` takes {} argument(s), {} given", name, n, args.len())));
                    }
                    Ok(())
                };
                if name == "old" {
                    arity(1)?;
                    if !in_ensures {
                        return Err(Diag::new("E-TYPE", e.pos, "`old(..)` is only allowed in `ensures`"));
                    }
                    return self.spec_expr(&args[0], false, bound);
                }
                // Arguments that stand for a linear value must name one.
                let check_lin = |a: &Expr| -> Result<(), Diag> {
                    let ok = match &a.kind {
                        ExprKind::Var(_) | ExprKind::Str(_) | ExprKind::Blob(_) => true,
                        ExprKind::Call(n, xs) => n == "old" && xs.len() == 1 && matches!(xs[0].kind, ExprKind::Var(_)),
                        _ => false,
                    };
                    if ok { Ok(()) } else { Err(Diag::new("E-TYPE", a.pos, "a value in this position must be a variable, `result` or `old(x)`")) }
                };
                if let Some(sf) = self.specs.iter().find(|s| &s.name == name) {
                    arity(sf.params.len())?;
                    for (p, a) in sf.params.iter().zip(args) {
                        let t = self.spec_expr(a, in_ensures, bound)?;
                        if t != *p.ty.base() {
                            return Err(Diag::new("E-TYPE", a.pos, format!("argument of `{}` must be {}, found {}", name, p.ty.base().name(), t.name())));
                        }
                        if p.ty.base().is_linear() {
                            check_lin(a)?;
                        }
                    }
                    return Ok(sf.ret.clone());
                }
                if let Some(g) = GHOSTS.iter().find(|g| g.0 == name) {
                    arity(if g.2 { 2 } else { 1 })?;
                    let t = self.spec_expr(&args[0], in_ensures, bound)?;
                    if t != g.1 {
                        return Err(Diag::new("E-TYPE", args[0].pos, format!("argument of `{}` must be {}, found {}", name, g.1.name(), t.name())));
                    }
                    check_lin(&args[0])?;
                    if g.2 {
                        let ti = self.spec_expr(&args[1], in_ensures, bound)?;
                        expect_ty(&ti, &Type::U64, args[1].pos, "an index")?;
                    }
                    return Ok(g.3.clone());
                }
                if !matches!(name.as_str(), "len" | "get" | "slen" | "sget") {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`{}` cannot be called in a specification (allowed: len, get, slen, sget, old, ghost accessors and spec fns)", name)));
                }
                let sig = &self.sigs[name.as_str()];
                arity(sig.params.len())?;
                let tb = self.spec_expr(&args[0], in_ensures, bound)?;
                if tb != *sig.params[0].base() {
                    return Err(Diag::new("E-TYPE", args[0].pos, format!("argument of `{}` must be {}", name, sig.params[0].base().name())));
                }
                check_lin(&args[0])?;
                if args.len() == 2 {
                    let ti = self.spec_expr(&args[1], in_ensures, bound)?;
                    expect_ty(&ti, &Type::U64, args[1].pos, "an index")?;
                }
                Ok(Type::U64)
            }
        }
    }
}

fn bin_type(op: BinOp, a: &Type, b: &Type, pos: Pos) -> Result<Type, Diag> {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Div | Rem => {
            expect_ty(a, &Type::U64, pos, &format!("the left operand of `{}`", op.text()))?;
            expect_ty(b, &Type::U64, pos, &format!("the right operand of `{}`", op.text()))?;
            Ok(Type::U64)
        }
        Lt | Le | Gt | Ge => {
            expect_ty(a, &Type::U64, pos, &format!("the left operand of `{}`", op.text()))?;
            expect_ty(b, &Type::U64, pos, &format!("the right operand of `{}`", op.text()))?;
            Ok(Type::Bool)
        }
        Eq | Ne => {
            if a != b || !(matches!(a, Type::U64 | Type::Bool)) {
                return Err(Diag::new("E-TYPE", pos, format!("cannot compare {} with {}", a.name(), b.name())));
            }
            Ok(Type::Bool)
        }
        And | Or | Implies => {
            expect_ty(a, &Type::Bool, pos, &format!("the left operand of `{}`", op.text()))?;
            expect_ty(b, &Type::Bool, pos, &format!("the right operand of `{}`", op.text()))?;
            Ok(Type::Bool)
        }
    }
}
