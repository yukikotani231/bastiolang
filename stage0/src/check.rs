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
        ("try_alloc", Avail::All, vec![U64], Buf),
        ("try_balloc", Avail::All, vec![U64], Bytes),
        ("blen", Avail::All, vec![r(Bytes)], U64),
        ("bget", Avail::All, vec![r(Bytes), U64], U8),
        ("bset", Avail::All, vec![m(Bytes), U64, U8], Unit),
        ("bcopy", Avail::All, vec![m(Bytes), U64, r(Bytes), U64, U64], Unit),
        ("bmove", Avail::All, vec![m(Bytes), U64, U64, U64], Unit),
        ("bfree", Avail::All, vec![Bytes], Unit),
        ("bfrom", Avail::All, vec![Str], Bytes),
        ("host_argc", Avail::Hosted, vec![r(Host)], U64),
        ("host_arg", Avail::Hosted, vec![r(Host), U64], Bytes),
        ("host_read", Avail::Hosted, vec![r(Host), r(Bytes), m(Bytes)], Bool),
        ("host_write", Avail::Hosted, vec![r(Host), r(Bytes), r(Bytes)], Bool),
        ("host_eprint", Avail::Hosted, vec![r(Host), r(Bytes)], Unit),
        ("host_env", Avail::Hosted, vec![r(Host), Str, m(Bytes)], Bool),
        ("host_cpus", Avail::Hosted, vec![r(Host)], U64),
        ("host_spawn", Avail::Hosted, vec![r(Host), r(Vec(std::boxed::Box::new(Bytes)))], Child),
        ("child_write", Avail::Hosted, vec![m(Child), r(Bytes)], Bool),
        ("child_close", Avail::Hosted, vec![m(Child)], Unit),
        ("child_read_line", Avail::Hosted, vec![m(Child), m(Bytes)], Bool),
        ("child_wait", Avail::Hosted, vec![Child], U64),
        ("slen", Avail::All, vec![Str], U64),
        ("sget", Avail::All, vec![Str, U64], U8),
        ("serial_write", Avail::All, vec![r(Serial), U8], Unit),
        ("serial_read", Avail::All, vec![r(Serial)], U8),
        ("serial_poll", Avail::Kernel, vec![r(Serial)], U64),
        ("pool_size", Avail::Mem, vec![r(Pool)], U64),
        ("pool_take", Avail::Mem, vec![m(Pool), U64], Frame),
        ("pool_give", Avail::Mem, vec![m(Pool), Frame], Unit),
        ("frame_id", Avail::Mem, vec![r(Frame)], U64),
        ("frame_read", Avail::Mem, vec![r(Frame), U64], U64),
        ("frame_write", Avail::Mem, vec![m(Frame), U64, U64], Unit),
        ("frame_read8", Avail::Mem, vec![r(Frame), U64], U8),
        ("frame_write8", Avail::Mem, vec![m(Frame), U64, U8], Unit),
        ("frame_to_bytes", Avail::Mem, vec![m(Bytes), U64, r(Frame), U64, U64], Unit),
        ("bytes_to_frame", Avail::Mem, vec![m(Frame), U64, r(Bytes), U64, U64], Unit),
        ("pt_new", Avail::Mem, vec![Frame, U64], PageTable),
        ("pt_free", Avail::Mem, vec![PageTable], Frame),
        ("pt_free_root", Avail::Kernel, vec![r(Mmu), PageTable], Frame),
        ("pt_has", Avail::Mem, vec![r(PageTable), U64], Bool),
        ("pt_writable", Avail::Mem, vec![r(PageTable), U64], Bool),
        ("pt_put", Avail::Mem, vec![m(PageTable), U64, PageTable], Unit),
        ("pt_take", Avail::Mem, vec![m(PageTable), U64], PageTable),
        ("pt_put_frame", Avail::Mem, vec![m(PageTable), U64, Frame, Bool], Unit),
        ("pt_take_frame", Avail::Mem, vec![m(PageTable), U64], Frame),
        ("pt_lease_count", Avail::Mem, vec![m(PageTable)], U64),
        ("shared_new", Avail::Mem, vec![Frame], Shared),
        ("shared_dup", Avail::Mem, vec![r(Shared)], Shared),
        ("shared_drop", Avail::Mem, vec![m(Pool), Shared], U64),
        ("shared_id", Avail::Mem, vec![r(Shared)], U64),
        ("pt_shared", Avail::Mem, vec![r(PageTable), U64], Bool),
        ("pt_put_shared", Avail::Mem, vec![m(PageTable), U64, Shared, Bool], Unit),
        ("pt_take_shared", Avail::Mem, vec![m(PageTable), U64], Shared),
        ("pt_dup_shared", Avail::Mem, vec![r(PageTable), U64], Shared),
        ("pool_take_dirty", Avail::Mem, vec![m(Pool), U64], Dirty),
        ("dirty_id", Avail::Mem, vec![r(Dirty)], U64),
        ("dirty_zero", Avail::Mem, vec![Dirty], Frame),
        ("frame_copy", Avail::Mem, vec![Dirty, r(Frame)], Frame),
        ("shared_copy", Avail::Mem, vec![Dirty, r(Shared)], Frame),
        ("dirty_fill", Avail::Mem, vec![Dirty, r(Bytes), U64], Frame),
        ("mmu_enter", Avail::Kernel, vec![m(Mmu), m(PageTable)], Lease),
        ("mmu_exit", Avail::Kernel, vec![m(Mmu), Lease], Unit),
        ("mmu_kernel", Avail::Kernel, vec![m(Mmu)], Unit),
        ("cpu_run", Avail::Kernel, vec![m(Cpu), r(Mmu)], U64),
        ("cpu_idle", Avail::Kernel, vec![m(Cpu)], U64),
        ("cpu_get", Avail::Kernel, vec![r(Cpu), U64], U64),
        ("cpu_set", Avail::Kernel, vec![m(Cpu), U64, U64], Unit),
        ("cpu_fpu_save", Avail::Kernel, vec![m(Cpu), m(Frame)], Unit),
        ("cpu_fpu_load", Avail::Kernel, vec![m(Cpu), r(Frame)], Unit),
        ("cpu_count", Avail::Kernel, vec![r(Cpu)], U64),
        ("cpu_tsc", Avail::Kernel, vec![r(Cpu)], U64),
        ("boot_fb", Avail::Kernel, vec![r(Cpu), U64], U64),
        ("boot_acpi", Avail::Kernel, vec![r(Cpu), U64, U64], U64),
        ("boot_info", Avail::Kernel, vec![r(Cpu), U64, U64], U64),
        ("port_claim", Avail::Mem, vec![m(Ports), U64, U64], PortIo),
        ("port_release", Avail::Mem, vec![m(Ports), PortIo], Unit),
        ("port_base", Avail::Mem, vec![r(PortIo)], U64),
        ("port_count", Avail::Mem, vec![r(PortIo)], U64),
        ("pio_in8", Avail::Mem, vec![m(PortIo), U64], U8),
        ("pio_in16", Avail::Mem, vec![m(PortIo), U64], U16),
        ("pio_in32", Avail::Mem, vec![m(PortIo), U64], U32),
        ("pio_out8", Avail::Mem, vec![m(PortIo), U64, U8], Unit),
        ("pio_out16", Avail::Mem, vec![m(PortIo), U64, U16], Unit),
        ("pio_out32", Avail::Mem, vec![m(PortIo), U64, U32], Unit),
        ("mmio_claim", Avail::Mem, vec![m(MmioSpace), U64, U64], Mmio),
        ("mmio_release", Avail::Mem, vec![m(MmioSpace), Mmio], Unit),
        ("mmio_base", Avail::Mem, vec![r(Mmio)], U64),
        ("mmio_len", Avail::Mem, vec![r(Mmio)], U64),
        ("mmio_read8", Avail::Mem, vec![m(Mmio), U64], U8),
        ("mmio_read16", Avail::Mem, vec![m(Mmio), U64], U16),
        ("mmio_read32", Avail::Mem, vec![m(Mmio), U64], U32),
        ("mmio_read64", Avail::Mem, vec![m(Mmio), U64], U64),
        ("mmio_write8", Avail::Mem, vec![m(Mmio), U64, U8], Unit),
        ("mmio_write16", Avail::Mem, vec![m(Mmio), U64, U16], Unit),
        ("mmio_write32", Avail::Mem, vec![m(Mmio), U64, U32], Unit),
        ("mmio_write64", Avail::Mem, vec![m(Mmio), U64, U64], Unit),
        ("dma_lend", Avail::Mem, vec![Frame], Dma),
        ("dma_take", Avail::Mem, vec![Dma], Frame),
        ("dma_id", Avail::Mem, vec![r(Dma)], U64),
        ("dma_addr", Avail::Mem, vec![r(Dma)], U64),
        ("dma_read8", Avail::Mem, vec![m(Dma), U64], U8),
        ("dma_read16", Avail::Mem, vec![m(Dma), U64], U16),
        ("dma_read32", Avail::Mem, vec![m(Dma), U64], U32),
        ("dma_read64", Avail::Mem, vec![m(Dma), U64], U64),
        ("dma_write8", Avail::Mem, vec![m(Dma), U64, U8], Unit),
        ("dma_write16", Avail::Mem, vec![m(Dma), U64, U16], Unit),
        ("dma_write32", Avail::Mem, vec![m(Dma), U64, U32], Unit),
        ("dma_write64", Avail::Mem, vec![m(Dma), U64, U64], Unit),
        ("dma_to_bytes", Avail::Mem, vec![m(Bytes), U64, m(Dma), U64, U64], Unit),
        ("bytes_to_dma", Avail::Mem, vec![m(Dma), U64, r(Bytes), U64, U64], Unit),
        ("sys_spawn", Avail::User, vec![r(Buf), U64, U64], U64),
        ("sys_wait", Avail::User, vec![U64], U64),
        ("sys_kill", Avail::User, vec![U64], U64),
        ("sys_yield", Avail::User, vec![], Unit),
        ("sys_getpid", Avail::User, vec![], U64),
        ("sys_info", Avail::User, vec![U64], U64),
        ("sys_ps", Avail::User, vec![], Unit),
        ("sys_poweroff", Avail::User, vec![], Unit),
        ("sys_call", Avail::User, vec![U64, U64, U64], U64),
        ("sys_send", Avail::User, vec![U64, U64, r(Bytes), U64], U64),
        ("sys_recv", Avail::User, vec![U64, U64, m(Bytes)], U64),
    ]
}

/// Ghost accessors, usable only in specifications: (name, argument type, has an index, result).
pub const GHOSTS: &[(&str, Type, bool, Type)] = &[
    ("frame_id", Type::Frame, false, Type::U64),
    ("pt_id", Type::PageTable, false, Type::U64),
    ("pt_level", Type::PageTable, false, Type::U64),
    ("pt_used", Type::PageTable, true, Type::Bool),
    ("pt_leases", Type::PageTable, false, Type::U64),
    ("pt_shared", Type::PageTable, true, Type::Bool),
    ("shared_id", Type::Shared, false, Type::U64),
    ("dirty_id", Type::Dirty, false, Type::U64),
    ("pool_has", Type::Pool, true, Type::Bool),
    ("pool_size", Type::Pool, false, Type::U64),
    ("mmu_root", Type::Mmu, false, Type::U64),
    ("port_base", Type::PortIo, false, Type::U64),
    ("port_count", Type::PortIo, false, Type::U64),
    ("port_free", Type::Ports, true, Type::Bool),
    ("mmio_base", Type::Mmio, false, Type::U64),
    ("mmio_len", Type::Mmio, false, Type::U64),
    ("mmio_free", Type::MmioSpace, true, Type::Bool),
    ("dma_id", Type::Dma, false, Type::U64),
];

pub fn is_ghost(name: &str) -> bool {
    GHOSTS.iter().any(|g| g.0 == name)
}

pub fn builtin_sigs() -> HashMap<String, Sig> {
    builtin_table().into_iter().map(|(n, _, params, ret)| (n.to_string(), Sig { params, ret })).collect()
}

/// Builtins over `Vec[T]`: generic, so their signatures are built per call.
pub const VEC_BUILTINS: &[&str] = &["vec_new", "vec_len", "vec_push", "vec_push_within", "vec_try_reserve", "vec_pop", "vec_get", "vec_set", "vec_swap", "vec_free"];

/// Builtins written with type arguments (`vec_new[u64]()`, `cast[u8](x)`).
pub const TYPED_BUILTINS: &[&str] = &["vec_new", "cast", "wrap", "try_box"];

/// Wrapping arithmetic: both operands and the result have one integer type.
pub const WRAP_BUILTINS: &[&str] = &["wrap_add", "wrap_sub", "wrap_mul"];

/// An integer literal written directly (constants are folded into literals
/// by the parser). It has no type of its own: it takes the integer type its
/// position asks for (docs/integers.md).
fn int_lit(e: &Expr) -> Option<i128> {
    match &e.kind {
        ExprKind::Int(v) => Some(*v as i128),
        ExprKind::Neg(x) => match x.kind {
            ExprKind::Int(v) => Some(-(v as i128)),
            _ => None,
        },
        _ => None,
    }
}

/// In a specification every integer is just a number, whatever its type.
fn spec_norm(t: Type) -> Type {
    if t.is_int() { Type::U64 } else { t }
}

/// Every builtin with where it is available, for `--trusted-base`.
pub fn builtin_list() -> Vec<(String, &'static str)> {
    let mut v: Vec<(String, &'static str)> = vec![("box".into(), "all"), ("try_box".into(), "all"), ("unbox".into(), "all")];
    v.extend(LOCK_BUILTINS.iter().map(|n| (n.to_string(), "all")));
    v.extend(ATOMIC_BUILTINS.iter().map(|n| (n.to_string(), "all")));
    v.push(("smp_run".into(), "kernel"));
    v.extend(PT_BORROW_BUILTINS.iter().map(|n| (n.to_string(), "hosted, kernel")));
    v.extend(VEC_BUILTINS.iter().map(|n| (n.to_string(), "all")));
    for (n, a, _, _) in builtin_table() {
        let a = match a {
            Avail::All => "all",
            Avail::Hosted => "hosted",
            Avail::Mem => "hosted, kernel",
            Avail::Kernel => "kernel",
            Avail::User => "user",
        };
        v.push((n.to_string(), a));
    }
    v
}

/// Builtins over `Lock[T]`: generic, so checked by hand like `box`.
pub const LOCK_BUILTINS: &[&str] = &["lock_new", "lock_end", "locked"];

/// Builtins that lend a page table's child (a table, or a page at level 1)
/// in place to a function they name, like `locked`.
pub const PT_BORROW_BUILTINS: &[&str] = &["pt_borrow", "pt_borrow_frame"];

/// Builtins over `Atomic[T]`: generic too.
pub const ATOMIC_BUILTINS: &[&str] = &["atomic_new", "atomic_end", "atomic_load", "atomic_store", "atomic_swap", "atomic_cas", "atomic_wrap_add"];

/// Can an `Atomic` hold a `T`? One machine word that C's `__atomic`
/// builtins handle without a lock: an integer, a bool, or a struct with a
/// single such field (which is how an atomic value gets an invariant).
pub fn atomic_word(t: &Type) -> bool {
    match t {
        Type::Bool => true,
        t if t.is_int() => true,
        Type::Struct(n) => struct_def(n).map_or(false, |d| d.fields.len() == 1 && (d.fields[0].1 == Type::Bool || d.fields[0].1.is_int())),
        _ => false,
    }
}

pub fn is_builtin(name: &str) -> bool {
    name == "box" || name == "try_box" || name == "unbox" || name == "cast" || name == "wrap" || name == "smp_run" || PT_BORROW_BUILTINS.contains(&name) || LOCK_BUILTINS.contains(&name) || ATOMIC_BUILTINS.contains(&name) || VEC_BUILTINS.contains(&name) || WRAP_BUILTINS.contains(&name) || builtin_table().iter().any(|b| b.0 == name)
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
        Target::Hosted => &[Type::Serial, Type::Pool, Type::Host, Type::Ports, Type::MmioSpace],
        Target::Kernel => &[Type::Serial, Type::Pool, Type::Mmu, Type::Cpu, Type::Ports, Type::MmioSpace],
        Target::User => &[Type::Serial],
    }
}

/// Struct and enum definitions: unique names, known field and payload
/// types, no type that contains itself.
pub fn check_structs(p: &Program) -> Result<(), Diag> {
    const RESERVED: &[&str] = &["u64", "u8", "u16", "u32", "i8", "i16", "i32", "i64", "bool", "Str", "Buf", "Bytes", "Host", "Child", "Serial", "Frame", "PageTable", "Pool", "Mmu", "Cpu", "Ports", "PortIo", "MmioSpace", "Mmio", "Dma", "Lease", "Shared", "Dirty", "Lock", "Atomic"];
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
            while let Type::Box(i) | Type::Vec(i) | Type::Lock(i) | Type::Atomic(i) = t {
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

/// Locates an error in a copy of a generic function (`dup[Buf]`).
pub fn in_copy(d: Diag, f: &str) -> Diag {
    if f.contains('[') { d.note(format!("in `{}`, the copy of a generic function for these type arguments", f)) } else { d }
}

/// The type of `try_box[T](..)`: the copy of core's `Boxed[T]` for `T`.
pub fn boxed_type(t: &Type) -> Type {
    Type::Enum(Type::Generic("Boxed".to_string(), vec![t.clone()]).name())
}

/// Does the enum name written in a pattern or an `is` test name the enum
/// `actual`? A generic enum may be written without its type arguments
/// (`Option::Some` for an `Option[u64]`), since the value's type has them.
fn names_enum(written: &str, actual: &str) -> bool {
    written == actual || (actual.starts_with(written) && actual[written.len()..].starts_with('['))
}

/// A type written in the program must name a known struct, and only linear
/// values can be borrowed.
fn known_type(t: &Type, pos: Pos) -> Result<(), Diag> {
    match t {
        Type::Struct(n) if struct_def(n).is_none() => Err(Diag::new("E-TYPE", pos, format!("unknown type `{}`", n))),
        Type::Enum(n) if enum_def(n).is_none() => Err(Diag::new("E-TYPE", pos, format!("unknown type `{}`", n))),
        Type::Box(i) | Type::Vec(i) | Type::Lock(i) | Type::Atomic(i) => known_type(i, pos),
        // Any value can be borrowed; a borrowed plain value reads as a copy.
        // (Generic code borrows `&T` whatever `T` turns out to be.)
        Type::Ref(i) | Type::MutRef(i) => known_type(i, pos),
        _ => Ok(()),
    }
}

fn overlaps(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{}.", b)) || b.starts_with(&format!("{}.", a))
}

/// A type a C caller can pass to an `export fn`: a number, or bytes it lends
/// (`&Bytes`, which becomes a pointer and a length).
fn c_param_type(t: &Type) -> bool {
    match t {
        Type::Ref(i) | Type::MutRef(i) => **i == Type::Bytes,
        t => c_value_type(t),
    }
}

/// A type an `export fn` can hand back to C: numbers, owned bytes, and
/// structs and enums made only of those.
fn c_return_type(t: &Type) -> bool {
    match t {
        Type::Unit | Type::Bytes => true,
        Type::Struct(n) => struct_def(n).map_or(false, |d| d.fields.iter().all(|(_, t)| c_return_type(t))),
        Type::Enum(n) => enum_def(n).map_or(false, |d| d.variants.iter().all(|(_, t)| t.as_ref().map_or(true, c_return_type))),
        t => c_value_type(t),
    }
}

/// A plain number C has a type for.
fn c_value_type(t: &Type) -> bool {
    matches!(t, Type::U64 | Type::U32 | Type::U16 | Type::U8 | Type::Bool)
}

/// The rules of the C boundary (see docs/language.md, "Calling Bastio from C").
fn check_exports(p: &Program, target: Target) -> Result<(), Diag> {
    for f in p.fns.iter().filter(|f| f.export) {
        if target != Target::Hosted {
            return Err(Diag::new("E-EXPORT", f.pos, "`export fn` is only supported on the hosted target"));
        }
        if f.name == "main" {
            return Err(Diag::new("E-EXPORT", f.pos, "`main` cannot be exported"));
        }
        if f.name.starts_with("bst_") {
            return Err(Diag::new("E-EXPORT", f.pos, "names starting with `bst_` are reserved for the runtime"));
        }
        if let Some(r) = f.requires.first() {
            return Err(Diag::new("E-EXPORT", r.pos, format!("`{}` is exported, so it cannot have a precondition", f.name))
                .note("a C caller cannot be proved to meet it; check the condition in the function and return an error instead"));
        }
        for prm in &f.params {
            if !c_param_type(&prm.ty) {
                return Err(Diag::new("E-EXPORT", prm.pos, format!("a parameter of an exported function cannot have type `{}`", prm.ty.name()))
                    .note("C can pass integers, `bool`, and `&Bytes` or `&mut Bytes` (a pointer and a length)"));
            }
            let len = format!("{}_len", prm.name);
            if *prm.ty.base() == Type::Bytes && f.params.iter().any(|q| q.name == len) {
                return Err(Diag::new("E-EXPORT", prm.pos, format!("the C length parameter of `{}` would be named `{}`, like another parameter", prm.name, len)));
            }
        }
        if !c_return_type(&f.ret) {
            return Err(Diag::new("E-EXPORT", f.pos, format!("an exported function cannot return `{}`", f.ret.name()))
                .note("C can receive integers, `bool`, `Bytes`, and structs and enums made only of those"));
        }
    }
    Ok(())
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
    check_exports(p, target)?;
    let main = p.fns.iter().find(|f| f.name == "main");
    // A library (a program with an `export fn`) needs no `main`.
    let Some(main) = main else {
        if p.fns.iter().any(|f| f.export) {
            return Ok(sigs);
        }
        return Err(Diag::new("E-TYPE", Pos::default(), "no `main` function"));
    };
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
    /// Where each place was last consumed, for the note under a later use.
    moved_at: HashMap<String, Pos>,
    /// See `FnDef::int_ops`.
    int_ops: HashMap<Pos, Type>,
    /// The user functions this one calls (`locked` and `smp_run` call the
    /// function they name), for `lock_order`.
    calls: Vec<String>,
    /// Each `locked` call: its position, the lock's class (the type of its
    /// contents) and the function run with the lock held.
    lock_sites: Vec<(Pos, String, String)>,
    /// The function's parameters, which `old(..)` in a loop invariant may name.
    params: Vec<String>,
    /// Checking a loop invariant, where `old(..)` is allowed.
    in_inv: std::cell::Cell<bool>,
    /// Checking the argument of `old(..)` in a loop invariant, which may
    /// name only parameters (they alone have a value at the call).
    old_only: std::cell::Cell<bool>,
}

pub fn check_program(p: &mut Program, target: Target) -> Result<HashMap<String, Sig>, Diag> {
    let sigs = signatures(p, target)?;
    let specs = p.specs.clone();
    for (i, sf) in specs.iter().enumerate() {
        check_spec_fn(sf, &specs[..i], &sigs, target)?;
    }
    for d in &p.structs {
        check_struct_invariants(d, &sigs, &specs, target)?;
    }
    let mut graph = Vec::new();
    for f in &mut p.fns {
        let (calls, sites) = check_fn(f, &sigs, &specs, target).map_err(|d| in_copy(d, &f.name))?;
        graph.push((f.name.clone(), calls, sites));
    }
    lock_order(&graph)?;
    Ok(sigs)
}

/// Rules out deadlock through locks, without annotations. A lock's class is
/// the type of its contents. While `locked(l, f)` holds a lock of class C,
/// `f` and everything it calls may take only locks of other classes, and the
/// classes must form no cycle (C taken before D somewhere and D before C
/// elsewhere). Then every processor takes locks in one global order of
/// classes, and no two can wait for each other.
fn lock_order(graph: &[(String, Vec<String>, Vec<(Pos, String, String)>)]) -> Result<(), Diag> {
    // The classes each function may take, itself or through its callees.
    let mut takes: HashMap<&str, Vec<String>> = HashMap::new();
    for (f, _, sites) in graph {
        let mut v: Vec<String> = Vec::new();
        for (_, c, _) in sites {
            if !v.contains(c) {
                v.push(c.clone());
            }
        }
        takes.insert(f, v);
    }
    let mut changed = true;
    while changed {
        changed = false;
        for (f, calls, _) in graph {
            for g in calls {
                let add: Vec<String> = takes.get(g.as_str()).map_or(Vec::new(), |v| v.clone());
                let mine = takes.get_mut(f.as_str()).unwrap();
                for c in add {
                    if !mine.contains(&c) {
                        mine.push(c);
                        changed = true;
                    }
                }
            }
        }
    }
    // Class C before class D whenever a lock of class D can be taken while
    // one of class C is held.
    let under = |f: &str| takes.get(f).cloned().unwrap_or_default();
    let mut edges: Vec<(String, String)> = Vec::new();
    for (_, _, sites) in graph {
        for (_, c, f) in sites {
            for d in under(f) {
                if !edges.contains(&(c.clone(), d.clone())) {
                    edges.push((c.clone(), d));
                }
            }
        }
    }
    let reaches = |from: &str, to: &str| {
        let mut seen: Vec<&str> = vec![from];
        let mut i = 0;
        while i < seen.len() {
            if seen[i] == to {
                return true;
            }
            for (a, b) in &edges {
                if a == seen[i] && !seen.contains(&b.as_str()) {
                    seen.push(b);
                }
            }
            i += 1;
        }
        false
    };
    for (name, _, sites) in graph {
        for (pos, c, f) in sites {
            for d in under(f) {
                if d == *c {
                    return Err(in_copy(Diag::new("E-LOCK-ORDER", *pos, format!("`{}` may take a lock of class {} while this one, also of class {}, is held", f, c, c))
                        .note("a processor spins forever on a lock it already holds; two locks with the same contents type cannot be nested. Give them different contents types to order them"), name));
                }
                if reaches(&d, c) {
                    return Err(in_copy(Diag::new("E-LOCK-ORDER", *pos, format!("`{}` may take a lock of class {} while this lock of class {} is held, but elsewhere {} is held while {} is taken", f, d, c, d, c))
                        .note("two processors taking them in opposite orders can wait for each other forever; take locks of these classes in one order everywhere"), name));
                }
            }
        }
    }
    Ok(())
}

/// The number of single-byte insertions, deletions and substitutions that
/// turn `a` into `b`.
fn edit_distance(a: &[u8], b: &[u8]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for i in 0..a.len() {
        let mut diag = row[0];
        row[0] = i + 1;
        for j in 0..b.len() {
            let up = row[j + 1];
            let sub = if a[i] == b[j] { diag } else { diag + 1 };
            row[j + 1] = sub.min(up + 1).min(row[j] + 1);
            diag = up;
        }
    }
    row[b.len()]
}

/// Adds `did you mean `x`?` to an error about the unknown name `name`: the
/// closest of `names` (generic instances, whose names have brackets, left
/// out), at most one edit away for a name of up to 4 bytes and two edits
/// otherwise; the first by byte order among equally close ones.
fn did_you_mean<'a>(d: Diag, name: &str, names: impl Iterator<Item = &'a str>) -> Diag {
    let limit = if name.len() <= 4 { 1 } else { 2 };
    let mut best: Option<(usize, &str)> = None;
    for n in names {
        if n.contains('[') {
            continue;
        }
        let k = edit_distance(name.as_bytes(), n.as_bytes());
        if k <= limit && best.map_or(true, |(bk, bn)| k < bk || (k == bk && n < bn)) {
            best = Some((k, n));
        }
    }
    match best {
        Some((_, n)) => d.note(format!("did you mean `{}`?", n)),
        None => d,
    }
}

fn new_ctx<'a>(sigs: &'a HashMap<String, Sig>, specs: &'a [SpecFn], target: Target, ret: Type) -> Ctx<'a> {
    Ctx { sigs, specs, target, ret, scopes: vec![Vec::new()], vars: HashMap::new(), moved: HashSet::new(), moved_at: HashMap::new(), int_ops: HashMap::new(), calls: Vec::new(), lock_sites: Vec::new(), params: Vec::new(), in_inv: std::cell::Cell::new(false), old_only: std::cell::Cell::new(false) }
}

/// What an opaque spec fn can take: values the verifier can hand to an
/// uninterpreted function as a fixed list of terms.
pub fn opaque_arg(t: &Type) -> bool {
    match t {
        Type::Ref(x) => opaque_arg(x),
        Type::Bool | Type::Buf | Type::Bytes => true,
        Type::Struct(n) => crate::ast::struct_def(n).map_or(false, |d| d.fields.iter().all(|(_, ft)| opaque_arg(ft))),
        t => t.is_int(),
    }
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
        if sf.opaque && !opaque_arg(&p.ty) {
            return Err(Diag::new("E-TYPE", p.pos, format!("a parameter of an opaque spec fn is an integer, bool, Buf or Bytes, or a struct of those (or a borrow of one), not {}", p.ty.name())));
        }
        if p.ty.is_linear() || p.ty.is_mut_ref() {
            return Err(Diag::new("E-TYPE", p.pos, "spec fn parameters are values (`u64`, `bool`) or shared borrows (`&T`)"));
        }
        cx.declare(&p.name, p.ty.clone(), false, p.pos)?;
    }
    let t = cx.spec_expr(&sf.body, false, &mut Vec::new())?;
    expect_ty(&t, &sf.ret, sf.body.pos, &format!("the body of spec fn `{}`", sf.name))
}

/// A struct's `invariant` clauses are specifications over its fields.
fn check_struct_invariants(d: &StructDef, sigs: &HashMap<String, Sig>, specs: &[SpecFn], target: Target) -> Result<(), Diag> {
    if d.invariants.is_empty() {
        return Ok(());
    }
    let mut cx = new_ctx(sigs, specs, target, Type::Unit);
    for (f, t) in &d.fields {
        cx.declare(f, t.clone(), false, d.pos)?;
    }
    for i in &d.invariants {
        let t = cx.spec_expr(i, false, &mut Vec::new())?;
        expect_ty(&t, &Type::Bool, i.pos, "a struct invariant")?;
    }
    Ok(())
}

/// Returns the functions `f` calls and its `locked` calls, for `lock_order`.
fn check_fn(f: &mut FnDef, sigs: &HashMap<String, Sig>, specs: &[SpecFn], target: Target) -> Result<(Vec<String>, Vec<(Pos, String, String)>), Diag> {
    let mut cx = new_ctx(sigs, specs, target, f.ret.clone());
    for p in &f.params {
        if p.ty.is_ref() && p.mutable {
            return Err(Diag::new("E-TYPE", p.pos, "a borrow cannot be `mut` (a `&mut` parameter is already assignable)"));
        }
        let mutable = p.mutable || p.ty.is_mut_ref();
        cx.declare(&p.name, p.ty.clone(), mutable, p.pos)?;
        cx.params.push(p.name.clone());
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
    f.int_ops = cx.int_ops;
    Ok((cx.calls, cx.lock_sites))
}

/// Checks that the literal `v` fits in the integer type `t`.
fn lit_fits(v: i128, t: &Type, pos: Pos) -> Result<(), Diag> {
    if v > t.int_max() || v < t.int_min() {
        return Err(Diag::new("E-TYPE", pos, format!("{} does not fit in {}", v, t.name())));
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
        // Functions, spec fns and builtins live apart: a call `f(..)` always
        // means the function, so a local may share its name.
        if self.vars.contains_key(name) {
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
            return Err(self.consumed_note(Diag::new("E-LINEAR-USE", pos, msg), &m));
        }
        if t.is_linear() {
            if matches!(info.ty, Type::Ref(_)) {
                return Err(Diag::new("E-LINEAR-USE", pos, format!("cannot move out of the shared borrow `{}`", path_root(path))));
            }
            if let Some(m) = self.desc_moved(path) {
                return Err(Diag::new("E-LINEAR-USE", pos, format!("`{}` is used after its field `{}` was moved out", path, m)));
            }
            self.moved.insert(path.to_string());
            self.moved_at.insert(path.to_string(), pos);
        }
        Ok(t)
    }

    /// Adds where `m` was consumed to an error about a use after that.
    fn consumed_note(&self, d: Diag, m: &str) -> Diag {
        match self.moved_at.get(m) {
            Some(p) => d.note(format!("`{}` was consumed at {}:{}", m, p.line, p.col)),
            None => d,
        }
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
                let t = match ty {
                    Some(want) => self.expr_as(init, &want.clone())?,
                    None => self.expr(init)?,
                };
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
                let t = self.expr_as(value, &want)?;
                expect_ty(&t, &want, value.pos, &format!("the value assigned to `{}`", name))?;
                if let Some(k) = name.rfind('.') {
                    if let Some(m) = self.prefix_moved(&name[..k]) {
                        return Err(self.consumed_note(Diag::new("E-LINEAR-USE", pos, format!("cannot assign to `{}`: `{}` was consumed", name, m)), &m));
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
                self.in_inv.set(true);
                for inv in invariants.iter() {
                    let t = self.spec_expr(inv, false, &mut Vec::new());
                    let t = t.and_then(|t| expect_ty(&t, &Type::Bool, inv.pos, "a loop invariant"));
                    t.inspect_err(|_| self.in_inv.set(false))?;
                }
                self.in_inv.set(false);
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
                let ret = self.ret.clone();
                let t = match e {
                    Some(e) => self.expr_as(e, &ret)?,
                    None => Type::Unit,
                };
                expect_ty(&t, &ret, pos, "the returned value")?;
                self.leak_check_all(pos, "at this `return`")?;
                Ok(true)
            }
            // Proved unreachable, so nothing owned here can leak.
            StmtKind::Unreachable => Ok(true),
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
                return Err(self.consumed_note(Diag::new("E-LINEAR-USE", scrut.pos, format!("`{}` is used after `{}` was consumed", p, m)), &m));
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
                    if !names_enum(pe, en) {
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
        self.vars.get(name).cloned().ok_or_else(|| {
            did_you_mean(Diag::new("E-TYPE", pos, format!("unknown variable `{}`", name)), name, self.vars.keys().map(|k| k.as_str()))
        })
    }

    /// A code expression in a position that expects `want`: a literal takes
    /// that type when it is an integer type.
    fn expr_as(&mut self, e: &Expr, want: &Type) -> Result<Type, Diag> {
        match int_lit(e) {
            Some(v) if want.is_int() => {
                lit_fits(v, want, e.pos)?;
                Ok(want.clone())
            }
            _ => self.expr(e),
        }
    }

    /// The two operands of a binary operator: a literal on one side takes
    /// the type of the other side.
    fn operands(&mut self, a: &Expr, b: &Expr) -> Result<(Type, Type), Diag> {
        if int_lit(a).is_some() && int_lit(b).is_none() {
            let tb = self.expr(b)?;
            let ta = self.expr_as(a, &tb)?;
            return Ok((ta, tb));
        }
        let ta = self.expr(a)?;
        let tb = self.expr_as(b, &ta)?;
        Ok((ta, tb))
    }

    /// Code expression: linear variables in value position are moved.
    fn expr(&mut self, e: &Expr) -> Result<Type, Diag> {
        match &e.kind {
            ExprKind::TypeApp(targs, inner) => match &inner.kind {
                // `vec_new[T]()`: the only builtin that needs its type written.
                ExprKind::Call(n, args) if n == "vec_new" => {
                    if targs.len() != 1 || !args.is_empty() {
                        return Err(Diag::new("E-TYPE", e.pos, "write `vec_new[T]()`: one type argument, no arguments"));
                    }
                    known_type(&targs[0], e.pos)?;
                    Ok(Type::Vec(Box::new(targs[0].clone())))
                }
                // `cast[T](x)` (proved to fit) and `wrap[T](x)` (modular).
                ExprKind::Call(n, args) if n == "cast" || n == "wrap" => {
                    if targs.len() != 1 || args.len() != 1 {
                        return Err(Diag::new("E-TYPE", e.pos, format!("write `{}[T](x)`: one type argument, one argument", n)));
                    }
                    if !targs[0].is_int() {
                        return Err(Diag::new("E-TYPE", e.pos, format!("`{}` converts to an integer type, not {}", n, targs[0].name())));
                    }
                    let t = self.expr(&args[0])?;
                    if !t.is_int() {
                        return Err(Diag::new("E-TYPE", args[0].pos, format!("`{}` converts an integer, found {}", n, t.name())));
                    }
                    // The type converted from: whether it can be negative
                    // matters to the proof.
                    self.int_ops.insert(e.pos, t);
                    Ok(targs[0].clone())
                }
                // `try_box[T](x)`: `Boxed[T]` from core/boxed.bastio, which
                // monomorphization has already copied for `T`.
                ExprKind::Call(n, args) if n == "try_box" => {
                    if targs.len() != 1 || args.len() != 1 {
                        return Err(Diag::new("E-TYPE", e.pos, "write `try_box[T](x)`: one type argument, one argument"));
                    }
                    known_type(&targs[0], e.pos)?;
                    let t = self.expr_as(&args[0], &targs[0])?;
                    if t != targs[0] {
                        return Err(Diag::new("E-TYPE", args[0].pos, format!("`try_box[{}]` boxes a {}, found {}", targs[0].name(), targs[0].name(), t.name())));
                    }
                    let bt = boxed_type(&targs[0]);
                    let Type::Enum(bn) = &bt else { unreachable!("an enum") };
                    let want = [("Ok", Type::Box(Box::new(t.clone()))), ("Full", t)];
                    let ok = enum_def(bn).is_some_and(|d| d.variants.len() == 2 && d.variants.iter().zip(&want).all(|((v, p), (w, wt))| v == w && p.as_ref() == Some(wt)));
                    if !ok {
                        return Err(Diag::new("E-TYPE", e.pos, "`try_box` returns `enum Boxed[T] { Ok(Box[T]), Full(T) }` from core/boxed.bastio, but `Boxed` here is defined differently"));
                    }
                    Ok(bt)
                }
                _ => unreachable!("removed by monomorphization"),
            },
            ExprKind::Int(_) => Ok(Type::U64),
            ExprKind::Bool(_) => Ok(Type::Bool),
            ExprKind::Str(_) | ExprKind::Blob(_) => Ok(Type::Str),
            ExprKind::MutBorrow(_) => Err(Diag::new("E-TYPE", e.pos, "`&mut x` is only allowed as a call argument")),
            ExprKind::Var(n) => {
                if matches!(self.var_info(n, e.pos)?.ty, Type::Ref(i) if i.is_linear()) {
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
                        let t = self.expr_as(x, pt)?;
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
                            return Err(self.consumed_note(Diag::new("E-LINEAR-USE", x.pos, format!("`{}` is used after `{}` was consumed", p, m)), &m));
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
                let actual = match &t {
                    Type::Enum(a) if names_enum(en, a) => a.clone(),
                    _ => return Err(Diag::new("E-TYPE", x.pos, format!("`is {}::{}` needs a `{}`, found {}", en, v, en, t.name()))),
                };
                if enum_def(&actual).and_then(|d| d.index(v)).is_none() {
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
                    let t = self.expr_as(x, ft)?;
                    expect_ty(&t, ft, x.pos, &format!("field `{}`", f))?;
                }
                Ok(Type::Struct(n.clone()))
            }
            ExprKind::Not(x) => {
                let t = self.expr(x)?;
                // On an unsigned integer, `!` flips every bit.
                if t.is_int() && !t.is_signed() {
                    self.int_ops.insert(e.pos, t.clone());
                    return Ok(t);
                }
                expect_ty(&t, &Type::Bool, x.pos, "the operand of `!`")?;
                Ok(Type::Bool)
            }
            ExprKind::Neg(x) => {
                // A negated literal with nothing to give it a signed type
                // would be a u64.
                let t = if int_lit(e).is_some() { Type::U64 } else { self.expr(x)? };
                if !t.is_signed() {
                    return Err(Diag::new("E-TYPE", e.pos, format!("unary minus needs a signed integer, found {}", t.name())));
                }
                self.int_ops.insert(e.pos, t.clone());
                Ok(t)
            }
            ExprKind::Bin(op, a, b) => {
                if *op == BinOp::Implies {
                    return Err(Diag::new("E-TYPE", e.pos, "`==>` is only allowed in specifications"));
                }
                if matches!(op, BinOp::And | BinOp::Or) {
                    let ta = self.expr(a)?;
                    let before_rhs = self.moved.clone();
                    let tb = self.expr(b)?;
                    if self.moved != before_rhs {
                        return Err(Diag::new("E-LINEAR-USE", b.pos, format!("the right side of `{}` may not run, so it cannot consume a linear value", op.text())));
                    }
                    return bin_type(*op, &ta, &tb, e.pos);
                }
                // A shift amount has its own type: a literal there is a u64.
                let (ta, tb) = if matches!(op, BinOp::Shl | BinOp::Shr) { (self.expr(a)?, self.expr(b)?) } else { self.operands(a, b)? };
                let t = bin_type(*op, &ta, &tb, e.pos)?;
                // The operands' type: the result's for arithmetic, and what a
                // comparison compares (signed or not matters to the C code).
                if ta.is_int() {
                    self.int_ops.insert(e.pos, ta.clone());
                }
                Ok(t)
            }
            ExprKind::Forall(..) => Err(Diag::new("E-TYPE", e.pos, "`forall` is only allowed in specifications")),
            ExprKind::Call(name, args) => {
                if name == "result" || name == "old" || (is_ghost(name) && !self.sigs.contains_key(name)) || self.specs.iter().any(|s| &s.name == name) {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`{}` is only allowed in specifications", name)));
                }
                if VEC_BUILTINS.contains(&name.as_str()) {
                    let sig = self.vec_sig(name, args, e.pos)?;
                    return self.call_with(name, sig, args, e.pos);
                }
                if WRAP_BUILTINS.contains(&name.as_str()) {
                    if args.len() != 2 {
                        return Err(Diag::new("E-TYPE", e.pos, format!("`{}` takes 2 argument(s), {} given", name, args.len())));
                    }
                    let (ta, tb) = self.operands(&args[0], &args[1])?;
                    if !ta.is_int() || ta != tb {
                        return Err(Diag::new("E-TYPE", e.pos, format!("`{}` needs two integers of the same type, found {} and {}", name, ta.name(), tb.name())));
                    }
                    self.int_ops.insert(e.pos, ta.clone());
                    return Ok(ta);
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
                // The verifier assumes `main` starts with every resource
                // (a full pool, the only Serial): a second, nested run of it
                // would hand the same pages to two owners.
                if name == "main" {
                    return Err(Diag::new("E-TYPE", e.pos, "`main` cannot be called"));
                }
                if name == "lock_new" || name == "lock_end" {
                    if args.len() != 1 {
                        return Err(Diag::new("E-TYPE", e.pos, format!("`{}` takes 1 argument(s), {} given", name, args.len())));
                    }
                    let t = self.expr(&args[0])?;
                    if name == "lock_new" {
                        if t.is_pinned() {
                            return Err(Diag::new("E-PIN", args[0].pos, format!("a {} cannot be put in a lock: it holds a capability of this processor", t.name()))
                                .note("`Cpu` and `Mmu` describe the processor that holds them, and a lock can be taken on any processor"));
                        }
                        // The verifier's view of the value does not say
                        // which integer type it has (an i64 reads as a
                        // u64), so the contents' type is recorded here.
                        self.int_ops.insert(e.pos, t.clone());
                        return Ok(Type::Lock(Box::new(t)));
                    }
                    return match t {
                        Type::Lock(i) => Ok(*i),
                        t => Err(Diag::new("E-TYPE", args[0].pos, format!("`lock_end` needs a lock, found {}", t.name()))),
                    };
                }
                if name == "locked" {
                    return self.locked(args, e.pos);
                }
                if ATOMIC_BUILTINS.contains(&name.as_str()) {
                    return self.atomic(name, args, e.pos);
                }
                if name == "smp_run" {
                    return self.smp_run(args, e.pos);
                }
                if PT_BORROW_BUILTINS.contains(&name.as_str()) {
                    return self.pt_borrow(name, args, e.pos);
                }
                let sig = self.sigs.get(name).cloned().ok_or_else(|| {
                    did_you_mean(Diag::new("E-TYPE", e.pos, format!("unknown function `{}`", name)), name, self.sigs.keys().map(|k| k.as_str()))
                })?;
                if !available(name, self.target) {
                    return Err(Diag::new("E-TARGET", e.pos, format!("`{}` is not available on this target", name)));
                }
                self.call_with(name, sig, args, e.pos)
            }
        }
    }

    /// The function a `locked` or `smp_run` argument names.
    fn named_fn(&self, builtin: &str, a: Option<&Expr>, pos: Pos) -> Result<(String, Sig), Diag> {
        let n = match a.map(|a| &a.kind) {
            Some(ExprKind::Var(n)) if n != "main" && !is_builtin(n) && self.sigs.contains_key(n) && !self.vars.contains_key(n) => n.clone(),
            _ => return Err(Diag::new("E-TYPE", a.map_or(pos, |a| a.pos), format!("`{}` needs the name of a function here", builtin))),
        };
        Ok((n.clone(), self.sigs[&n].clone()))
    }

    /// `locked(&l, f, args..)`: takes the lock, calls `f(&mut contents,
    /// args..)` and releases it. The contents are reachable only during the
    /// call, so no other processor can see them half updated.
    fn locked(&mut self, args: &[Expr], pos: Pos) -> Result<Type, Diag> {
        let lt = match args.first().and_then(place) {
            Some(p) => self.path_type(&p, args[0].pos)?.0,
            None => return Err(Diag::new("E-TYPE", args.first().map_or(pos, |a| a.pos), "the first argument of `locked` must name a lock")),
        };
        let Type::Lock(inner) = lt else {
            return Err(Diag::new("E-TYPE", args[0].pos, format!("`locked` needs a Lock, found {}", lt.name())));
        };
        let (f, sig) = self.named_fn("locked", args.get(1), pos)?;
        if sig.params.first() != Some(&m((*inner).clone())) {
            return Err(Diag::new("E-TYPE", args[1].pos, format!("`{}` must take `&mut {}` first, to receive the contents of the lock", f, inner.name())));
        }
        self.lock_sites.push((pos, inner.name(), f.clone()));
        let mut params = vec![r(Type::Lock(inner))];
        params.extend(sig.params[1..].iter().cloned());
        let mut rest = vec![args[0].clone()];
        rest.extend(args[2..].iter().cloned());
        self.call_with(&f, Sig { params, ret: sig.ret }, &rest, pos)
    }

    /// The `Atomic` builtins. `atomic_new` and `atomic_end` move the value in
    /// and out; the others take the atomic as a shared borrow, so any
    /// number of processors can use it at once.
    fn atomic(&mut self, name: &str, args: &[Expr], pos: Pos) -> Result<Type, Diag> {
        let word = |t: &Type, at: Pos| -> Result<(), Diag> {
            if atomic_word(t) {
                return Ok(());
            }
            Err(Diag::new("E-TYPE", at, format!("an Atomic cannot hold a {}", t.name()))
                .note("an Atomic holds one word: an integer, a bool, or a struct with a single field of one of those"))
        };
        if name == "atomic_new" || name == "atomic_end" {
            if args.len() != 1 {
                return Err(Diag::new("E-TYPE", pos, format!("`{}` takes 1 argument(s), {} given", name, args.len())));
            }
            let t = self.expr(&args[0])?;
            if name == "atomic_new" {
                word(&t, args[0].pos)?;
                if t.is_int() {
                    // The verifier's view of the value, which reads every
                    // integer as a u64, does not say which type it has.
                    self.int_ops.insert(pos, t.clone());
                }
                return Ok(Type::Atomic(Box::new(t)));
            }
            return match t {
                Type::Atomic(i) => Ok(*i),
                t => Err(Diag::new("E-TYPE", args[0].pos, format!("`atomic_end` needs an Atomic, found {}", t.name()))),
            };
        }
        let at = match args.first().and_then(place) {
            Some(p) => self.path_type(&p, args[0].pos)?.0,
            None => return Err(Diag::new("E-TYPE", args.first().map_or(pos, |a| a.pos), format!("the first argument of `{}` must name an atomic", name))),
        };
        let Type::Atomic(t) = at.clone() else {
            return Err(Diag::new("E-TYPE", args[0].pos, format!("`{}` needs an Atomic, found {}", name, at.name())));
        };
        word(&t, args[0].pos)?;
        let t = *t;
        let (params, ret) = match name {
            "atomic_load" => (vec![r(at)], t),
            "atomic_store" => (vec![r(at), t], Type::Unit),
            "atomic_swap" => (vec![r(at), t.clone()], t),
            "atomic_cas" => (vec![r(at), t.clone(), t], Type::Bool),
            _ => {
                if !t.is_int() {
                    return Err(Diag::new("E-TYPE", args[0].pos, format!("`atomic_wrap_add` needs an Atomic of an integer type, found {}", at.name())));
                }
                (vec![r(at), t.clone()], t)
            }
        };
        self.call_with(name, Sig { params, ret }, args, pos)
    }

    /// `pt_borrow(&mut t, i, f, args..)`: calls `f(&mut child, args..)` with
    /// the table in slot `i` of `t` left in place (`pt_borrow_frame`: the
    /// page in slot `i` of a level-1 table), so processors running on the
    /// tree keep seeing it. The verifier checks that `f` hands it back.
    fn pt_borrow(&mut self, name: &str, args: &[Expr], pos: Pos) -> Result<Type, Diag> {
        if matches!(self.target, Target::User) {
            return Err(Diag::new("E-TARGET", pos, format!("`{}` is not available on this target", name)));
        }
        let (f, sig) = self.named_fn(name, args.get(2), pos)?;
        let child = if name == "pt_borrow" { Type::PageTable } else { Type::Frame };
        if sig.params.first() != Some(&m(child.clone())) {
            return Err(Diag::new("E-TYPE", args[2].pos, format!("`{}` must take `&mut {}` first, to receive what `{}` lends", f, child.name(), name)));
        }
        let mut params = vec![m(Type::PageTable), Type::U64];
        params.extend(sig.params[1..].iter().cloned());
        let mut rest = vec![args[0].clone(), args[1].clone()];
        rest.extend(args[3..].iter().cloned());
        self.call_with(&f, Sig { params, ret: sig.ret }, &rest, pos)
    }

    /// `smp_run(f, args..)`: runs `f` on every processor at once and returns
    /// when all of them have returned. Each processor's call gets its own
    /// `Cpu` and `Mmu` (this processor's are the ones passed); every other
    /// argument is a shared borrow, read by all of them.
    fn smp_run(&mut self, args: &[Expr], pos: Pos) -> Result<Type, Diag> {
        if !matches!(self.target, Target::Kernel) {
            return Err(Diag::new("E-TARGET", pos, "`smp_run` is not available on this target"));
        }
        let (f, sig) = self.named_fn("smp_run", args.first(), pos)?;
        if sig.ret != Type::Unit {
            return Err(Diag::new("E-TYPE", args[0].pos, format!("`{}` must return nothing to run on every processor", f)));
        }
        for t in &sig.params {
            let ok = match t {
                Type::MutRef(i) => matches!(**i, Type::Cpu | Type::Mmu),
                Type::Ref(i) => !i.is_pinned(),
                _ => false,
            };
            if !ok {
                return Err(Diag::new("E-PIN", args[0].pos, format!("`{}` cannot run on every processor: its parameter of type {} would be shared between them", f, t.name()))
                    .note("each parameter must be `&mut Cpu`, `&mut Mmu` (every processor gets its own) or a shared borrow `&T` of a value with no `Cpu` or `Mmu` in it"));
            }
        }
        self.call_with(&f, sig, &args[1..], pos)
    }

    /// The signature of a `Vec` builtin, for the element type of the vector
    /// its first argument names.
    fn vec_sig(&mut self, name: &str, args: &[Expr], pos: Pos) -> Result<Sig, Diag> {
        if name == "vec_new" {
            return Err(Diag::new("E-TYPE", pos, "write the element type: `vec_new[T]()`"));
        }
        let p = match args.first().map(|a| &a.kind) {
            Some(ExprKind::MutBorrow(n)) => Some(n.clone()),
            Some(_) => place(&args[0]),
            None => None,
        };
        let p = p.ok_or_else(|| Diag::new("E-TYPE", pos, format!("the first argument of `{}` must name a vector", name)))?;
        let t = match self.path_type(&p, pos)?.0 {
            Type::Vec(t) => *t,
            t => return Err(Diag::new("E-TYPE", args[0].pos, format!("`{}` needs a Vec, found {}", name, t.name()))),
        };
        let v = Type::Vec(Box::new(t.clone()));
        let (params, ret) = match name {
            "vec_len" => (vec![r(v)], Type::U64),
            "vec_push" | "vec_push_within" => (vec![m(v), t.clone()], Type::Unit),
            "vec_try_reserve" => (vec![m(v), Type::U64], Type::Bool),
            "vec_pop" => (vec![m(v)], t.clone()),
            "vec_swap" => (vec![m(v), Type::U64, t.clone()], t.clone()),
            "vec_free" => (vec![v], Type::Unit),
            "vec_get" | "vec_set" if t.is_linear() => {
                let (code, why) = if name == "vec_get" { ("E-LINEAR-USE", "would copy") } else { ("E-LINEAR-LEAK", "would drop") };
                return Err(Diag::new(code, pos, format!("`{}` {} a linear {}", name, why, t.name())).note("use `vec_swap` to exchange an element, or `vec_pop`"));
            }
            "vec_get" => (vec![r(v), Type::U64], t.clone()),
            "vec_set" => (vec![m(v), Type::U64, t.clone()], Type::Unit),
            _ => unreachable!("a Vec builtin"),
        };
        Ok(Sig { params, ret })
    }

    /// Checks a call's arguments against `sig`: scalars, then borrows, then moves.
    fn call_with(&mut self, name: &str, sig: Sig, args: &[Expr], pos: Pos) -> Result<Type, Diag> {
        if !is_builtin(name) && !self.calls.iter().any(|c| c == name) {
            self.calls.push(name.to_string());
        }
        if sig.params.len() != args.len() {
            return Err(Diag::new("E-TYPE", pos, format!("`{}` takes {} argument(s), {} given", name, sig.params.len(), args.len())));
        }
        // Scalars first, then borrows, then moves: a scalar argument may
        // read a value that a later argument consumes (`set(b, i, get(b, j))`).
        for (pt, a) in sig.params.iter().zip(args) {
            if !pt.is_linear() && !pt.is_ref() {
                let t = self.expr_as(a, pt)?;
                // A builtin's u64 parameter (an index, a length, a value)
                // takes any narrower unsigned value: widening is free.
                if *pt == Type::U64 && t.is_int() && !t.is_signed() && t != Type::U64 && is_builtin(name) {
                    continue;
                }
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
            if let Some(m) = self.prefix_moved(&n) {
                return Err(self.consumed_note(Diag::new("E-LINEAR-USE", a.pos, format!("`{}` is borrowed after it was consumed", n)), &m));
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

    /// Specification expression: ghost, never consumes.
    fn spec_expr(&self, e: &Expr, in_ensures: bool, bound: &mut Vec<String>) -> Result<Type, Diag> {
        match &e.kind {
            // Only `vec_new[T]()` keeps its type arguments after monomorphization.
            ExprKind::TypeApp(..) => Err(Diag::new("E-TYPE", e.pos, "`vec_new` cannot be called in a specification")),
            ExprKind::Int(_) => Ok(Type::U64),
            ExprKind::Bool(_) => Ok(Type::Bool),
            ExprKind::Str(_) | ExprKind::Blob(_) => Ok(Type::Str),
            ExprKind::MutBorrow(_) => Err(Diag::new("E-TYPE", e.pos, "`&mut` has no meaning in a specification")),
            ExprKind::StructLit(..) | ExprKind::Variant(..) => Err(Diag::new("E-TYPE", e.pos, "struct and enum values cannot be built in a specification")),
            ExprKind::Is(x, en, v) => {
                let t = self.spec_expr(x, in_ensures, bound)?;
                let ok = matches!(&t, Type::Enum(a) if names_enum(en, a) && enum_def(a).and_then(|d| d.index(v)).is_some());
                if !ok {
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
                let t = self.var_info(n, e.pos)?.ty.base().clone();
                if self.old_only.get() && !self.params.contains(n) {
                    return Err(Diag::new("E-TYPE", e.pos, "`old(..)` in a loop invariant can only name the function's parameters"));
                }
                Ok(t)
            }
            ExprKind::Not(x) => {
                let t = self.spec_expr(x, in_ensures, bound)?;
                expect_ty(&t, &Type::Bool, x.pos, "the operand of `!`")?;
                Ok(Type::Bool)
            }
            ExprKind::Neg(x) => {
                let t = self.spec_expr(x, in_ensures, bound)?;
                if !t.is_int() {
                    return Err(Diag::new("E-TYPE", e.pos, format!("unary minus needs an integer, found {}", t.name())));
                }
                Ok(Type::U64)
            }
            ExprKind::Bin(op, a, b) => {
                if op.is_bitwise() {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`{}` is not available in specifications yet", op.text())));
                }
                let ta = spec_norm(self.spec_expr(a, in_ensures, bound)?);
                let tb = spec_norm(self.spec_expr(b, in_ensures, bound)?);
                // Structs and enums made of numbers compare field by field.
                if matches!(op, BinOp::Eq | BinOp::Ne) && ta == tb && matches!(ta, Type::Struct(_) | Type::Enum(_)) {
                    if !spec_plain(&ta) {
                        return Err(Diag::new("E-TYPE", e.pos, format!("cannot compare {} with {}: only structs and enums made of integers and booleans compare", ta.name(), tb.name())));
                    }
                    if !spec_place(a) || !spec_place(b) {
                        return Err(Diag::new("E-TYPE", e.pos, "both sides of a struct or enum comparison must name a value: a variable, a field, `vget(..)` or `old(..)`"));
                    }
                    return Ok(Type::Bool);
                }
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
                    if self.in_inv.get() {
                        self.in_inv.set(false);
                        self.old_only.set(true);
                        let t = self.spec_expr(&args[0], false, bound);
                        self.old_only.set(false);
                        self.in_inv.set(true);
                        return t;
                    }
                    if !in_ensures {
                        return Err(Diag::new("E-TYPE", e.pos, "`old(..)` is only allowed in `ensures` and loop invariants"));
                    }
                    return self.spec_expr(&args[0], false, bound);
                }
                // Arguments that stand for a linear value must name one.
                fn named(a: &Expr) -> bool {
                    match &a.kind {
                        ExprKind::Var(_) | ExprKind::Str(_) | ExprKind::Blob(_) => true,
                        ExprKind::Call(n, xs) if n == "old" => xs.len() == 1 && matches!(xs[0].kind, ExprKind::Var(_)),
                        // An element of a named vector.
                        ExprKind::Call(n, xs) if n == "vget" => xs.len() == 2 && named(&xs[0]),
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
                        let t = spec_norm(self.spec_expr(a, in_ensures, bound)?);
                        if t != spec_norm(p.ty.base().clone()) {
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
                        let ti = spec_norm(self.spec_expr(&args[1], in_ensures, bound)?);
                        expect_ty(&ti, &Type::U64, args[1].pos, "an index")?;
                    }
                    return Ok(g.3.clone());
                }
                if name == "vcount" {
                    // The number of elements equal to a value: integer
                    // elements only, so that equality is plain.
                    arity(2)?;
                    let t = self.spec_expr(&args[0], in_ensures, bound)?;
                    let Type::Vec(el) = &t else {
                        return Err(Diag::new("E-TYPE", args[0].pos, format!("argument of `vcount` must be a Vec, found {}", t.name())));
                    };
                    if !el.is_int() {
                        return Err(Diag::new("E-TYPE", args[0].pos, format!("`vcount` counts the elements of a vector of integers, not of {}", t.name())));
                    }
                    check_lin(&args[0])?;
                    let tx = spec_norm(self.spec_expr(&args[1], in_ensures, bound)?);
                    expect_ty(&tx, &Type::U64, args[1].pos, "the value counted")?;
                    return Ok(Type::U64);
                }
                if name == "bdec" || name == "bhex" {
                    // The decimal (hexadecimal) value of bytes `from..to`,
                    // each read as a digit.
                    arity(3)?;
                    let t = self.spec_expr(&args[0], in_ensures, bound)?;
                    if t != Type::Bytes {
                        return Err(Diag::new("E-TYPE", args[0].pos, format!("argument of `{}` must be Bytes, found {}", name, t.name())));
                    }
                    check_lin(&args[0])?;
                    for (k, what) in [(1, "the start"), (2, "the end")] {
                        let ti = spec_norm(self.spec_expr(&args[k], in_ensures, bound)?);
                        expect_ty(&ti, &Type::U64, args[k].pos, what)?;
                    }
                    return Ok(Type::U64);
                }
                if name == "has_bit" {
                    // Whether bit `i` of an integer is set.
                    arity(2)?;
                    for (k, what) in [(0, "the value"), (1, "the bit number")] {
                        let t = spec_norm(self.spec_expr(&args[k], in_ensures, bound)?);
                        expect_ty(&t, &Type::U64, args[k].pos, what)?;
                    }
                    return Ok(Type::Bool);
                }
                if name == "vlen" || name == "vcap" || name == "vget" {
                    arity(if name == "vget" { 2 } else { 1 })?;
                    let t = self.spec_expr(&args[0], in_ensures, bound)?;
                    let Type::Vec(el) = &t else {
                        return Err(Diag::new("E-TYPE", args[0].pos, format!("argument of `{}` must be a Vec, found {}", name, t.name())));
                    };
                    check_lin(&args[0])?;
                    if name != "vget" {
                        return Ok(Type::U64);
                    }
                    let ti = spec_norm(self.spec_expr(&args[1], in_ensures, bound)?);
                    expect_ty(&ti, &Type::U64, args[1].pos, "an index")?;
                    return Ok((**el).clone());
                }
                if !matches!(name.as_str(), "len" | "get" | "slen" | "sget" | "blen" | "bget") {
                    return Err(Diag::new("E-TYPE", e.pos, format!("`{}` cannot be called in a specification (allowed: len, get, slen, sget, blen, bget, bdec, bhex, has_bit, vlen, vcap, vget, vcount, old, ghost accessors and spec fns)", name)));
                }
                let sig = &self.sigs[name.as_str()];
                arity(sig.params.len())?;
                let tb = self.spec_expr(&args[0], in_ensures, bound)?;
                if tb != *sig.params[0].base() {
                    return Err(Diag::new("E-TYPE", args[0].pos, format!("argument of `{}` must be {}", name, sig.params[0].base().name())));
                }
                check_lin(&args[0])?;
                if args.len() == 2 {
                    let ti = spec_norm(self.spec_expr(&args[1], in_ensures, bound)?);
                    expect_ty(&ti, &Type::U64, args[1].pos, "an index")?;
                }
                Ok(Type::U64)
            }
        }
    }
}

/// Can a specification compare two values of type `t` with `==`: is it an
/// integer, a bool, or a struct or enum made of those?
fn spec_plain(t: &Type) -> bool {
    match t {
        Type::Bool => true,
        t if t.is_int() => true,
        Type::Struct(n) => struct_def(n).is_some_and(|d| d.fields.iter().all(|f| spec_plain(&f.1))),
        Type::Enum(n) => enum_def(n).is_some_and(|d| d.variants.iter().all(|v| v.1.as_ref().is_none_or(spec_plain))),
        _ => false,
    }
}

/// Does a specification expression name a value: a variable, a field of
/// one, `vget(..)` or `old(..)`? (The verifier compares such values part
/// by part.)
pub fn spec_place(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Var(_) | ExprKind::Field(..) => true,
        ExprKind::Call(n, _) => n == "vget" || n == "old",
        _ => false,
    }
}

fn bin_type(op: BinOp, a: &Type, b: &Type, pos: Pos) -> Result<Type, Diag> {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Div | Rem | Lt | Le | Gt | Ge => {
            // Both operands have one integer type; there is no promotion.
            let want = if a.is_int() { a.clone() } else { Type::U64 };
            expect_ty(a, &want, pos, &format!("the left operand of `{}`", op.text()))?;
            expect_ty(b, &want, pos, &format!("the right operand of `{}`", op.text()))?;
            Ok(if matches!(op, Lt | Le | Gt | Ge) { Type::Bool } else { want })
        }
        Eq | Ne => {
            if a != b || !(a.is_int() || *a == Type::Bool) {
                return Err(Diag::new("E-TYPE", pos, format!("cannot compare {} with {}", a.name(), b.name())));
            }
            Ok(Type::Bool)
        }
        And | Or | Implies => {
            expect_ty(a, &Type::Bool, pos, &format!("the left operand of `{}`", op.text()))?;
            expect_ty(b, &Type::Bool, pos, &format!("the right operand of `{}`", op.text()))?;
            Ok(Type::Bool)
        }
        BitAnd | BitOr | BitXor | Shl | Shr => {
            let unsigned = |t: &Type| t.is_int() && !t.is_signed();
            // Both operands have one type, except a shift amount.
            if !unsigned(a) || !unsigned(b) || (a != b && !matches!(op, Shl | Shr)) {
                return Err(Diag::new("E-TYPE", pos, format!("`{}` needs unsigned integers{}, found {} and {}", op.text(), if matches!(op, Shl | Shr) { "" } else { " of one type" }, a.name(), b.name())));
            }
            Ok(a.clone())
        }
    }
}
