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
#   usage: mutate.py BASTIOC FLAG TOOL SEED PER_FILE FILE...
import os, random, re, subprocess, sys

bastioc, flag, tool, seed, per_file = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4]), int(sys.argv[5])
random.seed(seed)
PUNCT = [b'(', b')', b'{', b'}', b'[', b']', b',', b';', b':', b'::', b'&', b'mut ', b'.', b'=',
         b'->', b'=>', b'_', b'1', b'x', b'is ', b'forall ']
IDENT = re.compile(rb'[A-Za-z_][A-Za-z0-9_]*')

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

runs = differ = errors = 0
codes = {}
for f in sys.argv[6:]:
    src = open(f, 'rb').read()
    path = os.path.join(os.path.dirname(f), '.mutant_' + os.path.basename(f))
    for _ in range(per_file):
        m = mutate(src)
        try:
            m.decode('utf-8')
        except UnicodeDecodeError:
            continue
        open(path, 'wb').write(m)
        try:
            want = subprocess.run([bastioc, flag, path], capture_output=True).stdout
            got = subprocess.run([tool, path], capture_output=True).stdout
        finally:
            os.remove(path)
        runs += 1
        err = [l for l in want.split(b'\n') if l.startswith(b'error ')]
        if err:
            errors += 1
            for code in err[0].split()[2:3]:
                codes[code.decode()] = codes.get(code.decode(), 0) + 1
        if want != got:
            differ += 1
            print('DIFFER: a mutant of %s (%s):' % (f, flag))
            print('    ' + m.decode('utf-8', 'replace').replace('\n', '\n    ')[:3000])
tally = ''.join(', %d %s' % (n, c) for c, n in sorted(codes.items()))
print('-- %s mutants: %d run (%d of them errors%s), %d differ' % (flag, runs, errors, tally, differ))
sys.exit(1 if differ else 0)
