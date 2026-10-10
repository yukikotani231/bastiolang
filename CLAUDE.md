# Bastio (the language)

A systems language whose memory safety is proved at compile time. The successor to fz/fzl. This is an open-source project: write all code, comments, docs and commit messages in English.

- Direction and invariants: [ROADMAP.md](ROADMAP.md)
- Language reference: [docs/language.md](docs/language.md)

## Build and test

```sh
cd stage0 && cargo build --release     # warnings are errors in CI
tests/run.sh                           # needs z3 and cc
```

Never copy test counts into prose: they go stale (a lesson from fz). Count the real thing instead:
`ls tests/pass tests/fail | grep -c bastio`

## Rules for new features

- When adding a guarantee, add a program to `tests/fail/` that breaks it and check that it is rejected with the right error code.
- Never get safety by adding runtime checks to generated code (the zero-overhead rule). Get it from a proof.
- Growing the trusted base (Z3, the C compiler, the runtime prelude) follows invariant 3 in ROADMAP.md: regenerate `tests/trusted_base.txt` with `bastioc0 --trusted-base` and say in the PR what grew and why.

## Commits

`<area>: <summary>`, where area is `stage0` / `lang` / `test` / `docs` / `ci` and so on. The body says honestly what can now be proved and what still cannot.
