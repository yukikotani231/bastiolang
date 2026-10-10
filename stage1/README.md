# stage1: bastioc1, the self-hosted compiler

bastioc1 is written in Bastio and built by stage0. It is grown one phase
at a time, and each phase is checked against stage0 on every Bastio source
in the repository (`difftest.sh`, run in CI):

| Phase | File | Compared with |
| --- | --- | --- |
| Lexer | `lexer.bastio` | `bastioc0 --dump-tokens` (`lexdump.bastio` prints the same format) |

`lexcases/` holds sources that exercise lexical errors and edge cases.
