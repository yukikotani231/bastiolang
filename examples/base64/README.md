# A proved base64 codec, called from C

A small, real decoder for untrusted input, written in Bastio and called from a
C program through a plain C header. This is the shape Bastio is meant to be
adopted in first: replace one parser in a C code base, keep the rest.

| File | What it is |
| --- | --- |
| `base64.bastio` | The codec: strict RFC 4648 base64 decoding and encoding. Proved |
| `base64.h` | The C API: `base64_decode`, `base64_encode`, `base64_free` |
| `base64_capi.c` | The C boundary: converts between the C API and the Bastio functions. Hand-written, trusted |
| `lib.bastio` | The library as one program (stage0 needs a `main`, see below) |
| `test.c` | The C harness: RFC 4648 vectors, a differential fuzz, a benchmark |
| `run.sh` | Builds and runs the harness under AddressSanitizer + UBSan (`run.sh bench`: throughput) |

`tests/pass/base64.bastio` runs the same vectors from Bastio, and
`tests/run.sh` runs `run.sh`, so both are in CI.

## What is proved

For every input, of any length and content, with no runtime check in the
generated C:
- no read or write outside a buffer;
- no integer overflow, underflow or division by zero;
- every byte written is `<= 255`;
- no leak and no double free: the output buffer is freed on each error path
  and handed to the caller on success;
- the output length: `(n + 2) / 3 * 4` for encoding; for decoding, the input
  length is a multiple of 4 and the output is between `n / 4 * 3 - 2` and
  `n / 4 * 3` bytes (`ensures` clauses in `base64.bastio`).

The annotations are loop invariants (two or three lines per loop) and an
`ensures` bound on each helper; the rest is found by the prover. Even the decoding table is checked: it is a string literal, and
the prover reads its bytes to show that every entry is at most 64.

A bug is a compile error with a counterexample. Leave out the check of the
fourth symbol in the main loop, and:

```
base64.bastio:133:5: error[E-VERIFY-BYTE]: could not prove that `((s2 % 4) * 64) + s3` fits in a byte (`<= 255`)
  |     bset(&mut out, o + 2, s2 % 4 * 64 + s3);
  |     ^
  = counterexample: i = 0, n = 8, o = 0, len(out) = 6, pad = 0, s0 = 63, s1 = 63, s2 = 3, s3 = 64, len(src) = 8
```

**Not proved:** that decoding is the inverse of encoding, or that the decoder
accepts exactly the canonical encodings. That is functional correctness, which
is opt-in in Bastio and not attempted here. `test.c` checks it by testing
instead: round trips of random data, and every verdict, error offset and
output byte compared with an independent table-driven C decoder on random and
mutated inputs.

## The C boundary

```c
#include "base64.h"

uint8_t *out; size_t n, bad;
if (base64_decode(src, len, &out, &n, &bad) == 0) {
    use(out, n);
    base64_free(out);
} else {
    report(bad);   /* offset of the first invalid byte */
}
```

stage0 has no way yet to export a function to C. Until it does,
`base64_capi.c` includes the generated C as one translation unit (its
functions are `static`), renames the unused `main` with the preprocessor, and
wraps the two functions. That file is trusted. What it relies on:
- `&Bytes` parameters are read-only (the type checker allows no write, free
  or move through them), so passing the caller's `const` buffer is sound;
- `b64_encode` `requires blen(src) <= 2^59`. A C caller cannot be proved to
  meet a precondition, so the wrapper checks it: at the boundary, a
  precondition becomes the caller's obligation, and here the caller is
  hand-written C;
- the result buffers come from the hosted prelude's `calloc`, so
  `base64_free` frees them with `free`;
- the tags of `enum Decoded` are numbered in declaration order.

An `export fn` (no `main` required, a non-static C symbol, a generated header,
preconditions on exported functions either ruled out or checked in a
generated wrapper) would remove this file. See ROADMAP.md.

## Speed

`run.sh bench` decodes 64 MiB and compares with the C reference decoder in
`test.c`, built with the same `-O2`. That reference is straightforward
table-driven C of the same strictness, not a tuned or SIMD decoder. On one
x86-64 Linux machine (2026-10-10) the Bastio decoder ran at about 1000 MiB/s
and the C reference at about 770 MiB/s: the proofs cost nothing at run time,
because they leave no checks behind. Numbers vary by machine; run it.
