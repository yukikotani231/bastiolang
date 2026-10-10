# Counting lines: the smallest library called from C

`lines.bastio` exports two proved functions, `count_byte` and `line_len`;
`main.c` reads standard input and uses them to print how many lines it has
and how long the longest one is. [Section 8 of the tutorial](../../docs/tutorial.md#8-calling-bastio-from-c)
walks through it, including why `line_len` returns an error value instead of
having a `requires`.

```text
$ examples/lines/run.sh
```

builds it under AddressSanitizer and UBSan and checks the output on a few
inputs. `tests/run.sh` runs it in CI, and checks that the copies of
`lines.bastio` and `main.c` in the tutorial are identical to these files.
