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

### Tools

- `tools/bastiofmt`: a source formatter written in Bastio (`bastiofmt file.bastio`,
  `--write`, `--check`, `-` for standard input). It lays a file out from its
  tokens and refuses to print anything that does not lex to the same tokens as
  its input. See [tools/README.md](tools/README.md).
- `tools/bastio-lsp`: a language server written in Bastio. Diagnostics from
  `bastioc1 --check` (and the lexer while typing), go to definition and hover
  in the file and its imports, document symbols and formatting.
- `editors/vim`: file type, syntax, indentation and a Neovim plugin that
  starts the language server.

### Compiler

- Diagnostics say what to do about the slips a newcomer meets first: `=`
  where `==` is meant, `+=`, `var` and `for`, a parameter without its type, a
  block that is never closed, a function that can reach its end without
  returning, an integer of the wrong type (with the `cast` and `wrap`
  spelling for the type wanted), a `return` of a value from a function with
  no return type, a linear value consumed in one branch of an `if`, a variant
  matched twice, and a failed proof of `vec_free` or `unreachable`. Each
  message has a golden test in `tests/diag`.
- `BASTIO_PROOF_CACHE=<dir>`: bastioc1 records each function it has proved
  (by a hash of its verification conditions, the preamble, the solver and the
  solver's version) and does not send it to the solver again. Checking
  `stage1/bastioc1.bastio` takes about 3.3 s with a warm cache instead of
  13.9 s; changing one function re-proves only that function. Off unless the
  variable is set; the cache is trusted like the solver (see
  docs/trusted-base.md). The key is a hash of the conditions' term structure,
  each shared term hashed once, so a warm check of the BastiOS kernel takes
  about 14 s instead of 27 s.

### Language

- `try_alloc` and `try_balloc`: allocation that returns an empty buffer when
  memory runs out instead of stopping the program. The prover makes the
  program check the length before using it.
- `vec_try_reserve` and `vec_push_within`: room for elements is reserved
  ahead of time, or the reservation reports that memory ran out, and pushes
  into reserved room never allocate. The prover tracks a vector's capacity
  (`vcap` in specifications). `alloc`, `balloc`, `vec_push` and `box` still
  stop the program on out of memory.
- `try_box[T](x)`: a box that reports running out of memory. It returns
  `Boxed::Ok` with the box, or `Boxed::Full` with `x` given back, from
  `enum Boxed[T]` in the new `core/boxed.bastio`.
- Page tables shared between processors: a processor takes a `Lease` on a
  top-level table with `mmu_enter` and gives it back with `mmu_exit`, and
  while a tree has leases nothing can be taken out of it, only added
  (`pt_leases`, `pt_lease_count`, `E-VERIFY-LEASE`). `pt_borrow` reaches
  lower tables and pages in place. `mmu_switch` is removed (#83, #88).
- Bulk copies proved in bounds, with no run-time checks: `bcopy` and `bmove`
  between and within `Bytes`, `frame_to_bytes` / `bytes_to_frame` for pages
  and `dma_to_bytes` / `bytes_to_dma` for lent pages. The prover knows what
  `bcopy` and `bmove` copy; page contents stay untracked (#91).
- `Dirty` pages: `pool_take_dirty` takes a page without zeroing it, and it
  becomes a `Frame` only by being overwritten whole (`dirty_zero`,
  `frame_copy`, `shared_copy`, or `dirty_fill` from a page's worth of
  bytes), so a kernel can take pages under a lock and fill them outside it,
  or fill a page from a file without zeroing it first, without ever
  exposing what they held.
- `pt_writable`: whether a level-1 page-table entry maps its frame
  writable (#81).
- `pt_set_access` and `pt_accessible`: a level-1 slot can keep its page
  while the program may not touch it (`PROT_NONE`), or only read it. The
  table still owns the page (`pt_has` stays true), so it is taken out and
  freed as before. The present bit is cleared and PTE bit 10 marks the
  slot, in the runtime prelude (trusted). Not in the native x86 backend.
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
- `opaque spec fn`: a spec fn whose body is used only in its own file and
  is an uninterpreted function everywhere else, so a library can describe
  its data abstractly. `core/ring`'s `Queue` is now proved to be a FIFO
  (`queue_at`).
- `has_bit(x, i)` in specifications, and what every bitwise operator does
  to each bit, so packed bitmaps are proved to change only the bit they
  set or clear.
- Imports are searched for in the library path (`--lib`), so
  `import "core/text.bastio"` works from anywhere (#85).

### Library

- Six new modules. `core/path` splits file names (`path_split`,
  `path_ext`), joins them (`path_join`) and cleans them lexically
  (`path_clean`); every span it returns is proved to lie inside the path.
  `core/set` is a `HashSet` of `u64`. `core/ascii` has character classes and
  case conversion, proved byte for byte. `core/lines` splits text into
  lines (`\n` and `\r\n`), proved to give spans with no newline in them.
  `core/format` pads columns, joins, replaces, quotes and prints sizes
  (`fmt_pad_left`, `fmt_join`, `fmt_replace_all`, `fmt_quoted`, `fmt_size`).
  `core/scan` splits at white space (`scan_words`), at a separator string
  (`scan_split_str`) and once at a byte (`scan_split_once`, for `key=value`).
  `std/host` gained `write_file`, `file_exists`, `env_get` and `host_args`.
- A reference page for every library module (docs/lib/, and in the site):
  each function with its contract as the compiler checks it, what it is for,
  and an example. `tests/libref.py` generates the pages from the doc
  comments in core/ and std/, and `tests/run.sh` checks that they are up to
  date, that every function has a doc comment and an example, and that
  every example builds, verifies and runs.
- `checked_add_u64`, `checked_sub_u64`, `checked_mul_u64` (`None` when the
  result does not fit), `sat_add_u64`, `sat_sub_u64`, `div_ceil_u64`,
  `align_down_u64` and `align_up_u64`, each proved to compute exactly that.
- `vec_find_u64` and `sorted_find_u64`: the first index of a value, with
  `None` proved to mean it does not occur; `vec_min_u64`, `vec_max_u64` and
  `vec_reverse_u64`; `bytes_rfind_byte`, the last index of a byte.
- A first `core/` and `std/`: byte-string comparison, search, trimming and
  splitting; text building on `Vec[u8]`; integer parsing and formatting for
  every sized type; console printing; endian loads and stores; a hash map;
  insertion sort and binary search; host files and standard error (#69).
  The hash map is generic, `HashMap[V]` (#86).
- Every `parse_*` is proved to return the value of its digits (`bdec`,
  `bhex`) (#93, #98).
- `core/ring` (`Ring`, `Queue`), `core/bitmap` (allocation maps) and
  `core/csum` (Internet checksum, CRC-32) (#98). The bitmap also comes
  packed, 64 entries a word (`Bits`).
- `core/slots`: `Slots[V]`, a growable table of numbered slots for process
  ids, file descriptors and the like, proved to hand out the lowest free
  id at or above a starting point, under a limit; `core/rows`'s `rows_grow`
  for tables of fixed-size rows in one `Buf` (new rows are zero); `bits_grow`. All three report
  running out of memory instead of stopping the program.
- Helpers BastiOS and bastioc1 each wrote by hand: `min_u64`, `max_u64`,
  `bytes_span_eq`, `bytes_copy_span`, writes into a fixed buffer that drop
  what does not fit (`bytes_put_byte`, `bytes_put_str`, `bytes_put_u64`),
  `vec_extend_u64`, `vec_clone_u64`, and `core/wtext` for text kept one
  character a word in a `Vec[u64]`.
- `merge_sort_u64`, O(n log n) with the insertion sort's proof, and
  `sort_by_key_u64`, a proved stable sort of indices by a key vector;
  `BytesMap[V]`, a hash map from byte strings; `examples/wordfreq.bastio`
  (#107).

### Compilers

- `--arch aarch64` builds for AArch64 with the C backend. Hosted programs
  build for AArch64 Linux in both compilers (#121). In bastioc1, the kernel
  and user targets get AArch64 preludes too: a PL011 console, VMSAv8-64
  page tables in TTBR0, EL0 entry through the platform, `svc` system calls,
  and the barriers AArch64's weaker ordering needs around MMIO and DMA. An
  AArch64 kernel has no `Ports`, its device window starts at 0, and
  `cpu_get`/`cpu_set` use AArch64's saved registers (#122, #123).
  `tests/aarch64.sh` runs the tests under `qemu-aarch64` and boots a small
  kernel on QEMU's virt machine.
- Clearer messages, the same in both compilers: `E-VERIFY-CAST`,
  `E-VERIFY-SHIFT` and `E-VERIFY-ALLOC` end with a hint like the other
  proof failures; a loop body that breaks an invariant shows its
  counterexample as `counterexample after one iteration:`, since the values
  are those after the iteration; a solver that cannot be started is one
  line naming the file, with no source line; `--help` prints the options.
- A counterexample shows the elements of a buffer or vector of at most 8
  scalars, after its length, when the failed proof is an `assert`,
  `requires`, `ensures` or invariant: `vlen(v) = 2, v = [3, 1]` says why a
  vector is not sorted. Bounds and arithmetic errors still show only the
  length.
- bastioc1 is the compiler the tests and benchmarks use: `tests/bastioc1.sh`
  builds it with stage0 and rebuilds it only when stage0 or stage1 changes.
  `BASTIOC` still picks another compiler.
- `bastioc1 --dump-smt` prints every query sent to Z3, as bastioc0 does.
- A test program marked `// stage1-only: <feature>` is skipped when the tests
  run with stage0, for features that land after the stage0 freeze.
- stage0 is frozen: new features land in bastioc1 only. A branch that
  changes `stage0/` must say why in a commit message, with `Stage0-only:`
  (no stage1 counterpart) or `Stage0-shared:` (a bug fix made in both
  compilers, mirror required); `tests/stage0_mirror.sh` checks it. The
  `// stage1-only:` header in BastiOS's `src/main.bastio` leaves its kernel
  out of difftest's comparisons with stage0.
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
- Each function is verified on a reset solver, so whether a proof near
  the work limit succeeds no longer depends on which functions shared its
  solver: the same program could be
  accepted with one `BASTIO_JOBS` and rejected with another. Found by the
  fuzzer below.
- `tests/fuzz.py`: random programs whose meaning the fuzzer computes
  itself (a proof that holds while some input overflows, divides by zero,
  shifts too far, indexes out of bounds or breaks an `assert` or `ensures`
  is a false proof; proved programs are also run, through both backends,
  against that meaning), and damaged copies of the tests (no compiler may
  crash or hang; one that is still accepted runs under ASan and UBSan).
  bastioc1 must agree with bastioc0 on every input. CI runs a short
  deterministic run.
- A change to `stage0/src` needs its stage1 mirror, or a `Stage0-only:`
  reason in a commit message; CI checks this on every pull request (#78).
- The kernel prelude (C and native) reads the boot loader's information
  defensively: a framebuffer whose address would wrap `addr + pitch * h`,
  or lies at 4 GiB, is not reported (the verifier takes each `boot_fb`
  field as below 2^32, which such an address broke), and a memory-map entry
  whose size is 0xFFFFFFFC no longer makes the page pool's walk loop forever
  at boot. `tests/prelude_boot.sh` feeds both.
- A 60 s wall-clock backstop on each Z3 query, on top of `:rlimit`: Z3 4.8.12
  (Ubuntu's, and CI's) does not count its work on some nonlinear queries and
  searched them forever, so `bastioc0 --check` hung on a 15-line function
  (a guarded `x * x` on an `i16`). Such a query now gives up. The fuzzers
  kill a timed-out compiler's z3 too, instead of leaving it spinning.
- `tests/progfuzz.py`: random whole programs (helpers, recursion, `&mut`
  calls, a struct, an enum, `Buf`, `Bytes` and `Vec` through bounded loops,
  every integer type) run by an exact reference interpreter, then built by
  bastioc0 and bastioc1 with both backends and by bastioc0's C under ASan
  and UBSan; every build must print what the interpreter printed and exit
  the same way, and a proof the interpreter's run breaks is a false proof.

### Fixed

- `wrap[T](..)`, `wrap_add`, `wrap_sub` and `wrap_mul` are written for the
  solver without `mod`: the result is a fresh value in the type's range
  that differs from the exact one by a multiple of 2^width. Z3 4.8.12 (the
  version Ubuntu ships, which CI installs) searched past its resource
  limit, forever, on a `mod` of a wrapped value under another `mod` or a
  product; `tests/fuzz.py --seed 1` hit 9 such inputs and every one now
  takes under a second (`tests/fail/wrap_twice_overflow.bastio`).
- A false proof in both compilers. A loop or call that changes a signed
  integer (also inside a struct, enum or box) forgot its value but gave the
  new one the range of `u64`, which leaves out the negative numbers, so an
  `i64` that went negative in a loop could be proved to be 0 after it
  (`tests/fail/loop_signed_range.bastio`, `call_signed_range.bastio`). A
  changed `u8`, `u16` or `u32` got the range of `u64` too, which only made
  true facts about it unprovable. Each now keeps its type's range.
- Another false proof in both compilers: a loop or call did not look
  inside the argument of `cast[T](..)` or `wrap[T](..)`, so a variable
  changed by a call written there kept its value from before
  (`tests/fail/loop_cast_call.bastio`).
- Another false proof in both compilers: what came out of a lock built by
  `lock_new` from an integer was taken for a `u64` whatever its type, so an
  `i64` that a `locked` call made negative could be proved to be at least 0
  after `lock_end` (`tests/fail/lock_signed_contents.bastio`,
  `lock_signed_box.bastio`). The lock now keeps the checker's type of its
  contents, which also makes a `u8` that comes out known to be at most 255.

### Documentation

- For contributors: `tools/setup-dev.sh` sets up a Debian or Ubuntu machine in one command (and a dev container runs it), `docs/architecture.md` maps the compiler's phases to files, and CONTRIBUTING.md explains the issue labels and where to find a first issue. A feature-request issue template and `.github/labels.yml` (synced by a workflow) were added.
- Releases: the release workflow also builds for AArch64 Linux (on an arm64 runner, tested there), makes the archives byte-for-byte repeatable, records build provenance attestations, and `docs/releasing.md` says how to verify a download and rebuild a release.
- A documentation index (`docs/README.md`), linked from the README, and a README layout table that lists every document. `docs/integers.md` says the sized integers are implemented. CONTRIBUTING.md has a "Getting oriented" section.

- `docs/bootstrap.md`: how stage0 builds bastioc1, how to check the
  bootstrap's fixed point, and what the stage0 freeze allows.
- The trusted base is listed and explained (`docs/trusted-base.md`, #66).
- `examples/qoi`: a QOI image codec called from C. Its buffers are sized by
  an untrusted header, and running out of memory is a result C sees, not an
  abort. "Calling Bastio from C" in the book is now a guide: building,
  vendoring the generated C, ownership, errors and out of memory, several
  libraries in one program, and what the proofs assume of the C side.
- The tutorial's section on calling Bastio from C walks through a new
  example, `examples/lines`, built on `export fn` and `--header` (#92).
- `tests/run.sh` checks that error messages shown in `docs/` are what the
  compiler prints, that file copies in `docs/` match the files, and that
  `examples/*.bastio` build and run (#92).
- `docs/proof-failures.md`, "Common proof failures": the errors newcomers
  meet first, each with the program, the message, why, and a fix; on the
  book's site after the tutorial.

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
