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

use crate::ast::*;
use crate::check::is_builtin;
use crate::diag::{Diag, Pos};
use crate::smt::{SatResult, Solver};
use std::collections::HashMap;

const U64_MAX: &str = "18446744073709551615";
/// Largest element count `alloc` accepts (2^60): keeps `n * 8` far from overflow.
const ALLOC_MAX: &str = "1152921504606846976";

#[derive(Clone, Debug, PartialEq)]
enum Val {
    U(String),
    B(String),
    Buf { len: String, data: String },
    Unit,
}

impl Val {
    fn term(&self) -> &str {
        match self {
            Val::U(t) | Val::B(t) => t,
            _ => panic!("not a scalar"),
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

pub struct Verifier<'p> {
    prog: &'p Program,
    solver: Solver,
    counter: usize,
    pub obligations: usize,
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

pub fn verify_program(p: &Program) -> Result<(usize, Vec<String>), Diag> {
    let solver = Solver::start().map_err(|e| Diag::new("E-SOLVER", Pos::default(), e))?;
    let mut v = Verifier { prog: p, solver, counter: 0, obligations: 0 };
    for f in &p.fns {
        v.verify_fn(f)?;
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

    fn fresh_u64(&mut self, hint: &str) -> Result<Val, Diag> {
        let n = self.fresh_name(hint);
        self.smt(&format!("(declare-const {} Int)", n))?;
        self.smt(&format!("(assert (and (<= 0 {n}) (<= {n} {})))", U64_MAX))?;
        Ok(Val::U(n))
    }

    fn fresh_bool(&mut self, hint: &str) -> Result<Val, Diag> {
        let n = self.fresh_name(hint);
        self.smt(&format!("(declare-const {} Bool)", n))?;
        Ok(Val::B(n))
    }

    fn fresh_len(&mut self, hint: &str) -> Result<String, Diag> {
        let n = self.fresh_name(&format!("len_{}", hint));
        self.smt(&format!("(declare-const {} Int)", n))?;
        self.smt(&format!("(assert (and (<= 0 {n}) (<= {n} {})))", ALLOC_MAX))?;
        Ok(n)
    }

    fn fresh_data(&mut self, hint: &str) -> Result<String, Diag> {
        let n = self.fresh_name(&format!("data_{}", hint));
        self.smt(&format!("(declare-const {} (Array Int Int))", n))?;
        Ok(n)
    }

    fn fresh_of(&mut self, ty: &Type, hint: &str) -> Result<Val, Diag> {
        match ty {
            Type::U64 => self.fresh_u64(hint),
            Type::Bool => self.fresh_bool(hint),
            Type::Buf | Type::RefBuf => Ok(Val::Buf { len: self.fresh_len(hint)?, data: self.fresh_data(hint)? }),
            Type::Unit => Ok(Val::Unit),
        }
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
                let mut cex = Vec::new();
                for n in names {
                    match st.env[&n].clone() {
                        Val::U(t) | Val::B(t) => {
                            let v = self.solver.get_value(&t).map_err(|e| Diag::new("E-SOLVER", pos, e))?;
                            cex.push(format!("{} = {}", n, v));
                        }
                        Val::Buf { len, .. } => {
                            let v = self.solver.get_value(&len).map_err(|e| Diag::new("E-SOLVER", pos, e))?;
                            cex.push(format!("len({}) = {}", n, v));
                        }
                        Val::Unit => {}
                    }
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
        let mut env = HashMap::new();
        for p in &f.params {
            let v = self.fresh_of(&p.ty, &p.name)?;
            env.insert(p.name.clone(), v);
        }
        let mut st = State { pc: Vec::new(), env: env.clone() };
        let cx = FnCtx { f, entry: env };
        for r in &f.requires {
            let t = self.spec(r, &st.env, None, &mut Vec::new())?;
            st.pc.push(t);
        }
        if let Some(end) = self.block(&f.body, st, &cx)? {
            // Only unit functions can fall through (checked earlier).
            self.check_ensures(&end, &cx, &Val::Unit, f.pos)?;
        }
        Ok(())
    }

    fn check_ensures(&mut self, st: &State, cx: &FnCtx, result: &Val, pos: Pos) -> Result<(), Diag> {
        for e in &cx.f.ensures {
            let t = self.spec(e, &cx.entry, Some(result), &mut Vec::new())?;
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

    fn stmt(&mut self, s: &Stmt, mut st: State, cx: &FnCtx) -> Result<Option<State>, Diag> {
        match &s.kind {
            StmtKind::Let { name, init, .. } => {
                let v = self.eval(init, &mut st)?;
                st.env.insert(name.clone(), v);
                Ok(Some(st))
            }
            StmtKind::Assign { name, value } => {
                let v = self.eval(value, &mut st)?;
                st.env.insert(name.clone(), v);
                Ok(Some(st))
            }
            StmtKind::Expr(e) => {
                self.eval(e, &mut st)?;
                Ok(Some(st))
            }
            StmtKind::Assert(e) => {
                let t = self.spec(e, &st.env, None, &mut Vec::new())?;
                self.prove(&st, &t, s.pos, "E-VERIFY-ASSERT", format!("`{}`", render(e)))?;
                st.pc.push(t);
                Ok(Some(st))
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
                            let (a, b) = (&t.env[name], &e.env[name]);
                            let merged = if a == b {
                                a.clone()
                            } else {
                                match (a, b) {
                                    (Val::U(x), Val::U(y)) => Val::U(format!("(ite {} {} {})", c, x, y)),
                                    (Val::B(x), Val::B(y)) => Val::B(format!("(ite {} {} {})", c, x, y)),
                                    (Val::Buf { len: l1, data: d1 }, Val::Buf { len: l2, data: d2 }) => Val::Buf {
                                        len: if l1 == l2 { l1.clone() } else { format!("(ite {} {} {})", c, l1, l2) },
                                        data: if d1 == d2 { d1.clone() } else { format!("(ite {} {} {})", c, d1, d2) },
                                    },
                                    _ => a.clone(),
                                }
                            };
                            env.insert(name.clone(), merged);
                        }
                        // Facts proved inside either arm (asserts) are kept as implications.
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
            StmtKind::While { cond, invariants, body } => {
                // 1. invariants hold on entry
                for inv in invariants {
                    let t = self.spec(inv, &st.env, None, &mut Vec::new())?;
                    self.prove(&st, &t, inv.pos, "E-VERIFY-INVARIANT", format!("the loop invariant `{}` on entry to the loop", render(inv)))?;
                }
                // 2. havoc what the body assigns
                let mut assigned = HashMap::new();
                assigned_vars(body, &mut assigned);
                let mut lst = st.clone();
                for (name, only_set) in assigned {
                    let Some(old) = st.env.get(&name).cloned() else { continue };
                    let nv = match old {
                        Val::Buf { len, .. } => {
                            let len = if only_set { len } else { self.fresh_len(&name)? };
                            Val::Buf { len, data: self.fresh_data(&name)? }
                        }
                        Val::U(_) => self.fresh_u64(&name)?,
                        Val::B(_) => self.fresh_bool(&name)?,
                        Val::Unit => Val::Unit,
                    };
                    lst.env.insert(name, nv);
                }
                for inv in invariants {
                    let t = self.spec(inv, &lst.env, None, &mut Vec::new())?;
                    lst.pc.push(t);
                }
                // 3. one arbitrary iteration preserves the invariants
                let c = self.eval(cond, &mut lst)?.term().to_string();
                let mut bst = lst.clone();
                bst.pc.push(c.clone());
                if let Some(end) = self.block(body, bst, cx)? {
                    for inv in invariants {
                        let t = self.spec(inv, &end.env, None, &mut Vec::new())?;
                        self.prove(&end, &t, inv.pos, "E-VERIFY-INVARIANT", format!("that the loop body preserves the invariant `{}`", render(inv)))?;
                    }
                }
                // 4. after the loop: invariants and the negated condition
                lst.pc.push(format!("(not {})", c));
                Ok(Some(lst))
            }
        }
    }

    /// Evaluates a code expression, proving its safety obligations on the way.
    fn eval(&mut self, e: &Expr, st: &mut State) -> Result<Val, Diag> {
        match &e.kind {
            ExprKind::Int(v) => Ok(Val::U(v.to_string())),
            ExprKind::Bool(b) => Ok(Val::B(b.to_string())),
            ExprKind::Var(n) => Ok(st.env[n].clone()),
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
                    sub.pc.push(guard);
                    let vb = self.eval(b, &mut sub)?;
                    let sop = if *op == BinOp::And { "and" } else { "or" };
                    return Ok(Val::B(format!("({} {} {})", sop, ta, vb.term())));
                }
                let vb = self.eval(b, st)?;
                let tb = vb.term().to_string();
                let (ra, rb) = (render(a), render(b));
                let v = match op {
                    BinOp::Add => {
                        let t = format!("(+ {} {})", ta, tb);
                        self.prove(st, &format!("(<= {} {})", t, U64_MAX), e.pos, "E-VERIFY-OVERFLOW", format!("that `{} + {}` does not overflow u64", ra, rb))?;
                        Val::U(t)
                    }
                    BinOp::Sub => {
                        self.prove(st, &format!("(>= {} {})", ta, tb), e.pos, "E-VERIFY-UNDERFLOW", format!("that `{} - {}` does not go below zero", ra, rb))?;
                        Val::U(format!("(- {} {})", ta, tb))
                    }
                    BinOp::Mul => {
                        let t = format!("(* {} {})", ta, tb);
                        self.prove(st, &format!("(<= {} {})", t, U64_MAX), e.pos, "E-VERIFY-OVERFLOW", format!("that `{} * {}` does not overflow u64", ra, rb))?;
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
        // Argument order mirrors the checker: scalars first, then buffers.
        let mut vals: Vec<Option<Val>> = vec![None; args.len()];
        for (i, a) in args.iter().enumerate() {
            if !matches!(&a.kind, ExprKind::Var(n) if matches!(st.env.get(n), Some(Val::Buf { .. }))) {
                vals[i] = Some(self.eval(a, st)?);
            }
        }
        for (i, a) in args.iter().enumerate() {
            if vals[i].is_none() {
                vals[i] = Some(self.eval(a, st)?);
            }
        }
        let vals: Vec<Val> = vals.into_iter().map(|v| v.unwrap()).collect();
        if is_builtin(name) {
            return self.builtin(name, args, &vals, pos, st);
        }
        let f = self.prog.fns.iter().find(|f| f.name == name).expect("checked");
        let mut callee_env = HashMap::new();
        for (p, v) in f.params.iter().zip(&vals) {
            callee_env.insert(p.name.clone(), v.clone());
        }
        for r in &f.requires {
            let t = self.spec(r, &callee_env, None, &mut Vec::new())?;
            self.prove(st, &t, pos, "E-VERIFY-REQUIRES", format!("the precondition `{}` of `{}`", render(r), name))?;
        }
        let res = self.fresh_of(&f.ret, &format!("{}_ret", name))?;
        for en in &f.ensures {
            let t = self.spec(en, &callee_env, Some(&res), &mut Vec::new())?;
            st.pc.push(t);
        }
        Ok(res)
    }

    fn builtin(&mut self, name: &str, args: &[Expr], vals: &[Val], pos: Pos, st: &mut State) -> Result<Val, Diag> {
        match name {
            "alloc" => {
                let n = vals[0].term().to_string();
                self.prove(st, &format!("(<= {} {})", n, ALLOC_MAX), pos, "E-VERIFY-ALLOC", format!("that the allocation size `{}` is at most 2^60 elements", render(&args[0])))?;
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
                    self.prove(st, &format!("(< {} {})", i, len), pos, "E-VERIFY-BOUNDS", format!("that the index `{}` is within `{}` (`{} < len({})`)", render(&args[1]), render(&args[0]), render(&args[1]), render(&args[0])))?;
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
                    self.prove(st, &format!("(< {} {})", i, len), pos, "E-VERIFY-BOUNDS", format!("that the index `{}` is within `{}` (`{} < len({})`)", render(&args[1]), render(&args[0]), render(&args[1]), render(&args[0])))?;
                    Ok(Val::Buf { len: len.clone(), data: format!("(store {} {} {})", data, i, vals[2].term()) })
                }
                _ => unreachable!(),
            },
            "free" | "print" => Ok(Val::Unit),
            _ => unreachable!(),
        }
    }

    /// Translates a ghost specification expression. No obligations.
    fn spec(&mut self, e: &Expr, env: &HashMap<String, Val>, result: Option<&Val>, bound: &mut Vec<String>) -> Result<String, Diag> {
        Ok(match &e.kind {
            ExprKind::Int(v) => v.to_string(),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Var(n) => {
                if bound.contains(n) {
                    format!("q_{}", sanitize(n))
                } else if n == "result" {
                    result.expect("checked").term().to_string()
                } else {
                    env[n].term().to_string()
                }
            }
            ExprKind::Not(x) => format!("(not {})", self.spec(x, env, result, bound)?),
            ExprKind::Bin(op, a, b) => {
                let ta = self.spec(a, env, result, bound)?;
                let tb = self.spec(b, env, result, bound)?;
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
                bound.push(v.clone());
                let tb = self.spec(body, env, result, bound)?;
                bound.pop();
                let q = format!("q_{}", sanitize(v));
                format!("(forall (({q} Int)) (=> (and (<= 0 {q}) (<= {q} {})) {}))", U64_MAX, tb)
            }
            ExprKind::Call(name, args) => {
                let buf = match &args[0].kind {
                    ExprKind::Var(n) if n == "result" => result.expect("checked").clone(),
                    ExprKind::Var(n) => env[n].clone(),
                    _ => return Err(Diag::new("E-TYPE", args[0].pos, "a buffer in a specification must be a variable")),
                };
                let Val::Buf { len, data } = buf else { unreachable!() };
                if name == "len" {
                    len
                } else {
                    let i = self.spec(&args[1], env, result, bound)?;
                    format!("(select {} {})", data, i)
                }
            }
        })
    }
}

/// Variables assigned anywhere in `stmts`, with a flag that is true when every
/// assignment has the shape `x = set(x, ..)` (which preserves the length).
fn assigned_vars(stmts: &[Stmt], out: &mut HashMap<String, bool>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Assign { name, value } => {
                let only_set = matches!(&value.kind, ExprKind::Call(f, args) if f == "set" && matches!(&args[0].kind, ExprKind::Var(n) if n == name));
                let e = out.entry(name.clone()).or_insert(true);
                *e = *e && only_set;
            }
            StmtKind::If { then_b, else_b, .. } => {
                assigned_vars(then_b, out);
                assigned_vars(else_b, out);
            }
            StmtKind::While { body, .. } => assigned_vars(body, out),
            _ => {}
        }
    }
}
