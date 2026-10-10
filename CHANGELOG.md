# Changelog

Notable changes to Bastio, the language and its compilers. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Bastio is
pre-release: before 1.0, any minor version may change the language
incompatibly.

The version is the one in `stage0/Cargo.toml`. A release is a tag `vX.Y.Z` on
`main`; `.github/workflows/release.yml` builds it (see
[docs/releasing.md](docs/releasing.md)).

## [Unreleased]

What will be 0.2.0.

### Language

- Page tables shared between processors: a processor takes a `Lease` on a
  top-level table with `mmu_enter` and gives it back with `mmu_exit`, and
  while a tree has leases nothing can be taken out of it, only added
  (`pt_leases`, `pt_lease_count`, `E-VERIFY-LEASE`). `pt_borrow` reaches
  lower tables and pages in place. `mmu_switch` is removed (#83, #88).
- Bulk copies proved in bounds, with no run-time checks: `bcopy` and `bmove`
  between and within `Bytes`, `frame_to_bytes` / `bytes_to_frame` for pages
  and `dma_to_bytes` / `bytes_to_dma` for lent pages. The prover knows what
  `bcopy` and `bmove` copy; page contents stay untracked (#91).
- `pt_writable`: whether a level-1 page-table entry maps its frame
  writable (#81).
- `boot_fb`, the framebuffer the boot loader reported, and a device window
  from 2 GiB (#67) up to 2^46 (#77).
- `boot_acpi`, a byte of one of the firmware's ACPI tables, so a kernel
  parses them (and the AML it needs) in Bastio, proved in bounds.
- `boot_info`, the platform's answers about the machine (BastiOS: the boot
  command line, the memory map, CPUID), unchecked input to the program.
- `old(x)` in loop invariants (#73).
- Generic `spec fn`s, and `vcount` for counting in specifications; local
  variables may share a function's name (#74).
- `==` on structs and enums in specifications (#86).
- Imports are searched for in the library path (`--lib`), so
  `import "core/text.bastio"` works from anywhere (#85).

### Library

- A first `core/` and `std/`: byte-string comparison, search, trimming and
  splitting; text building on `Vec[u8]`; integer parsing and formatting for
  every sized type; console printing; endian loads and stores; a hash map;
  insertion sort and binary search; host files and standard error (#69).
  The hash map is generic, `HashMap[V]` (#86).

### Compilers

- A native x86-64 backend in the self-hosted compiler (`stage1/x86.bastio`),
  ported from stage0's and checked byte for byte against it (#70). It
  passes up to four argument words in registers (#72) and covers `Child`
  (#76), `Lock`, `Atomic` and the device types (#80). bastioc1 builds
  itself with it (#76), and so do BastiOS's user programs (#82) and kernel
  (#84). It is driven by `stage1/asmdump.bastio`; `bastioc1 --backend
  native` is not exposed yet.
- bastioc1 prints every error sentence and hint that bastioc0 does, except
  a failed proof's sentence (#68); it has `--header` and `--trusted-base`
  (#64), proves in parallel and reads `$CC` (#71).
- Diagnostics name tokens as written, report a missing `;` at the end of the
  line, say where a linear value was consumed, and suggest a close name for
  an unknown variable or function (#87).
- An error shows its source line with the line number, the whole token at
  the error underlined, and its notes aligned under it; `tests/diag` holds
  golden transcripts, checked with both compilers (#102).
- bastioc1 checks its own source in 8.5 s instead of 11.5 s (hash indexes
  instead of linear scans by name), and proves in batches, freeing each
  batch's terms: 175 MB instead of 497 MB on its own source, 325 MB
  instead of 953 MB on the BastiOS kernel (#102).
- Z3 runs with a fixed work limit per query (`:rlimit`) instead of a 10 s
  timeout, so a proof no longer fails on a busy machine (#89).
- A change to `stage0/src` needs its stage1 mirror, or a `Stage0-only:`
  reason in a commit message; CI checks this on every pull request (#78).

### Documentation

- The trusted base is listed and explained (`docs/trusted-base.md`, #66).
- The tutorial's section on calling Bastio from C walks through a new
  example, `examples/lines`, built on `export fn` and `--header` (#92).
- `tests/run.sh` checks that error messages shown in `docs/` are what the
  compiler prints, that file copies in `docs/` match the files, and that
  `examples/*.bastio` build and run (#92).

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
