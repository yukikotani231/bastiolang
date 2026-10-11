#!/usr/bin/env python3
# Mutation test for a stage1 phase: damages every Bastio source in a few
# random ways and checks that the stage1 tool and stage0 (with FLAG) still
# print the same thing, which for most mutants is the position of the first
# error. Mutations: a word deleted, duplicated or cut off; punctuation
# dropped in; one identifier replaced by another from the same file; a
# `[..]` group removed. A mutant is written next to its original (as a
# hidden file, removed afterwards) so that its imports still resolve.
# Mutants that are not valid UTF-8 are skipped: stage0 cannot even read
# them. Deterministic: the seed and the number of mutants per file are
# arguments.
#   usage: mutate.py [--via DRIVER] [--smt | --verify | --driver] BASTIOC FLAG TOOL SEED PER_FILE FILE...
# With --via, each mutant temporarily replaces its original file (restored
# afterwards) and both compilers run on DRIVER, a program that imports it:
# that is how library files without a `main` reach the checker.
# FLAG and TOOL may hold several words (`--dump-check --target kernel`).
# With --smt (for the verifier: FLAG is `--check --dump-smt ..`), only the
# mutants stage0 verifies are compared, and stage0's count of proof
# obligations (on stderr) is part of what must match. With --verify (FLAG is
# `--dump-verify ..`), only the mutants that reach the verifier are compared:
# those stage0 verifies, and those it rejects with a failed proof. With
# --driver (TOOL is bastioc1 and FLAG `--check ..`), what both print on
# stderr and their exit status are compared, masked as difftest.sh masks
# them (the parts bastioc1 does not print yet).
import concurrent.futures, os, queue, random, re, shutil, subprocess, sys

args = sys.argv[1:]
via = None
if args[0] == '--via':
    via, args = args[1], args[2:]
smt = args[0] == '--smt'
verify = args[0] == '--verify'
driver = args[0] == '--driver'
if smt or verify or driver:
    args = args[1:]
bastioc, flag, tool, seed, per_file = args[0], args[1], args[2], int(args[3]), int(args[4])
random.seed(seed)
PUNCT = [b'(', b')', b'{', b'}', b'[', b']', b',', b';', b':', b'::', b'&', b'mut ', b'.', b'=',
         b'->', b'=>', b'_', b'1', b'x', b'is ', b'forall ']
IDENT = re.compile(rb'[A-Za-z_][A-Za-z0-9_]*')

def mask(want):
    # Mirrors mask() in difftest.sh.
    out = []
    for l in want.split(b'\n'):
        l = re.sub(rb'^(.*: error\[E-SOLVER\]): .*$', rb'\1', l)
        l = re.sub(rb'^(.*: error\[E-IMPORT\]: cannot read `[^`]*`): .*$', rb'\1', l)
        l = re.sub(rb'^(.*: error\[E-PARSE\]: include_bytes: cannot read .*): [^:]*$', rb'\1', l)
        l = re.sub(rb'^bastioc0: ', b'bastioc1: ', l)
        out.append(l)
    return b'\n'.join(out)

def mutate(src):
    r = random.random()
    if r < 0.2:
        idents = list(IDENT.finditer(src))
        if idents:
            a = random.choice(idents)
            b = random.choice(idents).group()
            return src[:a.start()] + b + src[a.end():]
    if r < 0.3:
        groups = list(re.finditer(rb'\[[^\[\]\n]*\]', src))
        if groups:
            g = random.choice(groups)
            return src[:g.start()] + src[g.end():]
    w = src.split(b' ')
    i = random.randrange(len(w))
    if r < 0.5:
        del w[i]
    elif r < 0.65:
        w.insert(i, w[random.randrange(len(w))])
    elif r < 0.8:
        w = w[:i]
    else:
        s = bytearray(src)
        j = random.randrange(len(s))
        s[j:j + 1] = random.choice(PUNCT)
        return bytes(s)
    return b' '.join(w)

# Every mutant is drawn first, in order, so the random sequence (and with
# it the set of mutants) does not depend on how many run at once. They then
# run JOBS at a time (default: the number of CPUs). A mutant is written to a
# file of its own: next to its original, or with --via into a private copy
# of the original's directory, made next to it (`.mutant_N_DIR`), so that
# imports resolve as they do from the original and the original is never
# touched.
mutants = []
for f in args[5:]:
    src = open(f, 'rb').read()
    for _ in range(per_file):
        m = mutate(src)
        try:
            m.decode('utf-8')
        except UnicodeDecodeError:
            continue
        mutants.append((f, m))

jobs = max(1, int(os.environ.get('JOBS') or os.cpu_count() or 1))
copies = queue.Queue()
made = []
if via:
    d = os.path.dirname(via) or '.'
    if any(os.path.dirname(f) != os.path.dirname(via) for f, _ in mutants):
        sys.exit('mutate.py: with --via, every FILE must be in the directory of DRIVER')
    for k in range(jobs):
        c = os.path.join(os.path.dirname(os.path.abspath(d)), '.mutant_%d_%d_%s' % (os.getpid(), k, os.path.basename(os.path.abspath(d))))
        shutil.rmtree(c, ignore_errors=True)
        shutil.copytree(d, c, symlinks=True)
        made.append(c)
        copies.put(c)

def run(i):
    f, m = mutants[i]
    if via:
        c = copies.get()
        rel = os.path.relpath(c)
        path = os.path.join(rel, os.path.basename(f))
        target = os.path.join(rel, os.path.basename(via))
    else:
        path = os.path.join(os.path.dirname(f), '.mutant_%d_%s' % (i, os.path.basename(f)))
        target = path
    open(path, 'wb').write(m)
    try:
        w = subprocess.run([bastioc] + flag.split() + [target], capture_output=True)
        want = w.stdout
        if smt:
            n = re.search(rb': (\d+) proof obligation', w.stderr)
            if w.returncode != 0 or not n:
                return None
            want += n.group(1) + b' proof obligation(s)\n'
        if verify and not re.search(rb'proof obligation|^error \S+ E-(VERIFY|SOLVER|EXPORT)', want, re.M):
            return None
        if driver:
            g = subprocess.run(tool.split() + flag.split() + [target], capture_output=True)
            got = g.stderr + b'exit %d\n' % g.returncode
            want = mask(w.stderr + b'exit %d\n' % w.returncode)
        else:
            got = subprocess.run(tool.split() + [target], capture_output=True).stdout
    finally:
        if via:
            shutil.copyfile(f, path)
            copies.put(c)
        else:
            os.remove(path)
    return want, got

try:
    with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        results = list(pool.map(run, range(len(mutants))))
finally:
    for c in made:
        shutil.rmtree(c, ignore_errors=True)

runs = differ = errors = unverified = 0
codes = {}
for (f, m), r in zip(mutants, results):
    if r is None:
        unverified += 1
        continue
    want, got = r
    runs += 1
    err = [l for l in want.split(b'\n') if l.startswith(b'error ')]
    if driver:
        err = [re.search(rb'error\[([A-Z-]*)\]', l) for l in want.split(b'\n')]
        err = [b'error x ' + e.group(1) for e in err if e]
    if err:
        errors += 1
        for code in err[0].split()[2:3]:
            codes[code.decode()] = codes.get(code.decode(), 0) + 1
    if want != got:
        differ += 1
        print('DIFFER: a mutant of %s (%s):' % (f, flag))
        print('    ' + m.decode('utf-8', 'replace').replace('\n', '\n    ')[:3000])
tally = ''.join(', %d %s' % (n, c) for c, n in sorted(codes.items()))
if smt:
    tally += ', %d not verified by stage0 (skipped)' % unverified
if verify:
    tally += ', %d not reaching the verifier (skipped)' % unverified
print('-- %s%s mutants: %d run (%d of them errors%s), %d differ' % (flag, ' via ' + via if via else '', runs, errors, tally, differ))
sys.exit(1 if differ else 0)
