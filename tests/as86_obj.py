#!/usr/bin/env python3
"""Assembles whole assembly files with GNU as and with stage1's `as86 --obj`
and compares what an object file holds: the bytes of .text, .rodata and .data,
the size of .bss, and the relocations (section, offset, type, symbol, addend).

usage: tests/as86_obj.py AS86 FILE.s...
"""
import re
import subprocess
import sys
import tempfile
import os

RTYPES = {
    "R_X86_64_PC32": 1,
    "R_X86_64_PLT32": 2,
    "R_X86_64_GOTPCREL": 3,
    "R_X86_64_GOTPCRELX": 3,
    "R_X86_64_REX_GOTPCRELX": 3,
}


def gnu(path, tmp):
    obj = os.path.join(tmp, "g.o")
    subprocess.run(["as", "--64", "-o", obj, path], check=True)
    secs = {}
    for name in (".text", ".rodata", ".data"):
        out = os.path.join(tmp, "s.bin")
        r = subprocess.run(["objcopy", "-O", "binary", "-j", name, obj, out],
                           capture_output=True)
        secs[name] = open(out, "rb").read().hex() if r.returncode == 0 and os.path.exists(out) else ""
        if os.path.exists(out):
            os.remove(out)
    sh = subprocess.run(["readelf", "-SW", obj], capture_output=True, text=True).stdout
    bss = 0
    for line in sh.splitlines():
        m = re.search(r"\.bss\s+NOBITS\s+\S+\s+\S+\s+(\S+)", line)
        if m:
            bss = int(m.group(1), 16)
    rel = []
    cur = None
    for line in subprocess.run(["readelf", "-rW", obj], capture_output=True, text=True).stdout.splitlines():
        m = re.match(r"Relocation section '\.rela(\.\S+)'", line)
        if m:
            cur = m.group(1)
            continue
        m = re.match(r"([0-9a-f]{16})\s+[0-9a-f]+\s+(\S+)\s+[0-9a-f]+\s+(\S+)(?: ([+-]) (\w+))?", line)
        if m and cur:
            add = int(m.group(5), 16) if m.group(5) else 0
            if m.group(4) == "-":
                add = -add
            rel.append((cur, int(m.group(1), 16), RTYPES[m.group(2)], m.group(3), add))
    return secs, bss, rel


def mine(as86, path):
    out = subprocess.run([as86, "--obj", path], capture_output=True, text=True).stdout.splitlines()
    secs = {}
    bss = 0
    rel = []
    errs = None
    i = 0
    while i < len(out):
        f = out[i].split()
        if f[0] == "S":
            if f[1] == ".bss":
                bss = int(f[2])
                i += 1
            else:
                secs[f[1]] = out[i + 1].strip()
                i += 2
        elif f[0] == "R":
            rel.append((f[1], int(f[2]), int(f[3]), f[4], int(f[5])))
            i += 1
        elif f[0] == "E":
            errs = (int(f[1]), int(f[2]))
            i += 1
        else:
            i += 1
    return secs, bss, rel, errs


def main():
    as86 = sys.argv[1]
    bad = 0
    for path in sys.argv[2:]:
        with tempfile.TemporaryDirectory() as tmp:
            gs, gb, gr = gnu(path, tmp)
        ms, mb, mr, errs = mine(as86, path)
        problems = []
        if errs and errs[0]:
            problems.append("assembler error on line %d (%d errors)" % (errs[1], errs[0]))
        for name in (".text", ".rodata", ".data"):
            if gs[name] != ms.get(name, ""):
                a, b = gs[name], ms.get(name, "")
                at = next((k for k in range(min(len(a), len(b))) if a[k] != b[k]), min(len(a), len(b)))
                problems.append("%s differs: first byte %d of %d (gnu) / %d (mine)" % (name, at // 2, len(a) // 2, len(b) // 2))
        if gb != mb:
            problems.append(".bss: gnu %d, mine %d" % (gb, mb))
        if sorted(gr) != sorted(mr):
            gset, mset = set(gr), set(mr)
            only_g = sorted(gset - mset)[:3]
            only_m = sorted(mset - gset)[:3]
            problems.append("relocations differ: gnu %d, mine %d; only gnu %s; only mine %s" % (len(gr), len(mr), only_g, only_m))
        if problems:
            bad += 1
            print("%s: %s" % (path, "; ".join(problems)))
    if bad:
        print("as86_obj.py: %d of %d files differ" % (bad, len(sys.argv) - 2))
        sys.exit(1)
    print("as86_obj.py: %d files are identical to GNU as" % (len(sys.argv) - 2))


main()
