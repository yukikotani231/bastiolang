# Bastio

Bastio is a systems language whose memory safety is **proved at compile time**, not checked at run time. It is the successor to fz/fzl, and the language BastiOS (`bastios`) will be written in.

- **No `unsafe`.** The language has no way to read memory through a raw integer address.
- **Linear ownership.** Every buffer is consumed exactly once, so use-after-free, double free and **leaks** are compile errors.
- **Proved indices and arithmetic.** Array indices, overflow, underflow and division by zero are proved safe by an SMT solver (Z3) during compilation.
- **Zero overhead.** Proofs are erased. The generated code has no bounds checks and no runtime checks of any kind.
- **Opt-in functional correctness.** Functions that carry `requires` / `ensures` / `invariant` are proved to meet them.

## Example

```
fn fill(mut b: Buf, v: u64) -> Buf
  ensures len(result) == len(b)
  ensures forall j: u64 :: j < len(result) ==> get(result, j) == v
{
  let mut i = 0;
  while i < len(b)
    invariant i <= len(b)
    invariant forall j: u64 :: j < i ==> get(b, j) == v
  {
    b = set(b, i, v);   // i < len(b) follows from the loop condition
    i = i + 1;          // cannot overflow: also proved automatically
  }
  return b;
}
```

A classic off-by-one (`while i <= len(a)`) is rejected with a counterexample:

```
tests/fail/oob_loop.bastio:9:9: error[E-VERIFY-BOUNDS]: could not prove that the index `i` is within `a` (`i < len(a)`)
  |     a = set(a, i, 1);
  |         ^
  = counterexample: len(a) = 4, i = 4
```

## How Bastio differs

| | Rust | Verus / ATS | Bastio |
| --- | --- | --- | --- |
| Raw memory in a kernel | `unsafe` blocks, checked by review | provable, with hand-written proofs | no raw access; hardware is reached through capability types |
| Index bounds | runtime check (panic) | proved, often with annotations | proved, mostly without annotations |
| Leaks | allowed (`mem::forget`) | depends on the tool | compile error |
| Proof style | none | specs plus proofs (Verus), proof terms (ATS) | SMT-checked specs; hand-written proofs are the exception |

## Scope

Bastio targets low-level code that has no runtime and where a memory-safety bug is costly:

- **Primary:** OS kernels and drivers (BastiOS).
- **Also:** embedded firmware and hypervisors, and safe libraries callable from C, such as parsers for images, fonts, compression and network protocols (code that reads untrusted input).
- **Not a goal:** web apps, scripting, GUI applications. Rust or a garbage-collected language is a better fit there.

## Build and test

Requirements: Rust (to build stage0), `z3`, and a C compiler (`cc`).

```sh
cd stage0 && cargo build --release
tests/run.sh       # must-pass and must-fail programs, zero-overhead check, sanitizer cross-check
stage0/target/release/bastioc0 -o hello examples/hello.bastio && ./hello
```

`bastioc0` options:
- `--check`: verify only
- `--emit-c`: write the generated C
- `--dump-smt`: print every query sent to the solver
- `--target kernel`: emit freestanding C for bare-metal x86-64 (used by BastiOS)

## Layout

| Path | Contents |
| --- | --- |
| `stage0/` | Throwaway bootstrap compiler in Rust, frozen once the self-hosted compiler works |
| `tests/pass/` | Programs that must verify and exit with a given code |
| `tests/fail/` | Programs that must be rejected with a given error code |
| `docs/language.md` | The v0 language reference |
| `ROADMAP.md` | The path to self-hosting and to a kernel |

## Status (v0)

The types are `u64`, `bool`, `Buf` (an owned buffer of u64), `&Buf` (a read-only borrow), `Str` (string literals) and `Serial` (the console capability). The backend emits C. What v0 does and does not guarantee is listed under "Soundness scope" in [docs/language.md](docs/language.md).
