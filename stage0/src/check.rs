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
//
// Structs: a struct with a linear field is itself linear. Ownership is
// tracked per place (`p`, `p.root`, `p.inner.buf`): a linear field can be
// moved out and later assigned back, and a struct counts as consumed once
// every linear field in it has been moved out. Scalar fields can be read as
// long as the struct itself has not been moved.

use crate::ast::*;
use crate::codegen::Target;
use crate::diag::{Diag, Pos};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone)]
pub struct Sig {
    pub params: Vec<Type>,
    pub ret: Type,
}

/// Which builtins exist depends on what the program runs on.
#[derive(Clone, Copy, PartialEq)]
enum Avail {
    All,
    /// Hosted programs only (they have an operating system under them).
    Hosted,
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
        ("balloc", Avail::All, vec![U64], Bytes),
        ("blen", Avail::All, vec![r(Bytes)], U64),
        ("bget", Avail::All, vec![r(Bytes), U64], U64),
        ("bset", Avail::All, vec![m(Bytes), U64, U64], Unit),
        ("bfree", Avail::All, vec![Bytes], Unit),
        ("bfrom", Avail::All, vec![Str], Bytes),
        ("host_argc", Avail::Hosted, vec![r(Host)], U64),
        ("host_arg", Avail::Hosted, vec![r(Host), U64], Bytes),
        ("host_read", Avail::Hosted, vec![r(Host), r(Bytes), m(Bytes)], Bool),
        ("host_write", Avail::Hosted, vec![r(Host), r(Bytes), r(Bytes)], Bool),
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
    name == "box" || name == "unbox" || builtin_table().iter().any(|b| b.0 == name)
}

fn available(name: &str, target: Target) -> bool {
    let Some(b) = builtin_table().into_iter().find(|b| b.0 == name) else { return true };
    match (b.1, target) {
        (Avail::All, _) => true,
        (Avail::Hosted, Target::Hosted) => true,
        (Avail::Mem, Target::Hosted | Target::Kernel) => true,
        (Avail::Kernel, Target::Kernel) => true,
        (Avail::User, Target::User) => true,
        _ => false,
    }
}

/// The capabilities `main` may ask for on each target.
fn main_caps(target: Target) -> &'static [Type] {
    match target {
        Target::Hosted => &[Type::Serial, Type::Pool, Type::Host],
        Target::Kernel => &[Type::Serial, Type::Pool, Type::Mmu, Type::Cpu],
        Target::User => &[Type::Serial],
    }
}

/// Struct and enum definitions: unique names, known field and payload
/// types, no type that contains itself.
pub fn check_structs(p: &Program) -> Result<(), Diag> {
    const RESERVED: &[&str] = &["u64", "bool", "Str", "Buf", "Bytes", "Host", "Serial", "Frame", "PageTable", "Pool", "Mmu", "Cpu"];
    // (name, position, the types it holds directly)
    let mut defs: Vec<(String, Pos, Vec<Type>)> = Vec::new();
    for d in &p.structs {
        defs.push((d.name.clone(), d.pos, d.fields.iter().map(|f| f.1.clone()).collect()));
    }
    for e in &p.enums {
        defs.push((e.name.clone(), e.pos, e.variants.iter().filter_map(|v| v.1.clone()).collect()));
    }
    for (i, (name, pos, held)) in defs.iter().enumerate() {
        if RESERVED.contains(&name.as_str()) || defs[..i].iter().any(|e| &e.0 == name) {
            return Err(Diag::new("E-TYPE", *pos, format!("type `{}` is already defined", name)));
        }
        for t in held {
            let mut t = t;
            while let Type::Box(i) = t {
                t = i;
            }
            if let Type::Struct(n) | Type::Enum(n) = t {
                if !defs.iter().any(|e| &e.0 == n) {
                    return Err(Diag::new("E-TYPE", *pos, format!("unknown type `{}`", n)));
                }
            }
        }
    }
    fn reaches(defs: &[(String, Pos, Vec<Type>)], from: &str, target: &str, seen: &mut Vec<String>) -> bool {
        let d = defs.iter().find(|d| d.0 == from).unwrap();
        d.2.iter().any(|t| match t {
            Type::Struct(n) | Type::Enum(n) if n == target => true,
            Type::Struct(n) | Type::Enum(n) if !seen.contains(n) => {
                seen.push(n.clone());
                reaches(defs, n, target, seen)
            }
            _ => false,
        })
    }
    for (name, pos, _) in &defs {
        if reaches(&defs, name, name, &mut Vec::new()) {
            return Err(Diag::new("E-TYPE", *pos, format!("type `{}` contains itself", name))
                .note("a value cannot hold itself; put the recursive part in a `Box[..]`"));
        }
    }
    Ok(())
}

/// A type written in the program must name a known struct, and only linear
/// values can be borrowed.
fn known_type(t: &Type, pos: Pos) -> Result<(), Diag> {
    match t {
        Type::Struct(n) if struct_def(n).is_none() => Err(Diag::new("E-TYPE", pos, format!("unknown type `{}`", n))),
        Type::Enum(n) if enum_def(n).is_none() => Err(Diag::new("E-TYPE", pos, format!("unknown type `{}`", n))),
        Type::Box(i) => known_type(i, pos),
        Type::Ref(i) | Type::MutRef(i) => {
            known_type(i, pos)?;
            if !i.is_linear() {
                return Err(Diag::new("E-TYPE", pos, format!("only linear values can be borrowed, found `{}`", t.name()))
                    .note("a struct without linear fields is a plain value: pass it by value"));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn overlaps(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{}.", b)) || b.starts_with(&format!("{}.", a))
}

pub fn signatures(p: &Program, target: Target) -> Result<HashMap<String, Sig>, Diag> {
    let mut sigs = builtin_sigs();
    for f in &p.fns {
        for prm in &f.params {
            known_type(&prm.ty, prm.pos)?;
        }
        known_type(&f.ret, f.pos)?;
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
    Ctx { sigs, specs, target, ret, scopes: vec![Vec::new()], vars: HashMap::new(), moved: HashSet::new() }
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
        known_type(&p.ty, p.pos)?;
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
        // A name may be reused once its earlier binding's scope has ended
        // (two arms binding `v`), but never shadow one still in scope.
        if self.vars.contains_key(name) || self.sigs.contains_key(name) || is_ghost(name) || self.specs.iter().any(|s| s.name == name) {
            return Err(Diag::new("E-TYPE", pos, format!("`{}` is already defined (shadowing is not allowed)", name)));
        }
        self.vars.insert(name.to_string(), VarInfo { ty, mutable });
        self.scopes.last_mut().unwrap().push(name.to_string());
        Ok(())
    }

    /// The type at a dotted path, and the info of the variable it starts from.
    fn path_type(&self, path: &str, pos: Pos) -> Result<(Type, VarInfo), Diag> {
        let mut parts = path.split('.');
        let info = self.var_info(parts.next().unwrap(), pos)?;
        let mut t = info.ty.base().clone();
        let mut sofar = path_root(path).to_string();
        for f in parts {
            // A box is looked through: `b.f` is the field `f` of the boxed struct.
            while let Type::Box(i) = t {
                t = *i;
            }
            let ft = match &t {
                Type::Struct(n) => struct_def(n).and_then(|d| d.field(f)).cloned(),
                _ => None,
            };
            t = ft.ok_or_else(|| Diag::new("E-TYPE", pos, format!("`{}` ({}) has no field `{}`", sofar, t.name(), f)))?;
            sofar = format!("{}.{}", sofar, f);
        }
        Ok((t, info))
    }

    /// The moved place that `path` is, or lies inside.
    fn prefix_moved(&self, path: &str) -> Option<String> {
        let mut p = path;
        loop {
            if self.moved.contains(p) {
                return Some(p.to_string());
            }
            match p.rfind('.') {
                Some(k) => p = &p[..k],
                None => return None,
            }
        }
    }

    /// A moved place strictly inside `path`.
    fn desc_moved(&self, path: &str) -> Option<String> {
        let pre = format!("{}.", path);
        let mut v: Vec<&String> = self.moved.iter().filter(|m| m.starts_with(&pre)).collect();
        v.sort();
        v.first().map(|m| m.to_string())
    }

    /// Nothing is left to own at `path`: it was moved, or (for a struct)
    /// every linear field in it was.
    fn consumed(&self, path: &str, t: &Type) -> bool {
        if !t.is_linear() || self.prefix_moved(path).is_some() {
            return true;
        }
        match t {
            Type::Struct(n) => struct_def(n).unwrap().fields.iter().all(|(f, ft)| self.consumed(&format!("{}.{}", path, f), ft)),
            _ => false,
        }
    }

    /// `path` holds a whole value again.
    fn unmove(&mut self, path: &str) {
        let pre = format!("{}.", path);
        self.moved.retain(|m| m != path && !m.starts_with(&pre));
    }

    /// The consumed linear leaves of every variable in scope: the ownership
    /// state, independent of whether a struct was moved whole or field by field.
    fn canon(&self) -> BTreeSet<String> {
        fn leaves(cx: &Ctx, path: &str, t: &Type, out: &mut BTreeSet<String>) {
            if !t.is_linear() {
                return;
            }
            match t {
                Type::Struct(n) => {
                    for (f, ft) in &struct_def(n).unwrap().fields {
                        leaves(cx, &format!("{}.{}", path, f), ft, out);
                    }
                }
                // The box itself is a resource, and so is what is inside it.
                Type::Box(i) => {
                    if cx.prefix_moved(path).is_some() {
                        out.insert(path.to_string());
                    } else {
                        leaves(cx, path, i, out);
                    }
                }
                _ => {
                    if cx.prefix_moved(path).is_some() {
                        out.insert(path.to_string());
                    }
                }
            }
        }
        let mut out = BTreeSet::new();
        for n in self.scopes.iter().flatten() {
            let t = &self.vars[n].ty;
            if !matches!(t, Type::Ref(_)) {
                leaves(self, n, t.base(), &mut out);
            }
        }
        out
    }

    /// Owned linear variables still holding a value (`&mut` parameters excluded).
    fn live_owned(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .scopes
            .iter()
            .flatten()
            .filter(|n| self.vars[*n].ty.is_linear() && !self.consumed(n, &self.vars[*n].ty))
            .cloned()
            .collect();
        v.sort();
        v
    }

    /// `&mut` parameters whose value (or part of it) has been moved out and not put back.
    fn missing_mut_refs(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .scopes
            .iter()
            .flatten()
            .filter(|n| self.vars[*n].ty.is_mut_ref() && (self.prefix_moved(n).is_some() || self.desc_moved(n).is_some()))
            .cloned()
            .collect();
        v.sort();
        v
    }

    /// A place used as a value: moves it if it is linear, copies it otherwise.
    fn use_place(&mut self, path: &str, pos: Pos) -> Result<Type, Diag> {
        let (t, info) = self.path_type(path, pos)?;
        if let Some(m) = self.prefix_moved(path) {
            let msg = if m == path { format!("`{}` is used after it was consumed", path) } else { format!("`{}` is used after `{}` was consumed", path, m) };
            return Err(Diag::new("E-LINEAR-USE", pos, msg));
        }
        if t.is_linear() {
            if matches!(info.ty, Type::Ref(_)) {
                return Err(Diag::new("E-LINEAR-USE", pos, format!("cannot move out of the shared borrow `{}`", path_root(path))));
            }
            if let Some(m) = self.desc_moved(path) {
                return Err(Diag::new("E-LINEAR-USE", pos, format!("`{}` is used after its field `{}` was moved out", path, m)));
            }
            self.moved.insert(path.to_string());
        }
        Ok(t)
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
                if self.vars[n].ty.is_linear() && !self.consumed(n, &self.vars[n].ty) {
                    return Err(Diag::new("E-LINEAR-LEAK", stmts.last().map(|s| s.pos).unwrap_or_default(), format!("`{}` goes out of scope while still owned", n))
                        .note("every linear value must be consumed exactly once: return it, pass it to a function that takes it by value, or `free` it"));
                }
            }
        }
        for n in &scope {
            self.vars.remove(n);
            self.unmove(n);
        }
        Ok(diverges)
    }

    fn stmt(&mut self, s: &mut Stmt) -> Result<bool, Diag> {
        let pos = s.pos;
        match &mut s.kind {
            StmtKind::Let { name, mutable, ty, init } => {
                let t = self.expr(init)?;
                if let Some(want) = ty {
                    known_type(want, pos)?;
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
                let root = path_root(name).to_string();
                let (want, info) = self.path_type(name, pos)?;
                if !info.mutable {
                    return Err(Diag::new("E-TYPE", pos, format!("`{}` is not mutable (declare it with `mut`)", root)));
                }
                let t = self.expr(value)?;
                expect_ty(&t, &want, value.pos, &format!("the value assigned to `{}`", name))?;
                if let Some(k) = name.rfind('.') {
                    if let Some(m) = self.prefix_moved(&name[..k]) {
                        return Err(Diag::new("E-LINEAR-USE", pos, format!("cannot assign to `{}`: `{}` was consumed", name, m)));
                    }
                }
                if want.is_linear() {
                    if !self.consumed(name, &want) {
                        return Err(Diag::new("E-LINEAR-LEAK", pos, format!("assigning to `{}` would drop the value it still owns", name))
                            .note("consume the old value first, e.g. `b = set(b, i, v);` or move the field out before putting a new one in"));
                    }
                    self.unmove(name);
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
                        let ce = self.canon();
                        self.moved = after_then.clone();
                        let ct = self.canon();
                        if ct != ce {
                            let diff: Vec<_> = ct.symmetric_difference(&ce).cloned().collect();
                            return Err(Diag::new("E-LINEAR-BRANCH", pos, format!("`{}` is consumed in only one branch of this `if`", diff.join("`, `"))));
                        }
                        self.moved = after_then.union(&after_else).cloned().collect();
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
                let cb = self.canon();
                let d = self.block(body)?;
                let ca = self.canon();
                if !d && ca != cb {
                    let diff: Vec<_> = ca.symmetric_difference(&cb).cloned().collect();
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
            StmtKind::Match { scrut, arms } => self.match_stmt(scrut, arms, pos),
            StmtKind::Expr(e) => {
                let t = self.expr(e)?;
                if t.is_linear() {
                    return Err(Diag::new("E-LINEAR-LEAK", pos, "this linear value is created and immediately dropped"));
                }
                Ok(false)
            }
        }
    }

    /// `match`: every variant handled once (E-MATCH), each payload either
    /// bound or, when it is linear, accounted for; the arms must leave
    /// ownership the same way, as the arms of an `if` must.
    fn match_stmt(&mut self, scrut: &Expr, arms: &mut [Arm], pos: Pos) -> Result<bool, Diag> {
        // A place of a borrowed enum is only inspected; anything else is
        // moved (or copied, when the enum is not linear).
        let borrowed = match place(scrut) {
            Some(p) => matches!(self.path_type(&p, scrut.pos)?.1.ty, Type::Ref(_)),
            None => false,
        };
        let t = if borrowed {
            let p = place(scrut).unwrap();
            if let Some(m) = self.prefix_moved(&p) {
                return Err(Diag::new("E-LINEAR-USE", scrut.pos, format!("`{}` is used after `{}` was consumed", p, m)));
            }
            self.path_type(&p, scrut.pos)?.0
        } else {
            self.expr(scrut)?
        };
        let Type::Enum(en) = &t else {
            return Err(Diag::new("E-TYPE", scrut.pos, format!("`match` needs an enum, found {}", t.name())));
        };
        let d = enum_def(en).unwrap();
        let owned_linear = !borrowed && t.is_linear();
        let mut covered: Vec<usize> = Vec::new();
        let mut wild = false;
        for a in arms.iter() {
            if wild {
                return Err(Diag::new("E-MATCH", a.pos, "this arm can never run: `_` above already matches everything left"));
            }
            match &a.pat {
                Pat::Wild => {
                    for (k, (v, pt)) in d.variants.iter().enumerate() {
                        if !covered.contains(&k) && owned_linear && pt.as_ref().map_or(false, |t| t.is_linear()) {
                            return Err(Diag::new("E-LINEAR-LEAK", a.pos, format!("`_` would drop the linear payload of `{}::{}`", en, v))
                                .note("match that variant by name and consume its payload"));
                        }
                    }
                    wild = true;
                }
                Pat::Variant(pe, v, b, _) => {
                    if pe != en {
                        return Err(Diag::new("E-TYPE", a.pos, format!("this pattern is a `{}`, but the value matched is a `{}`", pe, en)));
                    }
                    let k = d.index(v).ok_or_else(|| Diag::new("E-TYPE", a.pos, format!("`{}` has no variant `{}`", en, v)))?;
                    if covered.contains(&k) {
                        return Err(Diag::new("E-MATCH", a.pos, format!("`{}::{}` is matched twice", en, v)));
                    }
                    covered.push(k);
                    match (&d.variants[k].1, b) {
                        (None, Some(_)) => return Err(Diag::new("E-TYPE", a.pos, format!("`{}::{}` has no payload", en, v))),
                        (Some(_), None) => return Err(Diag::new("E-TYPE", a.pos, format!("`{}::{}` has a payload: write `{}::{}(x)` or `{}::{}(_)`", en, v, en, v, en, v))),
                        (Some(pt), Some(b)) if pt.is_linear() && b == "_" && owned_linear => {
                            return Err(Diag::new("E-LINEAR-LEAK", a.pos, format!("`_` would drop the linear payload of `{}::{}`", en, v)));
                        }
                        _ => {}
                    }
                }
            }
        }
        if !wild && covered.len() < d.variants.len() {
            let missing: Vec<String> = d.variants.iter().enumerate().filter(|(k, _)| !covered.contains(k)).map(|(_, v)| format!("{}::{}", en, v.0)).collect();
            return Err(Diag::new("E-MATCH", pos, format!("this `match` does not handle `{}`", missing.join("`, `"))));
        }
        // The arms, each from the same ownership state.
        let before = self.moved.clone();
        let mut outs: Vec<HashSet<String>> = Vec::new();
        for a in arms.iter_mut() {
            self.moved = before.clone();
            self.scopes.push(Vec::new());
            if let Pat::Variant(_, v, Some(b), m) = &a.pat {
                if b != "_" {
                    let pt = d.variants[d.index(v).unwrap()].1.clone().unwrap();
                    // Out of a shared borrow, a linear payload is bound as a
                    // shared borrow too: it can be read and lent, never moved.
                    if borrowed && pt.is_linear() {
                        if *m {
                            return Err(Diag::new("E-TYPE", a.pos, format!("`{}` borrows from `{}` and cannot be `mut`", b, place(scrut).unwrap())));
                        }
                        self.declare(b, Type::Ref(Box::new(pt)), false, a.pos)?;
                    } else {
                        self.declare(b, pt, *m, a.pos)?;
                    }
                }
            }
            let diverges = self.block(&mut a.body)?;
            let scope = self.scopes.pop().unwrap();
            if !diverges {
                for n in &scope {
                    if self.vars[n].ty.is_linear() && !self.consumed(n, &self.vars[n].ty) {
                        return Err(Diag::new("E-LINEAR-LEAK", a.pos, format!("`{}` goes out of scope while still owned", n))
                            .note("the payload bound by a pattern is owned by the arm: consume it"));
                    }
                }
            }
            for n in &scope {
                self.vars.remove(n);
                self.unmove(n);
            }
            if !diverges {
                outs.push(self.moved.clone());
            }
        }
        if outs.is_empty() {
            return Ok(true);
        }
        let mut canons = Vec::new();
        for o in &outs {
            self.moved = o.clone();
            canons.push(self.canon());
        }
        if let Some(k) = canons.iter().position(|c| *c != canons[0]) {
            let diff: Vec<_> = canons[0].symmetric_difference(&canons[k]).cloned().collect();
            return Err(Diag::new("E-LINEAR-BRANCH", pos, format!("`{}` is consumed in only some arms of this `match`", diff.join("`, `"))));
        }
        self.moved = outs.into_iter().flatten().collect();
        Ok(false)
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
                if let Type::Ref(_) = self.var_info(n, e.pos)?.ty {
                    return Err(Diag::new("E-LINEAR-USE", e.pos, format!("cannot move out of the shared borrow `{}`", n)));
                }
                self.use_place(n, e.pos)
            }
            ExprKind::Field(..) => {
                let p = place(e).ok_or_else(|| Diag::new("E-TYPE", e.pos, "a field can only be read from a variable or a field of one"))?;
                self.use_place(&p, e.pos)
            }
            ExprKind::Variant(en, v, payload) => {
                let d = enum_def(en).ok_or_else(|| Diag::new("E-TYPE", e.pos, format!("unknown enum `{}`", en)))?;
                let k = d.index(v).ok_or_else(|| Diag::new("E-TYPE", e.pos, format!("`{}` has no variant `{}`", en, v)))?;
                match (&d.variants[k].1, payload) {
                    (None, None) => {}
                    (Some(pt), Some(x)) => {
                        let t = self.expr(x)?;
                        expect_ty(&t, pt, x.pos, &format!("the payload of `{}::{}`", en, v))?;
                    }
                    (None, Some(_)) => return Err(Diag::new("E-TYPE", e.pos, format!("`{}::{}` has no payload", en, v))),
                    (Some(pt), None) => return Err(Diag::new("E-TYPE", e.pos, format!("`{}::{}` needs a payload of type {}", en, v, pt.name()))),
                }
                Ok(Type::Enum(en.clone()))
            }
            ExprKind::Is(x, en, v) => {
                // Reads the tag only: nothing is consumed.
                let t = match place(x) {
                    Some(p) => {
                        if let Some(m) = self.prefix_moved(&p) {
                            return Err(Diag::new("E-LINEAR-USE", x.pos, format!("`{}` is used after `{}` was consumed", p, m)));
                        }
                        self.path_type(&p, x.pos)?.0
                    }
                    None => {
                        let t = self.expr(x)?;
                        if t.is_linear() {
                            return Err(Diag::new("E-LINEAR-LEAK", x.pos, "`is` does not consume its operand, so this linear value would be dropped"));
                        }
                        t
                    }
                };
                if t != Type::Enum(en.clone()) {
                    return Err(Diag::new("E-TYPE", x.pos, format!("`is {}::{}` needs a `{}`, found {}", en, v, en, t.name())));
                }
                if enum_def(en).and_then(|d| d.index(v)).is_none() {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`{}` has no variant `{}`", en, v)));
                }
                Ok(Type::Bool)
            }
            ExprKind::StructLit(n, fs) => {
                let d = struct_def(n).ok_or_else(|| Diag::new("E-TYPE", e.pos, format!("unknown struct `{}`", n)))?;
                for (f, _) in &d.fields {
                    if !fs.iter().any(|x| &x.0 == f) {
                        return Err(Diag::new("E-TYPE", e.pos, format!("field `{}` of `{}` is missing", f, n)));
                    }
                }
                for (f, x) in fs {
                    let ft = d.field(f).ok_or_else(|| Diag::new("E-TYPE", x.pos, format!("`{}` has no field `{}`", n, f)))?;
                    let t = self.expr(x)?;
                    expect_ty(&t, ft, x.pos, &format!("field `{}`", f))?;
                }
                Ok(Type::Struct(n.clone()))
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
                if name == "box" || name == "unbox" {
                    // Generic over the boxed type, so not in the builtin table.
                    if args.len() != 1 {
                        return Err(Diag::new("E-TYPE", e.pos, format!("`{}` takes 1 argument(s), {} given", name, args.len())));
                    }
                    let t = self.expr(&args[0])?;
                    if name == "box" {
                        return Ok(Type::Box(Box::new(t)));
                    }
                    return match t {
                        Type::Box(i) => Ok(*i),
                        t => Err(Diag::new("E-TYPE", args[0].pos, format!("`unbox` needs a box, found {}", t.name()))),
                    };
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
                // Borrows: a place lent as `&mut` may not overlap any other
                // borrow in the same call (a place overlaps its fields).
                let mut borrowed: Vec<(String, bool)> = Vec::new();
                for (pt, a) in sig.params.iter().zip(args) {
                    if !pt.is_ref() {
                        continue;
                    }
                    let n = match (&a.kind, pt) {
                        (ExprKind::MutBorrow(n), Type::MutRef(_)) => n.clone(),
                        (_, Type::Ref(_)) if place(a).is_some() => place(a).unwrap(),
                        (_, Type::MutRef(_)) => return Err(Diag::new("E-TYPE", a.pos, format!("argument of `{}` must be `&mut x` for a variable or field `x`", name))),
                        _ => return Err(Diag::new("E-TYPE", a.pos, format!("argument of `{}` must name a value to borrow", name))),
                    };
                    let (t, info) = self.path_type(&n, a.pos)?;
                    if t != *pt.base() {
                        return Err(Diag::new("E-TYPE", a.pos, format!("argument of `{}` must be {}, found {}", name, pt.base().name(), t.name())));
                    }
                    if pt.is_mut_ref() {
                        if matches!(info.ty, Type::Ref(_)) {
                            return Err(Diag::new("E-LINEAR-USE", a.pos, format!("`{}` is a shared borrow and cannot be lent as `&mut`", path_root(&n))));
                        }
                        if !info.mutable {
                            return Err(Diag::new("E-TYPE", a.pos, format!("`{}` is not mutable (declare it with `mut`)", path_root(&n))));
                        }
                    }
                    if self.prefix_moved(&n).is_some() {
                        return Err(Diag::new("E-LINEAR-USE", a.pos, format!("`{}` is borrowed after it was consumed", n)));
                    }
                    if let Some(m) = self.desc_moved(&n) {
                        return Err(Diag::new("E-LINEAR-USE", a.pos, format!("`{}` is borrowed after its field `{}` was moved out", n, m)));
                    }
                    if let Some((q, _)) = borrowed.iter().find(|(q, qm)| overlaps(&n, q) && (*qm || pt.is_mut_ref())) {
                        let msg = if *q == n {
                            format!("`{}` is lent as `&mut` while also borrowed in this call", n)
                        } else {
                            format!("`{}` and `{}` overlap, and one of them is lent as `&mut` in this call", q, n)
                        };
                        return Err(Diag::new("E-LINEAR-USE", a.pos, msg));
                    }
                    borrowed.push((n, pt.is_mut_ref()));
                }
                for (pt, a) in sig.params.iter().zip(args) {
                    if pt.is_linear() {
                        if let Some(n) = place(a) {
                            if borrowed.iter().any(|(q, _)| overlaps(&n, q)) {
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
            ExprKind::StructLit(..) | ExprKind::Variant(..) => Err(Diag::new("E-TYPE", e.pos, "struct and enum values cannot be built in a specification")),
            ExprKind::Is(x, en, v) => {
                let t = self.spec_expr(x, in_ensures, bound)?;
                if t != Type::Enum(en.clone()) || enum_def(en).and_then(|d| d.index(v)).is_none() {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`is {}::{}` needs a `{}`, found {}", en, v, en, t.name())));
                }
                Ok(Type::Bool)
            }
            ExprKind::Field(b, f) => {
                let mut t = self.spec_expr(b, in_ensures, bound)?;
                while let Type::Box(i) = t {
                    t = *i;
                }
                match &t {
                    Type::Struct(n) => struct_def(n).and_then(|d| d.field(f)).cloned().ok_or_else(|| Diag::new("E-TYPE", e.pos, format!("`{}` has no field `{}`", n, f))),
                    // In a specification, `o.V` is the payload of variant V
                    // (meaningful when `o is E::V`).
                    Type::Enum(n) => enum_def(n)
                        .and_then(|d| d.index(f).and_then(|k| d.variants[k].1.clone()))
                        .ok_or_else(|| Diag::new("E-TYPE", e.pos, format!("`{}` has no variant `{}` with a payload", n, f))),
                    _ => Err(Diag::new("E-TYPE", e.pos, format!("{} has no fields", t.name()))),
                }
            }
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
                fn named(a: &Expr) -> bool {
                    match &a.kind {
                        ExprKind::Var(_) | ExprKind::Str(_) | ExprKind::Blob(_) => true,
                        ExprKind::Call(n, xs) => n == "old" && xs.len() == 1 && matches!(xs[0].kind, ExprKind::Var(_)),
                        ExprKind::Field(b, _) => named(b),
                        _ => false,
                    }
                }
                let check_lin = |a: &Expr| -> Result<(), Diag> {
                    if named(a) { Ok(()) } else { Err(Diag::new("E-TYPE", a.pos, "a value in this position must be a variable, a field of one, `result` or `old(x)`")) }
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
                if !matches!(name.as_str(), "len" | "get" | "slen" | "sget" | "blen" | "bget") {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`{}` cannot be called in a specification (allowed: len, get, slen, sget, blen, bget, old, ghost accessors and spec fns)", name)));
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
