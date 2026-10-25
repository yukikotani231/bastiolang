#!/usr/bin/env python3
"""Compares the AArch64 assembler (stage1/as64.bastio) with GNU as on a file of
instruction lines: every line must encode to the same word, and where GNU as
leaves a relocation the assembler must report the same type, symbol and
addend.

usage: tests/as64_forms.py <as64 binary> <file with one instruction per line>"""
import subprocess, sys, tempfile, os, struct, re

R = {
    'R_AARCH64_ADR_PREL_PG_HI21': 275, 'R_AARCH64_ADD_ABS_LO12_NC': 277,
    'R_AARCH64_LDST8_ABS_LO12_NC': 278, 'R_AARCH64_TSTBR14': 279,
    'R_AARCH64_CONDBR19': 280, 'R_AARCH64_JUMP26': 282, 'R_AARCH64_CALL26': 283,
    'R_AARCH64_LDST16_ABS_LO12_NC': 284, 'R_AARCH64_LDST32_ABS_LO12_NC': 285,
    'R_AARCH64_LDST64_ABS_LO12_NC': 286, 'R_AARCH64_ADR_GOT_PAGE': 311,
    'R_AARCH64_LD64_GOT_LO12_NC': 312,
}

def main():
    exe, forms = sys.argv[1], sys.argv[2]
    lines = [l.rstrip('\n') for l in open(forms)]
    with tempfile.TemporaryDirectory() as d:
        o = os.path.join(d, 'f.o')
        subprocess.run(['aarch64-linux-gnu-as', '-o', o, forms], check=True)
        b = os.path.join(d, 'f.bin')
        subprocess.run(['aarch64-linux-gnu-objcopy', '-O', 'binary', '-j', '.text', o, b], check=True)
        words = struct.unpack('<%dI' % (os.path.getsize(b) // 4), open(b, 'rb').read())
        rl = subprocess.run(['aarch64-linux-gnu-readelf', '-rW', o], check=True, capture_output=True, text=True).stdout
        rel = {}
        for l in rl.splitlines():
            m = re.match(r'^([0-9a-f]+)\s+[0-9a-f]+\s+(R_AARCH64_\w+)\s+[0-9a-f]+\s+(\S+)(?:\s*([+-])\s*([0-9a-f]+))?$', l.strip())
            if m:
                off = int(m.group(1), 16)
                add = int(m.group(5), 16) if m.group(5) else 0
                if m.group(4) == '-':
                    add = -add
                rel[off] = (R[m.group(2)], m.group(3), add)
        mine = subprocess.run([exe, forms], check=True, capture_output=True, text=True).stdout.split('\n')
    bad = 0
    # GNU as: one word per line
    if len(words) != len(lines):
        print('as64_forms.py: GNU as made %d words for %d lines' % (len(words), len(lines)))
        sys.exit(1)
    for i, l in enumerate(lines):
        ref = '%08x' % words[i]
        if i * 4 in rel:
            t, s, a = rel[i * 4]
            ref += ' R %d %s %d' % (t, s, a)
        if mine[i] != ref:
            bad += 1
            if bad <= 30:
                print('%-50s GNU %s  mine %s' % (l, ref, mine[i]))
    if bad:
        print('as64_forms.py: FAILED: %d of %d lines differ' % (bad, len(lines)))
        sys.exit(1)
    print('as64_forms.py: %d distinct instruction lines encode identically to GNU as' % len(lines))

main()
