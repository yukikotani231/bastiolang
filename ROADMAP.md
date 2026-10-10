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

## v2: self-hosting

- [ ] bastioc1, written in Bastio, produces the same output as stage0 (differential testing)
  - [x] Lexer (`stage1/lexer.bastio`, done 2026-10-10): the same tokens and lexical errors as stage0 on every source in this repository and in BastiOS (`stage1/difftest.sh`, run in CI)
  - [x] Parser (`stage1/parser.bastio`, done 2026-10-10): the same syntax tree as stage0 (`--dump-ast`) on every source, and the same first-error position on damaged copies of them (`stage1/mutate.py`)
  - [ ] Checker (types and linearity). Loading (imports) and monomorphization are done (2026-10-10, compared through `--dump-mono`); the checker proper is next
  - [ ] Verifier (talks to Z3 through a `Child`)
  - [ ] C code generation
- [ ] bastioc1 compiles itself and reaches a fixed point (gen2 == gen1)
- [ ] stage0 is frozen

## v3: toward a kernel

Started early, alongside v1, because the kernel is the primary target and it shapes what v1 needs.
- [x] Mutable borrows `&mut T` (see "Open design questions": settled)
- [x] Capabilities for physical pages and address spaces: `Pool`, `Frame`, `PageTable` (levels and slot ownership tracked), `Mmu` (the table in CR3 is never freed), `Cpu` (the user context; entering user mode)
- [x] `--target user` for programs under a Bastio kernel
- [x] A small kernel (BastiOS v0.2): boot, a page allocator whose bitmap is proved to agree with the pool, page tables, user processes with preemptive scheduling, system calls, fault isolation; tearing a process down returns every page
- [ ] More hardware capabilities: `PortIo`, `Mmio[R]`, `IrqOff`, so the UART, PIC and timer code moves from the C prelude and `platform.c` into Bastio
- [ ] Allocation failure returned in the type (no abort on OOM). Enums exist now (v1 item 3); what is left is changing `alloc`, `pool_take` and friends to return them
- [ ] Native x86-64 backend, without going through C (removes the C compiler from the trusted base)

## v4: Linux compatibility

Decided 2026-10-10: BastiOS should run existing Linux programs, not only programs written in Bastio. The way in is a Linux system-call compatibility layer.
- [ ] Load static x86-64 Linux ELF binaries into a user address space
- [ ] Accept the Linux system-call ABI (`syscall` instruction, Linux numbers and argument registers) next to the native one
- [ ] Translate the common calls (`read`, `write`, `open`, `close`, `mmap`, `brk`, `exit`, `fork`/`execve`, ...) onto BastiOS services, growing until a static busybox shell runs

The layer is written in Bastio, so it is held to the same memory-safety proofs as the rest of the kernel. This is the opposite of fz, where the compatibility layer was assembly kept outside verification. User programs themselves are not trusted; page tables isolate them, so a misbehaving Linux binary can only fault itself. Only the small entry stub (saving registers on `syscall`) belongs to the trusted assembly base.

## Performance

Safety must not cost speed at run time; that is the point of proving it instead of checking it. Three things are tracked:
1. **Generated code.** Zero runtime checks is already an invariant, but speed is not yet measured. Compare Bastio programs with equivalent hand-written C (same algorithm, `-O2`) on a small benchmark set: a byte-stream decoder, a page allocator loop, vector-heavy code.
2. **Compile time.** Most of it is Z3. Track proof time per file and the slowest obligations, so a spec that makes the solver crawl is noticed when it lands, not months later. Self-hosting (v2) must stay practical: stage1 compiling itself should take seconds, not minutes.
3. **Kernel paths.** In BastiOS: system-call round trip, context switch, page fault to resolution, process spawn and teardown.

- [ ] `bench/` with the benchmark set above and a script that prints one line per measurement (generated-code time vs C, compile and proof time per file)
- [ ] CI runs it and keeps the numbers as an artifact, so a regression shows up next to the change that caused it
- [ ] stage1 data structures: the phases written so far use linear searches (types, instances, places, variables). Fine for today's sources (a few ms per file); replace with hashing or indexed lookups before self-compilation if measurements show they dominate
- [ ] BastiOS: a `bench` shell command (or test mode) that times the kernel paths above in QEMU (relative numbers only; QEMU timing is not hardware timing)

**Review cadence.** Performance is reviewed at every roadmap milestone (each checked item in v2 to v4) and whenever a phase of the self-hosted compiler lands: run `bench/`, compare with the previous numbers, and note anything that got slower in the PR.

## Open design questions

- How to express concurrency in types: multiple cores, locks, sharing with interrupt handlers.
- Termination (currently a non-goal).
- How to write hand-written proofs (lemmas) for the cases where Z3 gets stuck on quantifiers.
