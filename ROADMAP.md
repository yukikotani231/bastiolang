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
1. [x] **Structs:** fields may hold linear types (`Frame`, `PageTable`, `Buf`, other structs); a struct with a linear field is linear; rules for moving fields out and back; a struct can be returned, so functions can hand back several values. (Done 2026-10-10. A table of linear values, such as a process table whose rows own page tables, still needs `Vec[T]` from item 5.)
2. **Bytes and strings:** `Bytes` (a linear u8 buffer), string literals (done: `Str`), file reading and writing.
3. **Enums and `match`:** to represent the AST and optional results (`Option`-like returns for allocation failure); exhaustiveness checking.
4. **Owning pointer `Box[T]`:** for recursive data such as trees.
5. **Generics by monomorphization:** `Vec[T]` and `Box[T]`, with a proved growable array in the standard library.
6. **Multiple files and modules.**
7. **Processes and pipes:** so the self-hosted compiler can call Z3 itself.

## v2: self-hosting

- [ ] bastioc1, written in Bastio, produces the same output as stage0 (differential testing)
- [ ] bastioc1 compiles itself and reaches a fixed point (gen2 == gen1)
- [ ] stage0 is frozen

## v3: toward a kernel

Started early, alongside v1, because the kernel is the primary target and it shapes what v1 needs.
- [x] Mutable borrows `&mut T` (see "Open design questions": settled)
- [x] Capabilities for physical pages and address spaces: `Pool`, `Frame`, `PageTable` (levels and slot ownership tracked), `Mmu` (the table in CR3 is never freed), `Cpu` (the user context; entering user mode)
- [x] `--target user` for programs under a Bastio kernel
- [x] A small kernel (BastiOS v0.2): boot, a page allocator whose bitmap is proved to agree with the pool, page tables, user processes with preemptive scheduling, system calls, fault isolation; tearing a process down returns every page
- [ ] More hardware capabilities: `PortIo`, `Mmio[R]`, `IrqOff`, so the UART, PIC and timer code moves from the C prelude and `platform.c` into Bastio
- [ ] Allocation failure returned in the type (no abort on OOM). Needs enums (v1)
- [ ] Native x86-64 backend, without going through C (removes the C compiler from the trusted base)

## Open design questions

- How to express concurrency in types: multiple cores, locks, sharing with interrupt handlers.
- Termination (currently a non-goal).
- How to write hand-written proofs (lemmas) for the cases where Z3 gets stuck on quantifiers.
