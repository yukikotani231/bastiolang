# stage1: bastioc1, the self-hosted compiler

bastioc1 is written in Bastio and built by stage0. It is grown one phase
at a time, and each phase is checked against stage0 on every Bastio source
in the repository (`difftest.sh`, run in CI):

| Phase | File | Compared with |
| --- | --- | --- |
| Lexer | `lexer.bastio` | `bastioc0 --dump-tokens` (`lexdump.bastio` prints the same format) |
| Parser | `parser.bastio` | `bastioc0 --dump-ast` (`astdump.bastio`), on every source and on damaged copies (`mutate.py`), so error positions are compared too |
| Loader and monomorphizer | `loader.bastio`, `mono.bastio` | `bastioc0 --dump-mono` (`monodump.bastio`): the whole program with imports followed, after monomorphization; also on damaged copies |
| Checker (types and linearity) | `check.bastio` | `bastioc0 --dump-check` (`checkdump.bastio`, with the target BastiOS sources need): the same program with every `let` given its type, or the same first error and error code; also on damaged copies, library files through a program that imports them (`mutate.py --via`) |

The syntax tree is an arena: one `Vec[Node]` of plain values, children
as indices, lists chained through `next`. All files of a program share one
source buffer, so a node's text range means the same in every file.

Types are interned (`Mono.tys`), so equal types have equal indices. A
generic definition is not copied per instance: an instance is the
definition plus its type arguments, applied whenever a type in it is read.

The checker tracks ownership per place (`x`, `x.f`, `x.f.g`), as stage0
does: places are interned, and the set of consumed places is compared
across the branches of an `if`, the arms of a `match` and a loop body.

`print.bastio` prints the tree for both dumps, and `out.bastio` holds the
output helpers the drivers share.

`lexcases/` and `monocases/` hold sources that exercise lexical and
monomorphization errors.
