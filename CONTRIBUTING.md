# Contributing to Bastio

Thanks for your interest. Bastio is pre-release: the language changes often, and large changes are best discussed in an issue first.

## Getting oriented

- [README.md](README.md) says what Bastio is and how to build and test it; [docs/README.md](docs/README.md) indexes the rest.
- [ROADMAP.md](ROADMAP.md) has the invariants (no raw memory access, zero overhead, a counted trusted base) that every change keeps.
- [docs/architecture.md](docs/architecture.md) maps the compiler's phases to files and says where to look for a given kind of change.
- [docs/bootstrap.md](docs/bootstrap.md) explains why new features go into `stage1/` and not `stage0/`, and [stage1/README.md](stage1/README.md) how the compiler is laid out.

To set up a machine in one command (Debian or Ubuntu), run `tools/setup-dev.sh`: it installs Z3, a C compiler and rustup, builds stage0 and bastioc1, and compiles and runs `examples/hello.bastio`. The repository also has a [dev container](.devcontainer/devcontainer.json) that runs it for you, in VS Code or GitHub Codespaces.

Bugs and ideas go in the issue tracker. A program the compiler accepts but should not (a soundness bug) goes through [SECURITY.md](SECURITY.md). A bug report with the smallest program that shows it and the compiler's output is the most useful kind.

## Finding something to work on

Issues labelled [`good first issue`](https://github.com/yukikotani231/bastiolang/labels/good%20first%20issue) are small and say where to start and how to know you are done. [`help wanted`](https://github.com/yukikotani231/bastiolang/labels/help%20wanted) marks larger work that is open to anyone. Comment on the issue before you start, so two people do not do the same thing. The other labels name the area (`area/compiler`, `area/library`, `area/tests`, ...) or the kind of change (`bug`, `enhancement`, `documentation`, `performance`, `proofs`); `.github/labels.yml` is the list.

A change that is not tied to an issue is welcome too, when it is small. For anything that changes the language or adds to the trusted base, open an issue first.

## Before you send a pull request

1. Merge the latest `main`, then run every check CI runs:
   ```sh
   tests/ci.sh
   ```
   It builds stage0 with warnings as errors, builds bastioc1 with it (`tests/bastioc1.sh`), runs `tests/run.sh`, `tests/native.sh`, `stage1/difftest.sh` and both editor grammars, and lists any step it skipped for a missing tool (z3, the tree-sitter CLI, npm). The test scripts run independent checks in parallel, one per CPU; set `JOBS=N` to use fewer (`JOBS=1` runs them one at a time). Their output is the same whatever `JOBS` is. New syntax must be added to `editors/` or the grammar checks fail.
2. A new guarantee comes with a program in `tests/fail/` that breaks it, with an `// expect-error: CODE` header naming the error it must be rejected with. A new feature also gets a program in `tests/pass/` (ROADMAP invariant 4). Each `// expect-text: TEXT` line in a `tests/fail/` program must appear in the error output, so a diagnostic's wording or hint can be tested too.
3. Programs in `docs/*.md` are tests as well (see `tests/run.sh`). A ```` ```text ```` block of compiler output right after a ```` ```bastio ```` block must match what the compiler prints for it, and a block after a `<!-- file: PATH -->` line must be identical to that file (`tests/doc_check.py`). When a change alters a message, update the docs that show it.
4. A library function (`core/`, `std/`) has a `///` doc comment and is called by an example in one, between two `/// ```` lines. The reference pages in `docs/lib/` are generated from them: run `tests/libref.py --write` after changing the library, and `tests/run.sh` checks that the pages are current and that every example builds, verifies and runs (`tests/libref.py` says how examples are written).
5. Safety comes from a proof, never from a runtime check in the generated code (the zero-overhead rule, ROADMAP invariant 2).
6. stage0 is frozen ([docs/bootstrap.md](docs/bootstrap.md)). A new feature goes into `stage1/` only, and its test programs start with `// stage1-only: <feature>`. A change to `stage0/` needs a commit message line saying why: `Stage0-only: <why>` when it has no stage1 counterpart, or `Stage0-shared: <why>` for a bug fix made in both compilers. `tests/stage0_mirror.sh` checks this.
7. If the trusted base grows (Z3, the C compiler, the runtime preludes), regenerate `tests/trusted_base.txt` with `bastioc0 --trusted-base` and say in the pull request what grew and why (ROADMAP invariant 3).
8. A change that makes generated code, the compiler or BastiOS measurably slower says so, with numbers (ROADMAP invariant 5).
9. A change a user would notice (the language, the compilers' options or output, the documentation site) gets a line under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md).

Don't copy counts (tests, lines, files) into prose; they go stale. Give the command that counts them instead.

## Commits

`<area>: <summary>`, where area is `stage0`, `stage1`, `lang`, `test`, `docs`, `ci` and so on. The body says honestly what can now be proved and what still cannot.

## License

Unless you say otherwise, any contribution you submit for inclusion is dual licensed as Apache-2.0 OR MIT, as described in [README.md](README.md#license), without any additional terms or conditions.
