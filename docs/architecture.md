# How the compiler is put together

A map for someone who wants to change the compiler or the library and needs to know where to look. [The README](../README.md) says what Bastio is, [bootstrap.md](bootstrap.md) explains the two compilers, and [stage1/README.md](../stage1/README.md) goes into the details of each phase. This page is the overview that connects them. For the kernel written in Bastio, see BastiOS's [architecture page](https://github.com/yukikotani231/bastios/blob/main/docs/architecture.md).

## What happens to a program

```text
  source files
      |  lexer          text -> tokens
      |  parser         tokens -> syntax tree
      |  loader, mono   follow imports, instantiate generics
      |  check          types and linear ownership
      |  verify         walk every function, record proof obligations
      |  solve          ask Z3 to discharge each obligation
      v
  a checked, proved program    <- a failed proof stops here with a counterexample
      |
      |  C backend              or            native backend
      |  codegen: one C file                  x86.bastio / irx86.bastio: assembly
      v                                       v
  cc builds the executable                    as and cc link it
```

Everything before the backend is the same for every target. The proofs are erased: the generated code has no checks, and `spec`, `ghost`, `requires`, `ensures` and `invariant` do not appear in it.

The compiler you run is `bastioc1`, written in Bastio (`stage1/`). `bastioc0`, in Rust (`stage0/`), builds it and is the oracle that `stage1/difftest.sh` compares it with. stage0 is frozen, so almost all work happens in `stage1/`.

## Where each phase lives

| Phase | stage1 (Bastio) | stage0 (Rust) | What to know |
| --- | --- | --- | --- |
| Lexer | `lexer.bastio` | `lexer.rs` | Tokens carry their byte range in one shared source buffer |
| Parser | `parser.bastio` | `parser.rs`, `ast.rs` | The tree is an arena of nodes; children are indices |
| Imports and generics | `loader.bastio`, `libenv.bastio`, `mono.bastio` | `mono.rs` | Types are interned, so equal types have equal indices |
| Types and linearity | `check.bastio` | `check.rs` | Ownership is tracked per place (`x`, `x.f`, `x.f.g`) |
| Proof obligations | `verify.bastio` | `verify.rs`, `smt.rs` | Turns each function into SMT-LIB commands; no solver needed yet |
| Solving | `solve.bastio` | `verify.rs` | Runs Z3 on several functions at once and reports the first failure with its counterexample |
| C backend | `codegen.bastio`, `cprelude.bastio` | `codegen.rs` | The runtime preludes for hosted, kernel and user targets are text in `cprelude.bastio` |
| Native backend | `x86.bastio` | `x86.rs` | One pass over the checked tree to assembly, then a peephole pass |
| IR (not the default path yet) | `ir.bastio`, `iropt*.bastio`, `irra.bastio`, `irx86.bastio`, `irarm64.bastio` | | See [ir.md](ir.md) |
| Driver | `bastioc1.bastio` | `main.rs` | Options, the order of the phases, error output |
| Diagnostics | spread over the phases above | `diag.rs` | Error codes such as `E-VERIFY-BOUNDS` are named where they are produced |

## The library and the tests

- `core/` and `std/` are Bastio source, proved like any other program. Their reference pages in `docs/lib/` are generated from the `///` comments (`tests/libref.py --write`).
- `tests/pass/` holds programs that must verify and exit with the code in their `// expect-exit: N` header. `tests/fail/` holds programs that must be rejected with the `// expect-error: CODE` named in their header. `tests/run.sh` runs both, and also builds the programs in `docs/*.md` and the library's examples.
- `stage1/difftest.sh` checks every phase of bastioc1 against stage0 on every Bastio source in the repository, so a change to stage1 that behaves differently from stage0 fails there. New features are the exception: they are stage1-only (the test starts with `// stage1-only: <feature>`).
- `tests/ci.sh` runs everything CI runs, in order.

## Where to look when you want to...

| You want to | Start at |
| --- | --- |
| Reject a program that is wrongly accepted, or accept one that is wrongly rejected | The error code in the message: search `stage1/` for it, then add a program to `tests/fail/` or `tests/pass/` |
| Change what a diagnostic says | The phase that names the error code; the docs show compiler output, so run `tests/run.sh` to find the pages to update |
| Add syntax | `lexer.bastio`, `parser.bastio`, then `check.bastio` and the backends; also `editors/` (the grammar checks fail otherwise) and `docs/language.md` |
| Add a proof rule | `verify.bastio`, with a `tests/fail/` program that the new rule makes rejected |
| Add a library function | `core/<module>.bastio` with a `///` doc comment and example, then `tests/libref.py --write` |
| Speed up generated code | `bench/run.sh` first, then the native backend or the IR; say what the numbers did in the pull request |
| Understand what the proofs trust | [trusted-base.md](trusted-base.md) and `bastioc0 --trusted-base` |

Good places to look for a first change are the issues labelled [`good first issue`](https://github.com/yukikotani231/bastiolang/labels/good%20first%20issue) and [CONTRIBUTING.md](../CONTRIBUTING.md#finding-something-to-work-on).
