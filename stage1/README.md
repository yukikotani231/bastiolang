# stage1: bastioc1, the self-hosted compiler

bastioc1 is written in Bastio and built by stage0, and builds itself:

```sh
stage0/target/release/bastioc0 -o bastioc1 stage1/bastioc1.bastio
./bastioc1 -o bastioc1-gen2 stage1/bastioc1.bastio   # the same executable
./bastioc1 -o b64 tests/pass/base64.bastio           # usage as for bastioc0
```

It was grown one phase at a time, and each phase is checked against stage0 on every Bastio source
in the repository (`difftest.sh`, run in CI):

| Phase | File | Compared with |
| --- | --- | --- |
| Lexer | `lexer.bastio` | `bastioc0 --dump-tokens` (`lexdump.bastio` prints the same format) |
| Parser | `parser.bastio` | `bastioc0 --dump-ast` (`astdump.bastio`), on every source and on damaged copies (`mutate.py`), so error positions are compared too |
| Loader and monomorphizer | `loader.bastio`, `mono.bastio` | `bastioc0 --dump-mono` (`monodump.bastio`): the whole program with imports followed, after monomorphization; also on damaged copies |
| Checker (types and linearity) | `check.bastio` | `bastioc0 --dump-check` (`checkdump.bastio`, with the target BastiOS sources need): the same program with every `let` given its type, or the same first error and error code; also on damaged copies, library files through a program that imports them (`mutate.py --via`) |
| Verifier (queries) | `verify.bastio` | `bastioc0 --check --dump-smt` (`smtdump.bastio`): every SMT command and the number of proof obligations, on every program stage0 verifies and on damaged copies that still verify (`mutate.py --smt`) |
| Verifier (proofs) | `solve.bastio` | `bastioc0 --dump-verify` (`verifydump.bastio`): both run Z3; the number of proof obligations, or the first failed proof's code, position, counterexample and notes, on every program that reaches the verifier and on damaged copies (`mutate.py --verify`) |
| C code generation | `codegen.bastio`, `cprelude.bastio` | `bastioc0 --dump-c` (`cdump.bastio`): the whole C file, byte for byte, on every program the checker accepts (the BastiOS kernel and user programs included) and on damaged copies |
| Driver | `bastioc1.bastio` | `bastioc0` itself: the stderr and exit status of `--check` on every source (with the sentence after `error[CODE]:` and the checker's hints masked, as bastioc1 does not print them yet), the C of `--emit-c`, the test programs it builds, which must exit as they say; and the fixed point: bastioc1 built by itself writes the same C for itself as stage0 |

The syntax tree is an arena: one `Vec[Node]` of plain values, children
as indices, lists chained through `next`. All files of a program share one
source buffer, so a node's text range means the same in every file.

Types are interned (`Mono.tys`), so equal types have equal indices. A
generic definition is not copied per instance: an instance is the
definition plus its type arguments, applied whenever a type in it is read.

The checker tracks ownership per place (`x`, `x.f`, `x.f.g`), as stage0
does: places are interned, and the set of consumed places is compared
across the branches of an `if`, the arms of a `match` and a loop body.

The verifier follows stage0's `verify.rs` function by function. SMT terms
live in an arena and are shared rather than copied; two terms are equal
when they print the same (`tm_eq`), which is what stage0's string
comparisons decide. Values (`Vl`) are immutable, so a state is cheap to
copy at a branch: its path condition and its variables' value ids.

Nothing the verifier does depends on the solver's answers, so it runs to
the end first, recording each obligation with what a failure would report
(its node, error code and the variables in scope). `solve.bastio` then
feeds the log to Z3, reading an answer after each `(check-sat)`, and at
the first failed proof asks for the counterexample's values, as stage0
does at that point.

The code generator builds text in a scratch pool and copies finished
lines out, generating every sub-expression before the text around it in
stage0's order, since that order numbers the temporaries. The runtime
preludes (`cprelude.bastio`) are copies of stage0's; the differential test
keeps them identical.

`print.bastio` prints the tree for both dumps, and `out.bastio` holds the
output helpers the drivers share.

`lexcases/` and `monocases/` hold sources that exercise lexical and
monomorphization errors, `verifycases/` programs that exercise corners of
the verifier the tests do not reach (matches where only some arms fall
through, `&&` lending `&mut`, what a loop havocs).
