#!/usr/bin/env python3
# Mutation test for the parser: damages every Bastio source in a few
# random ways (a word deleted, duplicated or cut off, punctuation dropped
# in) and checks that astdump and `bastioc0 --dump-ast` still print the
# same thing, which for most mutants is the position of the first error.
# Deterministic: the seed and the number of mutants per file are arguments.
#   usage: mutate.py BASTIOC ASTDUMP SEED PER_FILE FILE...
import os, random, subprocess, sys, tempfile

bastioc, astdump, seed, per_file = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
random.seed(seed)
PUNCT = [b'(', b')', b'{', b'}', b'[', b']', b',', b';', b':', b'::', b'&', b'mut ', b'.', b'=',
         b'->', b'=>', b'_', b'1', b'x', b'is ', b'forall ']
runs = differ = errors = 0
with tempfile.TemporaryDirectory() as tmp:
    path = os.path.join(tmp, 'm.bastio')
    for f in sys.argv[5:]:
        words = open(f, 'rb').read().split(b' ')
        for _ in range(per_file):
            w = list(words)
            r = random.random()
            i = random.randrange(len(w))
            if r < 0.3:
                del w[i]
            elif r < 0.5:
                w.insert(i, w[random.randrange(len(w))])
            elif r < 0.7:
                w = w[:i]
            else:
                s = bytearray(b' '.join(w))
                j = random.randrange(len(s))
                s[j:j + 1] = random.choice(PUNCT)
                w = [bytes(s)]
            m = b' '.join(w)
            open(path, 'wb').write(m)
            want = subprocess.run([bastioc, '--dump-ast', path], capture_output=True).stdout
            got = subprocess.run([astdump, path], capture_output=True).stdout
            runs += 1
            if b'error ' in want:
                errors += 1
            if want != got:
                differ += 1
                print('DIFFER: a mutant of %s:' % f)
                print('    ' + m.decode('utf-8', 'replace').replace('\n', '\n    ')[:2000])
print('-- parser mutants: %d run (%d of them errors), %d differ' % (runs, errors, differ))
sys.exit(1 if differ else 0)
