#!/usr/bin/env python3
"""Asks a second SMT solver (cvc5) the questions the compiler asks Z3.

Every proof in Bastio is an `unsat` answer from Z3. This script replays the
exact SMT-LIB session the compiler sends (`bastioc0 --check --dump-smt`) to Z3
and to cvc5, which shares no code with Z3, and compares the answers one
`check-sat` at a time:

  agree      both say unsat: the proof was confirmed by an independent solver
  unchecked  cvc5 gave up (unknown / its time limit) or the query uses Z3-only
             syntax (`lambda`, see docs/trusted-base-reduction.md), so it was
             not confirmed
  DISAGREE   Z3 says unsat and cvc5 says sat, or Z3 does not say unsat at all.
             Either solver is wrong or the encoding is outside what they both
             understand; this fails the run

usage: tests/crosscheck.py [-j N] [--tlimit MS] [--min-agree PCT] [file.bastio ...]
(default: every tests/pass program that stage0 compiles)
"""
import argparse, os, re, subprocess, sys, tempfile
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASTIOC0 = os.environ.get("CROSSCHECK_BASTIOC") or os.path.join(ROOT, "stage0/target/release/bastioc0")
OPTION = re.compile(r"^\(set-option :(rlimit|timeout) ")
ANSWER = re.compile(r"^(sat|unsat|unknown)$")


EXTRA = []


def session(path):
    """The commands the compiler sends for `path`, one per line, or None."""
    p = subprocess.run([BASTIOC0, "--check", "--dump-smt", *EXTRA, path],
                       capture_output=True, text=True)
    if p.returncode != 0:
        return None
    return [l for l in p.stdout.splitlines() if l.startswith("(") and not OPTION.match(l)]


TOKEN = re.compile(r"\(|\)|\|[^|]*\||[^\s()]+")


def parse(line):
    stack, cur = [], []
    for t in TOKEN.findall(line):
        if t == "(":
            stack.append(cur)
            cur = []
        elif t == ")":
            done, cur = cur, stack.pop()
            cur.append(done)
        else:
            cur.append(t)
    return cur[0]


def show(t):
    return t if isinstance(t, str) else "(" + " ".join(show(x) for x in t) + ")"


BINDERS = ("lambda", "forall", "exists")


def subst(t, k, v):
    if isinstance(t, str):
        return v if t == k else t
    if t and t[0] in BINDERS and any(b[0] == k for b in t[1]):
        return t  # k is bound again inside: nothing to replace
    return [subst(x, k, v) for x in t]


def beta(t):
    """Reads of a lambda array, `(select (lambda ((k Int)) B) i)`, become B
    with i for k. This is plain rewriting that keeps the query's meaning, and
    it leaves cvc5 a query without the Z3-only `lambda`."""
    if isinstance(t, str):
        return t
    t = [beta(x) for x in t]
    if (len(t) == 3 and t[0] == "select" and isinstance(t[1], list) and len(t[1]) == 3
            and t[1][0] == "lambda" and len(t[1][1]) == 1):
        return beta(subst(t[1][2], t[1][1][0][0], t[2]))
    return t


def reduce_lambdas(cmds):
    return [show(beta(parse(c))) if "lambda" in c else c for c in cmds]


def cvc5_view(cmds):
    """The same session for cvc5, plus which check-sats cvc5 cannot answer.

    A command with Z3-only syntax is replaced by `(assert true)`. That weakens
    the session, so every check-sat it can affect is marked unchecked: those
    in the same push scope, or all later ones when it sat outside any scope
    (until the next `(reset)`)."""
    out, skip, depth, tainted = [], [], 0, []  # tainted[d]: scope d lost a command
    base = False
    for c in cmds:
        if c.startswith("(reset"):
            depth, tainted, base = 0, [], False
        elif c.startswith("(push"):
            depth += 1
            tainted.append(False)
        elif c.startswith("(pop"):
            depth -= 1
            tainted.pop()
        if "lambda" in c:
            out.append("(assert true)")
            if depth == 0:
                base = True
            else:
                tainted[-1] = True
            continue
        out.append(c)
        if c.startswith("(check-sat"):
            skip.append(base or any(tainted))
    return out, skip


def run(solver_cmd, cmds):
    with tempfile.NamedTemporaryFile("w", suffix=".smt2", delete=False) as f:
        f.write("\n".join(cmds) + "\n")
    try:
        p = subprocess.run(solver_cmd + [f.name], capture_output=True, text=True, timeout=1800)
    finally:
        os.unlink(f.name)
    return [l for l in p.stdout.splitlines() if ANSWER.match(l)]


CHUNK = 300  # check-sats per piece of a long session
LEAVES = None  # the pool that runs the pieces, set by main


def pieces(cmds):
    """A session cut at `(reset)`, which forgets every declaration, so the
    pieces are independent; each holds about CHUNK queries."""
    out, cur, n = [], [], 0
    for c in cmds:
        if c.startswith("(reset") and n >= CHUNK:
            out.append(cur)
            cur, n = [], 0
        cur.append(c)
        n += c.startswith("(check-sat")
    out.append(cur)
    return out


def answers(cmds, tlimit):
    z3 = run(["z3", "-smt2", "-T:600"], cmds)
    view, skip = cvc5_view(reduce_lambdas(cmds))
    cv = run(["cvc5", "--lang", "smt2", "--incremental", f"--tlimit-per={tlimit}"], view)
    return z3, cv, skip


def check(path, tlimit, cmds=None):
    cmds = cmds or session(path)
    if cmds is None:
        return None
    ps = pieces(cmds)
    parts = list(LEAVES.map(lambda p: answers(p, tlimit), ps)) if LEAVES else [answers(p, tlimit) for p in ps]
    z3, cv, skip = [], [], []
    for (zz, cc, ss), p in zip(parts, ps):
        k = sum(1 for c in p if c.startswith("(check-sat"))
        z3 += (zz + ["missing"] * k)[:k]
        cv += (cc + ["missing"] * k)[:k]
        skip += ss
    n = len(skip)
    agree = unchecked = 0
    bad = []
    for i in range(n):
        a = z3[i] if i < len(z3) else "missing"
        b = cv[i]
        if a != "unsat":
            bad.append((i, a, b))
        elif skip[i] or b in ("unknown", "missing"):
            unchecked += 1
        elif b == "unsat":
            agree += 1
        else:
            bad.append((i, a, b))
    return agree, unchecked, bad


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="*")
    ap.add_argument("-j", type=int, default=os.cpu_count())
    ap.add_argument("--tlimit", type=int, default=20000, help="cvc5 time limit per query, ms")
    ap.add_argument("--min-agree", type=float, default=0.0,
                    help="fail if fewer than this percentage of queries are confirmed")
    ap.add_argument("--args", default="", help="extra bastioc0 flags, e.g. \"--target kernel --lib DIR\"")
    ap.add_argument("--selftest", action="store_true", help="check that a wrong proof is reported")
    a = ap.parse_args()
    EXTRA.extend(a.args.split())
    if a.selftest:
        wrong = ["(declare-const x Int)", "(push 1)", "(assert (> x 0))", "(check-sat)", "(pop 1)"]
        right = ["(declare-const x Int)", "(push 1)", "(assert (and (> x 0) (< x 0)))", "(check-sat)", "(pop 1)"]
        ok = check("", a.tlimit, wrong)[2] and check("", a.tlimit, right)[:2] == (1, 0)
        print("crosscheck selftest:", "ok" if ok else "FAILED")
        sys.exit(0 if ok else 1)
    files = a.files or sorted(
        os.path.join(ROOT, "tests/pass", f) for f in os.listdir(os.path.join(ROOT, "tests/pass"))
        if f.endswith(".bastio"))
    global LEAVES
    LEAVES = ThreadPoolExecutor(a.j)
    with ThreadPoolExecutor(max(1, a.j // 2)) as ex:
        results = list(ex.map(lambda f: check(f, a.tlimit), files))
    tot_a = tot_u = 0
    failed = False
    for f, r in zip(files, results):
        if r is None:
            continue
        ag, un, bad = r
        tot_a += ag
        tot_u += un
        for i, x, y in bad:
            failed = True
            print(f"DISAGREE {os.path.relpath(f, ROOT)}: query {i}: z3 {x}, cvc5 {y}")
    total = tot_a + tot_u
    pct = 100.0 * tot_a / total if total else 0.0
    print(f"crosscheck: {tot_a} of {total} proofs confirmed by cvc5 ({pct:.1f}%), "
          f"{tot_u} unchecked, {len(files)} programs")
    if failed:
        sys.exit(1)
    if pct < a.min_agree:
        print(f"crosscheck: fewer than {a.min_agree}% confirmed")
        sys.exit(1)


main()
