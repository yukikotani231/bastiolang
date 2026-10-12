# The IR between the checker and the native backend

Status: the data, the lowering of the first subset and a text form exist (`stage1/ir.bastio`, `stage1/irdump.bastio`, golden tests in `tests/ir`); nothing generates code from it yet. Tracks [#125](https://github.com/yukikotani231/bastiolang/issues/125), and is the base of #126 (register allocation), #127 (optimizations), #128 (vectorization), #124 (AArch64) and #129 (native as default).

## Why

`stage1/x86.bastio` (about 9,600 lines) writes assembly in one pass over the checked tree. Everything it knows about a value, it decides while walking: whether a variable lives in a register (`x86_assign_regs`, a pre-pass over the tree that counts uses), where a struct's words sit, what is left in `%rax`. A peephole pass cleans up after it. The items still open in the ROADMAP's native-backend entry (results built in registers, struct fields in registers, more argument registers, vectorization) all need to ask questions of the whole function (which values are live together, which loads repeat, which stores are dead), and a tree walk that has already written text cannot answer them.

Measured today (valgrind instructions, native over C, ROADMAP "Performance"): sieve 1.00, tree 1.17, utf8 1.33, vec 1.26, der 1.21, sum 1.24. The remaining gaps are exactly the ones above.

## Shape

One IR function per Bastio function instance (after `mono`), after verification, with ghost code (`spec`, `ghost`, `assert`, `requires`/`ensures`, loop invariants) already erased. Three-address code on virtual registers, in basic blocks:

- **Values** are 64-bit virtual registers (`v0`, `v1`, ...). Every sized integer is kept widened to 64 bits with its type's width recorded on the operation that can wrap (`wrap_add`, `wrap_mul`, casts); the proofs already guarantee that plain `+`, `-`, `*` do not overflow, so those need no normalisation. This is the one place the IR uses what the verifier proved: the backend never emits an overflow check, and now it also never emits a truncation it does not need.
- **Memory** is explicit: `load.w`, `store.w` (w = 1, 2, 4, 8 bytes) on an address that is a virtual register plus a constant. Frame slots, `Buf`/`Bytes`/`Vec` elements, `Box` and `Str` bytes are all addresses.
- **Blocks** end in `jmp`, `br v, then, else` or `ret`. `loop` and `while` lower to a header block with the exit test. `match` lowers to a `switch` on the enum tag.
- **Calls** take their arguments in virtual registers and return up to two words in registers (as today); bigger results are written through an address.
- **Not SSA at first.** Virtual registers may be assigned more than once. Building SSA (with block parameters rather than phi nodes) is a second step, taken only if the passes of #127 need it; a first allocator and constant folding work without it, and not requiring it keeps the lowering a straight walk of the tree.

Proof-derived facts are carried as annotations rather than as operations: a `load` or `store` that the proofs showed in bounds is simply a plain `load`/`store` (as in the C backend); #127 may later attach ranges and no-alias facts to values so a pass can use them.

## Scalar replacement

A struct or enum value whose address is never taken (never passed as `&`, never in an `&mut` borrow that outlives the statement, never stored through a `Buf`) is split into one virtual register per word at lowering time. Its fields are then ordinary values. Everything else stays in a frame slot. The checker already computes, for the register pre-pass, which names are borrowed or reassigned (`x86_borrows`, `x86_assigned`, `x86_lends`); the lowering reuses those answers instead of re-deriving them.

## Backends

`x86.bastio` becomes three parts: lowering (tree to IR, target independent), the register allocator (linear scan over live intervals, with the existing priorities as the first heuristics), and the emitter (IR to x86-64 text). The emitter is the small, target-specific piece, so AArch64 (#124) is the same IR with another emitter and its own allocator parameters (registers, calling convention).

## Getting there without a flag day

The backend is the compiler's largest file, covers the kernel and user targets (locks, device access, `smp_run`, the pool and page-table builtins, about 5,000 of its lines are builtins), and is compared byte for byte with stage0 by `asmdump --stage0` for the part it shares. A rewrite that has to cover all of it before anything runs would be unreviewable. So:

1. **Per-function fallback.** The new path lowers one function; if it meets a node it does not handle yet, it says so and that function goes through the existing generator. Both emit into the same assembly file with the existing calling convention, so mixed output links and runs. `difftest.sh` already builds and runs every `tests/pass` program, so each step is checked on the whole corpus.
2. **First subset:** integer and bool locals, control flow, calls of up to four words, `Buf`/`Vec` element access, structs of scalars. That is the code `bench/` measures and the part of the kernel that is not a builtin.
3. **Measure before widening.** `bench/run.sh` prints native against C; a step lands only if no `bench/` program is slower and the instruction counts do not rise. The done condition of #125 is exactly this, for all programs, with no function left on the old path.
4. **Then** builtins, `Lock`/device/SMP forms, and the removal of the old generator; stage0's native backend stays as the reference for the part it covers.

## Where it stands

`stage1/ir.bastio` has the operations, `ir_lower` (one function instance to IR, or the first reason it is outside the subset), a pass that drops code no jump reaches, and `ir_print`. `irdump FILE` prints the IR of every function of a program; `tests/ir.sh` compares it with `tests/ir/*.expected`. The subset so far is integer and bool parameters, locals and results; arithmetic, comparisons, `&&`, `||`, `!`, unary `-`; `let`, assignment to a variable, `if`, `while`, `return`, and calls of non-generic functions of the same kind. Not yet: the `wrap_*` builtins, `match`, structs and enums, `Buf`/`Bytes`/`Vec`, borrowed parameters. The IR is not in SSA form and has no blocks as such, only labels; the register allocator and the emitter come next.

## What this does not change

The verifier, the checker and the C backend. Ghost code is erased before lowering, as it is before C generation, so nothing the IR contains was ever unproved. The trusted base does not grow: the backend is already trusted to translate faithfully, and the IR is an internal form of the same translation. (The C compiler leaves the trusted base only when native is the default, #129.)

## Open questions

- Whether the AArch64 work starts from the first IR subset or waits for scalar replacement; the first is faster to a running program, the second avoids a second round of emitter changes.
- Whether SSA is worth building for #127, or whether value numbering over blocks and a liveness-based dead-store pass are enough for the loops BastiOS spends its time in. The benchmarks above say what is left on the table; the answer should come from where the remaining instructions are, not from the textbook.
