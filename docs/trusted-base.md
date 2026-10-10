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
| Z3 | When it answers `unsat`, the question really has no model | Proofs are SMT queries. The compiler only ever trusts `unsat`: `sat`, `unknown` and a timeout all reject the program, in stage0 (`stage0/src/verify.rs`, `SatResult::Unknown`) and in stage1 (`stage1/solve.bastio`). So a solver bug can make Bastio accept a wrong program only by answering `unsat` wrongly, never by giving up |
| The compiler (stage0, Rust; stage1, Bastio) | The checker enforces linearity, the verifier asks Z3 the right questions (every obligation, with sound encodings of arithmetic, arrays and capabilities), and code generation emits what was proved | This is the largest trusted part. stage1 is checked against stage0 by differential tests (`stage1/difftest.sh`: same tokens, syntax trees, types, SMT commands, verdicts and C), so a bug has to be in both to go unnoticed. Once stage0 is frozen, stage1 is the trusted compiler, built by itself from a stage0 bootstrap (the fixed point in ROADMAP v2) |
| `rustc` and Cargo | Build stage0 correctly | Only stage0; `rust-toolchain.toml` pins the version |
| The C compiler and linker | Translate the generated C as C11 says, including the memory model for `Lock` and `Atomic` | The default backend writes C. The generated C has no undefined behavior that the proofs do not rule out: no overflow, no out-of-bounds access, a fixed evaluation order (`operands()` in `stage0/src/codegen.rs` hoists side effects into temporaries). Converting an out-of-range value to a signed type is implementation-defined in C and modular in GCC and Clang; `wrap` to a signed type relies on that ([integers.md](integers.md)) |
| The C library (hosted target only) | `calloc`, `stdio`, `fork`/`exec`/pipes for `Child`, and pthreads | Hosted programs run on an operating system anyway |

The native backend (`--backend native`, hosted only, experimental) writes x86-64 assembly instead of C, which takes the C compiler out of the trusted base for the program's own code. The assembler and linker stay.

## 2. Runtime preludes

Each target links in a hand-written C prelude, emitted at the top of every generated file (`prelude_parts` in `stage0/src/codegen.rs`, copied byte for byte in `stage1/cprelude.bastio`). It is the only C a Bastio program runs on. `--trusted-base` lists each part with its count of non-blank lines and which targets link it.

| Part | Targets | What it does | Why it is C |
| --- | --- | --- | --- |
| `common` | all | The C types behind each Bastio type; the spin lock behind `Lock` | Atomics need the C11 memory model |
| `hosted` | hosted | Console on stdin/stdout, `Buf` allocation, files and arguments through `Host` | Wraps the C library |
| `bytes`, `bytes_arena` | all | `Bytes` allocation and access; growing a `Vec`'s storage | Allocation is below the language: Bastio has no raw memory to build an allocator from |
| `child` | hosted | `Child`: fork, exec, pipes, line reads | Wraps POSIX |
| `pages`, `pt_hook_hosted` | hosted, kernel | `Pool`, `Frame`, `PageTable`: zeroing a taken page, page-table entry encoding | These turn linear capability values into memory accesses at physical addresses. The proof tracks who owns each page; the prelude is trusted to touch only the page it is handed |
| `kernel` | kernel | COM1 for the kernel's own panic messages and for programs that use `Serial`, the 8 MiB arena behind `Buf` and `Bytes`, the page pool from the Multiboot memory map | The arena and the pool are where the kernel's memory comes from. The pool reads the boot loader's memory map, which is untrusted input from firmware; it reads only inside the identity-mapped first 1 GiB and caps the pool there |
| `kernel_cpu` | kernel | CR3 loads, the saved user context, FPU state save and load, entering user mode and idling, processor count, the kernel fault report | Control registers and privileged instructions |
| `ports`, `ports_kernel`, `ports_hosted` | hosted, kernel | One `in` or `out` instruction per port access (cells in memory on the hosted target, for testing drivers) | The instruction itself. Which ports a driver may touch is proved: claims are ghost |
| `mmio`, `mmio_kernel`, `mmio_hosted` | hosted, kernel | One volatile load or store per register access | Same as ports. Bounds, alignment and exclusive claims are proved |
| `dma` | hosted, kernel | Fences when a page is lent to a device and taken back; volatile access while lent | Ordering against a device is outside the language |
| `user` | user | System call stubs (`int 0x80`), buffered console, the 64 KiB arena | The kernel's ABI |
| `posix` | hosted | `_POSIX_C_SOURCE` | Needed before any header |

The native backend has its own runtime in assembly (`RUNTIME` in `stage0/src/x86.rs`), also listed and counted: printing, the console and the allocator, over the C library.

The compiler also generates small helpers for each `Box` and `Vec` element type and a thunk per `smp_run` call. Those are part of the compiler (section 1), not of the listing.

## 3. Builtins

Every builtin (the `builtin` lines of the listing, with the targets that have it) is an axiom: the checker gives it a type, the verifier gives it a specification, and the prelude implements it. What is trusted is that the implementation does what the specification says. Examples: `pool_take` returns a zeroed page that nothing else owns; `pt_take` hands back exactly the page that `pt_put` stored in that slot; `vec_get` reads the element the verifier thinks is there. A builtin whose run-time state differs from what the proof believes is a soundness bug, and one such bug has been found and fixed (`pt_has` on the kernel's own slot of a top-level table, #39).

**Counting lemmas.** `vcount(v, x)` (specifications only) is an uninterpreted function `cnt` of a vector's element array, its length and the value. What the prover knows about it is given as axioms, each a theorem about finite sequences proved by induction on the length, outside the solver:
- a count is between 0 and the length;
- an element at an index below the length is counted at least once;
- a value counted at least once is the element at some index below the length (`cnt_w` names that index);
- a push of `y` adds one to the count of `y`; a pop removes one from the count of the last element; `vec_set` and `vec_swap` of `y` at `i` remove one from the count of the old element at `i` and add one to the count of `y`.

`--trusted-base` lists them (the `lemma` lines). They are written out in `COUNT_LEMMAS` and `count_step` in `stage0/src/verify.rs` (`count_lemmas` and `count_step` in `stage1/verify.bastio`). They are sent only for functions whose specifications use `vcount`, or call one whose contract does, so a program that never counts is verified exactly as before.

## 4. Assumptions at the boundaries

**`export fn`.** A C caller cannot be proved to do anything, so the proof of an exported function assumes things about it. They are listed by `--trusted-base` (the `export assumes` lines) and written into the generated header: buffer pointers are valid for their lengths and not written during the call, `&mut Bytes` arguments do not overlap, and every argument is a valid value of its C type.

**The kernel target's platform.** A program built with `--target kernel` expects the platform it links with to provide: an identity map of the first 1 GiB, the device window (2 GiB up to 2^46) mapped uncached, with the code running on the kernel's own tables whenever it touches a claimed `Mmio` (a user address space shares only the first 512 GiB), the kernel's top-level page table `bst_kernel_pml4`, `_end` at the end of the image, a Multiboot information block, and the functions `bst_cpu_enter`, `bst_cpu_wait`, `bst_platform_ctx`, `bst_platform_cpus`, `bst_platform_smp_start` and `bst_platform_smp_join` (declared in the `kernel_cpu` prelude). BastiOS provides them in its `boot/` directory, which is its trusted base.

**The user target.** System call numbers and registers are BastiOS's ABI. A user program trusts the kernel to do what each call says; the kernel trusts the user program with nothing.

**Devices.** Capabilities prove that a driver touches only the ports, registers and lent pages it claimed. Whether the device behaves is trusted: that it writes only the pages a driver lent it, that it has stopped using a page when the driver takes it back (`dma_take`), and that the addresses the driver programs into it are those of pages it lent. The hardware and the emulator (QEMU) are trusted throughout.

## 5. What the proof does not cover

These are not trusted parts so much as properties nobody claims:
- Termination: a loop that never ends is not detected.
- Functional correctness, unless the program states it with `requires`, `ensures`, `invariant` and `assert`.
- The meaning of data: the proof knows a page is owned once, not what is in it.
- Timing and other side channels.

## 6. How it shrinks

- **The C compiler**, by the native backend (ROADMAP v3, "Native x86-64 backend"). It covers the hosted target today; the kernel and user targets need their preludes rewritten in assembly or in Bastio first.
- **Preludes into Bastio**, as capabilities appear that can express them. This has already happened once: BastiOS's UART driver used to be C in the kernel prelude, and now only the one `in` or `out` instruction behind each port access is trusted. The `kernel` prelude's COM1 code is still there for kernels that use `Serial` and for the prelude's own messages.
- **The Multiboot memory map** is parsed in trusted C because Bastio cannot read memory at an address it was not given as a capability (ROADMAP invariant 1). A read-only capability for boot-time firmware memory would move this, and BastiOS's ACPI parsing, into Bastio.
- **stage0** leaves when it is frozen; the trusted compiler is then stage1, which is checked by its own differential tests against stage0's last output and by the bootstrap fixed point.
