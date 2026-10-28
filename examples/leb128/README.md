# A proved LEB128 codec, called from C

Unsigned LEB128 is the variable-length integer of WebAssembly, DWARF and
protobuf: seven bits per byte, least significant group first, and the high bit
of a byte says that another byte follows. It is read from untrusted input
(a `.wasm` file, a debug section, a network message), which is where an
unproved index or a shift by 64 goes wrong. This example is the decoder and
encoder in Bastio, called from a C program through a header the compiler
generates.

| File | What it is |
| --- | --- |
| `leb128.bastio` | The codec: decode one value at an offset, encode one value into a buffer. Proved |
| `lib.bastio` | The C API: `export fn leb128_decode` and `leb128_encode_into`. Proved |
| `test.c` | The C harness: known vectors, the three refusals, a differential fuzz against a C decoder, round trips |
| `run.sh` | Generates `leb128.c` and `leb128.h` from `lib.bastio`, builds the harness and runs it under AddressSanitizer + UBSan |

`tests/run.sh` runs `run.sh`, so it is in CI.

## What is proved

For every buffer, offset and value, with no runtime check in the generated C:
- the decoder reads only inside the buffer and never more than 10 bytes;
- the number of bytes it reports is between 1 and 10 and fits what is left of
  the buffer after the offset;
- no shift is 64 or more, and there is no integer overflow, underflow or
  division by zero (the tenth byte is checked before it is shifted in, which
  is what makes the shift and the `|` safe);
- the encoder writes only inside the buffer it is lent, never more than 10
  bytes, and reports at most `blen(out)`;
- no leak and no double free: the library allocates nothing.

**Not proved:** that decoding inverts encoding, that the encoder's output is
the shortest encoding, or that `leb_size` agrees with what the encoder writes.
`test.c` checks those by testing: the vectors of the DWARF specification and
others, round trips of random values, and every verdict, value and length
compared with an independent C decoder on random bytes at random offsets and
on encodings with bytes changed.

## The decoder's verdicts

`leb128_decode(src, len, at)` returns one of:

| Result | When |
| --- | --- |
| `Ok` with `value` and `used` | A byte without the continuation bit ends the value within 10 bytes |
| `Truncated` | `at` is past the end, or the input ends before a byte without the continuation bit |
| `TooLong` | The tenth byte still has its continuation bit set (an eleventh byte would be needed) |
| `Overflow` | The tenth byte is more than 1, so the value needs more than 64 bits |

Redundant zero groups are accepted as long as the value stays within ten bytes
(`80 80 .. 00` is a long way to write 0, as it is in DWARF); `used` says how
much input the value took.

## The C boundary

```c
bst_e_Leb128Decoded leb128_decode(const uint8_t *src, size_t src_len, uint64_t at);
bst_e_Leb128Written leb128_encode_into(uint64_t x, uint8_t *out, size_t out_len);
```

```c
#include "leb128.h"

bst_e_Leb128Decoded r = leb128_decode(buf, len, offset);
if (r.tag == bst_e_Leb128Decoded_Ok) {
    use(r.u.Ok.value);
    offset += r.u.Ok.used;
} else {
    report(r.tag);
}
```

`leb128_encode_into` writes into a buffer the caller provides and returns the
number of bytes written, or `TooSmall` with the size needed. As for the other
exported functions, the proofs assume that `src` is readable for `src_len`
bytes and not written during the call, that `out` is writable for `out_len`
bytes and overlaps nothing else, and that the arguments are valid C values
(docs/language.md, "Calling Bastio from C").
