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

What the compiler needs in order to be written in Bastio, in order:
1. **Bytes and strings:** `Bytes` (a linear u8 buffer), string literals, file reading and writing.
2. **Structs:** fields may hold linear types; rules for partial moves.
3. **Enums and `match`:** to represent the AST; exhaustiveness checking.
4. **Owning pointer `Box[T]`:** for recursive data such as trees.
5. **Generics by monomorphization:** `Vec[T]` and `Box[T]`, with a proved growable array in the standard library.
6. **Multiple files and modules.**
7. **Processes and pipes:** so the self-hosted compiler can call Z3 itself.

## v2: self-hosting

- [ ] bastioc1, written in Bastio, produces the same output as stage0 (differential testing)
- [ ] bastioc1 compiles itself and reaches a fixed point (gen2 == gen1)
- [ ] stage0 is frozen

## v3: toward a kernel

- [ ] Native x86-64 backend, without going through C
- [ ] Capability primitives: `Frame[p]` (a physical page), `Mmio[R]`, `PortIo`, `IrqOff`
- [ ] Allocation failure returned in the type (no abort on OOM)
- [ ] A small kernel: boot, a physical page allocator (with functional correctness proved), page tables

## Open design questions

- How to express concurrency in types: multiple cores, locks, sharing with interrupt handlers.
- Whether to add mutable borrows `&mut`. Today "consume and return" (`b = set(b, ...)`) stands in for them.
- Termination (currently a non-goal).
- How to write hand-written proofs (lemmas) for the cases where Z3 gets stuck on quantifiers.
