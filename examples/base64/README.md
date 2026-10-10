# A proved base64 codec, called from C

A small, real decoder for untrusted input, written in Bastio and called from a
C program through a C header the compiler generates. This is the shape Bastio is meant to be
adopted in first: replace one parser in a C code base, keep the rest.

| File | What it is |
| --- | --- |
| `base64.bastio` | The codec: strict RFC 4648 base64 decoding and encoding. Proved |
| `lib.bastio` | The C API: `export fn base64_decode`, `base64_encode` and `base64_encode_into`. Proved |
| `test.c` | The C harness: RFC 4648 vectors, a differential fuzz, a benchmark |
| `run.sh` | Generates `base64.c` and `base64.h` from `lib.bastio`, builds the harness and runs it under AddressSanitizer + UBSan (`run.sh bench`: throughput) |

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
base64.bastio:139:39: error[E-VERIFY-OVERFLOW]: could not prove that `(s2 % 4) * 64 + s3` does not overflow u8
  |     bset(&mut out, o + 2, s2 % 4 * 64 + s3);
  |                                       ^
  = counterexample: ..., s0 = 63, s1 = 63, s2 = 3, s3 = 64, ...
```

**Not proved:** that decoding is the inverse of encoding, or that the decoder
accepts exactly the canonical encodings. That is functional correctness, which
is opt-in in Bastio and not attempted here. `test.c` checks it by testing
instead: round trips of random data, and every verdict, error offset and
output byte compared with an independent table-driven C decoder on random and
mutated inputs.

## The C boundary

`lib.bastio` marks the two API functions `export fn`, and
`bastioc0 --emit-c --header base64.h -o base64.c lib.bastio` turns them into
C functions with a generated header (`run.sh` does this):

```c
bst_e_Decoded base64_decode(const uint8_t *src, size_t src_len);
bst_e_Encoded base64_encode(const uint8_t *src, size_t src_len);
bst_e_Written base64_encode_into(const uint8_t *src, size_t src_len, uint8_t *out, size_t out_len);
```

`base64_encode_into` writes into a buffer the caller provides, so nothing
is allocated: it returns the number of bytes written, or `TooSmall` with
the size needed. Its `out` parameter is `&mut Bytes` in Bastio, and the
compiler proves that the function writes only inside it and never frees or
replaces it (the encoder writes through `bset`, which keeps the buffer;
anything else would be rejected with `E-EXPORT`). `test.c` checks it
against `base64_encode` with exact-size heap buffers under AddressSanitizer,
and that the bytes after the encoding are left alone.

```c
#include "base64.h"

bst_e_Decoded r = base64_decode(src, len);
if (r.tag == bst_e_Decoded_Ok) {
    use(r.u.Ok.data, r.u.Ok.len);
    free(r.u.Ok.data);
} else {
    report(r.u.Err);   /* offset of the first invalid byte */
}
```

There is no hand-written C between the caller and the proofs any more (an
earlier version needed a trusted wrapper, `base64_capi.c`). What remains:
- `b64_encode` `requires blen(src) <= 2^59`. A C caller cannot be proved to
  meet a precondition, so an exported function may not have one;
  `base64_encode` checks the length in Bastio and returns
  `Encoded::TooLong`, and that check is proved like the rest;
- the proofs assume what every exported function assumes of its C caller:
  `src` is readable for `src_len` bytes and not written during the call;
  `out` is writable for `out_len` bytes and overlaps nothing else; and the
  arguments are valid C values (docs/language.md, "Calling Bastio from C").

## Speed

`run.sh bench` decodes 64 MiB and compares with the C reference decoder in
`test.c`, built with the same `-O2`. That reference is straightforward
table-driven C of the same strictness, not a tuned or SIMD decoder. On one
x86-64 Linux machine (2026-10-10) the Bastio decoder ran at about 1000 MiB/s
and the C reference at about 770 MiB/s: the proofs cost nothing at run time,
because they leave no checks behind. Numbers vary by machine, and even by how
the compiler lays out the harness: on a second machine, the Bastio decoder
ran at about 670 MiB/s both before and after the switch to `export fn`, while
the reference moved between 480 and 700 MiB/s with the harness's own code.
Read them as "on par with plain C", and run it.
