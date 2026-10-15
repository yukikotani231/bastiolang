#!/usr/bin/env python3
"""Fuzzes bastioc0 and bastioc1: crashes, disagreements, and false proofs.

usage: tests/fuzz.py [--seed N] [--count N | --seconds S] [--mode gen|mutate|both]
                     [--bastioc1 PATH] [--keep DIR]

Two kinds of input:

- gen: random programs with a known meaning. Each is one function `f` over
  sized integers (with `requires` bounds on its parameters, `let`s, `if`s,
  `assert`s, a buffer it indexes, and sometimes an `ensures`) and a `main`
  that calls it. The fuzzer evaluates `f` itself, exactly, on the corner
  values and on random values its `requires` allow, and records every
  overflow, underflow, division by zero, bad shift, bad cast, index out of
  bounds, failed `assert` and broken `ensures`. If bastioc0 proves `f` and
  one of those happens on some input, the proof is false. When it proves
  `f`, `main` checks `f`'s result on a few inputs against the fuzzer's own
  evaluation, built through the C backend and the native one, so the code
  generators are checked too.

- mutate: the test programs and examples, damaged by deleting, repeating
  or swapping tokens, or replacing one identifier or number by another.
  Neither compiler may crash (a Rust panic, a signal, or an exit status
  other than 0, 1 or 2) or take longer than the time limit. A damaged
  hosted program that bastioc0 still accepts is built with AddressSanitizer
  and UBSan and run briefly: any report means a false proof (a leak, too,
  since linear values are proved to be freed).

Both compilers check every input, and bastioc1 must print what bastioc0
prints and exit the same way (masked as stage1/difftest.sh masks it), and
write the same C for what they accept. Without a bastioc1 (built from
stage1/bastioc1.bastio by bastioc0 when --bastioc1 is not given) only
bastioc0 is fuzzed.

Deterministic for a given seed and --count. Exits 1 if anything was found;
each finding is saved under --keep (default: fuzz-findings/) with a note.
"""
import argparse, collections, concurrent.futures, os, random, re, shutil, signal, subprocess, sys, tempfile, threading, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASTIOC = os.path.abspath(os.environ.get("BASTIOC", os.path.join(ROOT, "stage0/target/release/bastioc0")))
CC = os.environ.get("CC", "cc")
LIMIT = 150          # seconds a compiler may take on one input (above Z3's 60 s backstop)

# ---------------------------------------------------------------------------
# Integer types

TYPES = {"u8": (False, 8), "u16": (False, 16), "u32": (False, 32), "u64": (False, 64),
         "i8": (True, 8), "i16": (True, 16), "i32": (True, 32), "i64": (True, 64)}
UNSIGNED = [t for t in TYPES if not TYPES[t][0]]


def lo(t):
    s, w = TYPES[t]
    return -(1 << (w - 1)) if s else 0


def hi(t):
    s, w = TYPES[t]
    return (1 << (w - 1)) - 1 if s else (1 << w) - 1


def fits(v, t):
    return lo(t) <= v <= hi(t)


def wrap(v, t):
    s, w = TYPES[t]
    v &= (1 << w) - 1
    return v - (1 << w) if s and v >> (w - 1) else v


def lit(v):
    return f"-{-v}" if v < 0 else str(v)


class Violation(Exception):
    pass


def tdiv(a, b):
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b >= 0) else -q

# ---------------------------------------------------------------------------
# Expressions: tuples, typed. Each has a printer and an evaluator.
#   ('var', name, t) ('lit', v, t) ('bin', op, a, b, t) ('neg', a, t)
#   ('bnot', a, t) ('cast', a, t) ('wrap', a, t) ('wop', name, a, b, t)
#   ('get', i)  -> u64        ('blen',) -> u64
# Conditions: ('cmp', op, a, b) ('and', c, d) ('or', c, d) ('not', c)
#   ('true',) ('false',)


def ty(e):
    if e[0] in ("get", "blen"):
        return "u64"
    return e[-1]


def show(e):
    k = e[0]
    if k == "var":
        return e[1]
    if k == "lit":
        return lit(e[1])
    if k == "bin":
        return f"({show(e[2])} {e[1]} {show(e[3])})"
    if k == "neg":
        return f"(-{show(e[1])})"
    if k == "bnot":
        return f"(!{show(e[1])})"
    if k in ("cast", "wrap"):
        return f"{k}[{e[2]}]({show(e[1])})"
    if k == "wop":
        return f"{e[1]}({show(e[2])}, {show(e[3])})"
    if k == "get":
        return f"get(b, {show(e[1])})"
    if k == "blen":
        return "len(b)"
    if k == "cmp":
        return f"{show(e[2])} {e[1]} {show(e[3])}"
    if k == "and":
        return f"({show(e[1])}) && ({show(e[2])})"
    if k == "or":
        return f"({show(e[1])}) || ({show(e[2])})"
    if k == "not":
        return f"!({show(e[1])})"
    return k  # true, false


def ev(e, env, buf):
    k = e[0]
    if k == "var":
        return env[e[1]]
    if k == "lit":
        return e[1]
    if k == "bin":
        op, t = e[1], e[4]
        a = ev(e[2], env, buf)
        b = ev(e[3], env, buf)
        if op == "+":
            r = a + b
        elif op == "-":
            r = a - b
        elif op == "*":
            r = a * b
        elif op in ("/", "%"):
            if b == 0:
                raise Violation(f"division by zero in {show(e)}")
            q = tdiv(a, b)
            if not fits(q, t):
                raise Violation(f"{show(e)} overflows")   # MIN / -1 and MIN % -1
            r = q if op == "/" else a - b * q
        elif op == "&":
            r = a & b
        elif op == "|":
            r = a | b
        elif op == "^":
            r = a ^ b
        elif op in ("<<", ">>"):
            if b >= TYPES[t][1]:
                raise Violation(f"shift by {b} in {show(e)}")
            r = a << b if op == "<<" else a >> b
        if not fits(r, t):
            raise Violation(f"{show(e)} = {r} does not fit {t}")
        return r
    if k == "neg":
        r = -ev(e[1], env, buf)
        if not fits(r, e[2]):
            raise Violation(f"{show(e)} overflows")
        return r
    if k == "bnot":
        return hi(e[2]) ^ ev(e[1], env, buf)
    if k == "cast":
        v = ev(e[1], env, buf)
        if not fits(v, e[2]):
            raise Violation(f"{show(e)}: {v} does not fit")
        return v
    if k == "wrap":
        return wrap(ev(e[1], env, buf), e[2])
    if k == "wop":
        a = ev(e[2], env, buf)
        b = ev(e[3], env, buf)
        r = {"wrap_add": a + b, "wrap_sub": a - b, "wrap_mul": a * b}[e[1]]
        return wrap(r, e[4])
    if k == "get":
        i = ev(e[1], env, buf)
        if i >= len(buf):
            raise Violation(f"get(b, {i}) with len(b) = {len(buf)}")
        return buf[i]
    if k == "blen":
        return len(buf)
    if k == "cmp":
        a = ev(e[2], env, buf)
        b = ev(e[3], env, buf)
        return {"==": a == b, "!=": a != b, "<": a < b, "<=": a <= b, ">": a > b, ">=": a >= b}[e[1]]
    if k == "and":
        return ev(e[1], env, buf) and ev(e[2], env, buf)
    if k == "or":
        return ev(e[1], env, buf) or ev(e[2], env, buf)
    if k == "not":
        return not ev(e[1], env, buf)
    return k == "true"

# ---------------------------------------------------------------------------
# Random programs


class Gen:
    def __init__(self, rnd):
        self.r = rnd
        self.n = 0

    def value(self, t):
        r = self.r.random()
        if r < 0.3:
            return self.r.choice([lo(t), hi(t), 0, 1, lo(t) + 1, hi(t) - 1, -1 if TYPES[t][0] else 2])
        if r < 0.85:
            v = self.r.randint(-20, 300) if self.r.random() < 0.5 else self.r.randint(-5, 9)
        else:
            v = self.r.randint(lo(t), hi(t))
        return min(max(v, lo(t)), hi(t))

    def literal(self, t):
        v = self.value(t)
        # The most negative value cannot be written as `-N`: N does not fit.
        return ("lit", max(v, lo(t) + 1) if TYPES[t][0] else v, t)

    def leaf(self, t, scope):
        same = [v for v in scope if scope[v] == t]
        r = self.r.random()
        if same and r < 0.65:
            return ("var", self.r.choice(same), t)
        if scope and r < 0.85:
            v = self.r.choice(list(scope))
            if scope[v] == t:
                return ("var", v, t)
            return (self.conv(), ("var", v, scope[v]), t)
        if t == "u64" and self.has_buf and r < 0.9:
            return ("blen",)
        return self.literal(t)

    def expr(self, t, scope, depth):
        """An expression of type t that mentions a variable (so a literal in
        it takes its type from the other operand)."""
        if depth <= 0 or self.r.random() < 0.25:
            e = self.leaf(t, scope)
            if e[0] == "lit" or e[0] == "blen" and t != "u64":
                return self.leaf_var(t, scope)
            return e
        signed = TYPES[t][0]
        ops = ["+", "-", "*", "/", "%"] + ([] if signed else ["&", "|", "^", "<<", ">>"])
        r = self.r.random()
        if r < 0.55:
            op = self.r.choice(ops)
            a = self.expr(t, scope, depth - 1)
            if op in ("<<", ">>"):
                st = self.r.choice(UNSIGNED)
                b = ("lit", self.r.randint(0, TYPES[t][1] + 1), st) if self.r.random() < 0.6 else self.expr(st, scope, depth - 1)
                if b[0] == "lit":
                    b = ("lit", b[1], t)       # a literal shift amount takes the left operand's type
                return ("bin", op, a, b, t)
            b = self.literal(t) if self.r.random() < 0.4 else self.expr(t, scope, depth - 1)
            if self.r.random() < 0.5:
                a, b = b, a
            return ("bin", op, a, b, t)
        if r < 0.62 and signed:
            return ("neg", self.expr(t, scope, depth - 1), t)
        if r < 0.62:
            return ("bnot", self.expr(t, scope, depth - 1), t)
        if r < 0.72:
            return ("wop", self.r.choice(["wrap_add", "wrap_sub", "wrap_mul"]),
                    self.expr(t, scope, depth - 1), self.literal(t) if self.r.random() < 0.5 else self.expr(t, scope, depth - 1), t)
        if r < 0.85:
            s = self.r.choice(list(TYPES))
            return (self.conv(), self.expr(s, scope, depth - 1), t)
        if self.has_buf and t == "u64":
            return ("get", self.expr(self.r.choice(UNSIGNED), scope, depth - 1)
                    if self.r.random() < 0.7 else self.expr("u64", scope, depth - 1))
        return self.leaf_var(t, scope)

    def leaf_var(self, t, scope):
        v = self.r.choice(list(scope))
        if scope[v] == t:
            return ("var", v, t)
        return (self.conv(), ("var", v, scope[v]), t)

    def conv(self):
        return "cast" if self.r.random() < 0.3 else "wrap"

    def cond(self, scope, depth=1):
        r = self.r.random()
        if depth > 0 and r < 0.2:
            return (self.r.choice(["and", "or"]), self.cond(scope, depth - 1), self.cond(scope, depth - 1))
        if depth > 0 and r < 0.27:
            return ("not", self.cond(scope, depth - 1))
        t = self.r.choice([scope[v] for v in scope])
        a = self.expr(t, scope, 1)
        b = self.literal(t) if self.r.random() < 0.6 else self.expr(t, scope, 1)
        return ("cmp", self.r.choice(["==", "!=", "<", "<=", ">", ">="]), a, b)

    def spec_cond(self, scope):
        # `assert` and `ensures`: a variable against a literal, no arithmetic.
        v = self.r.choice(list(scope))
        return ("cmp", self.r.choice(["<", "<=", ">", ">=", "!="]), ("var", v, scope[v]), self.literal(scope[v]))

    def block(self, scope, mutable, depth, n):
        out = []
        for _ in range(n):
            r = self.r.random()
            if r < 0.4:
                t = self.r.choice(list(TYPES))
                name = f"v{self.n}"
                self.n += 1
                mut = self.r.random() < 0.4
                out.append(("let", name, t, self.expr(t, scope, self.r.randint(1, 3)), mut))
                scope = dict(scope)
                scope[name] = t
                if mut:
                    mutable = mutable + [name]
            elif r < 0.6 and mutable:
                v = self.r.choice(mutable)
                out.append(("assign", v, self.expr(scope[v], scope, self.r.randint(1, 3))))
            elif r < 0.75 and depth > 0:
                out.append(("if", self.cond(scope), self.block(scope, mutable, depth - 1, self.r.randint(1, 3)),
                            self.block(scope, mutable, depth - 1, self.r.randint(0, 2))))
            elif r < 0.9 and self.has_buf:
                out.append(("set", self.expr(self.r.choice(UNSIGNED), scope, 1), self.expr("u64", scope, 2)))
            else:
                out.append(("assert", self.spec_cond(scope)))
        self.last_scope = scope
        return out

    def program(self):
        self.n = 0
        self.has_buf = self.r.random() < 0.5
        self.buf_len = self.r.randint(1, 8)
        params = {}
        reqs = []
        for i in range(self.r.randint(1, 3)):
            t = self.r.choice(list(TYPES))
            p = f"p{i}"
            params[p] = t
            if self.r.random() < 0.7:          # a small window: most programs then prove
                a = min(max(self.r.randint(-60, 200), lo(t)), hi(t))
                b = min(max(a + self.r.randint(0, 100), lo(t)), hi(t))
            else:
                a, b = sorted([self.value(t), self.value(t)])
            if self.r.random() < 0.8:
                reqs.append(("cmp", ">=", ("var", p, t), ("lit", max(a, lo(t) + 1) if TYPES[t][0] else a, t)))
            if self.r.random() < 0.8:
                reqs.append(("cmp", "<=", ("var", p, t), ("lit", max(b, lo(t) + 1) if TYPES[t][0] else b, t)))
        rt = self.r.choice(list(TYPES))
        body = self.block(dict(params), [], 2, self.r.randint(1, 6))
        ret = self.expr(rt, self.last_scope_top(params, body), 2)
        ens = None
        if self.r.random() < 0.3:
            ens = ("cmp", self.r.choice(["<=", ">=", "!="]), ("var", "result", rt), self.literal(rt))
        return Prog(params, reqs, rt, body, ret, ens, self.has_buf, self.buf_len)

    def last_scope_top(self, params, body):
        # The variables in scope after the top-level statements.
        scope = dict(params)
        for s in body:
            if s[0] == "let":
                scope[s[1]] = s[2]
        return scope


class Prog:
    def __init__(self, params, reqs, rt, body, ret, ens, has_buf, buf_len):
        self.params, self.reqs, self.rt, self.body = params, reqs, rt, body
        self.ret, self.ens, self.has_buf, self.buf_len = ret, ens, has_buf, buf_len

    def source(self, checks):
        ps = ", ".join(f"{p}: {t}" for p, t in self.params.items())
        out = [f"fn f({ps}) -> {self.rt}"]
        for c in self.reqs:
            out.append(f"  requires {show(c)}")
        if self.ens:
            out.append(f"  ensures {show(self.ens)}")
        out.append("{")
        if self.has_buf:
            out.append(f"  let mut b = alloc({self.buf_len});")
        self.stmts(self.body, out, "  ")
        out.append(f"  let r: {self.rt} = {show(self.ret)};")
        if self.has_buf:
            out.append("  free(b);")
        out.append("  return r;")
        out.append("}")
        out.append("")
        out.append("fn main() -> u64 {")
        for k, (args, want) in enumerate(checks):
            a = ", ".join(lit(v) for v in args)
            out.append(f"  if f({a}) != {lit(want)} {{ return {k + 1}; }}")
        out.append("  return 0;")
        out.append("}")
        return "\n".join(out) + "\n"

    def stmts(self, ss, out, ind):
        for s in ss:
            k = s[0]
            if k == "let":
                out.append(f"{ind}let {'mut ' if s[4] else ''}{s[1]}: {s[2]} = {show(s[3])};")
            elif k == "assign":
                out.append(f"{ind}{s[1]} = {show(s[2])};")
            elif k == "if":
                out.append(f"{ind}if {show(s[1])} {{")
                self.stmts(s[2], out, ind + "  ")
                if s[3]:
                    out.append(f"{ind}}} else {{")
                    self.stmts(s[3], out, ind + "  ")
                out.append(f"{ind}}}")
            elif k == "set":
                out.append(f"{ind}b = set(b, {show(s[1])}, {show(s[2])});")
            elif k == "assert":
                out.append(f"{ind}assert {show(s[1])};")

    def run(self, args):
        """f's result on args, or raises Violation."""
        env = dict(zip(self.params, args))
        buf = [0] * self.buf_len
        self.exec(self.body, env, buf)
        r = ev(self.ret, env, buf)
        if not fits(r, self.rt):
            raise Violation("result does not fit")
        if self.ens and not ev(self.ens, {"result": r}, buf):
            raise Violation(f"ensures {show(self.ens)} is false: result = {r}")
        return r

    def exec(self, ss, env, buf):
        for s in ss:
            k = s[0]
            if k == "let":
                env[s[1]] = ev(s[3], env, buf)
            elif k == "assign":
                env[s[1]] = ev(s[2], env, buf)
            elif k == "if":
                inner = dict(env)
                self.exec(s[2] if ev(s[1], env, buf) else s[3], inner, buf)
                for v in env:               # assignments to outer variables survive
                    env[v] = inner[v]
            elif k == "set":
                i = ev(s[1], env, buf)
                v = ev(s[2], env, buf)
                if i >= len(buf):
                    raise Violation(f"set(b, {i}, ..) with len(b) = {len(buf)}")
                buf[i] = v
            elif k == "assert":
                if not ev(s[1], env, buf):
                    raise Violation(f"assert {show(s[1])} is false")

    def inputs(self, rnd):
        """Inputs the requires allow: corners first, then random ones."""
        ranges = []
        for p, t in self.params.items():
            a, b = lo(t), hi(t)
            for c in self.reqs:
                if c[2][1] == p:
                    if c[1] == ">=":
                        a = max(a, c[3][1])
                    else:
                        b = min(b, c[3][1])
            if a > b:
                return []
            ranges.append((a, b))
        corners = []
        for a, b in ranges:
            cs = {a, b, a + 1, b - 1, 0, 1, -1, (a + b) // 2}
            corners.append(sorted(v for v in cs if a <= v <= b))
        out = []
        for _ in range(300):
            out.append(tuple(rnd.choice(c) for c in corners))
        for _ in range(300):
            out.append(tuple(rnd.randint(a, b) for a, b in ranges))
        return list(dict.fromkeys(out))

# ---------------------------------------------------------------------------
# Running the compilers


def compile_(cmd, cwd, limit=LIMIT):
    # In a session of its own, so a timeout kills the compiler's z3 too:
    # killing only the compiler left z3 spinning, slowing every later input.
    t = time.time()
    p = subprocess.Popen(cmd, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, start_new_session=True)
    try:
        _, err = p.communicate(timeout=limit)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        p.communicate()
        return None, b"", time.time() - t
    return p.returncode, err, time.time() - t


def crashed(code, err):
    return code is None or code not in (0, 1, 2) or b"panicked" in err


def mask(want, got):
    # Mirrors mask() in stage1/difftest.sh and stage1/mutate.py.
    keep_msg = re.search(rb'error\[[A-Z-]*\]: ', got)
    keep_notes = re.search(rb'error\[E-(VERIFY|SOLVER)', want) or re.search(rb'^  = ', got, re.M)
    out = []
    for l in want.split(b'\n'):
        if not keep_notes and l.startswith(b'  = '):
            continue
        if not keep_msg:
            l = re.sub(rb'^(.*: error\[[A-Z-]*\]): .*$', rb'\1', l)
        l = re.sub(rb'^(.*: error\[E-IMPORT\]: cannot read `[^`]*`): .*$', rb'\1', l)
        l = re.sub(rb'^(.*: error\[E-PARSE\]: include_bytes: cannot read .*): [^:]*$', rb'\1', l)
        l = re.sub(rb'^bastioc0: ', b'bastioc1: ', l)
        out.append(l)
    return b'\n'.join(out)


def counterexample_free(err):
    # Counterexample values depend on the solver's search; compare the rest.
    return re.sub(rb'(  = counterexample(?: after one iteration)?: ).*', rb'\1...', err)


class Fuzz:
    def __init__(self, a):
        self.a = a
        self.tmp = tempfile.mkdtemp(prefix="bastio-fuzz-")
        self.keep = os.path.abspath(a.keep)
        self.found = 0
        self.stats = {}
        self.cur = threading.local()
        self.b1 = a.bastioc1
        self.san = subprocess.run([CC, "-fsanitize=address,undefined", "-x", "c", "-", "-o", os.path.join(self.tmp, "probe")],
                                  input=b"int main(void){return 0;}", capture_output=True).returncode == 0

    # Inputs are checked JOBS at a time (default: the number of CPUs), each
    # on a thread with its own scratch directory (`self.cur.tmp`). What one
    # finds and counts is kept with it and reported in input order, so the
    # output does not depend on JOBS.
    def stat(self, k):
        self.cur.stats.append(k)

    def finding(self, kind, src, note):
        self.cur.finds.append((kind, src, note))

    def report(self, kind, src, note):
        self.found += 1
        os.makedirs(self.keep, exist_ok=True)
        path = os.path.join(self.keep, f"{kind}-{self.found}.bastio")
        with open(path, "w") as f:
            f.write("".join(f"// {l}\n" for l in note.rstrip().split("\n")))
            f.write(src)
        print(f"FOUND {kind}: {path}\n    " + note.rstrip().split("\n")[0])

    def both(self, src, path, cwd, where):
        """Checks src with both compilers; returns bastioc0's exit status."""
        c0, e0, t0 = compile_([BASTIOC, "--check", path], cwd)
        if crashed(c0, e0):
            self.finding("crash0", src, f"bastioc0 --check: {'timeout' if c0 is None else f'exit {c0}'} ({where})\n{e0.decode(errors='replace')[-2000:]}")
            return None, e0
        if not self.b1:
            return c0, e0
        c1, e1, t1 = compile_([self.b1, "--check", path], cwd)
        if crashed(c1, e1):
            self.finding("crash1", src, f"bastioc1 --check: {'timeout' if c1 is None else f'exit {c1}'} ({where})\n{e1.decode(errors='replace')[-2000:]}")
            return c0, e0
        if c0 != c1 or counterexample_free(mask(e0, e1)) != counterexample_free(e1):
            self.finding("differ", src, f"bastioc0 and bastioc1 disagree ({where})\n--- bastioc0 exit {c0}:\n{e0.decode(errors='replace')}--- bastioc1 exit {c1}:\n{e1.decode(errors='replace')}")
        elif c0 == 0:
            o0 = os.path.join(self.cur.tmp, "c0.c")
            o1 = os.path.join(self.cur.tmp, "c1.c")
            r0 = compile_([BASTIOC, "--emit-c", "-o", o0, path], cwd)[0]
            r1 = compile_([self.b1, "--emit-c", "-o", o1, path], cwd)[0]
            if r0 != 0 or r1 != 0 or open(o0, "rb").read() != open(o1, "rb").read():
                self.finding("differ", src, f"bastioc0 and bastioc1 write different C ({where}; --emit-c exit {r0} / {r1})")
        return c0, e0

    # -- gen ----------------------------------------------------------------

    def gen_one(self, seed):
        rnd = random.Random(seed)
        g = Gen(rnd)
        p = g.program()
        ins = p.inputs(rnd)
        bad = None
        good = []
        for x in ins:
            try:
                good.append((x, p.run(x)))
            except Violation as v:
                if bad is None:
                    bad = (x, str(v))
        checks = good[:4]
        src = p.source(checks)
        d = os.path.join(self.cur.tmp, "gen")
        os.makedirs(d, exist_ok=True)
        path = os.path.join(d, "f.bastio")
        with open(path, "w") as f:
            f.write(src)
        c0, e0 = self.both(src, path, d, f"gen seed {seed}")
        if c0 is None:
            return
        if c0 == 0 and bad is not None:
            args = ", ".join(f"{n} = {lit(v)}" for n, v in zip(p.params, bad[0]))
            self.finding("falseproof", src, f"bastioc0 proves f, but with {args}: {bad[1]} (gen seed {seed})")
            return
        if c0 != 0:
            m = re.search(rb'error\[([A-Z-]+)\]', e0)
            self.stat(f"gen {m.group(1).decode() if m else 'rejected'}" + ("" if bad else " (no sampled input breaks it)"))
            if not bad and os.environ.get("FUZZ_KEEP_UNPROVED"):
                os.makedirs(self.keep, exist_ok=True)
                with open(os.path.join(self.keep, f"unproved-{seed}.bastio"), "w") as f:
                    f.write(src + "".join(f"// {l}\n" for l in e0.decode(errors="replace").split("\n")))
            return
        self.stat("gen proved")
        exe = os.path.join(d, "f")
        for backend in (["--backend", "c"], ["--backend", "native"]):
            code, err, _ = compile_([BASTIOC, *backend, "-o", exe, path], d)
            if code == 4 and backend[1] == "native":
                continue
            if code != 0:
                self.finding("build", src, f"bastioc0 {' '.join(backend)} fails on a proved program: exit {code} (gen seed {seed})\n{err.decode(errors='replace')[-2000:]}")
                return
            try:
                r = subprocess.run([exe], capture_output=True, timeout=10).returncode
            except subprocess.TimeoutExpired:
                r = "timeout"
            if r != 0:
                k = r if isinstance(r, int) and 0 < r <= len(checks) else None
                what = f"f({', '.join(lit(v) for v in checks[k - 1][0])}) is not {lit(checks[k - 1][1])}" if k else f"exit {r}"
                self.finding("miscompile", src, f"{' '.join(backend)}: {what} (gen seed {seed})")
                return

    # -- mutate ---------------------------------------------------------------

    def corpus(self):
        fs = []
        for d in ("tests/pass", "tests/fail", "examples", "core"):
            for root, _, names in os.walk(os.path.join(ROOT, d)):
                fs += [os.path.join(root, n) for n in names if n.endswith(".bastio")]
        # bastioc0 cannot compile a stage1-only program, so it would always disagree.
        return sorted(f for f in fs if not re.search(rb'^// stage1-only:', open(f, "rb").read(), re.M))

    TOKEN = re.compile(rb'[A-Za-z_][A-Za-z0-9_]*|0[xX][0-9a-fA-F_]+|[0-9][0-9_]*|"(?:[^"\\\n]|\\.)*"|==>|::|->|=>|==|!=|<=|>=|&&|\|\||<<|>>|//[^\n]*|\s+|.', re.S)

    def mutate(self, rnd, src):
        toks = self.TOKEN.findall(src)
        idx = [i for i, t in enumerate(toks) if not t.isspace() and not t.startswith(b"//")]
        if not idx:
            return src
        for _ in range(rnd.choice([1, 1, 1, 2, 3])):
            i = rnd.choice(idx)
            t = toks[i]
            r = rnd.random()
            if r < 0.2:
                toks[i] = b""
            elif r < 0.3:
                toks[i] = t + b" " + t
            elif r < 0.45 and re.fullmatch(rb'[0-9][0-9_]*|0[xX][0-9a-fA-F_]+', t):
                toks[i] = rnd.choice([b"0", b"1", b"2", b"255", b"256", b"65535", b"4294967296",
                                      b"18446744073709551615", b"18446744073709551616", str(int(t.replace(b"_", b""), 0) + rnd.choice([-1, 1])).encode()])
            elif r < 0.7 and re.match(rb'[A-Za-z_]', t):
                same = [toks[j] for j in idx if re.match(rb'[A-Za-z_]', toks[j])]
                toks[i] = rnd.choice(same)
            elif r < 0.85:
                ops = [b"<", b"<=", b">", b">=", b"==", b"!=", b"+", b"-", b"*", b"/", b"%", b"&&", b"||"]
                toks[i] = rnd.choice(ops) if t in ops else rnd.choice([b"(", b")", b"{", b"}", b"[", b"]", b",", b";", b":", b"&", b"mut ", b"."])
            else:
                j = rnd.choice(idx)
                toks[i], toks[j] = toks[j], toks[i]
        return b"".join(toks)

    def backends_agree(self, text, path, d, where):
        """An accepted hosted mutant must do the same through the C backend and the native one."""
        res = {}
        for backend in ("c", "native"):
            exe = os.path.join(self.cur.tmp, f"m.{backend}")
            code, err, _ = compile_([BASTIOC, "--backend", backend, "-o", exe, path], d)
            if code is None or (code == 4 and backend == "native"):
                return                      # too slow to prove, or not supported natively yet
            if code != 0:
                self.finding("build", text, f"bastioc0 --backend {backend} fails on an accepted mutant: exit {code} ({where})\n{err.decode(errors='replace')[-2000:]}")
                return
            try:
                r = subprocess.run([exe], capture_output=True, timeout=10, stdin=subprocess.DEVNULL)
            except subprocess.TimeoutExpired:
                return                      # termination is not proved
            res[backend] = (r.returncode, r.stdout)
        self.stat("mutant run on both backends")
        if res["c"] != res["native"]:
            self.finding("miscompile", text, f"the C and native backends disagree: C exit {res['c'][0]}, native exit {res['native'][0]}"
                         + ("" if res["c"][1] == res["native"][1] else ", and their output differs") + f" ({where})")

    def mutate_one(self, seed, corpus):
        rnd = random.Random(seed)
        orig = rnd.choice(corpus)
        src = open(orig, "rb").read()
        if re.search(rb'^// stage1-only:', src, re.M):
            self.stat("stage1-only seed skipped")      # bastioc0 is meant to reject it
            return
        m = self.mutate(rnd, src)
        try:
            text = m.decode()
        except UnicodeDecodeError:
            return
        # Next to the original, so its imports resolve.
        d = os.path.dirname(orig)
        path = os.path.join(d, f".fuzz-{os.getpid()}-{seed}.bastio")
        with open(path, "wb") as f:
            f.write(m)
        try:
            where = f"mutate seed {seed}, from {os.path.relpath(orig, ROOT)}"
            c0, _ = self.both(text, path, d, where)
            if c0 != 0:
                self.stat("mutant rejected")
                return
            self.stat("mutant accepted")
            hosted = not re.search(rb'^// target:', m, re.M) and re.search(rb'^fn main\(\)', m, re.M) and b"Host" not in m and b"Serial" not in m
            if hosted:
                self.backends_agree(text, path, d, where)
            if not (hosted and self.san):
                return
            c = os.path.join(self.cur.tmp, "m.c")
            exe = os.path.join(self.cur.tmp, "m.san")
            if compile_([BASTIOC, "--emit-c", "-o", c, path], d)[0] != 0:
                return
            r = subprocess.run([CC, "-fsanitize=address,undefined", "-fno-sanitize-recover=all", "-g", "-w", "-o", exe, c], capture_output=True)
            if r.returncode != 0:
                self.finding("build", text, f"the C bastioc0 wrote does not compile ({where})\n{r.stderr.decode(errors='replace')[-2000:]}")
                return
            try:
                # Running out of memory aborts, by design: not a finding.
                env = dict(os.environ, ASAN_OPTIONS="allocator_may_return_null=1")
                r = subprocess.run([exe], capture_output=True, timeout=10, stdin=subprocess.DEVNULL, env=env)
            except subprocess.TimeoutExpired:
                return                      # termination is not proved
            if b"stack-overflow" in r.stderr:
                return                      # nor is the depth of recursion
            # A request the allocator cannot give is a warning here, and
            # `try_alloc` and `try_balloc` are meant to ask for one and see it fail.
            r.stderr = b"\n".join(l for l in r.stderr.split(b"\n") if b"AddressSanitizer failed to allocate" not in l)
            if b"Sanitizer" in r.stderr or b"runtime error" in r.stderr:
                self.finding("falseproof", text, f"accepted, but the sanitizers report ({where})\n{r.stderr.decode(errors='replace')[:3000]}")
            self.stat("mutant run")
        finally:
            os.remove(path)

    def one(self, n, mode, corpus):
        """Checks input n on this thread: returns what it counted and found."""
        self.cur.tmp = tempfile.mkdtemp(dir=self.tmp)
        self.cur.stats, self.cur.finds = [], []
        seed = self.a.seed * 1000003 + n
        try:
            if mode == "gen":
                self.gen_one(seed)
            else:
                self.mutate_one(seed, corpus)
        finally:
            shutil.rmtree(self.cur.tmp, ignore_errors=True)
        return self.cur.stats, self.cur.finds

    def go(self):
        corpus = self.corpus()
        modes = ["gen", "mutate"] if self.a.mode == "both" else [self.a.mode]
        jobs = max(1, int(os.environ.get("JOBS") or os.cpu_count() or 1))
        start = time.time()
        n = 0
        pending = collections.deque()
        with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
            while True:
                while len(pending) < 2 * jobs:
                    if self.a.seconds is not None and time.time() - start >= self.a.seconds:
                        break
                    if self.a.seconds is None and n >= self.a.count:
                        break
                    pending.append(pool.submit(self.one, n, modes[n % len(modes)], corpus))
                    n += 1
                if not pending:
                    break
                stats, finds = pending.popleft().result()
                for k in stats:
                    self.stats[k] = self.stats.get(k, 0) + 1
                for f in finds:
                    self.report(*f)
        shutil.rmtree(self.tmp, ignore_errors=True)
        print("fuzz: " + ", ".join(f"{v} {k}" for k, v in sorted(self.stats.items())))
        print(f"fuzz: {n} inputs, {self.found} finding(s), {time.time() - start:.0f} s")
        return 1 if self.found else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--count", type=int, default=100)
    ap.add_argument("--seconds", type=float)
    ap.add_argument("--mode", choices=["gen", "mutate", "both"], default="both")
    ap.add_argument("--bastioc1")
    ap.add_argument("--no-bastioc1", action="store_true")
    ap.add_argument("--keep", default="fuzz-findings")
    a = ap.parse_args()
    if a.no_bastioc1:
        a.bastioc1 = None
    elif not a.bastioc1:
        a.bastioc1 = os.path.join(tempfile.mkdtemp(prefix="bastioc1-"), "bastioc1")
        r = subprocess.run([BASTIOC, "-o", a.bastioc1, os.path.join(ROOT, "stage1/bastioc1.bastio")], capture_output=True)
        if r.returncode != 0:
            sys.exit("fuzz: cannot build bastioc1\n" + r.stderr.decode())
    else:
        a.bastioc1 = os.path.abspath(a.bastioc1)
    sys.exit(Fuzz(a).go())


main()
