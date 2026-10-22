# The toolchain, targets and backends

This page explains what happens between a `.bastio` file and a running program: the two compilers, the options you will use, the three targets, the code generators and what each of them trusts. For a first run, see [Building the compiler](tutorial.md#1-building-the-compiler); for the full option list and generated-code details, the reference sections [Using the compiler](language.md#using-the-compiler) and [Targets and generated code](language.md#targets-and-generated-code).

## The pipeline

A build has four stages, and the first two are the point of the language:

1. **Check.** Parsing, types and ownership. A linear value that is leaked, used twice or dropped on one branch is rejected here (`E-LINEAR-*`).
2. **Prove.** For every function the compiler walks the body, turns each obligation (an index in bounds, an addition that cannot overflow, a precondition of a callee, a loop invariant that holds on entry and is preserved) into a question for Z3, and rejects the program unless Z3 answers that no counterexample exists. The line it prints at the end, `12 proof obligation(s) discharged`, is the count of questions answered. Contracts make this modular: a function is proved against its callers' `requires` and against its own `ensures`, never by looking inside another body.
3. **Erase.** Everything that was only for the proof, `requires`, `ensures`, `invariant`, `assert`, `spec fn`, ghost state of capabilities, leaves nothing behind. This is the zero-overhead rule: the generated code has no bounds checks, overflow checks or reference counts, because a proof replaced each of them.
4. **Generate.** assembly by default for hosted programs on x86-64 Linux (native-ir), or C with `--backend c` or `--emit-c`, and then the C compiler or the assembler and linker.

Because the proofs are all done by the end of stage 2, the choice of target or backend in stage 4 cannot make a program unsafe in the language's terms. It can only change what you have to trust (below) and how fast the result is.

## Two compilers

There are two compilers for one language:

| Compiler | Written in | Role |
| --- | --- | --- |
| `bastioc1` | Bastio (`stage1/`) | the compiler you use. New language features land here |
| `bastioc0` | Rust (`stage0/`) | the bootstrap: it builds `bastioc1` from source, and is the reference `bastioc1` is compared with. It is frozen |

`bastioc1` is proved memory safe by `bastioc0`, then compiles itself. [Bootstrapping](bootstrap.md) explains the check that they agree (the fixed point) and what the freeze allows.

```text
$ (cd stage0 && cargo build --release)    # bastioc0, from Rust
$ tests/bastioc1.sh                       # bastioc0 builds bastioc1; prints its path
stage0/target/release/bastioc1
```

Both take the same options and give the same answers; the tests run every program through both. When you only want to use the language, use `bastioc1`.

## The options you will use

```text
$ bastioc1 -o hello hello.bastio       # check, prove, generate C, run cc, link
bastioc1: hello.bastio: 12 proof obligation(s) discharged
$ ./hello
1
4
9
16
25
```

| Option | What it is for |
| --- | --- |
| `--check` | prove only, write nothing. Fast, and what an editor or a pre-commit hook wants |
| `--emit-c` | write the generated C (`out.c`) instead of building it. Read it to see exactly what the proofs bought: plain loops, plain array accesses |
| `--dump-smt` | print every query sent to Z3. For understanding a proof that fails or is slow |
| `--lib DIR` | where `import "core/..."` looks for the library. Needed outside the repository, or set `BASTIO_LIB` |
| `--header FILE` | with `export fn`, write a C header so C can call your code. See [Calling Bastio from C](tutorial.md#9-calling-bastio-from-c) |
| `--trusted-base` | print the hand-written runtime code that the chosen targets link in |

When a proof fails, the message names the obligation, shows the line, and gives a counterexample (values of the variables that break it). [Common proof failures](proof-failures.md) lists the usual ones with their fixes.

Z3 runs as a subprocess, found on `PATH`. A query Z3 answers `sat` or `unknown` (including running out of its time and work limits) is a rejection: the compiler trusts only `unsat`.

## Targets

A target says what program you are writing, and with that what `main` receives and which builtins exist.

| Target | You write | Produces |
| --- | --- | --- |
| `hosted` (the default) | an ordinary program for an existing OS | an executable that uses the C library. `Serial` is stdin and stdout, `Host` gives files and child processes, pages and device registers are simulated in ordinary memory |
| `kernel` | the kernel itself | freestanding code with no C library, entered through `bst_kmain`. Real `Mmu`, `Cpu`, ports, registers and DMA |
| `user` | a program that runs on a Bastio kernel such as [BastiOS](https://github.com/yukikotani231/bastios) | freestanding code entered at `bst_umain`, with `sys_*` system calls |

Calling a builtin the target does not have is `E-TARGET`, so a program tells you which world it lives in. The simulated pages and ports on `hosted` are deliberate: you can write and test a driver on your laptop, under the sanitizers if you like, and then build the same source with `--target kernel`.

The `kernel` and `user` targets write C or assembly rather than an executable, since linking needs the boot code and a linker script that belong to the kernel. [BastiOS's Makefile](https://github.com/yukikotani231/bastios) shows the whole recipe.

## Backends

| Backend | Option | What it does |
| --- | --- | --- |
| C | `--backend c` (default) | emits C11 and runs `cc -O2`. The most mature and the fastest, since the C compiler optimizes |
| native | `--backend native` | emits x86-64 assembly directly and assembles it. The C compiler is no longer in the trusted base for your program's code. Experimental |
| native-ir | `--backend native-ir` | the same output goal, through an intermediate representation with a register allocator. x86-64 only. The default for hosted programs on x86-64 Linux |

All three produce programs with identical behavior; `tests/native.sh` builds every test and benchmark both ways and compares exit status and output. A hosted program on x86-64 Linux is built with native-ir unless you pass `--backend c`, `--backend native` or `--emit-c`; macOS and AArch64 hosts (which have no ELF output) build through the C backend, as do `--arch aarch64` and the kernel and user targets. The default was switched when native-ir came within 2% of `cc -O2` in instruction count (callgrind) on every `bench/` program except sha256, which needs SIMD; `bench/run.sh` prints the current gap.

```text
$ bastioc1 --backend native --emit-asm -o hello.s hello.bastio
$ bastioc1 --backend native -o hello hello.bastio && ./hello
```

The intermediate representation is described in [the IR page](ir.md).

### Other processors

`--arch aarch64` selects 64-bit Arm. For a hosted program it only changes the C compiler: the same C is built with `aarch64-linux-gnu-gcc` (or `$CC`) and runs on Arm Linux, or under `qemu-aarch64`. With `--target kernel` or `--target user`, `bastioc1` has Arm preludes (a PL011 UART console, VMSAv8-64 page tables, exception-level entry), and `--backend native` with `--arch aarch64` is partial. `tests/aarch64.sh` builds every test program for Arm, runs it under QEMU and boots a small kernel.

## What you are trusting

A program proved memory safe is safe provided the following behave. The list is short on purpose and is counted by machine, not by hand:

- **Z3**: when it says `unsat`, there really is no counterexample.
- **The compiler** (the checker, the verifier's encodings, the code generator). Two independently written implementations compared on every test is the defense, but it is trust all the same.
- **The C compiler** and linker, if you use the C backend. The native backends remove this one for the program's own code.
- **The prelude**: the hand-written runtime a target links in (the allocator, the console, the page-table entry encoding, the hosted simulation). `bastioc1 --trusted-base` prints its size per target and per prelude section, and the output is checked into `tests/trusted_base.txt`: growing it shows up as a diff in review.
- **Anything you declare**: an `export fn` boundary, a device's real behavior. Contracts state what the proof assumes about the other side.

The page [The trusted base](trusted-base.md) lists every item, why it is there and how it could shrink, and the reference's [soundness scope](language.md#soundness-scope) says exactly what is guaranteed.

## Working on larger programs

- **Several files.** `import "path";` at the top of a file loads another (see [Programs and files](language.md#programs-and-files)); the library in `core/` and `std/` is imported this way. Files named together on the command line form one program.
- **Library without `main`.** A file with `export fn`s needs no `main`. Build it with `--emit-c --header lib.h` and compile the C with your own. The [examples](c-libraries.md) do this for base64, UTF-8, DER and QOI, each with a differential fuzz against a C implementation.
- **Slow proofs.** Z3 time grows with function size. Split long functions, state loop invariants as strongly as the loop needs and no stronger, and use `--dump-smt` to see what is being asked. The proof cache (`BASTIO_PROOF_CACHE=dir`) remembers answers across builds.
- **Editors.** Syntax highlighting for VS Code and tree-sitter grammars are in [`editors/`](https://github.com/yukikotani231/bastiolang/tree/main/editors).

## Running the project's checks

```text
$ tests/run.sh      # every test, every example in the docs, the doc transcripts
$ tests/ci.sh       # everything CI runs; run before each push
```

`tests/pass/` holds programs that must verify and run, `tests/fail/` one program for each rule that must be rejected with the right error code. The examples in this documentation are compiled by `tests/run.sh` too, so a page here that stops being true fails the build.
