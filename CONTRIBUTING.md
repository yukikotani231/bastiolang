# Contributing to Bastio

Thanks for your interest. Bastio is pre-release: the language changes often, and large changes are best discussed in an issue first.

## Before you send a pull request

1. Merge the latest `main`, then run every check CI runs:
   ```sh
   tests/ci.sh
   ```
   It builds stage0 with warnings as errors, runs `tests/run.sh`, `tests/native.sh`, `stage1/difftest.sh` and both editor grammars, and lists any step it skipped for a missing tool (z3, the tree-sitter CLI, npm). The test scripts run independent checks in parallel, one per CPU; set `JOBS=N` to use fewer (`JOBS=1` runs them one at a time). Their output is the same whatever `JOBS` is. A change to the stage0 checker, verifier or code generator must be mirrored in `stage1/` or `stage1/difftest.sh` fails; new syntax must be added to `editors/` or the grammar checks fail.
2. A new guarantee comes with a program in `tests/fail/` that breaks it, with an `// expect-error: CODE` header naming the error it must be rejected with. A new feature also gets a program in `tests/pass/` (ROADMAP invariant 4). Each `// expect-text: TEXT` line in a `tests/fail/` program must appear in the error output, so a diagnostic's wording or hint can be tested too.
3. Programs in `docs/*.md` are tests as well (see `tests/run.sh`). A ```` ```text ```` block of compiler output right after a ```` ```bastio ```` block must match what the compiler prints for it, and a block after a `<!-- file: PATH -->` line must be identical to that file (`tests/doc_check.py`). When a change alters a message, update the docs that show it.
4. A library function (`core/`, `std/`) has a `///` doc comment and is called by an example in one, between two `/// ```` lines. The reference pages in `docs/lib/` are generated from them: run `tests/libref.py --write` after changing the library, and `tests/run.sh` checks that the pages are current and that every example builds, verifies and runs (`tests/libref.py` says how examples are written).
5. Safety comes from a proof, never from a runtime check in the generated code (the zero-overhead rule, ROADMAP invariant 2).
6. If the trusted base grows (Z3, the C compiler, the runtime preludes), regenerate `tests/trusted_base.txt` with `bastioc0 --trusted-base` and say in the pull request what grew and why (ROADMAP invariant 3).
7. A change that makes generated code, the compiler or BastiOS measurably slower says so, with numbers (ROADMAP invariant 5).
8. A change a user would notice (the language, the compilers' options or output, the documentation site) gets a line under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md).

Don't copy counts (tests, lines, files) into prose; they go stale. Give the command that counts them instead.

## Commits

`<area>: <summary>`, where area is `stage0`, `stage1`, `lang`, `test`, `docs`, `ci` and so on. The body says honestly what can now be proved and what still cannot.

## License

Unless you say otherwise, any contribution you submit for inclusion is dual licensed as Apache-2.0 OR MIT, as described in [README.md](README.md#license), without any additional terms or conditions.
