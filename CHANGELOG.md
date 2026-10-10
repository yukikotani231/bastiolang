# Changelog

Notable changes to Bastio, the language and its compilers. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Bastio is
pre-release: before 1.0, any minor version may change the language
incompatibly.

The version is the one in `stage0/Cargo.toml`. A release is a tag `vX.Y.Z` on
`main`; `.github/workflows/release.yml` builds it (see
[docs/releasing.md](docs/releasing.md)).

## [Unreleased]

### Added

- Shared pages: `Shared`, one counted reference to a page several page
  tables map at once (`MAP_SHARED` memory after `fork`), with `shared_new`,
  `shared_dup`, `shared_drop`, `shared_id`, `pt_put_shared`,
  `pt_take_shared`, `pt_dup_shared` and the ghost `pt_shared(t, i)`. The
  proof keeps each reference used once; the count itself is in the runtime
  prelude (trusted). Not in the native x86 backend yet.

### Changed

- `pt_take_frame` now needs `!pt_shared(t, i)` (`E-VERIFY-SHARED`). Code that
  tears down a table it did not build itself tests `pt_shared` and takes
  such slots with `pt_take_shared`.

## [0.1.0] - not yet tagged

The first version: everything up to the self-hosting fixed point.

### Language

- Linear ownership: `Buf`, `Bytes`, structs with linear fields, `Box[T]`,
  `Vec[T]`, generic structs, enums and functions (monomorphized), enums with
  exhaustive `match`, `&T` and `&mut T` borrows. Use-after-free, double free
  and leaks are compile errors.
- Proofs by Z3, with counterexamples on failure: indices in bounds, no
  overflow, underflow or division by zero, `requires` / `ensures` /
  `invariant` / `assert`, and `unreachable;`.
- Integers `u8` to `u64` and `i8` to `i64`, proved never to wrap at any
  width; literals typed by context; `cast[T]` (proved to fit) and `wrap[T]`;
  bitwise operators with shift amounts proved in range.
- `import "path";` for programs that span files.
- Hosted programs: the `Host` capability for files and arguments, and child
  processes with pipes (`Child`).
- Kernel and user targets (`--target kernel`, `--target user`): capabilities
  for physical pages and page tables (`Pool`, `Frame`, `PageTable`, `Mmu`,
  `Cpu`), the serial console, I/O ports (`Ports`, `PortIo`), memory-mapped
  registers (`MmioSpace`, `Mmio`), DMA buffers (`Dma`), several processors
  (`cpu_count`, `smp_run`), `Lock[T]` with struct invariants, lock order
  checked at compile time (`E-LOCK-ORDER`), and `Atomic[T]`.
- The C boundary: `export fn` with a generated header (`--header`), taking
  `&Bytes` and `&mut Bytes` from C.

### Compilers

- `stage0` (Rust): the bootstrap compiler. C backend with no runtime checks,
  cross-checked under sanitizers; one solver scope per function and
  functions proved in parallel (`BASTIO_JOBS`); `--trusted-base` lists what
  the proofs assume; an experimental native x86-64 backend
  (`--backend native`, hosted target only).
- `stage1`: the self-hosted compiler, `bastioc1`, written in Bastio. Lexer,
  parser, checker, verifier and C code generator each produce the same
  output as stage0 (`stage1/difftest.sh`). bastioc1 builds itself and
  reaches a fixed point: gen1, gen2 and gen3 are identical, byte for byte.
  Not yet ported: stage0's error sentences, `--header`, `--trusted-base`,
  `--backend native` and parallel proving.

### Tooling and documentation

- A tutorial, a language reference, and design documents for sized integers
  and concurrency; every `bastio` example in them is compiled by the tests.
- Editor support: a tree-sitter grammar and a VS Code extension.
- `bench/`, comparing generated code with hand-written C.
- Proved C-callable libraries in `examples/`: base64, UTF-8 and DER.
- The documentation website (`site/`).
- Dual licensed Apache-2.0 OR MIT.

[Unreleased]: https://github.com/yukikotani231/bastiolang/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/yukikotani231/bastiolang/releases/tag/v0.1.0
