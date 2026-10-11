# The trusted base

A Bastio program is proved memory safe, but the proof rests on things that are not themselves proved. This page lists all of them, says why each one is there, and says how each could shrink. ROADMAP invariant 3 asks for the trusted base to be listed and counted mechanically; this page is the reading guide to that listing.

Sizes are not copied here, because copied numbers go stale. The hand-written part is counted by the compiler itself:

```sh
stage0/target/release/bastioc0 --trusted-base
```

Its output is checked into [`tests/trusted_base.txt`](../tests/trusted_base.txt), and `tests/run.sh` fails when the two differ, so any growth shows up in a diff and gets reviewed. The tools are counted with `wc -l stage0/src/*.rs` and `wc -l stage1/*.bastio`.

BastiOS has its own trusted base on top of this one (boot code, processor startup, interrupt entry); see [BastiOS's docs/trusted-base.md](https://github.com/yukikotani231/bastios/blob/main/docs/trusted-base.md).

## 1. Tools

| Part | What is trusted | Why it is there |
| --- | --- | --- |
| Z3 | When it answers `unsat`, the question really has no model | Proofs are SMT queries. The compiler only ever trusts `unsat`: `sat` and `unknown` (also when the solver reaches its work limit, `:rlimit`) reject the program, in stage0 (`stage0/src/verify.rs`, `SatResult::Unknown`) and in stage1 (`stage1/solve.bastio`). So a solver bug can make Bastio accept a wrong program only by answering `unsat` wrongly, never by giving up |
| The compiler (stage0, Rust; stage1, Bastio) | The checker enforces linearity, the verifier asks Z3 the right questions (every obligation, with sound encodings of arithmetic, arrays and capabilities), and code generation emits what was proved | This is the largest trusted part. stage1 is checked against stage0 by differential tests (`stage1/difftest.sh`: same tokens, syntax trees, types, SMT commands, verdicts and C), so a bug has to be in both to go unnoticed. Once stage0 is frozen, stage1 is the trusted compiler, built by itself from a stage0 bootstrap (the fixed point in ROADMAP v2) |
| `rustc` and Cargo | Build stage0 correctly | Only stage0; `rust-toolchain.toml` pins the version |
| The C compiler and linker | Translate the generated C as C11 says, including the memory model for `Lock` and `Atomic` | The default backend writes C. The generated C has no undefined behavior that the proofs do not rule out: no overflow, no out-of-bounds access, a fixed evaluation order (`operands()` in `stage0/src/codegen.rs` hoists side effects into temporaries). Converting an out-of-range value to a signed type is implementation-defined in C and modular in GCC and Clang; `wrap` to a signed type relies on that ([integers.md](integers.md)) |
| The C library (hosted target only) | `calloc`, `stdio`, `fork`/`exec`/pipes for `Child`, and pthreads | Hosted programs run on an operating system anyway |

The native backend (`--backend native`, hosted only, experimental) writes x86-64 assembly instead of C, which takes the C compiler out of the trusted base for the program's own code. The assembler and linker stay. stage1's port of it (`stage1/x86.bastio`) also builds BastiOS's kernel and user programs, with no C compiler for any Bastio code (the kernel's platform code in `boot/` stays C).

## 2. Runtime preludes

Each target links in a hand-written C prelude, emitted at the top of every generated file (`prelude_parts` in `stage0/src/codegen.rs`, copied byte for byte in `stage1/cprelude.bastio`). It is the only C a Bastio program runs on. `--trusted-base` lists each part with its count of non-blank lines and which targets link it.

| Part | Targets | What it does | Why it is C |
| --- | --- | --- | --- |
| `common` | all | The C types behind each Bastio type; the spin lock behind `Lock` | Atomics need the C11 memory model |
| `hosted` | hosted | Console on stdin/stdout, `Buf` allocation, files and arguments through `Host` | Wraps the C library |
| `bytes`, `bytes_arena` | all | `Bytes` allocation and access; growing a `Vec`'s storage, and reserving room in it (`bst_vec_try_grow`, trusted to leave the vector unchanged when it fails, to give at least the room asked for when it succeeds, and never to make the capacity more than 2^60 elements, which the prover assumes of every vector); the bulk copy routines behind `bcopy`, `bmove` and the page copies (`rep movsq` / `rep movsb` on x86-64, a byte loop backward for an overlapping `bmove`), trusted to copy exactly the `n` bytes the proof checked | Allocation is below the language: Bastio has no raw memory to build an allocator from |
| `child` | hosted | `Child`: fork, exec, pipes, line reads | Wraps POSIX |
| `pages`, `pt_hook_hosted` | hosted, kernel | `Pool`, `Frame`, `PageTable`: zeroing a taken page, page-table entry encoding, reading an entry in place for `pt_borrow`, the run-time lease count of each top-level table, the reference count of each shared page (`Shared`) | These turn linear capability values into memory accesses at physical addresses. The proof tracks who owns each page; the prelude is trusted to touch only the page it is handed. Since tables can be shared between processors, it is also trusted to write each entry with a single 64-bit store, present bit included, in release order (`__atomic_store_n`), and to keep the lease count exact (an atomic add in `mmu_enter`, an atomic subtract in `mmu_exit` after the CR3 load). Page numbers are dense over the runs of RAM (`bst_page_at` and `bst_frame_id` translate through a table of at most 32 runs), and the prelude is trusted to translate both ways exactly. The counts are indexed by page number, one 32-bit counter of each kind per pool page in RAM carved off the front of the pool, which the pool setup caps at 2^27 pages (the verifier's `FRAME_LIMIT`), so the index is always in bounds. For shared pages it is trusted to count references exactly (1 at `shared_new`, an atomic add at `shared_dup` and `pt_dup_shared`, an atomic subtract at `shared_drop`, which reports the last one) and to mark a slot that maps one with PTE bit 9, which the CPU ignores, so `pt_shared` reads the truth; the proof makes each `Shared` one reference that is used exactly once, so an exact count frees a page only when no table maps it. A page taken with `pool_take_dirty` is not zeroed; the prelude is trusted to overwrite every byte of it in `dirty_zero`, `frame_copy`, `shared_copy` and `dirty_fill`, the only ways it becomes a `Frame`. A table reaches CR3 only through `mmu_enter`, so every loaded table is counted |
| `kernel` | kernel | COM1 for the kernel's own panic messages and for programs that use `Serial`, the 8 MiB arena behind `Buf` and `Bytes` and its extension from RAM, the page pool from the Multiboot memory map, the check that `mmio_claim` stays off the pool's RAM | The arena and the pool are where the kernel's memory comes from. The pool reads the boot loader's memory map, which is untrusted input from firmware: it reads the map only inside the first 1 GiB, takes only available entries, clipped to page boundaries, above the kernel image and below `bst_ram_limit` (1 GiB if the platform does not define it). RAM above 2 GiB lies inside the device window, so `mmio_claim` halts on a block that overlaps the pool's RAM or its bookkeeping: without that check a claimed register block could alias pages the proofs hand out |
| `kernel_cpu` | kernel | CR3 loads (with the lease count in `mmu_enter` and `mmu_exit`), the saved user context, FPU state save and load, entering user mode and idling, processor count, the kernel fault report | Control registers and privileged instructions |
| `ports`, `ports_kernel`, `ports_hosted` | hosted, kernel | One `in` or `out` instruction per port access (cells in memory on the hosted target, for testing drivers) | The instruction itself. Which ports a driver may touch is proved: claims are ghost |
| `mmio`, `mmio_kernel`, `mmio_hosted` | hosted, kernel | One volatile load or store per register access | Same as ports. Bounds, alignment and exclusive claims are proved |
| `dma` | hosted, kernel | Fences when a page is lent to a device and taken back; volatile access while lent; compiler barriers around `dma_to_bytes` and `bytes_to_dma` | Ordering against a device is outside the language |
| `user` | user | System call stubs (`int 0x80`), buffered console, the 64 KiB arena | The kernel's ABI |
| `posix` | hosted | `_POSIX_C_SOURCE` | Needed before any header |

The native backend has its own runtime in assembly (`RUNTIME` in `stage0/src/x86.rs`), also listed and counted: printing, the console and the allocator, over the C library. stage1's backend adds routines for `Child`, `host_env`, `host_cpus` and the simulated devices (`x86_runtime1`), and for user programs and kernels it replaces the whole runtime: one over the kernel's system calls (`x86_runtime_user`, about 200 lines: the `user` and `bytes_arena` parts in assembly), and one for the kernel (`x86_runtime_kernel`, about 450 lines: the `kernel`, `kernel_cpu` and `bytes_arena` parts, calling the platform's C where the C prelude does). Ports, MMIO, DMA, page tables and control registers are single instructions in the generated code, as in the C prelude; the lease counts are a 256 KiB array (a 32-bit count per pool page) in the runtime's `.bss` (`bst_rt_pt_leases`), changed with `lock add` and `lock sub` in the code for `mmu_enter` and `mmu_exit`. The reference counts of shared pages are a second such array (`bst_rt_shared_refs`): `shared_new` stores 1, `shared_dup` and `pt_dup_shared` use `lock inc`, and `shared_drop` uses `lock dec` and reports the page when the count reaches zero; a slot mapping a shared page carries PTE bit 9, as in the C prelude. `dirty_zero` is `rep stosq` and `frame_copy`, `shared_copy` and `dirty_fill` are `rep movsq` over the whole page, as `pool_take` zeroes one. stage1's backend also rewrites its own output with a peephole pass (`x86_peephole`): a push and its pop become a move, with only register moves and arithmetic that leave the pushed operand's registers alone in between, a copy through `%rax` becomes a direct move where the next instruction overwrites `%rax`, and a chain of arithmetic computed in `%rax` and then moved to another register is computed in that register. It is trusted like the code generator itself, and checked the same way: every `tests/pass` program built natively must behave as the C build does.

The compiler also generates small helpers for each `Box` and `Vec` element type and a thunk per `smp_run` call. Those are part of the compiler (section 1), not of the listing.

## 3. Builtins

Every builtin (the `builtin` lines of the listing, with the targets that have it) is an axiom: the checker gives it a type, the verifier gives it a specification, and the prelude implements it. What is trusted is that the implementation does what the specification says. Examples: `pool_take` returns a zeroed page that nothing else owns; `pt_take` hands back exactly the page that `pt_put` stored in that slot; `vec_get` reads the element the verifier thinks is there. A builtin whose run-time state differs from what the proof believes is a soundness bug, and one such bug has been found and fixed (`pt_has` on the kernel's own slot of a top-level table, #39).

**Counting lemmas.** `vcount(v, x)` (specifications only) is an uninterpreted function `cnt` of a vector's element array, its length and the value. What the prover knows about it is given as axioms, each a theorem about finite sequences proved by induction on the length, outside the solver:
- a count is between 0 and the length;
- an element at an index below the length is counted at least once;
- a value counted at least once is the element at some index below the length (`cnt_w` names that index);
- a push of `y` adds one to the count of `y`; a pop removes one from the count of the last element; `vec_set` and `vec_swap` of `y` at `i` remove one from the count of the old element at `i` and add one to the count of `y`.

`--trusted-base` lists them (the `lemma` lines). They are written out in `COUNT_LEMMAS` and `count_step` in `stage0/src/verify.rs` (`count_lemmas` and `count_step` in `stage1/verify.bastio`). They are sent only for functions whose specifications use `vcount`, or call one whose contract does, so a program that never counts is verified exactly as before.

**The definition of `bdec`.** `bdec(b, from, to)` (specifications only) is an uninterpreted function `dec` of the bytes' data array and the range, defined by axioms: it is 0 for an empty range and `10 * dec(a, f, t - 1) + (a[t - 1] - 48)` otherwise. The recursive use is written with a copy, `dec_l`, which equals `dec` wherever `dec` is mentioned and is 0 on an empty range, but is never unfolded itself; that stops the solver from unfolding the definition forever. A definition by recursion on a decreasing range is consistent, but the solver does not check that, so it is listed (the `definition` line of `--trusted-base`). It is written out in `DEC_AXIOMS` in `stage0/src/verify.rs` (`dec_axioms` in `stage1/verify.bastio`) and sent only for functions whose specifications use `bdec`, the same way as the counting lemmas.

**The bits of a number.** `has_bit(x, i)` (specifications only) is `i < 64` and an uninterpreted predicate `bit(x, i)`. The prover is told that no bit of 0 is set and that bits 0 to w - 1 of 2^w - 1 are, for w = 8, 16, 32 and 64 (`BIT_AXIOMS`, `bit_axioms`), and, at each bitwise operator a function evaluates, which bits its result has: bit j of `x & y` is bit j of `x` and of `y`, of `x | y` either, of `x ^ y` exactly one, of `!x` (w bits wide) j < w and not bit j of `x`, of `x >> s` bit j + s of `x`, of `x << s` j >= s and bit j - s of `x`, of `x % 2^k` (a low mask) j < k and bit j of `x`, and `x % 2` is 1 exactly when bit 0 of `x` is set (`bit_fact` in both verifiers). These are facts about binary numerals; the solver does not check them. They are sent only for functions whose specifications use `has_bit`, the same way as the counting lemmas.

**The definition of `bhex`.** `bhex(b, from, to)` is the same in base 16 (`HEX_AXIOMS`, `hex_axioms`; functions `hex` and `hex_l`): a byte `c` counts as `c - 48` for `0-9`, `c - 87` for `a-f` and `c - 55` otherwise, which is right for `A-F`.

## 4. Assumptions at the boundaries

**`export fn`.** A C caller cannot be proved to do anything, so the proof of an exported function assumes things about it. They are listed by `--trusted-base` (the `export assumes` lines) and written into the generated header: buffer pointers are valid for their lengths and not written during the call, `&mut Bytes` arguments do not overlap, and every argument is a valid value of its C type.

**The kernel target's platform.** A program built with `--target kernel` expects the platform it links with to provide: an identity map of the first 1 GiB, and of all RAM below `bst_ram_limit` if it defines that (BastiOS maps RAM above 2 GiB write-back and sets it to 512 GiB, or 4 GiB without 1 GiB pages), the device window (2 GiB up to 2^46) mapped uncached, with the code running on the kernel's own tables whenever it touches a claimed `Mmio` (a user address space shares only the first 512 GiB), the kernel's top-level page table `bst_kernel_pml4`, `_end` at the end of the image, a Multiboot information block, and the functions `bst_cpu_enter`, `bst_cpu_wait`, `bst_platform_ctx`, `bst_platform_cpus`, `bst_platform_smp_start` and `bst_platform_smp_join` (declared in the `kernel_cpu` prelude), and, if it calls `boot_acpi`, `bst_platform_acpi`, which finds the ACPI tables and must answer 256 past a table's end, and, if it calls `boot_info`, `bst_platform_info`, of which nothing is assumed beyond returning. BastiOS provides them in its `boot/` directory, which is its trusted base.

**The user target.** System call numbers and registers are BastiOS's ABI. A user program trusts the kernel to do what each call says; the kernel trusts the user program with nothing.

**Devices.** Capabilities prove that a driver touches only the ports, registers and lent pages it claimed. Whether the device behaves is trusted: that it writes only the pages a driver lent it, that it has stopped using a page when the driver takes it back (`dma_take`), and that the addresses the driver programs into it are those of pages it lent. The hardware and the emulator (QEMU) are trusted throughout.

## 5. What the proof does not cover

These are not trusted parts so much as properties nobody claims:
- Termination: a loop that never ends is not detected.
- Functional correctness, unless the program states it with `requires`, `ensures`, `invariant` and `assert`.
- The meaning of data: the proof knows a page is owned once, not what is in it.
- Timing and other side channels.

## 6. How it shrinks

- **The C compiler**, by the native backend (ROADMAP v3, "Native x86-64 backend"). It covers the hosted target, and in stage1 also the kernel and user targets (their preludes rewritten in assembly), though BastiOS still builds with the C backend until bastioc1 offers the native one for them.
- **Preludes into Bastio**, as capabilities appear that can express them. This has already happened once: BastiOS's UART driver used to be C in the kernel prelude, and now only the one `in` or `out` instruction behind each port access is trusted. The `kernel` prelude's COM1 code is still there for kernels that use `Serial` and for the prelude's own messages.
- **The Multiboot memory map** is parsed in trusted C because Bastio cannot read memory at an address it was not given as a capability (ROADMAP invariant 1). A read-only capability for boot-time firmware memory would move this, and BastiOS's ACPI parsing, into Bastio.
- **stage0** leaves when it is frozen; the trusted compiler is then stage1, which is checked by its own differential tests against stage0's last output and by the bootstrap fixed point.
