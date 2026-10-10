# Contributing to Bastio

Thanks for your interest. Bastio is pre-release: the language changes often, and large changes are best discussed in an issue first.

## Before you send a pull request

1. Build and run every test (see "Build and test" in [README.md](README.md)):
   ```sh
   (cd stage0 && RUSTFLAGS="-D warnings" cargo build --release)
   tests/run.sh
   stage1/difftest.sh
   ```
   CI runs the same commands, and treats compiler warnings as errors.
2. A new guarantee comes with a program in `tests/fail/` that breaks it, with an `// expect-error: CODE` header naming the error it must be rejected with. A new feature also gets a program in `tests/pass/` (ROADMAP invariant 4).
3. Safety comes from a proof, never from a runtime check in the generated code (the zero-overhead rule, ROADMAP invariant 2).
4. If the trusted base grows (Z3, the C compiler, the runtime preludes), regenerate `tests/trusted_base.txt` with `bastioc0 --trusted-base` and say in the pull request what grew and why (ROADMAP invariant 3).
5. A change that makes generated code, the compiler or BastiOS measurably slower says so, with numbers (ROADMAP invariant 5).

Don't copy counts (tests, lines, files) into prose; they go stale. Give the command that counts them instead.

## Commits

`<area>: <summary>`, where area is `stage0`, `stage1`, `lang`, `test`, `docs`, `ci` and so on. The body says honestly what can now be proved and what still cannot.

## License

Unless you say otherwise, any contribution you submit for inclusion is licensed as described in [README.md](README.md#license), without any additional terms or conditions.
