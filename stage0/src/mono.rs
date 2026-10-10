// Monomorphization: generic structs, enums and functions are copied once per
// list of type arguments they are used with, and every later pass sees only
// the copies. A copy is named after its arguments (`Option[u64]`,
// `swap[Buf]`), and type parameters are replaced by those arguments
// everywhere in the definition, including its specifications.
//
// Type arguments are written out at every use (`swap[u64](a, b)`,
// `Option[u64]::None`); they are not inferred yet. A generic definition is
// checked and verified once per copy, so an error in it is reported for the
// first copy that has it.

use crate::ast::*;
use crate::diag::{Diag, Pos};
use std::collections::{HashMap, HashSet, VecDeque};

/// More copies than this means a definition instantiates itself with ever
/// larger arguments (`S[T]` holding `S[Box[T]]`).
const MAX_INSTANCES: usize = 1000;

struct Mono {
    structs: HashMap<String, StructDef>,
    enums: HashMap<String, EnumDef>,
    fns: HashMap<String, FnDef>,
    /// Instances already requested, by name.
    seen: HashSet<String>,
    queue: VecDeque<(String, Vec<Type>, String)>,
}

pub fn monomorphize(p: &mut Program) -> Result<(), Diag> {
    let mut m = Mono { structs: HashMap::new(), enums: HashMap::new(), fns: HashMap::new(), seen: HashSet::new(), queue: VecDeque::new() };
    // Names stay unique across generic and plain definitions.
    let mut types: Vec<&str> = Vec::new();
    for (n, pos) in p.structs.iter().map(|d| (&d.name, d.pos)).chain(p.enums.iter().map(|d| (&d.name, d.pos))) {
        if types.contains(&n.as_str()) {
            return Err(Diag::new("E-TYPE", pos, format!("type `{}` is already defined", n)));
        }
        types.push(n);
    }
    let mut names: Vec<&str> = Vec::new();
    for f in &p.fns {
        if names.contains(&f.name.as_str()) {
            return Err(Diag::new("E-TYPE", f.pos, format!("function `{}` is defined twice", f.name)));
        }
        names.push(&f.name);
    }
    for d in p.structs.iter().filter(|d| !d.tparams.is_empty()) {
        m.structs.insert(d.name.clone(), d.clone());
    }
    for d in p.enums.iter().filter(|d| !d.tparams.is_empty()) {
        m.enums.insert(d.name.clone(), d.clone());
    }
    for f in p.fns.iter().filter(|f| !f.tparams.is_empty()) {
        if f.name == "main" {
            return Err(Diag::new("E-TYPE", f.pos, "`main` cannot be generic"));
        }
        m.fns.insert(f.name.clone(), f.clone());
    }
    p.structs.retain(|d| d.tparams.is_empty());
    p.enums.retain(|d| d.tparams.is_empty());
    p.fns.retain(|f| f.tparams.is_empty());
    let none = HashMap::new();
    for d in &mut p.structs {
        for (_, t) in &mut d.fields {
            *t = m.ty(t, &none, d.pos)?;
        }
    }
    for d in &mut p.enums {
        for (_, t) in d.variants.iter_mut() {
            if let Some(t) = t {
                *t = m.ty(t, &none, d.pos)?;
            }
        }
    }
    for f in &mut p.fns {
        m.fn_def(f, &none)?;
    }
    for sf in &mut p.specs {
        for prm in &mut sf.params {
            prm.ty = m.ty(&prm.ty, &none, prm.pos)?;
        }
        sf.ret = m.ty(&sf.ret, &none, sf.pos)?;
        m.expr(&mut sf.body, &none)?;
    }
    let mut count = 0;
    while let Some((generic, args, inst)) = m.queue.pop_front() {
        count += 1;
        if count > MAX_INSTANCES {
            let pos = m.structs.get(&generic).map(|d| d.pos).or(m.enums.get(&generic).map(|d| d.pos)).or(m.fns.get(&generic).map(|f| f.pos)).unwrap_or_default();
            return Err(Diag::new("E-TYPE", pos, format!("more than {} generic instances (is `{}` instantiating itself with growing arguments?)", MAX_INSTANCES, generic)));
        }
        if let Some(d) = m.structs.get(&generic).cloned() {
            let s = subst_map(&d.tparams, &args);
            let mut fields = Vec::new();
            for (f, t) in &d.fields {
                fields.push((f.clone(), m.ty(t, &s, d.pos)?));
            }
            p.structs.push(StructDef { name: inst, tparams: Vec::new(), fields, pos: d.pos });
        } else if let Some(d) = m.enums.get(&generic).cloned() {
            let s = subst_map(&d.tparams, &args);
            let mut variants = Vec::new();
            for (v, t) in &d.variants {
                variants.push((v.clone(), match t {
                    Some(t) => Some(m.ty(t, &s, d.pos)?),
                    None => None,
                }));
            }
            p.enums.push(EnumDef { name: inst, tparams: Vec::new(), variants, pos: d.pos });
        } else {
            let mut f = m.fns[&generic].clone();
            let s = subst_map(&f.tparams, &args);
            f.name = inst;
            f.tparams = Vec::new();
            m.fn_def(&mut f, &s)?;
            p.fns.push(f);
        }
    }
    Ok(())
}

fn subst_map(params: &[String], args: &[Type]) -> HashMap<String, Type> {
    params.iter().cloned().zip(args.iter().cloned()).collect()
}

impl Mono {
    /// The instance `name[args]` of a generic definition, requested once.
    fn instance(&mut self, name: &str, args: Vec<Type>, arity: usize, what: &str, pos: Pos) -> Result<String, Diag> {
        if args.len() != arity {
            return Err(Diag::new("E-TYPE", pos, format!("{} `{}` takes {} type argument(s), {} given", what, name, arity, args.len())));
        }
        let inst = Type::Generic(name.to_string(), args.clone()).name();
        if self.seen.insert(inst.clone()) {
            self.queue.push_back((name.to_string(), args, inst.clone()));
        }
        Ok(inst)
    }

    fn ty(&mut self, t: &Type, s: &HashMap<String, Type>, pos: Pos) -> Result<Type, Diag> {
        Ok(match t {
            Type::Struct(n) if s.contains_key(n) => s[n].clone(),
            Type::Struct(n) if self.structs.contains_key(n) || self.enums.contains_key(n) => {
                return Err(Diag::new("E-TYPE", pos, format!("`{}` is generic: write its type arguments, e.g. `{}[u64]`", n, n)))
            }
            Type::Generic(n, args) => {
                let mut a = Vec::new();
                for x in args {
                    a.push(self.ty(x, s, pos)?);
                }
                let arity = match (self.structs.get(n), self.enums.get(n)) {
                    (Some(d), _) => d.tparams.len(),
                    (_, Some(d)) => d.tparams.len(),
                    _ => return Err(Diag::new("E-TYPE", pos, format!("`{}` is not a generic struct or enum", n))),
                };
                Type::Struct(self.instance(n, a, arity, "type", pos)?)
            }
            Type::Box(i) => Type::Box(Box::new(self.ty(i, s, pos)?)),
            Type::Vec(i) => Type::Vec(Box::new(self.ty(i, s, pos)?)),
            Type::Ref(i) => Type::Ref(Box::new(self.ty(i, s, pos)?)),
            Type::MutRef(i) => Type::MutRef(Box::new(self.ty(i, s, pos)?)),
            t => t.clone(),
        })
    }

    fn fn_def(&mut self, f: &mut FnDef, s: &HashMap<String, Type>) -> Result<(), Diag> {
        for prm in &mut f.params {
            prm.ty = self.ty(&prm.ty, s, prm.pos)?;
        }
        f.ret = self.ty(&f.ret, s, f.pos)?;
        for e in f.requires.iter_mut().chain(f.ensures.iter_mut()) {
            self.expr(e, s)?;
        }
        self.stmts(&mut f.body, s)
    }

    fn stmts(&mut self, ss: &mut [Stmt], s: &HashMap<String, Type>) -> Result<(), Diag> {
        for st in ss {
            match &mut st.kind {
                StmtKind::Let { ty, init, .. } => {
                    if let Some(t) = ty {
                        *t = self.ty(t, s, st.pos)?;
                    }
                    self.expr(init, s)?;
                }
                StmtKind::Assign { value, .. } => self.expr(value, s)?,
                StmtKind::If { cond, then_b, else_b } => {
                    self.expr(cond, s)?;
                    self.stmts(then_b, s)?;
                    self.stmts(else_b, s)?;
                }
                StmtKind::While { cond, invariants, body } => {
                    self.expr(cond, s)?;
                    for i in invariants {
                        self.expr(i, s)?;
                    }
                    self.stmts(body, s)?;
                }
                StmtKind::Return(Some(e)) | StmtKind::Assert(e) | StmtKind::Expr(e) => self.expr(e, s)?,
                StmtKind::Return(None) | StmtKind::Unreachable => {}
                StmtKind::Match { scrut, arms } => {
                    self.expr(scrut, s)?;
                    for a in arms {
                        self.stmts(&mut a.body, s)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn expr(&mut self, e: &mut Expr, s: &HashMap<String, Type>) -> Result<(), Diag> {
        let pos = e.pos;
        // Type arguments first: the inner expression is renamed to the instance.
        if let ExprKind::TypeApp(targs, inner) = &mut e.kind {
            let mut a = Vec::new();
            for t in targs.iter() {
                a.push(self.ty(t, s, pos)?);
            }
            // A builtin that takes type arguments keeps them (they say what
            // it makes), with type parameters replaced.
            if matches!(&inner.kind, ExprKind::Call(n, _) if crate::check::TYPED_BUILTINS.contains(&n.as_str())) {
                *targs = a;
                return self.children(inner, s);
            }
            let mut inner = (**inner).clone();
            match &mut inner.kind {
                ExprKind::Call(n, _) => {
                    let arity = self.fns.get(n).map(|f| f.tparams.len()).ok_or_else(|| Diag::new("E-TYPE", pos, format!("`{}` is not a generic function", n)))?;
                    *n = self.instance(n, a, arity, "function", pos)?;
                }
                ExprKind::StructLit(n, _) => {
                    let arity = self.structs.get(n).map(|d| d.tparams.len()).ok_or_else(|| Diag::new("E-TYPE", pos, format!("`{}` is not a generic struct", n)))?;
                    *n = self.instance(n, a, arity, "type", pos)?;
                }
                ExprKind::Variant(n, _, _) => {
                    let arity = self.enums.get(n).map(|d| d.tparams.len()).ok_or_else(|| Diag::new("E-TYPE", pos, format!("`{}` is not a generic enum", n)))?;
                    *n = self.instance(n, a, arity, "type", pos)?;
                }
                _ => unreachable!("the parser only applies type arguments to these"),
            }
            *e = inner;
            // The renamed expression's own children are rewritten below, and
            // its name is no longer generic.
            return self.children(e, s);
        }
        match &e.kind {
            ExprKind::Call(n, _) if self.fns.contains_key(n) => {
                return Err(Diag::new("E-TYPE", pos, format!("`{}` is generic: write its type arguments, e.g. `{}[u64](..)`", n, n)));
            }
            ExprKind::StructLit(n, _) | ExprKind::Variant(n, _, _) if self.structs.contains_key(n) || self.enums.contains_key(n) => {
                return Err(Diag::new("E-TYPE", pos, format!("`{}` is generic: write its type arguments, e.g. `{}[u64]`", n, n)));
            }
            _ => {}
        }
        self.children(e, s)
    }

    fn children(&mut self, e: &mut Expr, s: &HashMap<String, Type>) -> Result<(), Diag> {
        match &mut e.kind {
            ExprKind::Field(x, _) | ExprKind::Is(x, _, _) | ExprKind::Not(x) | ExprKind::Forall(_, x) => self.expr(x, s),
            ExprKind::Variant(_, _, Some(x)) => self.expr(x, s),
            ExprKind::StructLit(_, fs) => {
                for (_, x) in fs {
                    self.expr(x, s)?;
                }
                Ok(())
            }
            ExprKind::Bin(_, a, b) => {
                self.expr(a, s)?;
                self.expr(b, s)
            }
            ExprKind::Call(_, args) => {
                for a in args {
                    self.expr(a, s)?;
                }
                Ok(())
            }
            ExprKind::TypeApp(..) => self.expr(e, s),
            _ => Ok(()),
        }
    }
}
