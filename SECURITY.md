# Security policy

Bastio's promise is that a program the compiler accepts is memory safe.
A break in that promise is a security bug, and we would like to hear about
it privately first.

## What counts

- **Soundness bugs**: a program that `bastioc0` or `bastioc1` accepts, and
  that reads or writes memory it does not own, uses a value after it was
  consumed, leaks a linear value, overflows an integer the checker said
  could not overflow, or otherwise does something the language reference
  ([docs/language.md](docs/language.md)) says a checked program cannot do.
- **Bugs in the trusted base** that make generated code unsafe even though
  the proof went through: the runtime prelude, the code generators, or the
  way the compiler talks to Z3 (see [docs/trusted-base.md](docs/trusted-base.md)).
- The compiler crashing or hanging on hostile input is a normal bug; open
  an issue for it.

Bugs in BastiOS go to [its own policy](https://github.com/yukikotani231/bastios/blob/main/SECURITY.md).
If a BastiOS bug comes from the compiler accepting unsafe code, report it
here.

## How to report

Use GitHub's private vulnerability reporting:
[open a draft advisory](https://github.com/yukikotani231/bastiolang/security/advisories/new).
Please include the smallest program that shows the problem, the compiler
(`bastioc0` or `bastioc1`) and commit you used, and what you expected.

We will answer within a week, agree on a fix and a date with you, and
credit you in the advisory and the changelog unless you would rather not
be named. A soundness fix comes with a program in `tests/fail/` that the
compiler now rejects.

## Supported versions

Bastio is pre-1.0. Fixes go to `main` and the next release only.
