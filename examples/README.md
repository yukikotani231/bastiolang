# Examples

Every example here is built and run by `tests/run.sh`, so CI keeps them working.

| Example | What it shows |
| --- | --- |
| [`hello.bastio`](hello.bastio) | The smallest useful program: a buffer, two loops with invariants, printing. [Tutorial, section 1](../docs/tutorial.md#1-building-the-compiler) |
| [`wordfreq.bastio`](wordfreq.bastio) | The core library end to end: split a text into words, count them in a `BytesMap`, sort by count with `sort_by_key_u64`, print the top five. The library's proofs (spans inside the text, slots holding keys, indices in range) are what let it index without checks. [docs/core.md](../docs/core.md) |
| [`httpd.bastio`](httpd.bastio) | A small web server with `std/http` over real sockets: two routes and a 404. With no arguments it asks itself for each route and checks the answers; `httpd serve PORT` serves on 127.0.0.1 until killed. Hosted target only (`std/net` fails on Windows) |
| [`jsonapi.bastio`](jsonapi.bastio) | A small JSON API with `std/http` and `core/json`: `POST /greet` reads a name and a count from the body and answers a JSON greeting; bad JSON gets 400. With no arguments it sends itself requests and checks the answers; `jsonapi serve PORT` serves on 127.0.0.1 until killed. Hosted target only |
| [`lines`](lines) | The smallest library called from C: two `export fn`s, the generated header, a C `main`. [Tutorial, section 9](../docs/tutorial.md#9-calling-bastio-from-c) walks through it |
| [`base64`](base64) | A strict base64 codec for untrusted input, with a differential fuzz against a C decoder and a benchmark |
| [`leb128`](leb128) | An unsigned LEB128 (varint) decoder and encoder, as in WebAssembly and DWARF: fuzzed against a C decoder, with the over-long and overflowing cases refused |
| [`utf8`](utf8) | A strict UTF-8 validator and decoder, fuzzed the same way |
| [`der`](der) | A DER (ASN.1) structure checker: the parser shape behind X.509 certificates |
| [`http`](http) | A strict HTTP/1.1 request and response head parser for bytes from a socket: offsets in, no allocation, fuzzed against an independent state machine |
| [`qoi`](qoi) | A QOI image codec: buffers sized by an untrusted header, and out of memory reported to C instead of aborting |

A single-file example builds with `bastioc0 -o NAME NAME.bastio` and must exit
with 0. An example in a directory has a `run.sh` that builds it, links it with
its C side under AddressSanitizer and UBSan, and checks the result; it exits
non-zero on any failure.
