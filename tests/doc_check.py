"""Checks the parts of docs/*.md that tests/run.sh's block loop does not.

usage: doc_check.py [--fix] BASTIOC DOC.md...

Prints one FAIL line per problem and, last, "doc_check: P passed, F failed".
With --fix, transcripts that differ are replaced by what the compiler
prints (after a change to how errors are shown); review the diff.

Two kinds of check:

- Error transcripts. A ```text block that directly follows a ```bastio
  block and starts with `NAME.bastio:L:C: error[` (optionally after a
  `$ bastioc0 ...` line) is what the compiler prints for that program.
  The program, without its `// expect-...` and `// target: ...` header
  lines, is saved as NAME.bastio and checked, and the output must match
  line for line. Counterexample values depend on the Z3 version, so in a
  `= counterexample:` note only the names must match.

- Copies of files. A block whose fence line comes right after a line
  `<!-- file: PATH -->` is a copy of PATH (relative to the repository root),
  again without header lines, and must be identical to it.
"""

import os
import re
import subprocess
import sys
import tempfile

HEADER = re.compile(r"^// (expect-[a-z]+:|expect-ok$|target:)")
FILE_MARK = re.compile(r"^<!-- file: (\S+) -->$")
ERROR_LINE = re.compile(r"^(\S+\.bastio):\d+:\d+: error\[")


def blocks(lines):
    """(language, line number of the fence, body lines, file marker or None)"""
    out = []
    i = 0
    while i < len(lines):
        m = re.match(r"^```(\w*)$", lines[i])
        if not m:
            i += 1
            continue
        j = i + 1
        while j < len(lines) and not lines[j].startswith("```"):
            j += 1
        mark = FILE_MARK.match(lines[i - 1]) if i > 0 else None
        out.append((m.group(1), i + 1, lines[i + 1:j], mark and mark.group(1)))
        i = j + 1
    return out


def strip_header(body):
    target = None
    k = 0
    while k < len(body) and HEADER.match(body[k]):
        m = re.match(r"^// target: *(\S+)", body[k])
        if m:
            target = m.group(1)
        k += 1
    return body[k:], target


CEX = re.compile(r"^( *= counterexample: )(.*)$")


def same_line(got, want):
    g, w = CEX.match(got), CEX.match(want)
    if g and w and g.group(1) == w.group(1):
        names = lambda s: [p.split(" = ")[0] for p in s.split(", ")]
        return names(g.group(2)) == names(w.group(2))
    return got == want


def main():
    fix = len(sys.argv) > 1 and sys.argv[1] == "--fix"
    args = sys.argv[2:] if fix else sys.argv[1:]
    bastioc, docs = args[0], args[1:]
    if "/" in bastioc:
        bastioc = os.path.abspath(bastioc)
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    passed = failed = 0

    def fail(at, msg, detail=()):
        nonlocal failed
        failed += 1
        print(f"FAIL {at}: {msg}")
        for d in detail:
            print("    " + d)

    tmp = tempfile.mkdtemp()
    for doc in docs:
        lines = open(doc).read().split("\n")
        bs = blocks(lines)
        edits = []
        for k, (lang, line, body, mark) in enumerate(bs):
            at = f"{doc}:{line}"
            if mark:
                path = os.path.join(root, mark)
                if not os.path.exists(path):
                    fail(at, f"{mark} does not exist")
                    continue
                want = open(path).read().rstrip("\n").split("\n")
                if strip_header(body)[0] == want:
                    passed += 1
                else:
                    fail(at, f"block differs from {mark}; copy the file into the block")
                continue
            if lang != "text" or k == 0 or bs[k - 1][0] != "bastio":
                continue
            shown = body[1:] if body and body[0].startswith("$ ") else body
            m = ERROR_LINE.match(shown[0]) if shown else None
            if not m:
                continue
            src, target = strip_header(bs[k - 1][2])
            name = m.group(1)
            with open(os.path.join(tmp, name), "w") as f:
                f.write("\n".join(src) + "\n")
            r = subprocess.run(
                [bastioc, "--check", "--target", target or "hosted", name],
                cwd=tmp, capture_output=True, text=True)
            got = r.stderr.rstrip("\n").split("\n")
            if len(got) == len(shown) and all(map(same_line, got, shown)):
                passed += 1
            elif fix:
                start = line + (len(body) - len(shown))
                edits.append((start, start + len(shown), got))
                passed += 1
            else:
                fail(at, f"the compiler's output for {name} differs from this transcript",
                     ["got:"] + got + ["shown:"] + shown)
        if edits:
            for a, b, got in reversed(edits):
                lines[a:b] = got
            with open(doc, "w") as f:
                f.write("\n".join(lines))
            print(f"doc_check: rewrote {len(edits)} transcript(s) in {doc}")
    print(f"doc_check: {passed} passed, {failed} failed")
    return 1 if failed else 0


sys.exit(main())
