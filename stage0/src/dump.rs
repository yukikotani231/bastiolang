// `--dump-ast`: the parse of one file as S-expressions, one definition per
// line, every node with its `line:col`. The self-hosted parser prints the
// same format, so the two can be compared on every source file.
//
// Definitions come grouped (structs, enums, spec fns, fns), each group in
// source order. Constants are folded where they are used, so they do not
// appear themselves. Leading `import` lines are printed and skipped, not
// followed.

use crate::ast::*;
use crate::diag::Pos;

/// Set for `--dump-mono`, whose program spans several files: positions
/// then start with the file's index in load order.
pub static WITH_FILE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn p(pos: Pos) -> String {
    if WITH_FILE.load(std::sync::atomic::Ordering::Relaxed) {
        format!("{}:{}:{}", pos.file, pos.line, pos.col)
    } else {
        format!("{}:{}", pos.line, pos.col)
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn tparams(ps: &[String]) -> String {
    format!("[{}]", ps.join(" "))
}

fn params(ps: &[Param]) -> String {
    let mut s = String::from("(params");
    for x in ps {
        s.push_str(&format!(" (p {} {}{} {})", p(x.pos), if x.mutable { "mut " } else { "" }, x.name, x.ty.name()));
    }
    s.push(')');
    s
}

pub fn expr(e: &Expr) -> String {
    let at = p(e.pos);
    match &e.kind {
        ExprKind::Int(v) => format!("(int {} {})", at, v),
        ExprKind::Bool(b) => format!("(bool {} {})", at, b),
        ExprKind::Str(b) => format!("(str {} {})", at, hex(b)),
        ExprKind::Blob(b) => format!("(blob {} {})", at, b.len()),
        ExprKind::MutBorrow(path) => format!("(mutborrow {} {})", at, path),
        ExprKind::Field(x, f) => format!("(field {} {} {})", at, expr(x), f),
        ExprKind::StructLit(n, fs) => {
            let mut s = format!("(slit {} {}", at, n);
            for (f, x) in fs {
                s.push_str(&format!(" ({} {})", f, expr(x)));
            }
            s.push(')');
            s
        }
        ExprKind::Variant(en, v, x) => match x {
            Some(x) => format!("(variant {} {} {} {})", at, en, v, expr(x)),
            None => format!("(variant {} {} {})", at, en, v),
        },
        ExprKind::Is(x, en, v) => format!("(is {} {} {} {})", at, expr(x), en, v),
        ExprKind::Var(n) => format!("(var {} {})", at, n),
        ExprKind::Not(x) => format!("(not {} {})", at, expr(x)),
        ExprKind::Neg(x) => format!("(neg {} {})", at, expr(x)),
        ExprKind::Bin(op, a, b) => format!("(bin {} {} {} {})", at, op.text(), expr(a), expr(b)),
        ExprKind::Call(n, xs) => {
            let mut s = format!("(call {} {}", at, n);
            for x in xs {
                s.push(' ');
                s.push_str(&expr(x));
            }
            s.push(')');
            s
        }
        ExprKind::Forall(v, x) => format!("(forall {} {} {})", at, v, expr(x)),
        ExprKind::TypeApp(ts, x) => {
            format!("(tapp {} [{}] {})", at, ts.iter().map(|t| t.name()).collect::<Vec<_>>().join(" "), expr(x))
        }
    }
}

fn block(ss: &[Stmt]) -> String {
    let mut s = String::from("(block");
    for x in ss {
        s.push(' ');
        s.push_str(&stmt(x));
    }
    s.push(')');
    s
}

fn stmt(st: &Stmt) -> String {
    let at = p(st.pos);
    match &st.kind {
        StmtKind::Let { name, mutable, ty, init } => format!(
            "(let {} {}{} {} {})",
            at,
            if *mutable { "mut " } else { "" },
            name,
            ty.as_ref().map(|t| t.name()).unwrap_or_else(|| "_".into()),
            expr(init)
        ),
        StmtKind::Assign { name, value } => format!("(assign {} {} {})", at, name, expr(value)),
        StmtKind::If { cond, then_b, else_b } => format!("(if {} {} {} {})", at, expr(cond), block(then_b), block(else_b)),
        StmtKind::While { cond, invariants, body } => {
            let mut s = format!("(while {} {}", at, expr(cond));
            for i in invariants {
                s.push_str(&format!(" (inv {})", expr(i)));
            }
            s.push_str(&format!(" {})", block(body)));
            s
        }
        StmtKind::Return(Some(e)) => format!("(return {} {})", at, expr(e)),
        StmtKind::Return(None) => format!("(return {})", at),
        StmtKind::Unreachable => format!("(unreachable {})", at),
        StmtKind::Assert(e) => format!("(assert {} {})", at, expr(e)),
        StmtKind::Expr(e) => format!("(expr {} {})", at, expr(e)),
        StmtKind::Match { scrut, arms } => {
            let mut s = format!("(match {} {}", at, expr(scrut));
            for a in arms {
                let pat = match &a.pat {
                    Pat::Wild => "_".to_string(),
                    Pat::Variant(en, v, None, _) => format!("{}::{}", en, v),
                    Pat::Variant(en, v, Some(b), m) => format!("{}::{}({}{})", en, v, if *m { "mut " } else { "" }, b),
                };
                s.push_str(&format!(" (arm {} {} {})", p(a.pos), pat, block(&a.body)));
            }
            s.push(')');
            s
        }
    }
}

pub fn program(prog: &Program) -> String {
    let mut out = String::new();
    for d in &prog.structs {
        out.push_str(&format!("(struct {} {} {}", p(d.pos), d.name, tparams(&d.tparams)));
        for (f, t) in &d.fields {
            out.push_str(&format!(" ({} {})", f, t.name()));
        }
        out.push_str(")\n");
    }
    for d in &prog.enums {
        out.push_str(&format!("(enum {} {} {}", p(d.pos), d.name, tparams(&d.tparams)));
        for (v, t) in &d.variants {
            match t {
                Some(t) => out.push_str(&format!(" ({} {})", v, t.name())),
                None => out.push_str(&format!(" ({})", v)),
            }
        }
        out.push_str(")\n");
    }
    for s in &prog.specs {
        out.push_str(&format!("(spec {} {} {} {} {})\n", p(s.pos), s.name, params(&s.params), s.ret.name(), expr(&s.body)));
    }
    for f in &prog.fns {
        out.push_str(&format!("({} {} {} {} {} {}", if f.export { "export-fn" } else { "fn" }, p(f.pos), f.name, tparams(&f.tparams), params(&f.params), f.ret.name()));
        for r in &f.requires {
            out.push_str(&format!(" (requires {})", expr(r)));
        }
        for e in &f.ensures {
            out.push_str(&format!(" (ensures {})", expr(e)));
        }
        out.push_str(&format!(" {})\n", block(&f.body)));
    }
    out
}
