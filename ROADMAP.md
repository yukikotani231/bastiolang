# Roadmap

Direction, decided 2026-10-09:
- Proofs are SMT-style: annotations checked by Z3.
- Memory safety is mandatory.
- Functional correctness is opt-in.
- The compiler aims at self-hosting from the start.

`stage0/` (Rust) is throwaway and will be frozen once the compiler compiles itself.

## Scope

- **Primary target:** OS kernels and drivers (BastiOS, in a separate repository once kernel work starts).
- **Also targeted:** embedded firmware, hypervisors, and safe libraries callable from C (parsers and codecs that read untrusted input).
- **Not targeted:** web apps, scripting, GUI applications.

Design consequences:
1. The standard library splits into `core` (no OS required) and `std` (uses OS services).
2. Functions can call and be called through the C ABI. At that boundary, preconditions become obligations on the caller.
3. No mandatory runtime, ever.

## Invariants

1. The language never gains a way to read or write memory through a raw integer address. All hardware access goes through capability types.
2. Proofs, capabilities and specifications are ghost and leave nothing in the generated code (zero overhead).
3. The trusted base (assembly, built-in primitives, the runtime prelude) is listed and counted mechanically. Growth gets reviewed.
4. Every new feature comes with examples in both `tests/pass` and `tests/fail`: what should be rejected is shown to be rejected.
5. Performance is measured, not assumed (see "Performance"). A change that makes generated code, the compiler or the kernel measurably slower says so in its PR, with numbers.

## v0 (done)

The proof pipeline works end to end.
- [x] u64 / bool / linear `Buf` / borrowed `&Buf`
- [x] Linearity checking: use-after-free, double free, leaks, branch mismatch, borrow-and-move aliasing
- [x] Symbolic execution and Z3 proofs: bounds, overflow, division by zero, requires/ensures/invariant/assert
- [x] Errors with counterexamples
- [x] C backend with no runtime checks, cross-checked under sanitizers
- [x] String literals (`Str`) and a linear serial-console capability (`Serial`), with written bytes proved to fit in 8 bits
- [x] Freestanding kernel target (`--target kernel`); BastiOS boots in QEMU and runs a shell written in Bastio

## v1: language features needed for self-hosting

What the compiler needs in order to be written in Bastio. Structs come first (reordered 2026-10-10): writing the BastiOS kernel showed they are the biggest gap for kernels too. Without them its process table is one `Buf` cut into 32-word rows, the per-process page tables live in a level-5 table used as a container, and a function that walks four table levels must take every level out and put it back because it cannot return more than one value.
1. [x] **Structs:** fields may hold linear types (`Frame`, `PageTable`, `Buf`, other structs); a struct with a linear field is linear; rules for moving fields out and back; a struct can be returned, so functions can hand back several values. (Done 2026-10-10. Tables of linear values came with `Vec[T]` in item 5: BastiOS keeps its address spaces in a `Vec[Space]`.)
2. [x] **Bytes and strings:** `Bytes` (a linear u8 buffer, every element proved `<= 255`), string literals (`Str`), and file reading and writing through a linear `Host` capability (hosted only), with command-line arguments. (Done 2026-10-10.)
3. [x] **Enums and `match`:** to represent the AST and optional results (`Option`-like returns for allocation failure); exhaustiveness checking. (Done 2026-10-10: variants with one payload each, linear payloads, `is` tests, payloads and tags in proofs. Not yet generic, so each `Option`-like type is declared on its own until item 5.)
4. [x] **Owning pointer `Box[T]`:** for recursive data such as trees. (Done 2026-10-10: `box`/`unbox`, field places through a box, read-only walks of borrowed trees. The prover unfolds unknown recursive data only two levels deep, so facts about deeper parts cannot be proved yet; that needs recursive spec functions or inductive predicates, which belong with functional correctness rather than memory safety.)
5. [x] **Generics by monomorphization and `Vec[T]`:** generic structs, enums and functions (done 2026-10-10, type arguments written out at each use; inferring them is deferred: monomorphization runs before type checking, so inference would mean merging the two, and the self-hosted compiler can write type arguments out), and a built-in growable `Vec[T]` whose bounds and emptiness-before-free are proved (done 2026-10-10). Revised: `Vec[T]` is built in rather than written in Bastio, because a library version would need raw memory and per-slot initialization tracking that the language does not have; a vector whose slots below its length are always full needs neither. The prover tracks the contents of vectors of any element type (done 2026-10-10), so facts such as "every row's page table is a level-4 table" can be stated as invariants over a vector.
6. [x] **Multiple files and modules:** `import "path";` loads other files into the program, once each, with import cycles rejected (done 2026-10-10). Revised: all files share one namespace, as in C. Qualified names (`mod::f`) and private items are left for later: they matter for libraries shared between programs, and the self-hosted compiler can do without them.
7. [x] **Processes and pipes:** so the self-hosted compiler can call Z3 itself. (Done 2026-10-10: a linear `Child` from `host_spawn`, with line-based reads from its output; `tests/pass/child_z3.bastio` asks Z3 a question interactively.)
8. [x] **Sized integers** (design: [docs/integers.md](docs/integers.md), 2026-10-10): `u8` to `u64` and `i8` to `i64`, every width proved never to wrap; literals that take their type from context; `cast[T]` (proved to fit) and `wrap[T]` (modular) instead of a silently truncating `as`; C types at the ABI. Added because u64 alone forces range facts into special error codes (`E-VERIFY-BYTE`) and cannot express a 16-bit port or a 32-bit register. Four steps: unsigned widths and conversions (done 2026-10-10: `u8`, `u16`, `u32`, contextual literals, `cast`, `wrap`, `wrap_add`/`wrap_sub`/`wrap_mul`, in stage0 and the stage1 checker), then signed types (done 2026-10-10: `i8` to `i64`, unary minus, C's truncating division, in stage0 and the stage1 checker), both before stage1's verifier and code generator are written so those are written once; then bitwise operators (done 2026-10-10: `&`, `|`, `^`, `<<`, `>>`, `!x` on unsigned types; shift amounts proved below the width, left shifts proved to lose no bits), and the byte-shaped builtins moving to `u8` together with `PortIo` (done 2026-10-10: `bget`, `bset`, `sget`, `serial_read`/`serial_write` and `frame_read8`/`frame_write8` use `u8`; port, MMIO and DMA accesses use the type of their width; `E-VERIFY-BYTE` is gone, in bastiolang and BastiOS). Left over: `serial_poll` still returns a `u64` with 256 for "nothing waiting", rather than an enum.

## v2: self-hosting

- [ ] bastioc1, written in Bastio, produces the same output as stage0 (differential testing)
  - [x] Lexer (`stage1/lexer.bastio`, done 2026-10-10): the same tokens and lexical errors as stage0 on every source in this repository and in BastiOS (`stage1/difftest.sh`, run in CI)
  - [x] Parser (`stage1/parser.bastio`, done 2026-10-10): the same syntax tree as stage0 (`--dump-ast`) on every source, and the same first-error position on damaged copies of them (`stage1/mutate.py`)
  - [x] Checker (`stage1/loader.bastio`, `mono.bastio`, `check.bastio`, done 2026-10-10): imports, monomorphization, types and linearity, with the same result as stage0 (`--dump-mono`, `--dump-check`, including the type of every `let`) on every source and on damaged copies, library files included
  - [x] Verifier
    - [x] Queries (`stage1/verify.bastio`, done 2026-10-10): symbolic execution produces the same SMT-LIB commands as stage0, character for character, and the same number of proof obligations (`--dump-smt`), on every program stage0 verifies (the BastiOS kernel included) and on damaged copies that still verify. Every proof is assumed to succeed for now. Generating all 273,000 commands for stage1's largest program takes 0.7 s; stage0 takes 3.6 s on it, nearly all of it Z3
    - [x] Proofs (`stage1/solve.bastio`, done 2026-10-10): Z3 runs as a `Child` and gets exactly the commands stage0 sends it; the first failed proof is reported with stage0's error code, position, counterexample and notes (`--dump-verify`), on every program that reaches the verifier and on damaged copies (`mutate.py --verify`), most of which fail a proof. Proof time is Z3's: on one solver session it is the same as stage0's within measurement noise (5.3 s against 4.9 s on stage1's largest program, 3.5 s on the BastiOS kernel). stage0 now proves functions in parallel (#46) and is 2.6x faster on 4 cores (0.9 s against 2.6 s on the kernel); stage1 should do the same, with the dumps keeping one session
  - [ ] C code generation
  - [ ] The driver: one `bastioc1` program for the whole pipeline, with stage0's full error messages (the sentence and the source excerpt; the phases report codes, positions and notes today)
- [ ] bastioc1 compiles itself and reaches a fixed point (gen2 == gen1)
- [ ] stage0 is frozen

## v3: toward a kernel

Started early, alongside v1, because the kernel is the primary target and it shapes what v1 needs.
- [ ] **Concurrency** (design: [docs/concurrency.md](docs/concurrency.md), 2026-10-10). Data-race freedom becomes part of mandatory memory safety, with no `unsafe` escape: `par` and `spawn`/`join`, `Share[T]` (read-only sharing with ghost fractions instead of a reference count), `Lock[T]` and `Atomic[T]` guarded by type invariants, lock ranks proved at compile time, pinned per-core capabilities, and interrupt-safe locks. Prioritized because kernels and drivers need it and it is where Rust kernels still rely on `unsafe` (the first CVE in Rust-for-Linux code, CVE-2025-68260, was a race in an `unsafe` block). Implemented in the self-hosted compiler right after the fixed point, so the checker is not written twice, in four steps:
  - [ ] Fork-join and threads on the hosted target (`par`, `spawn`/`join`, `Share`, pinned types)
  - [ ] Locks and atomics (type invariants, `Lock`/`Held`, `Atomic`, `Ctx` with ranks)
  - [ ] BastiOS on several cores (`smp_start`, one kernel lock, then per-core run queues)
  - [ ] Interrupt handlers and interrupt-safe locks, with the first driver that needs them
- [x] Mutable borrows `&mut T` (see "Open design questions": settled)
- [x] Capabilities for physical pages and address spaces: `Pool`, `Frame`, `PageTable` (levels and slot ownership tracked), `Mmu` (the table in CR3 is never freed), `Cpu` (the user context; entering user mode)
- [x] `--target user` for programs under a Bastio kernel
- [x] A small kernel (BastiOS v0.2): boot, a page allocator whose bitmap is proved to agree with the pool, page tables, user processes with preemptive scheduling, system calls, fault isolation; tearing a process down returns every page
- [x] A file system (BastiOS, 2026-10-10): an in-memory file system whose data pages are owned by its files (removing a file gives each page back exactly once), a per-process open-file table, and open/close/read/write/seek/unlink/stat/readdir system calls. User programs make them through three generic system calls (`sys_call`, `sys_send`, `sys_recv`), so a new kernel call needs no compiler change; the Linux layer (v4) will use the same route
- [x] Files on a disk (BastiOS, 2026-10-10): a legacy ATA PIO driver whose port accesses are proved to stay inside its `PortIo` capabilities, and a disk file system (superblock, 32-entry directory, a fixed 1 MiB slot per file) under the same file layer as the in-memory one. Names under `disk/` survive reboots. No journal
- [ ] Running programs from files instead of images built into the kernel
- [x] `Ports` / `PortIo`: a driver claims its I/O ports once and can touch only those; no two owners per port. BastiOS's UART, PIC and timer drivers moved from the C prelude and `platform.c` into Bastio, and so did the interrupt acknowledgement (from the trap entry assembly into the kernel loop)
- [x] `MmioSpace` / `Mmio`: memory-mapped device registers in the 3-4 GiB device window, claimed once like ports, with bounds and alignment proved. BastiOS maps the window uncached at boot and reads the HPET through it (the local APIC still lives in trusted C until SMP moves it)
- [x] DMA buffers as linear in-flight tokens (docs/concurrency.md): `dma_lend` turns a page into a `Dma` that cannot be freed, mapped or reused until `dma_take`, and is touched only with volatile, aligned accesses. When the device is done is the driver's word
- [ ] Typed register layouts (`Mmio[R]`) if plain offsets prove clumsy
- [x] Networking, first step (BastiOS, 2026-10-10): virtio-net through `Dma` pages, and a minimal stack (Ethernet, ARP, IPv4 without options, ICMP echo, UDP echo) whose every read of a frame is proved to stay inside it whatever the length fields say. Tested in QEMU user-mode networking (ping the gateway, UDP echo through a port forward). Outgoing checksums are computed but not proved correct
- [ ] Sockets for user programs (UDP first, then TCP), shared with the Linux layer (v4) so musl programs can use them; DHCP instead of a fixed address
- [ ] `IrqOff` and interrupt handlers (docs/concurrency.md, step 4). Not needed while BastiOS treats interrupts as events with interrupts off in the kernel
- [ ] Allocation failure returned in the type (no abort on OOM). Enums exist now (v1 item 3); what is left is changing `alloc`, `pool_take` and friends to return them
- [ ] Native x86-64 backend, without going through C (removes the C compiler from the trusted base)

## C boundary

What the "safe libraries callable from C" target needs (Scope, design consequence 2).
- [x] A first demo: `examples/base64`, a strict base64 codec proved memory safe and called from a C harness, with a differential fuzz against a C decoder and a benchmark (done 2026-10-10). It first went through a hand-written wrapper, `base64_capi.c`; `export fn` replaced it
- [x] `export fn` (done 2026-10-10): a non-static C symbol with a generated header (`--header`), no `main` required for a library. Preconditions are ruled out (`E-EXPORT`): a C caller cannot be proved to meet one, so the function checks the condition itself and returns an error, which keeps generated code free of checks. Integers, `bool` and `&Bytes` (a pointer and a length) go in; numbers, `Bytes` and structs and enums of those come out. What the proofs assume of the C caller is listed by `--trusted-base`. stage1 parses and checks it too. `examples/base64` no longer needs its hand-written wrapper
- [x] Caller-provided output buffers (done 2026-10-10): an `export fn` may take `&mut Bytes` (`uint8_t *p, size_t p_len`), so a library need not allocate. That the function never frees or replaces the buffer is proved from the `&mut` summaries the verifier already infers (only shape-keeping writes, through any depth of calls), so the wrapper has nothing to copy back. `examples/base64` gained `base64_encode_into`
- [x] More C-callable libraries (2026-10-10): `examples/utf8` (strict UTF-8 validation, UTF-32 decoding into a caller's buffer) and `examples/der` (DER/ASN.1 structure checking and walking), each with a differential fuzz against an independent C implementation under ASan + UBSan, and a C twin in `bench/` (`bench/utf8` 1.02-1.04x, `bench/der` 0.75-0.77x the time of C)

## v4: Linux compatibility

Decided 2026-10-10: BastiOS should run existing Linux programs, not only programs written in Bastio. The way in is a Linux system-call compatibility layer.
- [ ] Load static x86-64 Linux ELF binaries into a user address space
- [ ] Accept the Linux system-call ABI (`syscall` instruction, Linux numbers and argument registers) next to the native one
- [ ] Translate the common calls (`read`, `write`, `open`, `close`, `mmap`, `brk`, `exit`, `fork`/`execve`, ...) onto BastiOS services, growing until a static busybox shell runs

The layer is written in Bastio, so it is held to the same memory-safety proofs as the rest of the kernel. This is the opposite of fz, where the compatibility layer was assembly kept outside verification. User programs themselves are not trusted; page tables isolate them, so a misbehaving Linux binary can only fault itself. Only the small entry stub (saving registers on `syscall`) belongs to the trusted assembly base.

## Performance

Safety must not cost speed at run time; that is the point of proving it instead of checking it. Three things are tracked:
1. **Generated code.** Compare Bastio programs with equivalent hand-written C (same algorithm, `-O2`). A first measurement (2026-10-10, array sum, sieve, binary tree, vector push) found tight loops on par with C, and two runtime gaps: `Vec` growth (`bst_vec_grow` used `calloc` plus a byte-by-byte copy, about 3x slower pushes; fixed with `realloc`, now 1.04x) and allocation order in tree building (about 1.4x; fixed by allocating a box before evaluating its contents, so trees are laid out parent first, now 0.97x). A native backend (v3) would lose the C compiler's optimizations, so it must be measured against the C backend before it replaces it.
2. **Compile time.** Most of it is Z3. Track proof time per file and the slowest obligations, so a spec that makes the solver crawl is noticed when it lands, not months later. Self-hosting (v2) must stay practical: stage1 compiling itself should take seconds, not minutes.
3. **Kernel paths.** In BastiOS: system-call round trip, context switch, page fault to resolution, process spawn and teardown.

- [x] Fix `Vec` growth in the C prelude (`realloc` instead of `calloc` plus a byte copy): `bench/vec` went from 2.96x to 1.04x the time of C
- [ ] BastiOS: `user_byte` walks all four page-table levels for every byte it copies, and the kernel allocator scans its free list linearly and zeroes every page; measure, then batch per page
- [x] `bench/`, starting from the programs of the first measurement (sum, sieve, tree, vec, each with its C twin), with a script that prints one line per measurement (generated-code time vs C, compile and proof time per file): `bench/run.sh`
- [x] CI runs it and keeps the numbers as an artifact (`bench-<sha>`), so a regression shows up next to the change that caused it
- [x] One solver scope per function (2026-10-10): the verifier used to keep every function's declarations and assumptions in one Z3 context, so each function's queries also carried the quantified facts of all earlier ones. Popping them after each function took the BastiOS kernel from 4.5 s to 1.6 s and `stage1/checkdump.bastio` from 9.3 s to 2.3 s. `BASTIO_TIME=1` prints the proof time of each function
- [x] Functions proved in parallel (2026-10-10): since each function has its own scope, the functions are shared out among several Z3 processes (`BASTIO_JOBS`, default the CPU count, at most 8). The error reported is still that of the first failing function in program order. The kernel had grown back to 4.3 s; on 4 CPUs it now takes 1.4 s
- [ ] stage1 data structures: the phases written so far use linear searches (types, instances, places, variables). Fine for today's sources (checking all of stage1, about 6,000 lines, takes 47 ms, the same as stage0); replace with hashing or indexed lookups before self-compilation if measurements show they dominate
- [ ] BastiOS: a `bench` shell command (or test mode) that times the kernel paths above in QEMU (relative numbers only; QEMU timing is not hardware timing)

**Review cadence.** Performance is reviewed at every roadmap milestone (each checked item in v2 to v4) and whenever a phase of the self-hosted compiler lands: run `bench/`, compare with the previous numbers, and note anything that got slower in the PR.

## Open design questions

- Concurrency: settled in [docs/concurrency.md](docs/concurrency.md); its own open questions (monotonic facts, blocking, channels, multi-threaded user processes, weaker atomics, RCU) are listed there.
- Termination (currently a non-goal).
- How to write hand-written proofs (lemmas) for the cases where Z3 gets stuck on quantifiers.
