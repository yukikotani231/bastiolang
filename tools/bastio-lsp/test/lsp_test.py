#!/usr/bin/env python3
"""Drives bastio-lsp over standard input and output.

usage: lsp_test.py LSP BASTIOC

LSP is the language server's executable, BASTIOC the compiler it runs for
diagnostics. Prints one FAIL line per problem and exits 1 if there is any.
"""
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LSP, BASTIOC = sys.argv[1], sys.argv[2]
fails = []


def check(cond, what):
    if not cond:
        fails.append(what)
        print("FAIL", what)


def frame(m):
    b = json.dumps(m).encode()
    return b"Content-Length: %d\r\n\r\n" % len(b) + b


def parse(out):
    msgs, i = [], 0
    while i < len(out):
        h = out.index(b"\r\n\r\n", i)
        n = int(out[i:h].split(b":")[1])
        msgs.append(json.loads(out[h + 4:h + 4 + n]))
        i = h + 4 + n
    return msgs


def session(steps, raw=None):
    """Runs the requests in `steps` (a list of (method, params, is_request));
    returns the responses by id, the notifications and the exit status."""
    msgs = [{"jsonrpc": "2.0", "id": 1, "method": "initialize",
             "params": {"initializationOptions": {"bastioc": BASTIOC}}}]
    for k, (method, params, req) in enumerate(steps):
        m = {"jsonrpc": "2.0", "method": method, "params": params}
        if req:
            m["id"] = 100 + k
        msgs.append(m)
    msgs.append({"jsonrpc": "2.0", "id": 999, "method": "shutdown"})
    msgs.append({"jsonrpc": "2.0", "method": "exit"})
    data = b"".join(frame(m) for m in msgs) if raw is None else raw
    p = subprocess.run([LSP], input=data, stdout=subprocess.PIPE, timeout=300)
    out = parse(p.stdout)
    resp = {m["id"]: m for m in out if "id" in m}
    notes = [m for m in out if "id" not in m]
    return resp, notes, p.returncode


def uri(name):
    return "file://" + os.path.join(HERE, name)


def text(name):
    with open(os.path.join(HERE, name)) as f:
        return f.read()


def where(name, needle, nth=1):
    """The (line, character) of the start of the nth `needle` in file `name`."""
    t, at = text(name), -1
    for _ in range(nth):
        at = t.index(needle, at + 1)
    line = t.count("\n", 0, at)
    return {"line": line, "character": at - (t.rfind("\n", 0, at) + 1)}


def open_doc(name):
    return ("textDocument/didOpen", {"textDocument": {
        "uri": uri(name), "languageId": "bastio", "version": 1, "text": text(name)}}, False)


def request(method, name, pos=None):
    p = {"textDocument": {"uri": uri(name)}}
    if pos is not None:
        p["position"] = pos
    return ("textDocument/" + method, p, True)


# ---- A program that is fine: queries ----
steps = [
    open_doc("main.bastio"),
    request("hover", "main.bastio", where("main.bastio", "double")),
    request("definition", "main.bastio", where("main.bastio", "double")),
    request("definition", "main.bastio", where("main.bastio", "Pair")),
    request("definition", "main.bastio", where("main.bastio", "Line", 2)),
    request("definition", "main.bastio", where("main.bastio", "len", 2)),
    request("hover", "main.bastio", where("main.bastio", "LIMIT")),
    request("hover", "main.bastio", where("main.bastio", "n)")),
    request("definition", "main.bastio", where("main.bastio", "a)")),
    request("hover", "main.bastio", {"line": 1, "character": 0}),
    request("documentSymbol", "lib.bastio"),
    request("formatting", "main.bastio"),
]
# The symbols are asked for a document that is not open: it must be.
steps.insert(1, open_doc("lib.bastio"))
resp, notes, rc = session(steps)
check(rc == 0, "exit status 0 after shutdown and exit")
caps = resp[1]["result"]["capabilities"]
check(caps["hoverProvider"] and caps["definitionProvider"] and caps["documentFormattingProvider"],
      "capabilities")
diags = [n for n in notes if n["method"] == "textDocument/publishDiagnostics"]
check(len(diags) == 2 and all(d["params"]["diagnostics"] == [] for d in diags),
      "a correct program and a correct library have no diagnostics: %r" % diags)

h = resp[102]["result"]["contents"]["value"]
check("fn double(x: u64) -> u64\n  requires x < 1000" in h, "hover shows the signature: %r" % h)
check("Doubles `x`." in h and "small enough" in h, "hover shows the doc comment: %r" % h)

d = resp[103]["result"]
check(d["uri"] == uri("lib.bastio") and d["range"]["start"] == where("lib.bastio", "double"),
      "go to a function in an imported file: %r" % d)
d = resp[104]["result"]
check(d["uri"] == uri("lib.bastio") and d["range"]["start"] == where("lib.bastio", "Pair"),
      "go to a struct: %r" % d)
d = resp[105]["result"]
check(d["uri"] == uri("lib.bastio") and d["range"]["start"] == where("lib.bastio", "Line"),
      "go to a variant: %r" % d)
d = resp[106]["result"]
check(d["uri"] == uri("main.bastio") and d["range"]["start"] == where("main.bastio", "len"),
      "go to a match binding: %r" % d)
h = resp[107]["result"]["contents"]["value"]
check("const LIMIT: u64" in h, "hover on a constant: %r" % h)
h = resp[108]["result"]["contents"]["value"]
check("let n" in h and "local variable" in h, "hover on a local: %r" % h)
d = resp[109]["result"]
check(d["uri"] == uri("lib.bastio") and d["range"]["start"] == where("lib.bastio", "a:"),
      "go to a field: %r" % d)
check(resp[110]["result"] is None, "nothing to say about a keyword")
syms = {s["name"]: s for s in resp[111]["result"]}
check(set(syms) == {"double", "Pair", "Shape", "LIMIT"}, "document symbols: %r" % list(syms))
check([c["name"] for c in syms["Shape"]["children"]] == ["Dot", "Line"], "enum variants are children")
check([c["name"] for c in syms["Pair"]["children"]] == ["a", "b"], "struct fields are children")
check(resp[112]["result"] == [], "a formatted file has no formatting edits")

# ---- Diagnostics ----
resp, notes, rc = session([open_doc("bad.bastio")])
d = notes[0]["params"]["diagnostics"]
check(len(d) == 1 and d[0]["code"] == "E-TYPE" and d[0]["range"]["start"] == where("bad.bastio", "true"),
      "a type error is reported where the compiler says: %r" % d)
check(d[0]["range"]["end"]["character"] - d[0]["range"]["start"]["character"] == 4,
      "its range is as wide as the compiler's carets")

resp, notes, rc = session([open_doc("proof.bastio")])
d = notes[0]["params"]["diagnostics"]
check(len(d) == 1 and d[0]["code"] == "E-VERIFY-BOUNDS" and "len(b) = 4" in d[0]["message"],
      "a failed proof comes with its counterexample: %r" % d)

# A library (no `main`) is checked through a program that imports it.
resp, notes, rc = session([open_doc("lib.bastio")])
check(notes[0]["params"]["diagnostics"] == [], "a library with correct proofs has no diagnostics")

broken = "fn main() -> u64 { return 1 # 2; }\n"
# The lexer alone, as the text changes, without running the compiler.
resp, notes, rc = session([
    open_doc("main.bastio"),
    ("textDocument/didChange", {"textDocument": {"uri": uri("main.bastio")},
                                "contentChanges": [{"text": broken}]}, False),
    ("textDocument/didClose", {"textDocument": {"uri": uri("main.bastio")}}, False),
])
ds = [n["params"]["diagnostics"] for n in notes]
check(len(ds) == 3 and ds[0] == [] and ds[2] == [], "open and close publish no diagnostics: %r" % ds)
check(len(ds[1]) == 1 and "unexpected character `#`" in ds[1][0]["message"]
      and ds[1][0]["range"]["start"] == {"line": 0, "character": broken.index("#")},
      "a lexical error while typing: %r" % ds[1])

# ---- Formatting ----
messy = "fn main()->u64{\nreturn   1 ;\n}\n"
resp, notes, rc = session([
    ("textDocument/didOpen", {"textDocument": {"uri": "file:///tmp/messy.bastio", "text": messy}}, False),
    ("textDocument/formatting", {"textDocument": {"uri": "file:///tmp/messy.bastio"}}, True),
])
edit = resp[101]["result"]
check(len(edit) == 1 and edit[0]["newText"] == "fn main() -> u64 {\n  return 1;\n}\n",
      "formatting replaces the text: %r" % edit)

# ---- Protocol ----
resp, notes, rc = session([("workspace/nothing", {}, True), ("$/cancelRequest", {"id": 5}, False)])
check(resp[100]["error"]["code"] == -32601, "an unknown request gets an error")
check(rc == 0, "an unknown notification is ignored")
resp, notes, rc = session([], raw=b"Content-Length: 5\r\n\r\n{nope")
check(rc == 1, "garbage ends in a non-zero exit status at the end of input, not a crash")
resp, notes, rc = session([], raw=b"")
check(rc == 1, "end of input without exit is a non-zero status")
# Escapes and non-BMP characters in the text keep positions in UTF-16 units.
src = "// é\U0001F600\nfn main() -> u64 { return 1; }\nfn other() -> u64 { return main(); }\n"
resp, notes, rc = session([
    ("textDocument/didOpen", {"textDocument": {"uri": "file:///tmp/utf.bastio", "text": src}}, False),
    ("textDocument/definition", {"textDocument": {"uri": "file:///tmp/utf.bastio"},
                                 "position": {"line": 2, "character": 31}}, True),
])
r = resp[101]["result"]
check(r and r["range"]["start"] == {"line": 1, "character": 3}, "definition in a file with non-ASCII text: %r" % r)

sys.exit(1 if fails else 0)
