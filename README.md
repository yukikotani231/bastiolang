# Bastio

Bastio is a systems language whose memory safety is **proved at compile time**, not checked at run time. It is the language BastiOS (`bastios`) is written in.

- **No `unsafe`.** The language has no way to read memory through a raw integer address.
- **Linear ownership.** Every buffer is consumed exactly once, so use-after-free, double free and **leaks** are compile errors.
- **Proved indices and arithmetic.** Array indices, overflow, underflow and division by zero are proved safe by an SMT solver (Z3) during compilation.
- **Zero overhead.** Proofs are erased. The generated code has no bounds checks and no runtime checks of any kind.
- **Opt-in functional correctness.** Functions that carry `requires` / `ensures` / `invariant` are proved to meet them.

New here? Start with the [tutorial](docs/tutorial.md), then keep [common proof failures](docs/proof-failures.md) at hand: it shows the errors people meet first and how to fix each. The [language reference](docs/language.md) has every rule. The examples in all three are compiled by the test suite, so they match the compiler.

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
 9 |     a = set(a, i, 1);
   |         ^^^
   = counterexample: len(a) = 4, i = 4
   = check the index against the length first, with an `if` or a `requires`, or keep it in range with a loop `invariant`
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

Requirements:

| Tool | Tested with | Notes |
| --- | --- | --- |
| Rust | 1.97.0 | Pinned in `rust-toolchain.toml`; rustup installs it on the first build |
| Z3 | 4.8.12 (Ubuntu 24.04 package), 5.1.0 | Must be on `PATH` as `z3`; the compiler runs it as a subprocess |
| C compiler | GCC 13 | Called as `cc`; the generated C is C11 |
| Python 3 | 3.12, 3.13 | Only for `stage1/difftest.sh` |

Tested on x86-64 Linux (Ubuntu 24.04), which is what CI runs; other platforms are untested. On Ubuntu: `sudo apt-get install z3 build-essential python3`, plus [rustup](https://rustup.rs).

```sh
git clone https://github.com/yukikotani231/bastiolang && cd bastiolang
(cd stage0 && cargo build --release)
tests/bastioc1.sh     # stage0 builds bastioc1, the compiler (once; about half a minute)
stage0/target/release/bastioc1 -o hello examples/hello.bastio && ./hello
tests/run.sh          # must-pass and must-fail programs, zero-overhead check, sanitizer cross-check
stage1/difftest.sh    # the self-hosted compiler's phases against stage0
```

bastioc1 is the compiler written in Bastio (`stage1/`); stage0's `bastioc0` only bootstraps it and is the oracle difftest compares it with. The tests use bastioc1 unless `BASTIOC` names another compiler. Options (bastioc0 takes the same; `--help` lists them):
- `--check`: verify only
- `--emit-c`: write the generated C
- `--dump-smt`: print every query sent to the solver
- `--target kernel`, `--target user`: emit freestanding C for BastiOS's kernel or its programs
- `--arch aarch64`: build for AArch64 Linux (hosted target, C backend; `tests/aarch64.sh` runs the tests under `qemu-aarch64`)
- `--backend native`: generate x86-64 assembly directly instead of C (`tests/native.sh` checks it against the C backend); `--emit-asm` writes the assembly
- `--header FILE`: write a C header for the `export fn`s of a library
- `--lib DIR`: where `import "core/..."` looks (also `BASTIO_LIB`)
- `--trusted-base`: list what the proofs trust

## Layout

| Path | Contents |
| --- | --- |
| `stage0/` | The bootstrap compiler in Rust: it builds bastioc1 and is the oracle for `stage1/difftest.sh`; being frozen (ROADMAP v2) |
| `stage1/` | bastioc1, the compiler, written in Bastio (see `stage1/README.md`) |
| `tests/pass/` | Programs that must verify and exit with a given code |
| `tests/fail/` | Programs that must be rejected with a given error code |
| `core/`, `std/` | The library: byte strings, text, numbers, printing, a hash map, sorting (`core/`, every target) and host files (`std/`); see `docs/core.md` |
| `examples/` | Small programs to start from (`hello.bastio`, `lines`; see `examples/README.md`), and proved libraries called from C through `export fn`: `base64`, `utf8` (validator and decoder) and `der` (ASN.1 structure checker), each with a differential fuzz against a C implementation |
| `docs/tutorial.md` | A tour of the language for newcomers |
| `docs/proof-failures.md` | The proof failures newcomers meet first, and how to fix each |
| `docs/language.md` | The language reference |
| `docs/core.md` | The core and std libraries: what each function does and what is proved |
| `docs/concurrency.md` | The concurrency design (not implemented yet) |
| `ROADMAP.md` | The path to self-hosting and to a kernel |

## Status

Pre-release. The language and the compiler change without notice, and nothing is stable yet. stage0 implements the language described in [docs/language.md](docs/language.md), whose "Soundness scope" section lists what is and is not guaranteed; the self-hosted compiler in `stage1/` is in progress. [ROADMAP.md](ROADMAP.md) has the plan.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.
