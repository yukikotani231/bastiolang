# Calling Bastio from C

The first place Bastio is meant to be used is inside a C program: replace the
one parser or decoder that reads untrusted input with a proved one, and keep
the rest. An `export fn` is a proved Bastio function with a plain C
signature, and `bastioc0 --header` writes the C header for it. This page is
the recipe; the [language reference](language.md#calling-bastio-from-c)
has the exact rules, and the [tutorial](tutorial.md#8-calling-bastio-from-c)
builds the smallest library step by step.

## The examples

Each is a library for untrusted input with a C test driver that runs it
under AddressSanitizer and UBSan, compares it with an independent C
implementation on random and damaged inputs, and measures its speed. Each
one's `run.sh` builds and tests it, and `tests/run.sh` runs all of them in
CI.

- [base64](examples-base64.md): a strict codec, with an encoder that writes
  into a caller's buffer.
- [UTF-8](examples-utf8.md): a strict validator and decoder.
- [DER](examples-der.md): the structure checker under X.509 certificates.
- [QOI](examples-qoi.md): an image codec. It allocates buffers sized by an
  attacker-controlled header, so it shows how a library reports running out
  of memory instead of aborting.

## Building a library

```text
bastioc0 --emit-c --header qoi.h -o qoi.c lib.bastio
cc -std=c11 -O2 -c qoi.c
cc -std=c11 -O2 app.c qoi.o
```

The first line proves the library (it needs `z3` on `PATH`) and writes
plain C11. Nothing after it needs Bastio: you can check the generated
`qoi.c` and `qoi.h` into the C project and build them with its own compiler
flags, and regenerate them when the Bastio source changes, as with any
generated parser. The proofs are about the Bastio source, so regenerate
rather than edit the C.

What the generated C is:
- one `.c` file with the library and the runtime prelude it needs. Everything
  but the exported functions is `static`, so the object exports exactly the
  `export fn` names. Optimized, it is small: the QOI codec is about 4 KB of
  code, and needs only `calloc` and `free` from the C library;
- a header with the C types of the results and a declaration of each
  exported function, its `ensures` clauses copied above it as comments. They
  are proved, so the caller may rely on them;
- C11 for a hosted POSIX system: the prelude includes `<unistd.h>` and a
  few other POSIX headers for features of whole programs, though a library's
  optimized object calls nothing from them.

Compile the generated C with warnings off (`-w`): the prelude defines
helpers a given library does not use.

## Ownership

- An input is `&Bytes` in Bastio, a `const uint8_t *` and a `size_t` in C.
  The library only reads it and never keeps it.
- An output buffer the caller owns is `&mut Bytes`, `uint8_t *` and
  `size_t`. The compiler proves that the library writes only inside it and
  never frees or replaces it.
- A returned `Bytes` (also inside a returned struct or enum) is a
  `bst_bytes { len, data }` that belongs to the caller: release `data` with
  `free()`.
- A returned enum is a struct with a `tag` and a union `u` of the payloads.
  The header defines a `bst_e_Name_Variant` constant for each tag.

## Errors and running out of memory

An exported function has no `requires`: a C caller cannot be proved to meet
it. The library checks its inputs and returns an error variant instead, and
those checks are proved like the rest. So a library's results are enums such
as QOI's:

```c
bst_e_QoiDecoded d = qoi_decode(file, len, max_pixels);
switch (d.tag) {
case bst_e_QoiDecoded_Ok:          use(&d.u.Ok); free(d.u.Ok.pixels.data); break;
case bst_e_QoiDecoded_Err:         /* d.u.Err: offset of the first bad byte */ break;
case bst_e_QoiDecoded_TooBig:      /* over max_pixels */ break;
case bst_e_QoiDecoded_OutOfMemory: break;
}
```

Allocation in Bastio aborts the process when memory runs out (`balloc`,
`vec_push`), which is right for a program and wrong for a library. A
library allocates with `try_balloc` (or `vec_try_reserve` for a `Vec`)
instead, which gives back an empty buffer when memory runs out, and the
prover makes it test the length before the buffer is used. The
`OutOfMemory` variant is then an ordinary result, freed of everything on the
way out: QOI's test caps AddressSanitizer's allocations and checks that a
huge header gets `OutOfMemory` and leaks nothing. Better still, let the
caller bound the size, as `qoi_decode`'s `max_pixels` does, or write into
the caller's buffer, as `base64_encode_into` and `qoi_encode_into` do.

## Several libraries in one program

Each library's object exports only its own functions, so several link into
one program. The C type names in the headers come from the Bastio type
names (`QoiDecoded` becomes `bst_e_QoiDecoded`), and two headers that define
the same name cannot be included in the same C file. Give a library's
exported types a prefix, as QOI does; `bst_bytes` is shared and guarded.

## What the proofs assume of the C side

The proofs cover the Bastio code for every input a C caller can pass, with
no runtime checks added. They rest on what nothing can check from the Bastio
side, and the header states it: an input buffer is readable for its length
and not written during the call; an output buffer is writable for its length
and overlaps no other; every argument is a valid value of its C type. And the
C compiler, the C library's allocator and Z3 are trusted (see
[the trusted base](https://github.com/yukikotani231/bastiolang/blob/main/docs/trusted-base.md)).
