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


def session(steps, raw=None, opts=None):
    """Runs the requests in `steps` (a list of (method, params, is_request));
    returns the responses by id, the notifications and the exit status."""
    msgs = [{"jsonrpc": "2.0", "id": 1, "method": "initialize",
             "params": {"initializationOptions": dict({"bastioc": BASTIOC}, **(opts or {}))}}]
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

# ---- References, highlights, rename ----
def at_req(method, name, pos, extra=None):
    p = {"textDocument": {"uri": uri(name)}, "position": pos}
    p.update(extra or {})
    return ("textDocument/" + method, p, True)


def refs(name, pos, decl=True):
    return at_req("references", name, pos, {"context": {"includeDeclaration": decl}})


def starts(locs, name):
    return sorted((l["range"]["start"]["line"], l["range"]["start"]["character"])
                  for l in locs if l["uri"] == uri(name))


def pos_t(p):
    return (p["line"], p["character"])


steps = [
    open_doc("main.bastio"), open_doc("lib.bastio"),
    refs("main.bastio", where("main.bastio", "double")),                   # 102
    refs("main.bastio", where("main.bastio", "double"), False),            # 103
    refs("lib.bastio", where("lib.bastio", "double")),                     # 104
    refs("main.bastio", where("main.bastio", "a)")),                       # 105
    refs("main.bastio", where("main.bastio", "n)")),                       # 106
    at_req("documentHighlight", "main.bastio", where("main.bastio", "len", 2)),  # 107
    at_req("rename", "lib.bastio", where("lib.bastio", "double"), {"newName": "twice"}),  # 108
    at_req("rename", "main.bastio", where("main.bastio", "n)"), {"newName": "m"}),        # 109
    at_req("rename", "main.bastio", where("main.bastio", "n)"), {"newName": "fn"}),       # 110
    at_req("prepareRename", "main.bastio", where("main.bastio", "fn")),                  # 111
    at_req("prepareRename", "main.bastio", where("main.bastio", "Pair")),                # 112
    refs("main.bastio", where("main.bastio", "Dot")),                      # 113
    refs("main.bastio", where("main.bastio", "p =")),                      # 114
]
resp, notes, rc = session(steps)
caps = resp[1]["result"]["capabilities"]
check(caps["referencesProvider"] and caps["documentHighlightProvider"] and caps["renameProvider"],
      "capabilities for references, highlights and rename")
r = resp[102]["result"]
check(starts(r, "lib.bastio") == [pos_t(where("lib.bastio", "double"))]
      and starts(r, "main.bastio") == [pos_t(where("main.bastio", "double"))] and len(r) == 2,
      "references to a function across files: %r" % r)
r = resp[103]["result"]
check(len(r) == 1 and r[0]["uri"] == uri("main.bastio"), "references without the declaration: %r" % r)
check(len(resp[104]["result"]) == 2, "references from the declaration see the use in another open file")
r = resp[105]["result"]
check(starts(r, "lib.bastio") == [pos_t(where("lib.bastio", "a:"))]
      and starts(r, "main.bastio") == sorted([pos_t(where("main.bastio", "a:")), pos_t(where("main.bastio", "a)"))]),
      "references to a field: its declaration, its initialiser and a use: %r" % r)
r = resp[106]["result"]
check(starts(r, "main.bastio") == sorted([pos_t(where("main.bastio", "n =")), pos_t(where("main.bastio", "n)"))])
      and len(r) == 2, "references to a local: %r" % r)
r = resp[107]["result"]
check(len(r) == 2 and all(h["kind"] == 1 for h in r), "highlights of a match binding: %r" % r)
ch = resp[108]["result"]["changes"]
check(set(ch) == {uri("lib.bastio"), uri("main.bastio")}
      and all(e["newText"] == "twice" for v in ch.values() for e in v)
      and len(ch[uri("lib.bastio")]) == 1 and len(ch[uri("main.bastio")]) == 1,
      "renaming a function edits every open file that names it: %r" % ch)
ch = resp[109]["result"]["changes"]
check(list(ch) == [uri("main.bastio")] and len(ch[uri("main.bastio")]) == 2, "renaming a local: %r" % ch)
check(resp[110]["error"]["code"] == -32602, "a keyword is not a name: %r" % resp[110])
check(resp[111]["result"] is None, "nothing to rename on a keyword")
check(resp[112]["result"]["placeholder"] == "Pair", "prepareRename names the token: %r" % resp[112])
r = resp[113]["result"]
check(len(r) == 2 and starts(r, "lib.bastio") == [pos_t(where("lib.bastio", "Dot"))],
      "references to a variant: %r" % r)
r = resp[114]["result"]
check(len(r) == 2, "references to a local declared by `let` and used after it: %r" % r)

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

# The compiler checks the buffer, not the file on disk: this file does not
# exist, and the text is what has the error. Nothing is left behind.
ghost = "ghost.bastio"
fresh = "fn main() -> u64 {\n  return true;\n}\n"
resp, notes, rc = session([("textDocument/didOpen", {"textDocument": {"uri": uri(ghost), "text": fresh}}, False)])
d = notes[0]["params"]["diagnostics"]
check(len(d) == 1 and d[0]["code"] == "E-TYPE" and d[0]["range"]["start"] == {"line": 1, "character": 2},
      "the buffer of a file that is not on disk is checked: %r" % d)
check(sorted(f for f in os.listdir(HERE) if f.startswith(".")) == [], "the checked copy of the buffer is removed")

# A buffer that differs from the saved file: the file is fine, the text is not.
resp, notes, rc = session([open_doc("main.bastio"),
    ("textDocument/didSave", {"textDocument": {"uri": uri("main.bastio")}}, False)])
check(all(n["params"]["diagnostics"] == [] for n in notes), "a file that is fine has no diagnostics")
edited = text("main.bastio").replace("return 2;", "return true;")
resp, notes, rc = session([
    ("textDocument/didOpen", {"textDocument": {"uri": uri("main.bastio"), "text": edited}}, False)])
d = notes[0]["params"]["diagnostics"]
check(len(d) == 1 and d[0]["code"] == "E-TYPE", "an unsaved change is checked when the document is opened: %r" % d)

# checkOnChange: the compiler also runs as the text changes.
change = ("textDocument/didChange", {"textDocument": {"uri": uri("main.bastio")},
                                     "contentChanges": [{"text": edited}]}, False)
resp, notes, rc = session([open_doc("main.bastio"), change], opts={"checkOnChange": True})
ds = [n["params"]["diagnostics"] for n in notes]
check(len(ds) == 2 and ds[0] == [] and len(ds[1]) == 1 and ds[1][0]["code"] == "E-TYPE",
      "with checkOnChange an edit is checked at once: %r" % ds)
resp, notes, rc = session([open_doc("main.bastio"), change])
ds = [n["params"]["diagnostics"] for n in notes]
check(len(ds) == 2 and ds[1] == [], "without it an edit only gets the lexer's diagnostics: %r" % ds)

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
