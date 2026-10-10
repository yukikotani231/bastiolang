# Examples

Every example here is built and run by `tests/run.sh`, so CI keeps them working.

| Example | What it shows |
| --- | --- |
| [`hello.bastio`](hello.bastio) | The smallest useful program: a buffer, two loops with invariants, printing. [Tutorial, section 1](../docs/tutorial.md#1-building-the-compiler) |
| [`wordfreq.bastio`](wordfreq.bastio) | The core library end to end: split a text into words, count them in a `BytesMap`, sort by count with `sort_by_key_u64`, print the top five. The library's proofs (spans inside the text, slots holding keys, indices in range) are what let it index without checks. [docs/core.md](../docs/core.md) |
| [`lines`](lines) | The smallest library called from C: two `export fn`s, the generated header, a C `main`. [Tutorial, section 8](../docs/tutorial.md#8-calling-bastio-from-c) walks through it |
| [`base64`](base64) | A strict base64 codec for untrusted input, with a differential fuzz against a C decoder and a benchmark |
| [`utf8`](utf8) | A strict UTF-8 validator and decoder, fuzzed the same way |
| [`der`](der) | A DER (ASN.1) structure checker: the parser shape behind X.509 certificates |

A single-file example builds with `bastioc0 -o NAME NAME.bastio` and must exit
with 0. An example in a directory has a `run.sh` that builds it, links it with
its C side under AddressSanitizer and UBSan, and checks the result; it exits
non-zero on any failure.
