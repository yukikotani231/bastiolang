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

# ---- Completion and signature help ----
# Each case is a buffer with `|` where the cursor is; the buffers are not on
# disk (they import lib.bastio from the test directory), and are half typed.
CASES = {
    "word": "import \"lib.bastio\";\nfn main() -> u64 {\n  let count = 1;\n  let total = cou|\n}\n",
    "scope": "import \"lib.bastio\";\nfn other(zed: u64) -> u64 { return zed; }\nfn main() -> u64 {\n  let count = 1;\n  |\n}\n",
    "keyword": "fn main() -> u64 {\n  ret|\n}\n",
    "field": "import \"lib.bastio\";\nfn main() -> u64 {\n  let p = Pair { a: 1, b: 2 };\n  let x = p.|\n}\n",
    "variant": "import \"lib.bastio\";\nfn main() -> u64 {\n  let s = Shape::|\n}\n",
    "literal": "import \"lib.bastio\";\nfn main() -> u64 {\n  let p = Pair { a: 1, |\n}\n",
    "comment": "fn main() -> u64 {\n  // ret|\n}\n",
    "string": "import \"lib|\";\nfn main() -> u64 { return 0; }\n",
    "sig0": "import \"lib.bastio\";\nfn main() -> u64 {\n  let n = double(|\n}\n",
    "sig1": "fn add(a: u64, b: Vec[u64]) -> u64 { return a; }\nfn main() -> u64 {\n  let n = add(1, |\n}\n",
    "nosig": "fn add(a: u64) -> u64 { return a; }\nfn main() -> u64 {\n  let n = (1 + |\n}\n",
    "sigvariant": "import \"lib.bastio\";\nfn main() -> u64 {\n  let s = Shape::Line(|\n}\n",
}


def case_uri(k):
    return uri("case_%s.bastio" % k)


def cursor_of(t):
    at = t.index("|")
    return t.replace("|", ""), {"line": t.count("\n", 0, at), "character": at - (t.rfind("\n", 0, at) + 1)}


steps, rid = [], {}
for k, t in CASES.items():
    body, pos = cursor_of(t)
    steps.append(("textDocument/didOpen", {"textDocument": {
        "uri": case_uri(k), "languageId": "bastio", "version": 1, "text": body}}, False))
for k, t in CASES.items():
    body, pos = cursor_of(t)
    method = "signatureHelp" if k.startswith(("sig", "nosig")) else "completion"
    rid[k] = 100 + len(steps)
    steps.append(("textDocument/" + method, {"textDocument": {"uri": case_uri(k)}, "position": pos}, True))
resp, notes, rc = session(steps)
caps = resp[1]["result"]["capabilities"]
check("completionProvider" in caps and "signatureHelpProvider" in caps, "capabilities for completion and signature help")


def labels(k):
    return [i["label"] for i in resp[rid[k]]["result"]["items"]]


def item(k, name):
    return next(i for i in resp[rid[k]]["result"]["items"] if i["label"] == name)


l = labels("word")
check(l == ["count"],
      "a word completes to what starts with it, a local first: %r" % l)
check(item("word", "count")["kind"] == 6 and "let count" in item("word", "count")["detail"],
      "a local is a variable with its declaration: %r" % item("word", "count"))
l = labels("scope")
check({"count", "double", "Pair", "Shape", "LIMIT", "other", "main", "fn"} <= set(l) and "zed" not in l and "a" not in l and "Dot" not in l,
      "everything in scope, and nothing from another function, nor fields nor variants: %r" % l)
check(item("scope", "double")["kind"] == 3 and item("scope", "Pair")["kind"] == 22
      and item("scope", "Shape")["kind"] == 13 and item("scope", "LIMIT")["kind"] == 21
      and "fn double(x: u64) -> u64" in item("scope", "double")["detail"],
      "kinds and details of definitions: %r" % item("scope", "double"))
check(labels("keyword") == ["return"], "keywords: %r" % labels("keyword"))
check(sorted(labels("field")) == ["a", "b"], "after a dot: the fields: %r" % labels("field"))
check(item("field", "a")["kind"] == 5 and item("field", "a")["detail"] == "a: u64", "a field: %r" % item("field", "a"))
check(sorted(labels("variant")) == ["Dot", "Line"] and item("variant", "Line")["kind"] == 20
      and item("variant", "Line")["detail"] == "Line(u64)",
      "after `::`: the variants: %r" % labels("variant"))
check(sorted(labels("literal")) == ["a", "b"], "in a struct literal: its fields: %r" % labels("literal"))
check(labels("comment") == [] and labels("string") == [], "nothing in a comment or a string")

s = resp[rid["sig0"]]["result"]
sig = s["signatures"][0]
check(sig["label"].startswith("fn double(x: u64) -> u64") and [p["label"] for p in sig["parameters"]] == ["x: u64"]
      and s["activeParameter"] == 0 and "Doubles `x`." in sig["documentation"]["value"],
      "signature help: %r" % s)
s = resp[rid["sig1"]]["result"]
check(s["activeParameter"] == 1 and [p["label"] for p in s["signatures"][0]["parameters"]] == ["a: u64", "b: Vec[u64]"],
      "the second parameter, with brackets in a type: %r" % s)
check(resp[rid["nosig"]]["result"] is None, "no signature outside a call")
s = resp[rid["sigvariant"]]["result"]
check(s["signatures"][0]["label"] == "Line(u64)" and s["activeParameter"] == 0, "signature of a variant: %r" % s)

# ---- Workspace symbols ----
def wsym(q):
    return ("workspace/symbol", {"query": q}, True)


steps = [open_doc("main.bastio"), wsym("dbl"), wsym("LINE"), wsym(""), wsym("zzz")]
resp, notes, rc = session(steps)
check(resp[1]["result"]["capabilities"].get("workspaceSymbolProvider"), "capability for workspace symbols")
r = resp[101]["result"]
check(len(r) == 1 and r[0]["name"] == "double" and r[0]["kind"] == 12 and r[0]["location"]["uri"] == uri("lib.bastio")
      and r[0]["location"]["range"]["start"] == where("lib.bastio", "double"),
      "the letters of the query in order find a function in an imported file: %r" % r)
r = resp[102]["result"]
check([(x["name"], x.get("containerName")) for x in r] == [("Line", "Shape")], "case is ignored, a variant has its enum: %r" % r)
names = sorted(x["name"] for x in resp[103]["result"])
check(names == sorted(["main", "double", "Pair", "a", "b", "Shape", "Dot", "Line", "LIMIT"]),
      "an empty query lists the definitions of the program once: %r" % names)
check(resp[104]["result"] == [], "no match")

# ---- Semantic tokens ----
def sem(name):
    return ("textDocument/semanticTokens/full", {"textDocument": {"uri": uri(name)}}, True)


def decode_tokens(data):
    out, line, ch = {}, 0, 0
    for i in range(0, len(data), 5):
        dl, dc, ln, ty, md = data[i:i + 5]
        line, ch = line + dl, (ch + dc if dl == 0 else dc)
        out[(line, ch)] = (ln, ty, md)
    return out


resp, notes, rc = session([open_doc("main.bastio"), open_doc("lib.bastio"), sem("main.bastio"), sem("lib.bastio")])
leg = resp[1]["result"]["capabilities"]["semanticTokensProvider"]["legend"]
check(leg["tokenTypes"][:3] == ["function", "struct", "enum"] and leg["tokenModifiers"] == ["declaration", "readonly"],
      "the legend: %r" % leg)


def tok(res, name, pos, nth=1):
    at = where(name, pos, nth)
    return res.get((at["line"], at["character"]))


m = decode_tokens(resp[102]["result"]["data"])
check(tok(m, "main.bastio", "double") == (6, 0, 0), "a call is a function: %r" % (tok(m, "main.bastio", "double"),))
check(tok(m, "main.bastio", "Pair") == (4, 1, 0), "a struct literal names a struct")
check(tok(m, "main.bastio", "n =") == (1, 5, 1), "a `let` is a declared variable")
check(tok(m, "main.bastio", "n)") == (1, 5, 0), "a use of a local is a variable")
check(tok(m, "main.bastio", "a:") == (1, 4, 0), "a field in a literal is a property")
check(tok(m, "main.bastio", "LIMIT") == (5, 5, 2), "a constant is read-only")
check(tok(m, "main.bastio", "Line") == (4, 3, 0), "a variant is an enum member: %r" % (tok(m, "main.bastio", "Line"),))
check(tok(m, "main.bastio", "Shape") == (5, 2, 0), "an enum")
check(tok(m, "main.bastio", "u64") is None, "built-in types are left to the syntax")
l = decode_tokens(resp[103]["result"]["data"])
check(tok(l, "lib.bastio", "double") == (6, 0, 1), "a declared function")
check(tok(l, "lib.bastio", "x:") == (1, 6, 1), "a parameter, declared: %r" % (tok(l, "lib.bastio", "x:"),))
check(tok(l, "lib.bastio", "LIMIT") == (5, 5, 3), "a declared constant")

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

# ---- Code actions ----
def fixes(src, name="/tmp/fix_test.bastio"):
    """Opens `src` (the compiler checks it), asks for the code actions of the
    diagnostic it reports, and returns (title, text with the edit applied)."""
    u = "file://" + name
    resp, notes, rc = session([
        ("textDocument/didOpen", {"textDocument": {"uri": u, "languageId": "bastio", "version": 1, "text": src}}, False),
    ])
    diags = [d for n in notes if n["method"] == "textDocument/publishDiagnostics" for d in n["params"]["diagnostics"]]
    if not diags:
        return None
    d = diags[0]
    resp, notes, rc = session([
        ("textDocument/didOpen", {"textDocument": {"uri": u, "languageId": "bastio", "version": 1, "text": src}}, False),
        ("textDocument/codeAction", {"textDocument": {"uri": u}, "range": d["range"],
                                     "context": {"diagnostics": [d]}}, True),
    ])
    out = []
    lines = src.split("\n")
    for a in resp[101]["result"]:
        t = src
        for e in sorted(a["edit"]["changes"][u], key=lambda e: (e["range"]["start"]["line"], e["range"]["start"]["character"]), reverse=True):
            def off(p):
                return sum(len(l) + 1 for l in lines[:p["line"]]) + p["character"]
            t = t[:off(e["range"]["start"])] + e["newText"] + t[off(e["range"]["end"]):]
        out.append((a["title"], t))
    return out


r = fixes("fn main() -> u64 {\n  let x = 1\n  return x;\n}\n")
check(r == [("Insert `;`", "fn main() -> u64 {\n  let x = 1;\n  return x;\n}\n")], "fix: insert a semicolon: %r" % r)
r = fixes("fn main() -> u64 {\n  let x = 1;\n  x = 2;\n  return x;\n}\n")
check(r == [("Declare `x` with `mut`", "fn main() -> u64 {\n  let mut x = 1;\n  x = 2;\n  return x;\n}\n")], "fix: declare with mut: %r" % r)
r = fixes("enum Color { Red, Green, Blue }\nfn main() -> u64 {\n  let c = Color::Red;\n  match c {\n    Color::Red => { return 1; },\n    Color::Green => { return 2; },\n  }\n  return 0;\n}\n")
check(r and r[0][0] == "Add the arm for `Color::Blue`" and "    Color::Blue => {},\n  }" in r[0][1], "fix: add a match arm: %r" % r)
r = fixes("enum Opt { Some(u64), None }\nfn main() -> u64 {\n  let o = Opt::Some(1);\n  match o {\n    Opt::None => { return 0; },\n  }\n  return 0;\n}\n")
check(r and "Opt::Some(_) => {}," in r[0][1], "fix: the new arm of a variant with a payload: %r" % r)
r = fixes("fn main() -> u64 {\n  let count = 3;\n  return cuont + 1;\n}\n")
check(r == [("Change to `count`", "fn main() -> u64 {\n  let count = 3;\n  return count + 1;\n}\n")], "fix: a misspelled variable: %r" % r)
r = fixes("fn helper() -> u64 { return 1; }\nfn main() -> u64 {\n  return helpr();\n}\n")
check(r == [("Change to `helper`", "fn helper() -> u64 { return 1; }\nfn main() -> u64 {\n  return helper();\n}\n")], "fix: a misspelled function: %r" % r)
r = fixes("struct Point { x: u64 }\nfn get(p: Pont) -> u64 { return 0; }\nfn main() -> u64 { return 0; }\n")
check(r == [("Change to `Point`", "struct Point { x: u64 }\nfn get(p: Point) -> u64 { return 0; }\nfn main() -> u64 { return 0; }\n")], "fix: a misspelled type: %r" % r)
r = fixes("fn get(p: Zzzzzz) -> u64 { return 0; }\nfn main() -> u64 { return 0; }\n")
check(r == [], "no fix for a type that is nothing like another: %r" % r)
r = fixes("struct P { a: u64, ok: bool }\nfn main() -> u64 {\n  let p = P { a: 1 };\n  return p.a;\n}\n")
check(r == [("Add the field `ok`", "struct P { a: u64, ok: bool }\nfn main() -> u64 {\n  let p = P { a: 1, ok: false };\n  return p.a;\n}\n")], "fix: a missing field: %r" % r)
r = fixes("struct P { a: u64, b: i32 }\nfn main() -> u64 {\n  let p = P { a: 1, };\n  return p.a;\n}\n")
check(r and "P { a: 1, b: 0, }" in r[0][1], "fix: a missing field after a comma: %r" % r)
r = fixes("struct B { n: u64, v: Buf }\nfn main() -> u64 {\n  let p = B { n: 1 };\n  return 0;\n}\n")
check(r == [], "no fix for a field that has no obvious value: %r" % r)
r = fixes("fn main() -> u64 {\n  return zzzzzz;\n}\n")
check(r == [], "no fix for a name that is nothing like another: %r" % r)

# An unknown name that another open document defines: offer its import, after
# the imports the file has (a new directory name is `../`-relative).
mu = uri("fix_main.bastio")
main_src = 'fn main() -> u64 {\n  return double(2);\n}\n'
open_main = ("textDocument/didOpen", {"textDocument": {"uri": mu, "languageId": "bastio", "version": 1, "text": main_src}}, False)
open_lib = open_doc("lib.bastio")
resp, notes, rc = session([open_lib, open_main])
d = [d for n in notes if n["params"]["uri"] == mu for d in n["params"]["diagnostics"]][0]
resp, notes, rc = session([open_lib, open_main,
    ("textDocument/codeAction", {"textDocument": {"uri": mu}, "range": d["range"], "context": {"diagnostics": [d]}}, True)])
acts = resp[102]["result"]
check([a["title"] for a in acts] == ["Import `lib.bastio`"], "fix: import titles: %r" % acts)
e = acts[0]["edit"]["changes"][mu][0]
check(e["newText"] == 'import "lib.bastio";\n' and e["range"]["start"] == {"line": 0, "character": 0}, "fix: the import edit: %r" % e)

# ---- Inlay hints ----
src = (
    "fn area(w: u64, h: u64) -> u64 requires w < 1000 && h < 1000 { return w * h; }\n"
    "struct P { a: u64 }\n"
    "enum E { A, B(u64) }\n"
    "fn main() -> u64 {\n"
    "  let w = 3;\n"
    "  let a = area(w, 4);\n"
    "  let p = P { a: 1 };\n"
    "  let e = E::B(2);\n"
    "  let n = 5;\n"
    "  let m = n;\n"
    "  let q: u64 = 7;\n"
    "  let r = q;\n"
    "  let s = \"hi\";\n"
    "  let ok = true;\n"
    "  let lt = n < 10;\n"
    "  let both = ok && n == 1;\n"
    "  let not = !ok;\n"
    "  let sum = n + 1;\n"
    "  let t = q * 2 + r;\n"
    "  let c = cast[u8](q);\n"
    "  return a + p.a + m;\n"
    "}\n")
iu = "file:///tmp/inlay_test.bastio"
resp, notes, rc = session([
    ("textDocument/didOpen", {"textDocument": {"uri": iu, "languageId": "bastio", "version": 1, "text": src}}, False),
    ("textDocument/inlayHint", {"textDocument": {"uri": iu}, "range": {"start": {"line": 0, "character": 0}, "end": {"line": 22, "character": 0}}}, True),
    ("textDocument/inlayHint", {"textDocument": {"uri": iu}, "range": {"start": {"line": 5, "character": 0}, "end": {"line": 5, "character": 30}}}, True),
])
hints = [(h["position"]["line"], h["position"]["character"], h["label"]) for h in resp[101]["result"]]
want = [(4, 7, ": u64"), (5, 7, ": u64"), (5, 18, "h:"), (6, 7, ": P"), (7, 7, ": E"), (8, 7, ": u64"), (11, 7, ": u64"), (12, 7, ": Str"), (13, 8, ": bool"), (14, 8, ": bool"), (15, 10, ": bool"), (16, 9, ": bool"), (18, 7, ": u64"), (19, 7, ": u8")]
check(hints == want, "inlay hints: %r" % hints)
check(len(resp[102]["result"]) == 2, "inlay hints only in the range: %r" % resp[102]["result"])

# ---- Folding ranges and links ----
src = (
    "import \"lib.bastio\";\n"          # 0
    "// one\n"                           # 1
    "// two\n"                           # 2
    "fn main() -> u64 {\n"               # 3
    "  let x = 1;\n"                     # 4
    "  if x > 0 {\n"                     # 5
    "    return 1;\n"                    # 6
    "  }\n"                              # 7
    "  return 0;\n"                      # 8
    "}\n"                                # 9
    "fn tiny() -> u64 {\n"               # 10
    "  return 1;\n"                      # 11
    "}\n")                               # 12
fu = uri("fold_test.bastio")
resp, notes, rc = session([
    ("textDocument/didOpen", {"textDocument": {"uri": fu, "languageId": "bastio", "version": 1, "text": src}}, False),
    ("textDocument/foldingRange", {"textDocument": {"uri": fu}}, True),
    ("textDocument/documentLink", {"textDocument": {"uri": fu}}, True),
    ("textDocument/selectionRange", {"textDocument": {"uri": fu}, "positions": [{"line": 6, "character": 12}]}, True),
])
sel = []
node = resp[103]["result"][0]
while node:
    r = node["range"]
    sel.append((r["start"]["line"], r["start"]["character"], r["end"]["line"], r["end"]["character"]))
    node = node.get("parent")
want = [(6, 11, 6, 12), (5, 12, 7, 2), (5, 11, 7, 3), (3, 18, 9, 0), (3, 17, 9, 1), (0, 0, 13, 0)]
check(sel == want, "selection ranges: %r" % sel)
folds = sorted((f["startLine"], f["endLine"], f.get("kind", "")) for f in resp[101]["result"])
check(folds == [(1, 2, "comment"), (3, 8, ""), (5, 6, ""), (10, 11, "")], "folding ranges: %r" % folds)
links = resp[102]["result"]
check(len(links) == 1 and links[0]["target"] == uri("lib.bastio")
      and links[0]["range"] == {"start": {"line": 0, "character": 8}, "end": {"line": 0, "character": 18}},
      "import link: %r" % links)

# ---- Code lenses ----
src = "fn used() -> u64 { return 1; }\nfn unused() -> u64 { return 2; }\nfn main() -> u64 { return used() + used(); }\n"
lu = uri("lens_test.bastio")
resp, notes, rc = session([
    ("textDocument/didOpen", {"textDocument": {"uri": lu, "languageId": "bastio", "version": 1, "text": src}}, False),
    ("textDocument/codeLens", {"textDocument": {"uri": lu}}, True),
])
lens = [(l["range"]["start"]["line"], l["command"]["title"]) for l in resp[101]["result"]]
check(lens == [(0, "2 references"), (1, "0 references"), (2, "0 references")], "code lenses: %r" % lens)

# ---- Call hierarchy ----
src = ("fn leaf() -> u64 { return 1; }\n"
       "fn mid() -> u64 { return leaf() + leaf(); }\n"
       "fn top() -> u64 { return mid() + leaf(); }\n")
cu = uri("calls_test.bastio")
resp, notes, rc = session([
    ("textDocument/didOpen", {"textDocument": {"uri": cu, "languageId": "bastio", "version": 1, "text": src}}, False),
    ("textDocument/prepareCallHierarchy", {"textDocument": {"uri": cu}, "position": {"line": 0, "character": 4}}, True),
])
item = resp[101]["result"][0]
check(item["name"] == "leaf" and item["uri"] == cu, "call hierarchy item: %r" % item)
resp, notes, rc = session([
    ("textDocument/didOpen", {"textDocument": {"uri": cu, "languageId": "bastio", "version": 1, "text": src}}, False),
    ("callHierarchy/incomingCalls", {"item": item}, True),
    ("callHierarchy/outgoingCalls", {"item": dict(item, data=dict(item["data"], ns=src.index("mid")))}, True),
])
inc = sorted((c["from"]["name"], len(c["fromRanges"])) for c in resp[101]["result"])
check(inc == [("mid", 2), ("top", 1)], "incoming calls: %r" % inc)
out = sorted((c["to"]["name"], len(c["fromRanges"])) for c in resp[102]["result"])
check(out == [("leaf", 2)], "outgoing calls: %r" % out)

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
