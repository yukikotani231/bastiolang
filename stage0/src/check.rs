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
use crate::diag::{Diag, Pos};
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub struct Sig {
    pub params: Vec<Type>,
    pub ret: Type,
}

pub fn builtin_sigs() -> HashMap<String, Sig> {
    let mut m = HashMap::new();
    m.insert("alloc".into(), Sig { params: vec![Type::U64], ret: Type::Buf });
    let rbuf = || Type::Ref(Box::new(Type::Buf));
    let rser = || Type::Ref(Box::new(Type::Serial));
    m.insert("len".into(), Sig { params: vec![rbuf()], ret: Type::U64 });
    m.insert("get".into(), Sig { params: vec![rbuf(), Type::U64], ret: Type::U64 });
    m.insert("slen".into(), Sig { params: vec![Type::Str], ret: Type::U64 });
    m.insert("sget".into(), Sig { params: vec![Type::Str, Type::U64], ret: Type::U64 });
    m.insert("serial_write".into(), Sig { params: vec![rser(), Type::U64], ret: Type::Unit });
    m.insert("serial_read".into(), Sig { params: vec![rser()], ret: Type::U64 });
    m.insert("set".into(), Sig { params: vec![Type::Buf, Type::U64, Type::U64], ret: Type::Buf });
    m.insert("free".into(), Sig { params: vec![Type::Buf], ret: Type::Unit });
    m.insert("print".into(), Sig { params: vec![Type::U64], ret: Type::Unit });
    m
}

pub fn is_builtin(name: &str) -> bool {
    matches!(name, "alloc" | "len" | "get" | "set" | "free" | "print" | "slen" | "sget" | "serial_write" | "serial_read")
}

pub fn signatures(p: &Program) -> Result<HashMap<String, Sig>, Diag> {
    let mut sigs = builtin_sigs();
    for f in &p.fns {
        if is_builtin(&f.name) {
            return Err(Diag::new("E-TYPE", f.pos, format!("`{}` is a builtin and cannot be redefined", f.name)));
        }
        if sigs.contains_key(&f.name) {
            return Err(Diag::new("E-TYPE", f.pos, format!("function `{}` is defined twice", f.name)));
        }
        if f.ret.is_ref() {
            return Err(Diag::new("E-TYPE", f.pos, "a function cannot return a borrow `&Buf`"));
        }
        sigs.insert(f.name.clone(), Sig { params: f.params.iter().map(|p| p.ty.clone()).collect(), ret: f.ret.clone() });
    }
    match sigs.get("main") {
        Some(s) if s.params.is_empty() && s.ret == Type::U64 => {}
        Some(s) if s.params == vec![Type::Serial] && s.ret == Type::Serial => {}
        Some(_) => {
            let f = p.fns.iter().find(|f| f.name == "main").unwrap();
            return Err(Diag::new("E-TYPE", f.pos, "`main` must be `fn main() -> u64` or `fn main(con: Serial) -> Serial`"));
        }
        None => return Err(Diag::new("E-TYPE", Pos { line: 1, col: 1 }, "no `main` function")),
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
    ret: Type,
    scopes: Vec<Vec<String>>,
    vars: HashMap<String, VarInfo>,
    /// Buf variables that have been consumed.
    moved: HashSet<String>,
    all_names: HashSet<String>,
}

pub fn check_program(p: &mut Program) -> Result<HashMap<String, Sig>, Diag> {
    let sigs = signatures(p)?;
    for f in &mut p.fns {
        check_fn(f, &sigs)?;
    }
    Ok(sigs)
}

fn check_fn(f: &mut FnDef, sigs: &HashMap<String, Sig>) -> Result<(), Diag> {
    let mut cx = Ctx {
        sigs,
        ret: f.ret.clone(),
        scopes: vec![Vec::new()],
        vars: HashMap::new(),
        moved: HashSet::new(),
        all_names: HashSet::new(),
    };
    for p in &f.params {
        if p.ty.is_ref() && p.mutable {
            return Err(Diag::new("E-TYPE", p.pos, "a borrow cannot be `mut`"));
        }
        cx.declare(&p.name, p.ty.clone(), p.mutable, p.pos)?;
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
        if name == "result" {
            return Err(Diag::new("E-TYPE", pos, "`result` is reserved for `ensures` clauses"));
        }
        if self.all_names.contains(name) || self.sigs.contains_key(name) {
            return Err(Diag::new("E-TYPE", pos, format!("`{}` is already defined (shadowing is not allowed)", name)));
        }
        self.all_names.insert(name.to_string());
        self.vars.insert(name.to_string(), VarInfo { ty, mutable });
        self.scopes.last_mut().unwrap().push(name.to_string());
        Ok(())
    }

    fn live_bufs(&self) -> Vec<String> {
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

    fn leak_check_all(&self, pos: Pos, when: &str) -> Result<(), Diag> {
        let live = self.live_bufs();
        if !live.is_empty() {
            return Err(Diag::new("E-LINEAR-LEAK", pos, format!("`{}` is still owned {}", live.join("`, `"), when))
                .note("every linear value must be consumed exactly once: return it, pass it to a function that takes it by value, or `free` it"));
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
                        .note("every Buf must be consumed exactly once: return it, pass it to a function that takes `Buf`, or `free` it"));
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
                expect_ty(&t, &info.ty, value.pos, &format!("the value assigned to `{}`", name))?;
                if info.ty.is_linear() {
                    if !self.moved.contains(name.as_str()) {
                        return Err(Diag::new("E-LINEAR-LEAK", pos, format!("assigning to `{}` would drop the value it still owns", name))
                            .note("consume the old buffer first, e.g. `b = set(b, i, v);`"));
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
                            return Err(Diag::new("E-LINEAR-BRANCH", pos, format!("buffer `{}` is consumed in only one branch of this `if`", diff.join("`, `"))));
                        }
                        Ok(false)
                    }
                }
            }
            StmtKind::While { cond, invariants, body } => {
                let before_cond = self.moved.clone();
                let t = self.expr(cond)?;
                if self.moved != before_cond {
                    return Err(Diag::new("E-LINEAR-USE", cond.pos, "a `while` condition runs many times, so it cannot consume a buffer"));
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
                        .note("a loop body must leave every outer buffer owned exactly as it found it"));
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

    /// Code expression: Buf variables in value position are moved.
    fn expr(&mut self, e: &Expr) -> Result<Type, Diag> {
        match &e.kind {
            ExprKind::Int(_) => Ok(Type::U64),
            ExprKind::Bool(_) => Ok(Type::Bool),
            ExprKind::Str(_) => Ok(Type::Str),
            ExprKind::Var(n) => {
                let info = self.var_info(n, e.pos)?;
                match info.ty {
                    t if t.is_linear() => {
                        if self.moved.contains(n.as_str()) {
                            return Err(Diag::new("E-LINEAR-USE", e.pos, format!("`{}` is used after it was consumed", n)));
                        }
                        self.moved.insert(n.clone());
                        Ok(t)
                    }
                    Type::Ref(_) => Err(Diag::new("E-LINEAR-USE", e.pos, format!("cannot move out of the borrow `{}`", n))),
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
                    return Err(Diag::new("E-LINEAR-USE", b.pos, format!("the right side of `{}` may not run, so it cannot consume a buffer", op.text())));
                }
                bin_type(*op, &ta, &tb, e.pos)
            }
            ExprKind::Forall(..) => Err(Diag::new("E-TYPE", e.pos, "`forall` is only allowed in specifications")),
            ExprKind::Call(name, args) => {
                if name == "result" {
                    return Err(Diag::new("E-TYPE", e.pos, "`result` is only allowed in `ensures`"));
                }
                let sig = self.sigs.get(name).cloned().ok_or_else(|| Diag::new("E-TYPE", e.pos, format!("unknown function `{}`", name)))?;
                if sig.params.len() != args.len() {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`{}` takes {} argument(s), {} given", name, sig.params.len(), args.len())));
                }
                // Scalars first, then borrows, then moves: a scalar argument may
                // read a buffer that a later argument consumes (`set(b, i, get(b, j))`).
                for (pt, a) in sig.params.iter().zip(args) {
                    if !pt.is_linear() && !pt.is_ref() {
                        let t = self.expr(a)?;
                        expect_ty(&t, pt, a.pos, &format!("argument of `{}`", name))?;
                    }
                }
                let mut borrowed = HashSet::new();
                for (pt, a) in sig.params.iter().zip(args) {
                    if pt.is_ref() {
                        let n = match &a.kind {
                            ExprKind::Var(n) => n,
                            _ => return Err(Diag::new("E-TYPE", a.pos, format!("argument of `{}` must name a value to borrow", name))),
                        };
                        let info = self.var_info(n, a.pos)?;
                        if info.ty.base() != pt.base() {
                            return Err(Diag::new("E-TYPE", a.pos, format!("argument of `{}` must be {}, found {}", name, pt.base().name(), info.ty.name())));
                        }
                        if self.moved.contains(n.as_str()) {
                            return Err(Diag::new("E-LINEAR-USE", a.pos, format!("`{}` is borrowed after it was consumed", n)));
                        }
                        borrowed.insert(n.clone());
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
            ExprKind::Str(_) => Ok(Type::Str),
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
                Ok(self.var_info(n, e.pos)?.ty)
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
                if !matches!(name.as_str(), "len" | "get" | "slen" | "sget") {
                    return Err(Diag::new("E-TYPE", e.pos, format!("only `len`, `get`, `slen` and `sget` may be called in specifications, not `{}`", name)));
                }
                let sig = &self.sigs[name.as_str()];
                if sig.params.len() != args.len() {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`{}` takes {} argument(s), {} given", name, sig.params.len(), args.len())));
                }
                let tb = self.spec_expr(&args[0], in_ensures, bound)?;
                if tb.base() != sig.params[0].base() {
                    return Err(Diag::new("E-TYPE", args[0].pos, format!("argument of `{}` must be {}", name, sig.params[0].base().name())));
                }
                if !matches!(&args[0].kind, ExprKind::Var(_) | ExprKind::Str(_)) {
                    return Err(Diag::new("E-TYPE", args[0].pos, format!("the first argument of `{}` in a specification must be a variable", name)));
                }
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
