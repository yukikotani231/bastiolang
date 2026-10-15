# Shrinking the trust in the solver and the verifier

[The trusted base](trusted-base.md) lists Z3 as a trusted tool: when it answers `unsat`, the compiler accepts the proof. This page compares ways to trust less, and says which one is done and which come next. It belongs to ROADMAP v3 and v7 (the trusted base shrinks as the backends and the checkers grow).

What can go wrong between a Bastio program and an accepted proof:

1. **The solver answers `unsat` for a satisfiable question.** A bug in Z3, or in the way it handles a theory (nonlinear integers, arrays, quantifiers).
2. **The question is the wrong one.** The verifier's encoding of arithmetic, arrays, capabilities and the hand-written axioms (counting lemmas, `bdec`, `bhex`, `has_bit`; see section 3 of the trusted base) does not mean what the program means.
3. **The compiler drops an obligation.** Symbolic execution forgets a path or a check.

Only the first is about the solver. The options below address 1 and 2 differently.

## Options

### A. Proof certificates, checked by a small independent checker

Ask the solver for a proof object together with `unsat` (Z3 `(set-option :proof true)` and `get-proof`; cvc5 can print Alethe or LFSC proofs) and check it with a program small enough to read. A bug in the solver then cannot make the compiler accept a wrong program: it can only produce a certificate the checker rejects.

- Gain: trust moves from the solver (millions of lines) to the checker (thousands).
- Cost: the proofs mix linear and nonlinear integer arithmetic, arrays, uninterpreted functions and quantified axioms. Z3's proof format is not stable between versions and leaves many steps as "trust me" rules (`th-lemma`); Alethe from cvc5 is better specified and has checkers (Carcara, Isabelle/Coq reconstruction) but covers fewer of the theories we use, and the checker would have to be written in Bastio to stay in the language. Proof objects for the BastiOS kernel (273,000 commands) are large.
- Verdict: the right end state, a project of its own. Start by finding which obligations get proofs the checker can handle (the large linear-arithmetic majority) and leave the rest to option B.

### B. Cross-checking with a second solver

Send every query also to cvc5, which shares no code with Z3, and require the same answer.

- Gain: an `unsat` that only one of two independently written solvers believes is caught. Bugs in different solvers on the same query are rare, so this removes most of the risk of 1 for a small amount of code, and it works on every theory the two share.
- Cost: still trusts the two solvers not to be wrong together, and gives no certificate. Queries that use syntax only Z3 reads are not confirmed (below), and cvc5 gives up on some nonlinear queries.
- Verdict: first step; done (below).

### C. A smaller, plainer encoding

Everything the encoding adds beyond the program is trusted twice: the axioms, and the lambda-style array updates. Shrinking it helps both A and B.

- Replace the `lambda` array terms used for bulk copies (`bcopy`, `bmove`) with standard SMT-LIB (`store` chains, or a quantified equality), so that cvc5 can read them and a certificate checker does not need Z3 extensions.
- Prove each axiom (counting lemmas, `bdec`, `bhex`, bit facts) once, outside the solver, in a proof assistant, and keep it as a checked lemma file.
- Verdict: next. It is mostly work in `stage1/verify.bastio`, which the compiler IR work owns, so it is coordinated separately.

### D. Replaying the unsat core

Re-asking the solver only the assumptions its unsat core names, with a second configuration or seed. This catches a solver that reaches `unsat` through an unsound preprocessing step, but not a bug in the core theory solver. It is cheaper than A and weaker than B, so it is not planned.

## What is done: `tests/crosscheck.py`

For every program in `tests/pass`, the script takes the exact session the compiler sends (`bastioc0 --check --dump-smt`), replays it through Z3 and through cvc5, and compares the answers query by query. The result for each query is one of:

- **confirmed**: both say `unsat`.
- **unchecked**: cvc5 gave up (its per-query time limit) or the query contains Z3-only syntax (`lambda`). The script drops the offending assertion for cvc5 and marks every query that could be affected as unchecked, so a weaker session can never turn into a false disagreement or a false confirmation.
- **disagreement**: Z3 does not say `unsat` for a program the compiler accepts, or cvc5 says `sat` where Z3 says `unsat`. Either one fails the run, and the query is named.

The percentage of confirmed queries is printed on every run. It is the number to raise: lowering the unchecked share (option C first, then a longer cvc5 limit) is how this step grows into A's coverage.

The script uses stage0's dump because the two compilers produce the same commands (`stage1/difftest.sh`), and it needs `z3` and `cvc5` on the path (`apt install z3 cvc5`). Run it with `tests/ci.sh`.

## What is not covered

Option B says nothing about 2 and 3: a wrong encoding is wrong for both solvers. Those are covered by the differential tests between the two compilers, by `tests/fail` (every guarantee has a program that breaks it), and by the mutation tests (`stage1/mutate.py`).
