# Bytes, bits and sized integers

This is the second part of the [tutorial](tutorial.md). It covers the types you reach for when you parse a file format, talk to a device or checksum a packet: `u8`, `u16`, `u32`, the signed types, and the bitwise operators. If you have not read the [first part](tutorial.md), read at least sections 2 to 5 of it: contracts, loops and invariants are used here without explanation.

As in the first part, every `bastio` block is compiled by `tests/run.sh`. The first line says what must happen: `// expect-exit: N` means the program is accepted, run and exits with `N`; `// expect-error: CODE` means the compiler rejects it with that code.

## 1. One rule: nothing wraps

C lets an unsigned addition wrap and a signed one do anything it likes. Bastio does neither. For every integer type, `+`, `-` and `*` are *proved* to stay in the range of the type, exactly as `u64` arithmetic is in the first part. When the compiler cannot prove it, it refuses the program:

```bastio
// expect-error: E-VERIFY-OVERFLOW
fn main() -> u64 {
  let a: u8 = 200;
  let b: u8 = 100;
  let c = a + b;          // 300 does not fit in a u8
  return cast[u64](c);
}
```

Here the compiler even knows the values. In real code it knows what you told it: a test, a `requires` line, a loop bound. With a test, the same addition is accepted:

```bastio
// expect-exit: 42
fn add_checked(a: u8, b: u8) -> u8 {
  if a <= 255 - b {
    return a + b;         // proved: a + b <= 255
  }
  return 255;             // saturate instead
}

fn main() -> u64 {
  if add_checked(200, 100) != 255 || add_checked(20, 10) != 30 {
    return 1;
  }
  return 42;
}
```

There is no way to get a wrapped result by accident. When you *want* modular arithmetic, for a checksum or a hash, you say so by name: `wrap_add`, `wrap_sub`, `wrap_mul` (section 4).

The types are:

| Unsigned | Signed | C type |
| --- | --- | --- |
| `u8` | `i8` | `uint8_t`, `int8_t` |
| `u16` | `i16` | `uint16_t`, `int16_t` |
| `u32` | `i32` | `uint32_t`, `int32_t` |
| `u64` | `i64` | `uint64_t`, `int64_t` |

They are plain values: copied freely, usable as fields, `Vec` elements and type arguments. A `Vec[u8]` stores one byte per element, and a struct of integers has the layout of the same C struct, which is what lets [C call into Bastio](tutorial.md#9-calling-bastio-from-c) without wrappers.

## 2. No implicit conversions

C promotes a `uint8_t` to `int` before it does anything, and then converts back without telling you. Bastio has no promotion: both operands of an operator have the same type, and mixing types is a type error.

```bastio
// expect-error: E-TYPE
fn main() -> u64 {
  let small: u8 = 1;
  let big: u32 = 2;
  let sum = small + big;  // which type is the result? the compiler will not guess
  return 0;
}
```

A literal has no type of its own. It takes the type its position asks for: `small + 1` makes the `1` a `u8`, and `let x: u16 = 7;` makes it a `u16`. When nothing asks, it is a `u64`. A literal that does not fit is an error: `let b: u8 = 256;` is rejected.

To change type you write a conversion. There are two, and the difference is the whole point:

- **`cast[T](x)` keeps the number.** The compiler must prove that the value fits in `T`. Widening (`u8` to `u32`) always fits, so it costs nothing to write. Narrowing works when the program already knows the value is small.
- **`wrap[T](x)` keeps the low bits.** It reduces the number modulo 2^width, like a C cast, and has no obligation. Use it when dropping bits is what you mean.

```bastio
// expect-exit: 42
fn digit(n: u64) -> u8
  requires n < 10
{
  return cast[u8](n) + 48;     // the contract makes the narrowing safe
}

fn main() -> u64 {
  let wide: u32 = 70000;
  let low: u16 = wrap[u16](wide);       // 70000 mod 65536 = 4464
  let up: u64 = cast[u64](digit(7));    // widening: no proof needed
  if low != 4464 || up != 55 {
    return 1;
  }
  return 42;
}
```

If you narrow without knowing the value is small, the compiler says so, and offers the two ways out: prove it, or admit you mean to drop bits.

```bastio
// expect-error: E-VERIFY-CAST
fn low_byte(n: u64) -> u8 {
  return cast[u8](n);          // n may be 256 or more
}

fn main() -> u64 {
  return cast[u64](low_byte(300));
}
```

```bastio
// expect-exit: 42
fn low_byte(n: u64) -> u8 {
  if n < 256 {
    return cast[u8](n);        // proved by the test
  }
  return wrap[u8](n);          // or: n modulo 256, on purpose
}

fn main() -> u64 {
  if low_byte(5) != 5 || low_byte(300) != 44 {
    return 1;
  }
  return 42;
}
```

The builtins that take a length or an index (`get`, `bget`, `alloc`, ...) accept any unsigned type where they expect a `u64`, since widening is free. A signed value is not accepted there, because it may be negative: convert it with `cast[u64]`, which the compiler proves non-negative.

## 3. Bytes in, integers out

Binary formats are bytes. `Bytes` is an owned buffer of `u8`: `balloc(n)` makes `n` zero bytes, `bget(b, i)` and `bset(&mut b, i, v)` read and write one, and every access carries the obligation `i < blen(b)`. Building a 16-bit value from two of them is a shift and an or:

```bastio
// expect-exit: 42
fn read_le16(b: &Bytes, at: u64) -> u16
  requires at + 2 <= blen(b)
{
  return (cast[u16](bget(b, at + 1)) << 8) | cast[u16](bget(b, at));
}

fn main() -> u64 {
  let mut b = balloc(4);
  bset(&mut b, 0, 0x34);
  bset(&mut b, 1, 0x12);
  let v = read_le16(b, 0);
  bfree(b);
  if v != 0x1234 {
    return 1;
  }
  return 42;
}
```

Look at what the proof is doing. Each `cast[u16]` of a byte is a widening and is free. The shift `<< 8` must lose no bits, and the compiler sees the operand is at most 255, so the result is at most 65280 and fits. The two reads need `at + 1 < blen(b)`, which the `requires` provides. Nothing is checked when the program runs.

You do not have to write these helpers yourself. [`core/endian.bastio`](lib-core-endian.md) has `load_le16`, `load_be32`, `store_le64` and the rest, each proved in bounds, and the native backends turn them into a single load or store.

Here is a small record format: a magic number, a length and a one-byte checksum. The checksum is the sum of the other bytes modulo 256, which is exactly what `wrap_add` is for:

```bastio
// expect-exit: 42
import "core/endian.bastio";
import "core/option.bastio";

const MAGIC: u64 = 0xB451;
const HEADER: u64 = 5;

fn checksum(b: &Bytes, n: u64) -> u8
  requires n <= blen(b)
{
  let mut sum: u8 = 0;
  let mut i = 0;
  while i < n
    invariant i <= n
  {
    sum = wrap_add(sum, bget(b, i));
    i = i + 1;
  }
  return sum;
}

fn write_record(len: u16) -> Bytes
  ensures blen(result) == HEADER
{
  let mut b = balloc(HEADER);
  store_be16(&mut b, 0, cast[u16](MAGIC));
  store_be16(&mut b, 2, len);
  let sum = checksum(b, 4);
  bset(&mut b, 4, sum);
  return b;
}

/// The record's length, if the magic number and checksum are right.
fn read_record(b: &Bytes) -> Option[u16] {
  if blen(b) < HEADER || cast[u64](load_be16(b, 0)) != MAGIC {
    return Option[u16]::None;
  }
  if checksum(b, 4) != bget(b, 4) {
    return Option[u16]::None;
  }
  return Option[u16]::Some(load_be16(b, 2));
}

fn main() -> u64 {
  let b = write_record(1234);
  let good = read_record(b);
  let mut bad = b;
  bset(&mut bad, 3, 99);            // damage the length
  let corrupt = read_record(bad);
  bfree(bad);
  let mut ok = false;
  match good {
    Option::Some(n) => { ok = n == 1234; }
    Option::None => {}
  }
  match corrupt {
    Option::Some(n) => { ok = false; }
    Option::None => {}
  }
  if ok {
    return 42;
  }
  return 1;
}
```

Notice what the compiler never asks about: whether the buffer is long enough (the `blen(b) < HEADER` test, and the one in `checksum`'s contract, answer it), or whether `sum` overflowed (it is allowed to wrap, and said so).

## 4. Wrapping on purpose

`wrap_add(a, b)`, `wrap_sub(a, b)` and `wrap_mul(a, b)` take two operands of one type and give `a + b`, `a - b` or `a * b` modulo 2^width. They exist for the things that really are modular: checksums, hash functions, linear congruential generators.

```bastio
// expect-exit: 42
/// FNV-1a over a byte string, 32 bits.
fn fnv1a(b: &Bytes) -> u32 {
  let mut h: u32 = 2166136261;
  let mut i = 0;
  while i < blen(b)
    invariant i <= blen(b)
  {
    h = wrap_mul(h ^ cast[u32](bget(b, i)), 16777619);
    i = i + 1;
  }
  return h;
}

fn main() -> u64 {
  let b = bfrom("a");
  let h = fnv1a(b);
  bfree(b);
  if h == 3826002220 {
    return 42;
  }
  return 1;
}
```

Anything that wraps is spelled `wrap_` or `wrap[...]`, so you can find every place a program drops bits by searching for it.

## 5. Bit twiddling

`&`, `|`, `^`, `<<`, `>>` and `!x` work on unsigned integers (`!` is the bitwise not on integers, the logical not on `bool`). They bind tighter than comparisons, as in Rust, so `x & 1 == 1` means `(x & 1) == 1`. Two rules keep them safe:

- A shift amount must be proved less than the width of the type (`E-VERIFY-SHIFT`), because C leaves a larger shift undefined.
- A left shift must lose no bits (`E-VERIFY-OVERFLOW`). To drop the high bits on purpose, mask first.

```bastio
// expect-exit: 42
const READY: u64 = 3;

fn set_bit(flags: u32, k: u64) -> u32
  requires k < 32
{
  return flags | (cast[u32](1) << k);
}

fn clear_bit(flags: u32, k: u64) -> u32
  requires k < 32
{
  return flags & !(cast[u32](1) << k);
}

fn main() -> u64 {
  let mut f: u32 = 0;
  f = set_bit(f, READY);
  f = set_bit(f, 5);
  f = clear_bit(f, 5);
  if f == 8 && f >> READY & 1 == 1 {
    return 42;
  }
  return 1;
}
```

The prover reasons about masks and shifts exactly when they are by a constant (`x & 255` is `x % 256`, `x >> 8` is `x / 256`), and gives only bounds for the general case, such as `x & y <= x`. If a proof needs to know *which* bits a function changed, you say so with `has_bit(x, i)` in a contract:

```bastio
// expect-exit: 42
fn set_bit(w: u64, k: u64) -> u64
  requires k < 64
  ensures has_bit(result, k)
  ensures forall j: u64 :: j != k ==> has_bit(result, j) == has_bit(w, j)
{
  return w | (1 << k);
}

fn main() -> u64 {
  let w = set_bit(4, 1);
  assert has_bit(w, 1);
  assert has_bit(w, 5) == has_bit(4, 5);   // every other bit is as it was
  if w == 6 {
    return 42;
  }
  return 1;
}
```

This is how a page-table or device-register driver states "this write sets the present bit and leaves the rest alone", and the compiler checks the claim.

## 6. Signed integers

`i8` to `i64` follow the same rules, with the addition that the negative end of the range matters too. The obligations are the ones C leaves undefined: `-x` and `x / -1` overflow when `x` is the smallest value, and division by zero is rejected as always. Division truncates toward zero and `%` takes the sign of the dividend, as in C.

```bastio
// expect-error: E-VERIFY-OVERFLOW
fn negate(x: i32) -> i32 {
  return -x;                // fails for x == -2147483648
}

fn main() -> u64 {
  let n = negate(5);
  return 0;
}
```

```bastio
// expect-exit: 42
fn abs(x: i32) -> i32
  requires x > -2147483648
  ensures result >= 0
{
  if x < 0 {
    return -x;              // proved: x is not the minimum
  }
  return x;
}

fn main() -> u64 {
  let n: i32 = -42;
  let a = abs(n);
  return cast[u64](a);      // fits: abs promised a non-negative result
}
```

