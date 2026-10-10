# stage1: bastioc1, the self-hosted compiler

bastioc1 is written in Bastio and built by stage0. It is grown one phase
at a time, and each phase is checked against stage0 on every Bastio source
in the repository (`difftest.sh`, run in CI):

| Phase | File | Compared with |
| --- | --- | --- |
| Lexer | `lexer.bastio` | `bastioc0 --dump-tokens` (`lexdump.bastio` prints the same format) |
| Parser | `parser.bastio` | `bastioc0 --dump-ast` (`astdump.bastio`), on every source and on damaged copies (`mutate.py`), so error positions are compared too |

The syntax tree is an arena: one `Vec[Node]` of plain values, children
as indices, lists chained through `next`. `out.bastio` holds the output
helpers the dump drivers share.

`lexcases/` holds sources that exercise lexical errors and edge cases.
