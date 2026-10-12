# Bootstrapping and the stage0 freeze

Bastio has two compilers:

- `bastioc1` is the compiler. It is written in Bastio (`stage1/`), and it is the one users run, the tests use and BastiOS builds with.
- `bastioc0` is the bootstrap compiler. It is written in Rust (`stage0/`) and has two jobs. It builds bastioc1 from source, and it is the oracle that `stage1/difftest.sh` compares bastioc1 against.

stage0 is frozen ([ROADMAP.md](../ROADMAP.md), v2). New language features go into stage1 only. stage0 changes only to keep the bootstrap working, or to fix a bug that both compilers share. This page explains how the bootstrap works, how to check it, and what the freeze allows.

## Building bastioc1 from nothing

You need the tools listed in the [README](../README.md): Rust, Z3 and a C compiler.

```sh
(cd stage0 && cargo build --release)    # bastioc0, from Rust
tests/bastioc1.sh                       # bastioc0 builds stage0/target/release/bastioc1
```

`tests/bastioc1.sh` runs `bastioc0 -o stage0/target/release/bastioc1 stage1/bastioc1.bastio` and prints the path of the result. As with any program, bastioc0 first proves bastioc1's source memory safe, then writes C and calls `cc`. That takes about half a minute. The script rebuilds bastioc1 only when the binary is missing or older than bastioc0 or any `stage1/*.bastio`, so the test scripts can call it every time.

BastiOS does the same in its own Makefile. `BASTIOC0` names bastioc0 in a bastiolang checkout next to it. bastioc0 writes `build/bastioc1.c`, the host's C compiler builds `build/bastioc1` from it, and every kernel and user program is then built with that bastioc1.

## Checking the bootstrap

The bootstrap is correct when bastioc1 built by bastioc0 and bastioc1 built by itself write the same C for bastioc1. That is a fixed point: from then on, building it again changes nothing.

```sh
b1=stage0/target/release/bastioc1
b0=stage0/target/release/bastioc0
$b1 -o /tmp/gen2 stage1/bastioc1.bastio            # bastioc1 builds itself
/tmp/gen2 --emit-c -o /tmp/gen2.c stage1/bastioc1.bastio
$b0 --emit-c -o /tmp/gen1.c stage1/bastioc1.bastio
cmp /tmp/gen1.c /tmp/gen2.c && echo "fixed point"
```

`stage1/difftest.sh` runs this check on every run (the line "fixed point"). It also runs the same check for the native backend: bastioc1 assembled by `asmdump` must write the same C as stage0. CI runs difftest on every pull request, split into independent parts (`DIFFTEST_PARTS`, see the script's header) that run on separate machines, with the tools built by stage0 and by the native backend.

The fixed point shows that the two compilers agree on bastioc1's own source. It does not show that either compiler is correct. That is what the rest of difftest is for: it compares each phase of bastioc1 with bastioc0 on every Bastio source in the repository and in a BastiOS checkout next to it, and on damaged copies of those sources.

## What the freeze allows

### New features go into stage1 only

A new feature is new syntax, a new type rule, a new builtin, or a new kind of proof. It is implemented in `stage1/` alone.

Its test programs in `tests/pass/`, `tests/fail/` and `tests/user/` start with this header:

```text
// stage1-only: <the feature>
```

Because of the header:
- `tests/run.sh` and `tests/native.sh` skip the program when `BASTIOC` is stage0 (`tests/stage1_only.sh`).
- difftest leaves the program out of its corpus, since stage0 has no answer to compare with. Instead, difftest checks that stage0 really rejects it: a `tests/pass` program must fail, and a `tests/fail` program must fail with a different error from the one it expects. When stage0 handles a marked program after all, difftest reports `STALE` and fails, because the marker would hide that program from the oracle for no reason.

The same header works in BastiOS:
- A user program in `user/` that carries it is left out of difftest in the same way.
- The kernel is one program, because `src/main.bastio` imports every module. So the header in `src/main.bastio` leaves the whole kernel out of every comparison with stage0, and difftest checks that stage0 rejects the kernel.

Marking the kernel costs a large part of difftest's coverage, so it waits for a feature that BastiOS cannot do without. Until then, a builtin row that BastiOS needs can still go into both compilers as a shared change (below).

### stage1's source stays in stage0's language

bastioc0 must still be able to build bastioc1, so `stage1/*.bastio` uses only what stage0 accepts. No extra check enforces this: every CI run builds bastioc1 with bastioc0 first, and a change that breaks the build fails there.

This rule is what keeps stage0 alive. Dropping it would mean bootstrapping from bastioc1's generated C kept in the repository. That is an open question in the ROADMAP, because it moves the bootstrap's trust from readable Rust to generated C.

### stage0 changes say why

`tests/stage0_mirror.sh` runs in `tests/ci.sh` and on every pull request. It fails a branch that changes anything under `stage0/` unless one of the branch's commit messages has one of these lines:

| Line | For | Also needed |
| --- | --- | --- |
| `Stage0-only: <why>` | A change with no stage1 counterpart: a bootstrap fix, the native backend (`x86.rs`, whose stage1 version deliberately differs), comments or formatting, a dependency | Nothing |
| `Stage0-shared: <why>` | The same change made in both compilers so that the oracle keeps up: a fix to a bug both share (soundness fixes first), or a builtin row BastiOS needs while its kernel is still in difftest's corpus | Each changed `stage0/src/*.rs` comes with a change to the stage1 file that mirrors it (the map is in the script) |

A stage0 change with neither line is a new feature landing in stage0, and it fails. The guard checks only that the line is there; the reason is for reviewers. difftest still checks that a shared change behaves the same in both compilers.

The model for a shared fix is the Z3 hang in signed `wrap` that fuzzing found. The fix went into both compilers, so the oracle did not keep the bug and difftest still agreed afterwards.
