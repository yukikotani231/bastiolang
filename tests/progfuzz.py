#!/usr/bin/env python3
"""Differential fuzzing of the back ends with random whole programs.

usage: tests/progfuzz.py [--seed N] [--count N | --seconds S] [--jobs J]
                         [--bastioc1 PATH | --no-bastioc1] [--keep DIR]

tests/fuzz.py checks one straight-line function at a time. This fuzzer
writes whole programs: a struct, an enum, helper functions (pure ones that
read a borrowed buffer, one that changes a `&mut` scalar and a `&mut Buf`,
one recursive one), and a `main` that uses a `Buf`, `Bytes` and a
`Vec[u64]` through bounded `while` loops, `if`s, `match`es, calls, and
arithmetic on every sized integer type. Most of the arithmetic is total
(`wrap_*`, masks, shifts by constants, division by a constant or by
`x % c + 1`); the rest is ordinary `+ - * / %` under an `if` that bounds
its operands, so the prover has to use the path condition. A quarter of
the programs also contain a few unguarded operations, which the prover
should mostly reject.

Every program prints as it goes (`print`, also from inside helpers called
in the middle of an expression, which checks evaluation order), prints its
buffers at the end, and returns a value. The fuzzer runs it itself, in an
exact reference interpreter, to get the expected output and exit status
and to find any overflow, division by zero or failed bound on that run.

Then:
- bastioc0 --check must not crash. If it accepts a program the interpreter
  found a violation in, the proof is false.
- bastioc1 --check must print what bastioc0 prints (masked as
  stage1/difftest.sh masks it) and exit the same way, and write the same C.
- An accepted program is built five ways: bastioc0 with the C back end and
  the native one, bastioc1 with both, and bastioc0's C under
  AddressSanitizer and UBSan. Each build must print exactly what the
  interpreter printed and exit with its status. A sanitizer report, or any
  difference, is a finding. (The native back end cannot be sanitized; its
  output is compared instead.)

Deterministic for a given seed and --count. Exits 1 if anything was found;
each finding is saved under --keep (default: progfuzz-findings/) with a
note. Compilers and programs that run over the time limit are killed with
their whole process group, so no z3 is left behind.
"""
import argparse, concurrent.futures, math, os, random, re, shutil, signal, subprocess, sys, tempfile, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASTIOC = os.path.abspath(os.environ.get("BASTIOC", os.path.join(ROOT, "stage0/target/release/bastioc0")))
CC = os.environ.get("CC", "cc")
LIMIT = 300          # seconds a compiler may take on one program (Z3's backstop is 60 s a query)
RUN_LIMIT = 10       # seconds a built program may run

TYPES = {"u8": (False, 8), "u16": (False, 16), "u32": (False, 32), "u64": (False, 64),
         "i8": (True, 8), "i16": (True, 16), "i32": (True, 32), "i64": (True, 64)}
UNSIGNED = [t for t in TYPES if not TYPES[t][0]]


def signed(t):
    return TYPES[t][0]


def width(t):
    return TYPES[t][1]


def lo(t):
    return -(1 << (width(t) - 1)) if signed(t) else 0


def hi(t):
    return (1 << (width(t) - 1)) - 1 if signed(t) else (1 << width(t)) - 1


def fits(v, t):
    return lo(t) <= v <= hi(t)


def wrap(v, t):
    v &= (1 << width(t)) - 1
    return v - (1 << width(t)) if signed(t) and v >> (width(t) - 1) else v


def widens(s, t):
    return lo(t) <= lo(s) and hi(s) <= hi(t)


def lit(v):
    return f"-{-v}" if v < 0 else str(v)


def tdiv(a, b):
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b >= 0) else -q


class Violation(Exception):
    pass


class Return(Exception):
    def __init__(self, v):
        self.v = v

# ---------------------------------------------------------------------------
# Expressions are tuples whose last element is their type (conditions have
# none). The leaves of the scope are "places": scalar variables and struct
# fields (`s.f0`), so a field reads and assigns like a variable.
#   ('var', place, t)  ('lit', v, t)
#   ('wop', wrap_add|wrap_sub|wrap_mul, a, b, t)   ('bit', &|^|, a, b, t)
#   ('bnot', a, t)  ('shr', a, k, t)  ('shl', a, k, t) as ((a & mask) << k)
#   ('modl', a, c, t) ('divl', a, c, t): by a positive constant
#   ('divv', a, b, c, t): a / (b % c + 1), unsigned
#   ('conv', wrap|cast, a, t): cast only where it widens
#   ('get', buf, i, u64) ('bget', buf, i, u8) ('vget', i, u64): index i % length
#   ('call', fn, args, t): args are ('val', e) | ('struct', name) | ('buf', name)
#   ('raw', op, a, b, t): plain + - * / %, safe only if something proves it
# Conditions: ('cmp', op, a, b) ('and', c, d) ('or', c, d) ('not', c)
#   ('is', variant) ('true',)


def ty(e):
    return e[-1]


def show(e):
    k = e[0]
    if k == "var":
        return e[1]
    if k == "lit":
        return lit(e[1])
    if k == "wop":
        return f"{e[1]}({show(e[2])}, {show(e[3])})"
    if k == "bit":
        return f"({show(e[2])} {e[1]} {show(e[3])})"
    if k == "bnot":
        return f"(!{show(e[1])})"
    if k == "shr":
        return f"({show(e[1])} >> {e[2]})"
    if k == "shl":
        return f"(({show(e[1])} & {(1 << (width(e[3]) - e[2])) - 1}) << {e[2]})"
    if k == "modl":
        return f"({show(e[1])} % {e[2]})"
    if k == "divl":
        return f"({show(e[1])} / {e[2]})"
    if k == "divv":
        return f"({show(e[1])} / ({show(e[2])} % {e[3]} + 1))"
    if k == "conv":
        return f"{e[1]}[{e[3]}]({show(e[2])})"
    if k == "get":
        return f"get({e[1]}, {show(e[2])} % len({e[1]}))"
    if k == "bget":
        return f"bget({e[1]}, {show(e[2])} % blen({e[1]}))"
    if k == "vget":
        return f"vec_get(v, {show(e[1])} % vec_len(v))"
    if k == "call":
        return f"{e[1]}({', '.join(show(a[1]) if a[0] == 'val' else a[1] for a in e[2])})"
    if k == "raw":
        return f"({show(e[2])} {e[1]} {show(e[3])})"
    if k == "cmp":
        return f"{show(e[2])} {e[1]} {show(e[3])}"
    if k == "and":
        return f"({show(e[1])}) && ({show(e[2])})"
    if k == "or":
        return f"({show(e[1])}) || ({show(e[2])})"
    if k == "not":
        return f"!({show(e[1])})"
    if k == "is":
        return f"e is E::{e[1]}"
    return k   # true

# ---------------------------------------------------------------------------
# Programs


class Fn:
    def __init__(self, name, kind, params, ret, reqs, body, result, ens=None):
        # params: [(name, 'val'|'mut'|'struct'|'buf'|'mutbuf', t)]
        self.name, self.kind, self.params, self.ret = name, kind, params, ret
        self.reqs, self.body, self.result, self.ens = reqs, body, result, ens


class Ctx:
    """What a piece of code can see while it is generated."""

    def __init__(self, scope, mutable, bufs, calls, main, loops):
        self.scope = scope        # place -> type
        self.mutable = mutable    # places that can be assigned
        self.bufs = bufs          # {'b': 'Buf'|'Bytes'|'Vec', ...} readable here
        self.calls = calls        # helpers this code may call
        self.main = main
        self.loops = loops        # loop depth still allowed
        self.noread = set()       # buffers an argument may not read (moved or lent in the same call)

    def child(self):
        c = Ctx(dict(self.scope), list(self.mutable), self.bufs, self.calls, self.main, self.loops)
        c.noread = self.noread
        return c


class Gen:
    def __init__(self, rnd):
        self.r = rnd
        self.n = 0
        self.raw_rate = 0.04 if rnd.random() < 0.25 else 0.0

    def fresh(self, p):
        self.n += 1
        return f"{p}{self.n}"

    def value(self, t, near=None):
        r = self.r.random()
        if r < 0.25:
            v = self.r.choice([lo(t), hi(t), 0, 1, lo(t) + 1, hi(t) - 1, -1, 2, 7])
        elif r < 0.85:
            v = self.r.randint(-40, 300)
        else:
            v = self.r.randint(lo(t), hi(t))
        return min(max(v, lo(t) + (1 if signed(t) else 0)), hi(t))

    def literal(self, t):
        return ("lit", self.value(t), t)

    def place(self, t, ctx):
        same = [p for p in ctx.scope if ctx.scope[p] == t]
        if same and self.r.random() < 0.7:
            return ("var", self.r.choice(same), t)
        p = self.r.choice(list(ctx.scope))
        return self.convert(("var", p, ctx.scope[p]), t)

    def convert(self, e, t):
        s = ty(e)
        if s == t:
            return e
        if widens(s, t) and self.r.random() < 0.5:
            return ("conv", "cast", e, t)
        return ("conv", "wrap", e, t)

    def index(self, ctx, depth):
        return self.expr("u64", ctx, depth)

    def readable(self, ctx, kind):
        return [b for b, k in ctx.bufs.items() if k == kind and b not in ctx.noread]

    def expr(self, t, ctx, depth):
        """An expression of type t mentioning a place, so its literals are typed."""
        if depth <= 0 or self.r.random() < 0.2:
            return self.place(t, ctx)
        if self.raw_rate and self.r.random() < self.raw_rate:
            op = self.r.choice(["+", "-", "*", "/", "%"])
            a = self.expr(t, ctx, depth - 1)
            b = self.literal(t) if self.r.random() < 0.5 else self.expr(t, ctx, depth - 1)
            return ("raw", op, a, b, t)
        u = not signed(t)
        choices = ["wop", "wop", "modl", "divl", "conv", "conv"]
        if u:
            choices += ["bit", "bit", "bnot", "shr", "shl", "divv"]
        if self.readable(ctx, "Buf"):
            choices += ["get"]
        if self.readable(ctx, "Bytes"):
            choices += ["bget"]
        if self.readable(ctx, "Vec"):
            choices += ["vget"]
        calls = self.callable(ctx)
        if calls:
            choices += ["call", "call"]
        k = self.r.choice(choices)
        if k == "wop":
            a = self.expr(t, ctx, depth - 1)
            b = self.literal(t) if self.r.random() < 0.4 else self.expr(t, ctx, depth - 1)
            if self.r.random() < 0.5:
                a, b = b, a
            return ("wop", self.r.choice(["wrap_add", "wrap_sub", "wrap_mul"]), a, b, t)
        if k == "bit":
            a = self.expr(t, ctx, depth - 1)
            b = self.literal(t) if self.r.random() < 0.4 else self.expr(t, ctx, depth - 1)
            return ("bit", self.r.choice(["&", "|", "^"]), a, b, t)
        if k == "bnot":
            return ("bnot", self.expr(t, ctx, depth - 1), t)
        if k == "shr":
            return ("shr", self.expr(t, ctx, depth - 1), self.r.randint(0, width(t) - 1), t)
        if k == "shl":
            return ("shl", self.expr(t, ctx, depth - 1), self.r.randint(0, width(t) - 1), t)
        if k == "modl":
            return ("modl", self.expr(t, ctx, depth - 1), self.r.choice([c for c in (1, 2, 3, 7, 10, 16, 255, 1000, hi(t)) if c <= hi(t)]), t)
        if k == "divl":
            return ("divl", self.expr(t, ctx, depth - 1), self.r.choice([c for c in (1, 2, 3, 5, 16, 100, hi(t)) if c <= hi(t)]), t)
        if k == "divv":
            return ("divv", self.expr(t, ctx, depth - 1), self.expr(t, ctx, depth - 1),
                    self.r.choice([1, 2, 9, 100, hi(t) - 1]), t)
        if k == "conv":
            return self.convert(self.expr(self.r.choice(list(TYPES)), ctx, depth - 1), t)
        if k == "get":
            return self.convert(("get", self.r.choice(self.readable(ctx, "Buf")), self.index(ctx, depth - 1), "u64"), t)
        if k == "bget":
            return self.convert(("bget", self.r.choice(self.readable(ctx, "Bytes")), self.index(ctx, depth - 1), "u8"), t)
        if k == "vget":
            return self.convert(("vget", self.index(ctx, depth - 1), "u64"), t)
        f = self.r.choice(calls)
        return self.convert(("call", f.name, self.args(f, ctx, depth - 1), f.ret), t)

    def callable(self, ctx):
        """The helpers this code can call: one taking the struct needs one to
        pass, one taking the buffer needs it readable."""
        buf = "b" if ctx.main else "hb"
        out = []
        for f in ctx.calls:
            if f.kind == "mut":
                continue
            kinds = [p[1] for p in f.params]
            if "buf" in kinds and (buf not in ctx.bufs or buf in ctx.noread):
                continue
            if "struct" in kinds and not ("s.f0" in ctx.scope if ctx.main else "q.f0" in ctx.scope):
                continue
            out.append(f)
        return out

    def args(self, f, ctx, depth):
        out = []
        for name, kind, t in f.params:
            if kind == "buf":
                out.append(("buf", "b" if ctx.main else "hb"))
            elif kind == "struct":
                out.append(("struct", "s" if ctx.main else "q"))
            else:
                e = self.expr(t, ctx, depth)
                bound = next((c[3][1] for c in f.reqs if c[0] == "cmp" and c[2][1] == name), None)
                if bound is not None:
                    e = ("modl", e, bound + 1, t)
                out.append(("val", e))
        return out

    def cond(self, ctx, depth=1):
        r = self.r.random()
        if depth > 0 and r < 0.2:
            return (self.r.choice(["and", "or"]), self.cond(ctx, depth - 1), self.cond(ctx, depth - 1))
        if depth > 0 and r < 0.27:
            return ("not", self.cond(ctx, depth - 1))
        if ctx.main and r < 0.35:
            return ("is", self.r.choice(["A", "B", "C"]))
        t = self.r.choice(list(TYPES))
        a = self.expr(t, ctx, 1)
        b = self.literal(t) if self.r.random() < 0.6 else self.expr(t, ctx, 1)
        return ("cmp", self.r.choice(["==", "!=", "<", "<=", ">", ">="]), a, b)

    def guarded(self, ctx):
        """`if <bounds> { x = a op b; }`: ordinary arithmetic the prover can
        prove only from the condition."""
        x = self.r.choice(ctx.mutable)
        t = ctx.scope[x]
        vs = [p for p in ctx.scope if ctx.scope[p] == t]
        a = ("var", self.r.choice(vs), t)
        b = ("var", self.r.choice(vs), t) if self.r.random() < 0.6 else None
        op = self.r.choice(["+", "-", "*", "/", "%"])
        H, R = hi(t) // 2, math.isqrt(hi(t))
        conds = []

        def window(e, m):
            conds.append(("cmp", "<=", e, ("lit", m, t)))
            if signed(t):
                conds.append(("cmp", ">=", e, ("lit", -m, t)))
        if op in "+*":
            m = H if op == "+" else R
            window(a, m)
            if b is None:
                b = ("lit", self.r.randint(-m if signed(t) else 0, m), t)
            else:
                window(b, m)
        elif op == "-":
            if signed(t):
                window(a, H)
                if b is None:
                    b = ("lit", self.r.randint(-H, H), t)
                else:
                    window(b, H)
            else:
                if b is None:
                    b = ("lit", self.r.randint(0, 50), t)
                conds.append(("cmp", ">=", a, b))
        else:
            if b is None:
                b = ("lit", self.r.randint(1, min(hi(t), 300)), t)
            else:
                conds.append(("cmp", ">", b, ("lit", 0, t)))
        c = conds[0] if conds else ("true",)
        for d in conds[1:]:
            c = ("and", c, d)
        if self.r.random() < 0.3:
            c = ("and", c, self.cond(ctx, 0))
        return ("if", c, [("assign", x, ("raw", op, a, b, t))], [])

    def block(self, ctx, depth, n, kinds):
        out = []
        for _ in range(n):
            r = self.r.random()
            if r < 0.22:
                t = self.r.choice(list(TYPES))
                name = self.fresh("x")
                mut = self.r.random() < 0.5
                out.append(("let", name, t, self.expr(t, ctx, self.r.randint(1, 3)), mut))
                ctx.scope[name] = t
                if mut:
                    ctx.mutable.append(name)
            elif r < 0.37 and ctx.mutable:
                p = self.r.choice(ctx.mutable)
                out.append(("assign", p, self.expr(ctx.scope[p], ctx, self.r.randint(1, 3))))
            elif r < 0.45 and ctx.mutable:
                out.append(self.guarded(ctx))
            elif r < 0.53 and depth > 0:
                out.append(("if", self.cond(ctx), self.block(ctx.child(), depth - 1, self.r.randint(1, 3), kinds),
                            self.block(ctx.child(), depth - 1, self.r.randint(0, 2), kinds)))
            elif r < 0.6 and depth > 0 and ctx.loops > 0:
                i = self.fresh("i")
                inner = ctx.child()
                inner.loops -= 1
                inner.scope[i] = "u64"
                out.append(("while", i, self.r.randint(0, 6), self.block(inner, depth - 1, self.r.randint(1, 4), kinds)))
            elif r < 0.67:
                t = self.r.choice(list(TYPES))
                out.append(("print", self.expr(t, ctx, self.r.randint(1, 2))))
            elif r < 0.72 and "set" in kinds:
                b = "b" if ctx.main else "hb"
                ctx.noread = {b}
                out.append(("set", b, self.index(ctx, 1), self.expr("u64", ctx, 2)))
                ctx.noread = set()
            elif r < 0.76 and "bytes" in kinds:
                ctx.noread = {"by"}
                out.append(("bset", self.index(ctx, 1), self.expr("u8", ctx, 2)))
                ctx.noread = set()
            elif r < 0.82 and "vec" in kinds:
                ctx.noread = {"v"}
                if self.r.random() < 0.6:
                    out.append(("push", self.expr("u64", ctx, 2)))
                else:
                    out.append(("vset", self.index(ctx, 1), self.expr("u64", ctx, 2)))
                ctx.noread = set()
            elif r < 0.87 and "mcall" in kinds and self.mut_fn:
                f = self.mut_fn
                t = f.params[0][2]
                ps = [p for p in ctx.mutable if ctx.scope[p] == t]
                if ps:
                    p = self.r.choice(ps)
                    sub = ctx.child()
                    sub.scope = {q: u for q, u in ctx.scope.items() if q != p and q.split(".")[0] != p.split(".")[0]}
                    sub.noread = {"b"}
                    if p.startswith("s."):
                        sub.calls = [g for g in ctx.calls if all(q[1] != "struct" for q in g.params)]
                    if sub.scope:
                        out.append(("mcall", f.name, p, self.expr("u64", sub, 2)))
            elif r < 0.93 and "enum" in kinds:
                if self.r.random() < 0.5:
                    variant = self.r.choice(["A", "B", "C"])
                    pay = None if variant == "B" else self.expr(self.enum_a if variant == "A" else "u64", ctx, 2)
                    out.append(("eassign", variant, pay))
                elif depth > 0:
                    arms = []
                    for variant in self.r.sample(["A", "B", "C"], self.r.randint(1, 3)):
                        sub = ctx.child()
                        z = None
                        if variant != "B" and self.r.random() < 0.8:
                            z = self.fresh("z")
                            sub.scope[z] = self.enum_a if variant == "A" else "u64"
                        arms.append((variant, z, self.block(sub, depth - 1, self.r.randint(0, 2), kinds)))
                    wild = None
                    if len(arms) < 3:
                        wild = self.block(ctx.child(), depth - 1, self.r.randint(0, 2), kinds)
                    out.append(("match", arms, wild))
        return out

    def program(self):
        self.n = 0
        self.nb = self.r.randint(1, 6)
        self.ny = self.r.randint(1, 6)
        self.fields = [self.r.choice(list(TYPES)) for _ in range(2)]
        self.enum_a = self.r.choice(list(TYPES))
        fns = []
        # Pure helpers: scalars, sometimes the struct and a borrowed Buf.
        for _ in range(self.r.randint(0, 3)):
            name = self.fresh("h")
            params, reqs, scope = [], [], {}
            for _ in range(self.r.randint(1, 3)):
                p, t = self.fresh("p"), self.r.choice(list(TYPES))
                params.append((p, "val", t))
                scope[p] = t
                if not signed(t) and self.r.random() < 0.4:
                    reqs.append(("cmp", "<=", ("var", p, t), ("lit", self.r.choice([0, 1, 9, 100, hi(t) // 2]), t)))
            bufs = {}
            if self.r.random() < 0.4:
                params.append(("q", "struct", "S"))
                scope["q.f0"], scope["q.f1"] = self.fields
            if self.r.random() < 0.5:
                params.append(("hb", "buf", "Buf"))
                reqs.append(("cmp", "==", ("len", "hb"), ("lit", self.nb, "u64")))
                bufs["hb"] = "Buf"
            ctx = Ctx(scope, [], bufs, list(fns), False, 1)
            body = self.block(ctx, 2, self.r.randint(0, 4), set())
            ret = self.r.choice(list(TYPES))
            fns.append(Fn(name, "pure", params, ret, reqs, body, self.expr(ret, ctx, 3)))
        # A recursive helper.
        if self.r.random() < 0.5:
            name, t, d = self.fresh("r"), self.r.choice(list(TYPES)), self.r.randint(0, 12)
            n, a = self.fresh("n"), self.fresh("a")
            ctx = Ctx({n: "u64", a: t}, [], {}, list(fns), False, 0)
            base = self.expr(t, ctx, 2)
            pre = self.block(ctx, 1, self.r.randint(0, 2), set())
            step = self.expr(t, ctx, 2)
            reqs = [("cmp", "<=", ("var", n, "u64"), ("lit", d, "u64"))]
            f = Fn(name, "rec", [(n, "val", "u64"), (a, "val", t)], t, reqs, None, None)
            f.base, f.pre, f.step = base, pre, step
            fns.append(f)
        # A helper that changes a scalar and a buffer it borrows mutably.
        self.mut_fn = None
        if self.r.random() < 0.6:
            name, t = self.fresh("m"), self.r.choice(list(TYPES))
            x, k = self.fresh("x"), self.fresh("k")
            ctx = Ctx({x: t, k: "u64"}, [x], {"hb": "Buf"}, list(fns), False, 1)
            body = self.block(ctx, 2, self.r.randint(1, 4), {"set"})
            reqs = [("cmp", "==", ("len", "hb"), ("lit", self.nb, "u64"))]
            self.mut_fn = Fn(name, "mut", [(x, "mut", t), ("hb", "mutbuf", "Buf"), (k, "val", "u64")], None, reqs, body, None, reqs)
            fns.append(self.mut_fn)
        scope = {"acc": "u64", "s.f0": self.fields[0], "s.f1": self.fields[1]}
        ctx = Ctx(scope, ["acc", "s.f0", "s.f1"], {"b": "Buf", "by": "Bytes", "v": "Vec"}, fns, True, 2)
        self.v0 = [self.value("u64") & 0xffff for _ in range(self.r.randint(1, 3))]
        self.acc0 = self.value("u64")
        self.s0 = [self.literal(t) for t in self.fields]
        body = self.block(ctx, 3, self.r.randint(3, 12), {"set", "bytes", "vec", "mcall", "enum"})
        result = self.expr("u64", ctx, 3)
        tops = [p for p in ctx.scope]
        return Prog(self, fns, body, result, tops)


class Prog:
    def __init__(self, g, fns, body, result, tops):
        self.fns, self.body, self.result, self.tops = fns, body, result, tops
        self.nb, self.ny, self.fields, self.enum_a = g.nb, g.ny, g.fields, g.enum_a
        self.v0, self.acc0, self.s0 = g.v0, g.acc0, g.s0
        self.byname = {f.name: f for f in fns}

    # -- source ---------------------------------------------------------------

    def spec(self, c):
        if c[0] == "cmp" and c[2][0] == "len":
            return f"len({c[2][1]}) {c[1]} {c[3][1]}"
        return show(c)

    def source(self):
        out = [f"struct S {{ f0: {self.fields[0]}, f1: {self.fields[1]} }}",
               f"enum E {{ A({self.enum_a}), B, C(u64) }}", ""]
        for f in self.fns:
            ps = []
            for name, kind, t in f.params:
                ps.append(f"{name}: " + {"val": t, "mut": f"&mut {t}", "struct": "S", "buf": "&Buf", "mutbuf": "&mut Buf"}[kind])
            out.append(f"fn {f.name}({', '.join(ps)})" + (f" -> {f.ret}" if f.ret else ""))
            for c in f.reqs:
                out.append(f"  requires {self.spec(c)}")
            for c in f.ens or []:
                out.append(f"  ensures {self.spec(c)}")
            out.append("{")
            inv = ["len(hb) == %d" % self.nb] if any(p[1] in ("buf", "mutbuf") for p in f.params) else []
            if f.kind == "rec":
                n, a = f.params[0][0], f.params[1][0]
                out.append(f"  if {n} == 0 {{")
                out.append(f"    return {show(f.base)};")
                out.append("  }")
                self.stmts(f.pre, out, "  ", inv)
                out.append(f"  return {f.name}({n} - 1, {show(f.step)});")
            else:
                self.stmts(f.body, out, "  ", inv)
                if f.ret:
                    out.append(f"  return {show(f.result)};")
            out.append("}")
            out.append("")
        inv = [f"len(b) == {self.nb}", f"blen(by) == {self.ny}", "vlen(v) >= 1"]
        out.append("fn main() -> u64 {")
        out.append(f"  let mut b = alloc({self.nb});")
        out.append(f"  let mut by = balloc({self.ny});")
        out.append("  let mut v = vec_new[u64]();")
        for x in self.v0:
            out.append(f"  vec_push(&mut v, {x});")
        out.append(f"  let mut acc: u64 = {self.acc0};")
        out.append(f"  let mut s = S {{ f0: {show(self.s0[0])}, f1: {show(self.s0[1])} }};")
        out.append("  let mut e = E::B;")
        self.stmts(self.body, out, "  ", inv)
        for p in self.tops:
            out.append(f"  print({self.u64(p)});")
        out.append("  let mut k: u64 = 0;")
        out.append(f"  while k < len(b) invariant k <= len(b) {{ print(get(b, k)); k = k + 1; }}")
        out.append("  k = 0;")
        out.append(f"  while k < blen(by) invariant k <= blen(by) {{ print(cast[u64](bget(by, k))); k = k + 1; }}")
        out.append("  k = 0;")
        out.append(f"  while k < vec_len(v) invariant k <= vlen(v) {{ print(vec_get(v, k)); k = k + 1; }}")
        out.append(f"  let r: u64 = {show(self.result)};")
        out.append("  free(b);")
        out.append("  bfree(by);")
        out.append("  vec_free(v);")
        out.append("  return r;")
        out.append("}")
        return "\n".join(out) + "\n"

    def u64(self, p):
        return p if self.typeof(p) == "u64" else f"wrap[u64]({p})"

    def typeof(self, p):
        for s in self.walk(self.body):
            if s[0] == "let" and s[1] == p:
                return s[2]
        return {"acc": "u64", "s.f0": self.fields[0], "s.f1": self.fields[1]}[p]

    def walk(self, ss):
        for s in ss:
            yield s
            if s[0] == "if":
                yield from self.walk(s[2])
                yield from self.walk(s[3])
            elif s[0] == "while":
                yield from self.walk(s[3])
            elif s[0] == "match":
                for _, _, b in s[1]:
                    yield from self.walk(b)
                yield from self.walk(s[2] or [])

    def stmts(self, ss, out, ind, inv):
        for s in ss:
            k = s[0]
            if k == "let":
                out.append(f"{ind}let {'mut ' if s[4] else ''}{s[1]}: {s[2]} = {show(s[3])};")
            elif k == "assign":
                out.append(f"{ind}{s[1]} = {show(s[2])};")
            elif k == "if":
                out.append(f"{ind}if {show(s[1])} {{")
                self.stmts(s[2], out, ind + "  ", inv)
                if s[3]:
                    out.append(f"{ind}}} else {{")
                    self.stmts(s[3], out, ind + "  ", inv)
                out.append(f"{ind}}}")
            elif k == "while":
                i, n = s[1], s[2]
                out.append(f"{ind}let mut {i}: u64 = 0;")
                out.append(f"{ind}while {i} < {n}")
                for c in [f"{i} <= {n}"] + inv:
                    out.append(f"{ind}  invariant {c}")
                out.append(f"{ind}{{")
                self.stmts(s[3], out, ind + "  ", inv)
                out.append(f"{ind}  {i} = {i} + 1;")
                out.append(f"{ind}}}")
            elif k == "print":
                e = s[1]
                out.append(f"{ind}print({show(e if ty(e) == 'u64' else ('conv', 'wrap', e, 'u64'))});")
            elif k == "set":
                out.append(f"{ind}{s[1]} = set({s[1]}, {show(s[2])} % len({s[1]}), {show(s[3])});")
            elif k == "bset":
                out.append(f"{ind}bset(&mut by, {show(s[1])} % blen(by), {show(s[2])});")
            elif k == "push":
                out.append(f"{ind}vec_push(&mut v, {show(s[1])});")
            elif k == "vset":
                out.append(f"{ind}vec_set(&mut v, {show(s[1])} % vec_len(v), {show(s[2])});")
            elif k == "mcall":
                out.append(f"{ind}{s[1]}(&mut {s[2]}, &mut b, {show(s[3])});")
            elif k == "eassign":
                out.append(f"{ind}e = E::{s[1]}" + (f"({show(s[2])});" if s[2] else ";"))
            elif k == "match":
                out.append(f"{ind}match e {{")
                for variant, z, b in s[1]:
                    pat = f"E::{variant}" + ("" if variant == "B" else f"({z or '_'})")
                    out.append(f"{ind}  {pat} => {{")
                    self.stmts(b, out, ind + "    ", inv)
                    out.append(f"{ind}  }},")
                if s[2] is not None:
                    out.append(f"{ind}  _ => {{")
                    self.stmts(s[2], out, ind + "    ", inv)
                    out.append(f"{ind}  }},")
                out.append(f"{ind}}}")

    # -- reference interpreter -----------------------------------------------

    def run(self):
        """(stdout, exit status) of the program, or raises Violation."""
        self.out = []
        self.steps = 0
        env = {"b": [0] * self.nb, "by": [0] * self.ny, "v": list(self.v0), "acc": self.acc0,
               "s.f0": self.s0[0][1], "s.f1": self.s0[1][1], "e": ("B", None)}
        self.exec(self.body, env)
        for p in self.tops:
            self.out.append(wrap(env[p], "u64"))
        self.out += env["b"] + env["by"] + env["v"]
        r = self.ev(self.result, env)
        return "".join(f"{x}\n" for x in self.out).encode(), r & 255

    def call(self, f, args, env):
        self.steps += 1
        if self.steps > 200000:
            raise Violation("runs too long")
        fe = {}
        for (name, kind, t), a in zip(f.params, args):
            if a[0] == "buf":
                fe[name] = env[a[1]]
            elif a[0] == "struct":
                fe[name + ".f0"], fe[name + ".f1"] = env[a[1] + ".f0"], env[a[1] + ".f1"]
            else:
                fe[name] = a[1]
        for c in f.reqs:
            if not self.cond(c, fe):
                raise Violation(f"{f.name}: requires {self.spec(c)} is false")
        if f.kind == "rec":
            n = f.params[0][0]
            if fe[n] == 0:
                return self.ev(f.base, fe)
            self.exec(f.pre, fe)
            step = self.ev(f.step, fe)
            return self.call(f, [("val", fe[n] - 1), ("val", step)], env)
        self.exec(f.body, fe)
        return self.ev(f.result, fe) if f.ret else fe

    def exec(self, ss, env):
        for s in ss:
            k = s[0]
            if k == "let":
                env[s[1]] = self.ev(s[3], env)
            elif k == "assign":
                env[s[1]] = self.ev(s[2], env)
            elif k == "if":
                self.exec(s[2] if self.cond(s[1], env) else s[3], env)
            elif k == "while":
                env[s[1]] = 0
                while env[s[1]] < s[2]:
                    self.exec(s[3], env)
                    env[s[1]] += 1
            elif k == "print":
                self.out.append(wrap(self.ev(s[1], env), "u64"))
            elif k == "set":
                buf = env[s[1]]
                i = self.ev(s[2], env) % len(buf)
                buf[i] = self.ev(s[3], env)
            elif k == "bset":
                i = self.ev(s[1], env) % len(env["by"])
                env["by"][i] = self.ev(s[2], env)
            elif k == "push":
                env["v"].append(self.ev(s[1], env))
            elif k == "vset":
                i = self.ev(s[1], env) % len(env["v"])
                env["v"][i] = self.ev(s[2], env)
            elif k == "mcall":
                f = self.byname[s[1]]
                kv = self.ev(s[3], env)
                fe = self.call(f, [("val", env[s[2]]), ("buf", "b"), ("val", kv)], env)
                env[s[2]] = fe[f.params[0][0]]
            elif k == "eassign":
                env["e"] = (s[1], None if s[2] is None else self.ev(s[2], env))
            elif k == "match":
                variant, pay = env["e"]
                for av, z, b in s[1]:
                    if av == variant:
                        if z:
                            env[z] = pay
                        self.exec(b, env)
                        break
                else:
                    self.exec(s[2], env)

    def cond(self, c, env):
        k = c[0]
        if k == "cmp":
            if c[2][0] == "len":
                return len(env[c[2][1]]) == c[3][1]
            a, b = self.ev(c[2], env), self.ev(c[3], env)
            return {"==": a == b, "!=": a != b, "<": a < b, "<=": a <= b, ">": a > b, ">=": a >= b}[c[1]]
        if k == "and":
            return self.cond(c[1], env) and self.cond(c[2], env)
        if k == "or":
            return self.cond(c[1], env) or self.cond(c[2], env)
        if k == "not":
            return not self.cond(c[1], env)
        if k == "is":
            return env["e"][0] == c[1]
        return True

    def ev(self, e, env):
        k, t = e[0], e[-1]
        if k == "var":
            return env[e[1]]
        if k == "lit":
            return e[1]
        if k == "wop":
            a, b = self.ev(e[2], env), self.ev(e[3], env)
            return wrap({"wrap_add": a + b, "wrap_sub": a - b, "wrap_mul": a * b}[e[1]], t)
        if k == "bit":
            a, b = self.ev(e[2], env), self.ev(e[3], env)
            return {"&": a & b, "|": a | b, "^": a ^ b}[e[1]]
        if k == "bnot":
            return hi(t) ^ self.ev(e[1], env)
        if k == "shr":
            return self.ev(e[1], env) >> e[2]
        if k == "shl":
            return (self.ev(e[1], env) & ((1 << (width(t) - e[2])) - 1)) << e[2]
        if k == "modl":
            a = self.ev(e[1], env)
            return a - e[2] * tdiv(a, e[2])
        if k == "divl":
            return tdiv(self.ev(e[1], env), e[2])
        if k == "divv":
            a, b = self.ev(e[1], env), self.ev(e[2], env)
            return a // (b % e[3] + 1)
        if k == "conv":
            v = self.ev(e[2], env)
            if e[1] == "cast" and not fits(v, t):
                raise Violation(f"{show(e)}: {v} does not fit")
            return wrap(v, t)
        if k == "get":
            buf = env[e[1]]
            return buf[self.ev(e[2], env) % len(buf)]
        if k == "bget":
            buf = env[e[1]]
            return buf[self.ev(e[2], env) % len(buf)]
        if k == "vget":
            return env["v"][self.ev(e[1], env) % len(env["v"])]
        if k == "call":
            f = self.byname[e[1]]
            args = [("val", self.ev(a[1], env)) if a[0] == "val" else a for a in e[2]]
            return self.call(f, args, env)
        if k == "raw":
            op = e[1]
            a, b = self.ev(e[2], env), self.ev(e[3], env)
            if op in "/%":
                if b == 0:
                    raise Violation(f"division by zero in {show(e)}")
                q = tdiv(a, b)
                if not fits(q, t):
                    raise Violation(f"{show(e)} overflows")
                r = q if op == "/" else a - b * q
            else:
                r = {"+": a + b, "-": a - b, "*": a * b}[op]
            if not fits(r, t):
                raise Violation(f"{show(e)} = {r} does not fit {t}")
            return r
        raise ValueError(k)

# ---------------------------------------------------------------------------
# Running things


def run(cmd, cwd=None, limit=LIMIT, env=None):
    """(exit status or None on timeout, stdout, stderr). A timeout kills the
    whole process group, so a compiler's z3 does not outlive it."""
    p = subprocess.Popen(cmd, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, start_new_session=True, env=env)
    try:
        out, err = p.communicate(timeout=limit)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, err = p.communicate()
        return None, out, err
    return p.returncode, out, err


def crashed(code, err):
    return code is None or code not in (0, 1, 2) or b"panicked" in err


def mask(want, got):
    # Mirrors mask() in tests/fuzz.py and stage1/difftest.sh.
    keep_msg = re.search(rb'error\[[A-Z-]*\]: ', got)
    keep_notes = re.search(rb'error\[E-(VERIFY|SOLVER)', want) or re.search(rb'^  = ', got, re.M)
    out = []
    for l in want.split(b'\n'):
        if not keep_notes and l.startswith(b'  = '):
            continue
        if not keep_msg:
            l = re.sub(rb'^(.*: error\[[A-Z-]*\]): .*$', rb'\1', l)
        l = re.sub(rb'^bastioc0: ', b'bastioc1: ', l)
        out.append(l)
    return re.sub(rb'(  = counterexample: ).*', rb'\1...', b'\n'.join(out))


def one(seed, b1, san):
    """Generates, interprets, compiles and runs one program. Returns
    (stats, findings), findings being (kind, source, note)."""
    rnd = random.Random(seed)
    p = Gen(rnd).program()
    src = p.source()
    try:
        want_out, want_code = p.run()
        bad = None
    except Violation as v:
        bad = str(v)
    stats, found = [], []
    tmp = tempfile.mkdtemp(prefix="bastio-progfuzz-")
    try:
        path = os.path.join(tmp, "p.bastio")
        with open(path, "w") as f:
            f.write(src)
        where = f"progfuzz seed {seed}"
        c0, _, e0 = run([BASTIOC, "--check", path], tmp)
        if crashed(c0, e0):
            found.append(("crash0", src, f"bastioc0 --check: {'timeout' if c0 is None else f'exit {c0}'} ({where})\n{e0.decode(errors='replace')[-2000:]}"))
            return stats, found
        if b1:
            c1, _, e1 = run([b1, "--check", path], tmp)
            if crashed(c1, e1):
                found.append(("crash1", src, f"bastioc1 --check: {'timeout' if c1 is None else f'exit {c1}'} ({where})\n{e1.decode(errors='replace')[-2000:]}"))
            elif c0 != c1 or mask(e0, e1) != re.sub(rb'(  = counterexample: ).*', rb'\1...', e1):
                found.append(("differ", src, f"bastioc0 and bastioc1 disagree ({where})\n--- bastioc0 exit {c0}:\n{e0.decode(errors='replace')}--- bastioc1 exit {c1}:\n{e1.decode(errors='replace')}"))
        if c0 != 0:
            m = re.search(rb'error\[([A-Z-]+)\]', e0)
            stats.append(f"rejected {m.group(1).decode() if m else '?'}" + (" (the run breaks it)" if bad else " (the run does not break it)"))
            return stats, found
        if bad:
            found.append(("falseproof", src, f"bastioc0 proves the program, but running it: {bad} ({where})"))
            return stats, found
        stats.append("proved")
        builds = [("bastioc0 --backend c", [BASTIOC, "--backend", "c"]),
                  ("bastioc0 --backend native", [BASTIOC, "--backend", "native"])]
        if b1:
            builds += [("bastioc1 --backend c", [b1, "--backend", "c"]),
                       ("bastioc1 --backend native", [b1, "--backend", "native"])]
            r0 = run([BASTIOC, "--emit-c", "-o", "c0.c", path], tmp)[0]
            r1 = run([b1, "--emit-c", "-o", "c1.c", path], tmp)[0]
            if r0 != 0 or r1 != 0 or open(os.path.join(tmp, "c0.c"), "rb").read() != open(os.path.join(tmp, "c1.c"), "rb").read():
                found.append(("differ", src, f"bastioc0 and bastioc1 write different C ({where}; --emit-c exit {r0} / {r1})"))
        for i, (name, cmd) in enumerate(builds):
            exe = os.path.join(tmp, f"x{i}")
            code, _, err = run(cmd + ["-o", exe, path], tmp)
            if code == 4 and "native" in name:
                stats.append(f"{name}: unsupported")
                continue
            if code != 0:
                found.append(("build", src, f"{name} fails on a proved program: {'timeout' if code is None else f'exit {code}'} ({where})\n{err.decode(errors='replace')[-2000:]}"))
                continue
            check(found, src, name, run([exe], tmp, RUN_LIMIT), want_out, want_code, where)
        if san:
            c = os.path.join(tmp, "s.c")
            exe = os.path.join(tmp, "s")
            if run([BASTIOC, "--emit-c", "-o", c, path], tmp)[0] == 0:
                code, _, err = run([CC, "-fsanitize=address,undefined", "-fno-sanitize-recover=all", "-g", "-O1", "-w", "-o", exe, c], tmp)
                if code != 0:
                    found.append(("build", src, f"the C bastioc0 wrote does not compile ({where})\n{err.decode(errors='replace')[-2000:]}"))
                else:
                    check(found, src, "bastioc0 C with ASan/UBSan", run([exe], tmp, RUN_LIMIT * 3), want_out, want_code, where)
        return stats, found
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def check(found, src, name, res, want_out, want_code, where):
    code, out, err = res
    if b"Sanitizer" in err or b"runtime error" in err:
        found.append(("sanitizer", src, f"{name}: the sanitizers report ({where})\n{err.decode(errors='replace')[:3000]}"))
    elif code != want_code or out != want_out:
        diff = ""
        wl, gl = want_out.split(b"\n"), out.split(b"\n")
        for j in range(max(len(wl), len(gl))):
            a = wl[j] if j < len(wl) else b"(end)"
            b = gl[j] if j < len(gl) else b"(end)"
            if a != b:
                diff = f"first difference at output line {j + 1}: expected {a.decode()}, got {b.decode()}"
                break
        found.append(("miscompile", src, f"{name}: exit {'timeout' if code is None else code}, expected {want_code}; {diff or 'same output'} ({where})"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--count", type=int, default=50)
    ap.add_argument("--seconds", type=float)
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 2))
    ap.add_argument("--bastioc1")
    ap.add_argument("--no-bastioc1", action="store_true")
    ap.add_argument("--keep", default="progfuzz-findings")
    ap.add_argument("--print", type=int, metavar="SEED", help="print the program for SEED and its expected output, and exit")
    a = ap.parse_args()
    if a.print is not None:
        p = Gen(random.Random(a.print)).program()
        sys.stdout.write(p.source())
        try:
            out, code = p.run()
            sys.stdout.write("// expected exit %d, output:\n" % code + "".join(f"//   {l}\n" for l in out.decode().split("\n")[:-1]))
        except Violation as v:
            sys.stdout.write(f"// running it: {v}\n")
        return 0
    b1 = None
    if a.bastioc1:
        b1 = os.path.abspath(a.bastioc1)
    elif not a.no_bastioc1:
        b1 = os.path.join(tempfile.mkdtemp(prefix="bastioc1-"), "bastioc1")
        code, _, err = run([BASTIOC, "-o", b1, os.path.join(ROOT, "stage1/bastioc1.bastio")], limit=1200)
        if code != 0:
            sys.exit("progfuzz: cannot build bastioc1\n" + err.decode())
    probe = tempfile.mkdtemp(prefix="bastio-progfuzz-")
    san = subprocess.run([CC, "-fsanitize=address,undefined", "-x", "c", "-", "-o", os.path.join(probe, "probe")],
                         input=b"int main(void){return 0;}", capture_output=True).returncode == 0
    shutil.rmtree(probe, ignore_errors=True)
    if not san:
        print("progfuzz: the C compiler has no ASan/UBSan; those runs are skipped")
    keep = os.path.abspath(a.keep)
    stats, nfound, n = {}, 0, 0
    start = time.time()
    with concurrent.futures.ProcessPoolExecutor(a.jobs) as ex:
        pending = set()
        nxt = 0

        def more():
            nonlocal nxt
            while len(pending) < a.jobs * 2:
                if a.seconds is not None and time.time() - start >= a.seconds:
                    return
                if a.seconds is None and nxt >= a.count:
                    return
                pending.add(ex.submit(one, a.seed * 1000003 + nxt, b1, san))
                nxt += 1
        more()
        while pending:
            done, _ = concurrent.futures.wait(pending, return_when=concurrent.futures.FIRST_COMPLETED)
            for fut in done:
                pending.discard(fut)
                st, found = fut.result()
                n += 1
                for s in st:
                    stats[s] = stats.get(s, 0) + 1
                for kind, src, note in found:
                    nfound += 1
                    os.makedirs(keep, exist_ok=True)
                    path = os.path.join(keep, f"{kind}-{nfound}.bastio")
                    with open(path, "w") as f:
                        f.write("".join(f"// {l}\n" for l in note.rstrip().split("\n")))
                        f.write(src)
                    print(f"FOUND {kind}: {path}\n    " + note.rstrip().split("\n")[0], flush=True)
            more()
    print("progfuzz: " + ", ".join(f"{v} {k}" for k, v in sorted(stats.items())))
    print(f"progfuzz: {n} programs, {nfound} finding(s), {time.time() - start:.0f} s")
    return 1 if nfound else 0


if __name__ == "__main__":
    sys.exit(main())
