# Sized integers

Status: design, 2026-10-10. Not implemented yet; see "Implementation plan" for the steps and ROADMAP.md v1 item 8 for where they sit.

Bastio has one integer type, `u64`, left over from v0. Anything narrower is a `u64` plus a range fact: `Bytes` elements are proved `<= 255`, `serial_write` and `frame_write8` have their own error code (`E-VERIFY-BYTE`) for "fits in a byte", and the kernel cannot express a 16-bit I/O port or a 32-bit device register at all, which is one reason the PIC, PIT and UART code still lives in C (`boot/platform.c` in BastiOS). This document adds `u8`, `u16`, `u32`, `i8`, `i16`, `i32` and `i64`, the conversions between them, and (in a later step) bitwise operators.

## Goals

1. **Never wraps, at every width.** As with `u64` today, `+ - *` on any integer type are proved to stay in range. There is no silent truncation anywhere in the language; losing bits is always written out.
2. **Zero overhead.** A `u8` is a C `uint8_t` and costs nothing more. Range facts are ghost.
3. **Types replace ad-hoc range facts.** "This is a byte" becomes the type `u8`, so the prover no longer needs a special obligation for it, and C code sees the real width at the ABI.
4. **Little annotation.** Literals adapt to their position, and widening conversions are proved without help.

## The types

| Type | Range | C type |
| --- | --- | --- |
| `u8` | 0 to 2^8 - 1 | `uint8_t` |
| `u16` | 0 to 2^16 - 1 | `uint16_t` |
| `u32` | 0 to 2^32 - 1 | `uint32_t` |
| `u64` | 0 to 2^64 - 1 | `uint64_t` |
| `i8` | -2^7 to 2^7 - 1 | `int8_t` |
| `i16` | -2^15 to 2^15 - 1 | `int16_t` |
| `i32` | -2^31 to 2^31 - 1 | `int32_t` |
| `i64` | -2^63 to 2^63 - 1 | `int64_t` |

All are plain (non-linear) values. They can be fields, enum payloads, `Vec` elements (a `Vec[u8]` stores one byte per element), `Box` contents, type arguments and `const` types.

There is no `usize`. Bastio targets x86-64 only, and `u64` already plays that role: lengths are `u64`, as today. If a 32-bit target ever appears, `usize` can be added then.

## Arithmetic

- Both operands of `+ - * / %` and of the comparisons must have the same type; the result has that type. Mixing types is `E-TYPE` and is fixed with a conversion (below). There is no implicit promotion, unlike C.
- The prover checks each operation against the range of its type:

| Operation | Obligation | Code |
| --- | --- | --- |
| `a + b`, `a * b` | result <= the type's maximum | `E-VERIFY-OVERFLOW` (as today) |
| `a + b`, `a * b` (signed) | result >= the type's minimum | `E-VERIFY-UNDERFLOW` |
| `a - b` | result in range (unsigned: `a >= b`, as today) | `E-VERIFY-UNDERFLOW` / `E-VERIFY-OVERFLOW` |
| `a / b`, `a % b` | `b != 0` | `E-VERIFY-DIVZERO` (as today) |
| `a / b`, `a % b` (signed) | not (`a` is the minimum and `b == -1`) | `E-VERIFY-OVERFLOW` |
| `-a` (signed only) | `a` is not the minimum | `E-VERIFY-OVERFLOW` |

- Signed division truncates toward zero and `%` takes the sign of the dividend, as in C. (SMT-LIB's `div` and `mod` round differently for negative operands, so the verifier encodes C's rule explicitly; see "Encoding".)
- Unary minus is new and applies only to signed types. Unsigned values never go negative, so `-x` on an unsigned type is `E-TYPE`.

## Literals

An integer literal has no type of its own. It takes the integer type its position asks for:
- the declared type of a `let`, `const`, parameter, struct field, enum payload or return value;
- the type of the other operand of a binary operator (`x + 1` with `x: u8` makes `1` a `u8`);
- `u64` when nothing constrains it (`let n = 5;`, or `2 + 3`).

So existing programs keep their meaning: every literal that is a `u64` today stays one.

Whether a literal fits its type is decided by the checker, not the prover, because it is a constant: `let b: u8 = 256;` is `E-TYPE` ("256 does not fit in u8"). A minus sign directly in front of a literal belongs to the literal, so `let m: i8 = -128;` is accepted.

## Conversions

Two builtins, both with the target type written as a type argument, in the syntax generic calls already use:

| Builtin | Meaning | Obligation |
| --- | --- | --- |
| `cast[T](x)` | the same number, as type `T` | `x` fits in `T` (`E-VERIFY-CAST`) |
| `wrap[T](x)` | `x` modulo 2^width, read as `T` (two's complement for signed `T`; what a C cast does) | none |

`cast` is the normal conversion. It never changes the value; it asks the prover to show the value fits. Widening (`u8` to `u32`, `u32` to `i64`) always fits, so the obligation is discharged from the type alone and costs no annotation. Narrowing works whenever the program already knows the value is small:

```
fn hex_digit(n: u64) -> u8
  requires n < 16
{
  if n < 10 { return cast[u8](n) + 48; }   // '0' + n
  return cast[u8](n) + 87;                 // 'a' + n - 10
}
```

`wrap` is for code that means to drop bits or reinterpret them: checksums, hashes, taking the low byte of a word, turning a `u64` system-call result into an `i64` error code. In specifications, `wrap[u8](x)` is `x mod 256`.

The name is deliberately not `as`. In Rust and C, a cast truncates silently; in Bastio `cast` never does, and a familiar keyword with a different meaning would mislead.

Wrapping arithmetic for hashes and checksums comes as three builtins on operands of the same type: `wrap_add(a, b)`, `wrap_sub(a, b)`, `wrap_mul(a, b)`. They have no obligation and mean `wrap[T](a + b)` and so on. (Writing that out directly does not work for `u64`, whose sum may not fit any type.)

There is no conversion between integers and `bool`. `cast`, `wrap` and the `wrap_*` names are builtins, so a program cannot define functions with those names.

## Builtins and existing APIs

- **`u64` parameters of the builtins** (indexes, lengths, values: `get`, `set`, `bget`, `vec_get`, `alloc`, `print`, ...) accept any unsigned integer type, since widening to `u64` is free and always fits. A signed value is not accepted there: it may be negative, so it is converted with `cast[u64]` (proved non-negative) or `wrap[u64]`. Lengths are still returned as `u64`. Functions the program defines get no such exception.
- `print` takes `u64` only for now; a signed value is printed after `cast[u64]` or `wrap[u64]`. Printing with a sign can come later.
- An expression made only of literals has no position to take a type from and is `u64`, so `-7 / 2` on its own is `E-TYPE`; write it where a signed type is expected (`let n: i8 = -7;`).
- `Bytes`, `serial_write`, `serial_read` and `frame_read8`/`frame_write8` move to `u8` in step 4 (below). Until then they keep their `u64` signatures and range facts, so nothing breaks.

## Specifications

Specification expressions keep evaluating over mathematical integers. Every integer value, of any type, is just a number there, so specifications mix types freely and need no conversions: with `result: u8` and `n: u64`, `ensures result == n + 48` is fine. (`forall` ranges over `u64` only at first; `forall x: u8 :: ...`, ranging over 0 to 255, comes later.)

## C ABI and layout

- Each type maps to the matching `<stdint.h>` type, so a struct of integer fields has exactly the layout of the same C struct (same field order, natural alignment). This is what `export fn` (ROADMAP, "C boundary") needs to hand structs and integers to C without wrappers.
- Enum layout stays unspecified; enums do not cross the C boundary.
- Generated C computes each operation in a type at least as wide as `int` (C promotes `uint8_t` operands to `int` anyway) and converts the result back. The prover has shown that the mathematical result is in the range of the operation's type, and that range fits in the promoted C type, so no C operation overflows and the conversion back never changes the value.
- `wrap` to a signed type converts an out-of-range value to a signed C type, which C leaves implementation-defined. GCC and Clang document it as modular, and the C compiler is already in the trusted base; the generated code computes in the unsigned type of the same width and converts once, so this is the only place it matters.

## Bitwise operators (step 3)

Kernels and decoders need them: page-table entries, device registers, the PIT divisor split into bytes, little-endian loads (BastiOS's `read_u64` builds a word from eight bytes with multiplications today).

- `&`, `|`, `^`, `<<`, `>>` and `!x` (bitwise not, as in Rust; the language has no `~`) on unsigned types, both operands of the same type except for shifts.
- A shift amount can be any unsigned type and is proved less than the width (`E-VERIFY-SHIFT`).
- `x << k` follows the never-wraps rule: it is proved not to lose bits (`E-VERIFY-OVERFLOW`). Dropping the high bits on purpose is `wrap[T](cast[u64](x) << k)` or masking first.
- Signed operands are left out at first. Code that needs them converts with `wrap`.
- Binary `&` does not collide with the borrow `&x`: a borrow is a prefix in argument position, a bitwise and is infix.

## Encoding in the prover

- Values stay SMT integers, as now. Each value of type `T` gets the range fact of `T` when it is created (parameters, reads from buffers and fields, results of calls), exactly as `u64` values get `[0, 2^64)` today. Arithmetic is unchanged; only the bound in each overflow obligation depends on the type.
- Signed `/` and `%` are encoded with C's truncating rule: `q = sign(a) * sign(b) * (|a| div |b|)`, `r = a - b * q`.
- `cast` is the identity on the integer plus the obligation. `wrap` to an unsigned type is `mod 2^w`; to a signed type it is `((x + 2^(w-1)) mod 2^w) - 2^(w-1)`.
- Bitwise operators are where the integer encoding is weakest, since Z3's integer theory has no bitwise operations. The common kernel cases are rewritten into arithmetic Z3 already handles well: `x & (2^k - 1)` is `x mod 2^k`, `x >> k` with a constant `k` is `x div 2^k`, `x << k` with a constant `k` is `x * 2^k`. Everything else goes through `int2bv`/`bv2nat` on the operands, plus cheap facts that let most proofs avoid the bit-vector terms entirely (`x & y <= x`, `x & y <= y`, `x | y >= x`, `x ^ y <= 2^w - 1`). Proof times are watched in `bench/` (ROADMAP invariant 5).
- The alternative, making every value a bit-vector, was rejected: today's proofs (bounds, lengths, page counts) are linear integer arithmetic, which bit-vectors make much slower, and specifications are meant to read as mathematics.

## Hardware access (step 4)

Sized types are what the v3 item "`PortIo`, `Mmio[R]`, `IrqOff`" needs:

```
fn port_in8(io: &PortIo, port: u16) -> u8
fn port_out8(io: &PortIo, port: u16, v: u8)
fn port_in16 / port_out16 / port_in32 / port_out32   // u16 and u32 values
```

The port number cannot exceed 16 bits and the value cannot exceed the access width, and neither needs a proof: the types say it. Memory-mapped registers get their width from the register's declared type in the same way.

At the same time the byte-shaped builtins take and return `u8`: `bget -> u8`, `bset(.., v: u8)`, `serial_write(c, b: u8)`, `serial_read -> u8`, `frame_read8 -> u8`, `frame_write8(.., b: u8)`. `E-VERIFY-BYTE` disappears, since "fits in a byte" is now the type. This changes existing code (stage1's lexer compares `bget` results with literals, which keeps working, but code that does `n * 10 + bget(..)` on a `u64` needs `cast[u64](..)`), so it lands as one change across bastiolang and BastiOS. `serial_poll`'s "256 means nothing" result becomes an enum.

## Error codes

| Code | New or changed |
| --- | --- |
| `E-TYPE` | operands of different integer types; a literal that does not fit its type; unary minus on an unsigned type |
| `E-VERIFY-CAST` | new: the argument of `cast[T]` fits in `T` |
| `E-VERIFY-SHIFT` | new (step 3): a shift amount is less than the width |
| `E-VERIFY-OVERFLOW` / `E-VERIFY-UNDERFLOW` | now relative to the operation's type |
| `E-VERIFY-BYTE` | removed in step 4 |

## Implementation plan

Each step comes with `tests/pass` and `tests/fail` programs (ROADMAP invariant 4), including a failing program per new obligation, and is checked under UBSan like every passing test.

1. **Unsigned widths and conversions** (done 2026-10-10). `u8`, `u16`, `u32`; literal typing; `cast`, `wrap`, `wrap_add`/`wrap_sub`/`wrap_mul`; index parameters of any unsigned type; C types. Touches stage0's parser (type names), `mono.rs` (type arguments on builtins), `check.rs`, `verify.rs` and `codegen.rs`, and stage1's parser and checker, which must give the same `--dump-check` result (CI's difftest runs every test program through both).
2. **Signed types.** `i8` to `i64`, unary minus, the signed division encoding. Done 2026-10-10, in stage0 and the stage1 checker.
3. **Bitwise operators**, with the encoding above.
4. **Byte-shaped builtins move to `u8`, and `PortIo`** (with BastiOS moving the PIC, PIT and UART setup out of `platform.c`).

Steps 1 and 2 come before stage1's verifier and code generator are written, so those are written once, with sized integers, rather than retrofitted. Steps 3 and 4 can follow the self-hosting fixed point.

## Open questions

- Whether `Bytes` should become `Vec[u8]` once step 4 lands. They differ today: `Bytes` has a fixed length and is what files and arguments come in as; a `Vec` grows. Keeping both is the default.
- Saturating arithmetic (`sat_add`) for counters. Not needed by anything yet.
- Literal suffixes (`5u8`). Not needed while literals take their type from context; `cast[u8](5)` covers the rare case where no context exists.
