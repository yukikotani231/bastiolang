# Bastio language reference

Bastio is a systems language whose memory safety is proved at compile time. This document describes the language that the bootstrap compiler, `bastioc0` (in `stage0/`), accepts on the current `main` branch. The self-hosted compiler in `stage1/` is being written to accept exactly the same language; see [ROADMAP.md](../ROADMAP.md) for how far it has got and for what is planned.

Bastio is pre-release. Anything here may change. New to the language? Start with the [tutorial](tutorial.md).

**About the examples.** Every example in a `bastio` code block below is a whole program, and `tests/run.sh` compiles each one. Its first line says what must happen: `// expect-exit: N` (it verifies, runs and exits with N), `// expect-error: CODE` (it is rejected with that error), or `// expect-ok` (it verifies; used for kernel programs, which cannot run on the build machine). So if an example here is wrong, the test suite fails. Grammar sketches that are not programs are shown as plain text.

## Contents

1. [A first program](#a-first-program)
2. [Using the compiler](#using-the-compiler)
3. [Lexical structure](#lexical-structure)
4. [Programs and files](#programs-and-files)
5. [Types](#types)
6. [Ownership: linear values and borrows](#ownership-linear-values-and-borrows)
7. [Functions and contracts](#functions-and-contracts)
8. [Statements and expressions](#statements-and-expressions)
9. [Structs](#structs)
10. [Enums and `match`](#enums-and-match)
11. [Boxes](#boxes)
12. [Generics](#generics)
13. [Vectors](#vectors)
14. [Buffers, bytes and strings](#buffers-bytes-and-strings)
15. [The host: files and processes](#the-host-files-and-processes)
16. [Kernel capabilities](#kernel-capabilities)
17. [Specifications](#specifications)
18. [How verification works](#how-verification-works)
19. [Error codes](#error-codes)
20. [Targets and generated code](#targets-and-generated-code)
21. [Calling Bastio from C](#calling-bastio-from-c)
22. [Soundness scope](#soundness-scope)
23. [Not in the language yet](#not-in-the-language-yet)

## A first program

```bastio
// expect-exit: 30
// Sum the first n elements of a buffer. The compiler proves that every
// index is in bounds, that no addition overflows, and that the buffer is
// freed exactly once. The generated C has no runtime checks.

fn sum(b: &Buf, n: u64) -> u64
  requires n <= len(b) && n <= 1000
  requires forall i: u64 :: i < len(b) ==> get(b, i) <= 1000
  ensures result <= n * 1000
{
  let mut total = 0;
  let mut i = 0;
  while i < n
    invariant i <= n && total <= i * 1000
  {
    total = total + get(b, i);
    i = i + 1;
  }
  return total;
}

fn main() -> u64 {
  let mut b = alloc(4);
  b = set(b, 0, 5);
  b = set(b, 1, 10);
  b = set(b, 2, 15);
  let s = sum(b, 4);
  free(b);
  return s;
}
```

Take away `requires n <= len(b)` and the program is rejected with `E-VERIFY-BOUNDS` and a counterexample. Take away `free(b)` and it is rejected with `E-LINEAR-LEAK`.

## Using the compiler

```text
bastioc0 [--check] [--emit-c] [--backend c|native] [--target hosted|kernel|user] [--arch x86_64|aarch64] [--header F] [-o out] <file.bastio>...
```

| Option | Effect |
| --- | --- |
| (none) | Check, verify, emit C and compile it with `cc -O2` into an executable (`a.out` unless `-o` is given) |
| `--check` | Check and verify only |
| `--emit-c` | Write the generated C (`out.c` unless `-o` is given) instead of an executable |
| `--target T` | `hosted` (the default), `kernel` or `user`; see [Targets](#targets-and-generated-code). `kernel` and `user` imply `--emit-c` |
| `--arch A` | `x86_64` (the default) or `aarch64`: the processor to build for. AArch64 works with the C backend; hosted programs compile with `aarch64-linux-gnu-gcc` unless `$CC` is set. The kernel and user targets for AArch64 are bastioc1's only. See [Targets](#targets-and-generated-code) |
| `--backend native` | Experimental: generate x86-64 assembly directly instead of C, assemble it with `as` and link it with `cc`. Hosted target only, and not every builtin yet (`Child`, `Lock`, the device types and libraries without `main` are missing); see [Targets](#targets-and-generated-code) |
| `--os O` | bastioc1 only: `linux` (the default), `macos` or `windows`. `windows` (with `--backend native-ir`) writes a PE executable with bastioc1's own assembler and linker (`--emit-asm` gives the assembly: the C library's functions are called through thunks that follow the Win64 convention). With `--backend native --arch aarch64` (Apple Silicon) or `--backend native-ir` (Intel), `macos` writes Mach-O assembly (Apple's `as`, linked with `cc`; the runtime is the C library's, `print` goes over `putchar`). Files and child processes of the host runtime are written but untested there so far |
| `--emit-asm` | With `--backend native`: write the assembly (`out.s` unless `-o` is given) instead of an executable |
| `--header F` | Also write a C header declaring the program's `export fn`s to `F`; see [Calling Bastio from C](#calling-bastio-from-c) |
| `--dump-smt` | Also print the queries sent to Z3 |
| `--trusted-base` | Print the hand-written C each target links in, and every builtin with the targets that have it |
| `--dump-tokens`, `--dump-ast`, `--dump-mono`, `--dump-check` | Print the result of one compiler phase. These exist so the self-hosted compiler can be tested against stage0 |

`bastioc0` runs `z3` (found on `PATH`) as a subprocess and `cc` (or `$CC`) to compile the C it emits. Errors are printed with the source line, a caret and, for proof failures, a counterexample. Exit status: 0 on success, 1 when the program is rejected, 2 for a usage or file error, 3 when the C compiler (or, with `--backend native`, the assembler or linker) fails (which is a compiler bug), 4 when the native backend does not support something the program uses yet.

## Lexical structure

- **Comments** run from `//` to the end of the line. There are no block comments.
- **Identifiers** are ASCII letters, digits and `_`, not starting with a digit.
- **Integer literals** are decimal (`4096`) or hexadecimal (`0x1000`, `0X1000`). `_` may separate digits (`1_000_000`, `0xffff_0000`). A literal must fit in 64 bits, and in the integer type its position gives it. A negative number is unary minus on a literal, where a signed type is expected.
- **String literals** are written in double quotes and cannot span lines. Escapes: `\n`, `\r`, `\t`, `\0`, `\\`, `\"`.
- **Keywords:** `assert` `const` `else` `ensures` `enum` `export` `false` `fn` `forall` `if` `import` `invariant` `is` `let` `match` `mut` `requires` `return` `spec` `struct` `true` `unreachable` `while`. `opaque` is a keyword only before `spec fn`. The names `result`, `old` and `include_bytes`, and the names of builtins, have special meanings described below.

## Programs and files

A program is a list of definitions in any order, except that a constant must be defined before it is used:

```text
fn name[T, ...](p: T, mut q: T, ...) -> R  requires ...  ensures ...  { ... }
export fn name(p: T, ...) -> R  ensures ...  { ... }
spec fn name(p: T, ...) -> bool { <specification expression> }
struct Name[T, ...] { field: T, ... }
enum Name[T, ...] { Variant(T), Variant, ... }
const NAME: u64 = <constant expression>;
```

A function with no `-> R` returns nothing. Parameters and `let` bindings are immutable unless declared `mut`.

Every program has a `main`, where it starts. `main` cannot be called (`E-TYPE`): the proof assumes it starts holding every resource the target hands it, such as the whole page pool, so a nested run would give the same pages to two owners.

**Constants** have type `u64`. The expression is folded at compile time and may use literals, other constants and arithmetic.

**Several files.** A file may begin with `import "path";` lines, before any definition. A path that starts with `.` or `/` is relative to the importing file (or absolute). Any other path is looked for next to the importing file, then in each directory above it, then in each directory given with `--lib DIR` on the command line, in order, then in each directory of the environment variable `BASTIO_LIB` (separated by `:`); the first place that has the file wins. So `import "core/text.bastio"` finds the library from anywhere inside this repository, and from elsewhere with `--lib path/to/bastiolang` or `BASTIO_LIB=path/to/bastiolang`. The walk up follows the path as written: a file named by a relative path stops at the current directory. Each imported file is loaded, with its own imports, before the rest of the importing file is read, so its constants can be used there. A file imported more than once is loaded once. Files that import each other, or an import that cannot be read, are `E-IMPORT`. Files named together on the command line also form one program, in command-line order. `--pkg NAME=DIR` makes `import "NAME/rest"` the file `rest` in `DIR`, tried before every other place; the `bastio` tool passes one for each package of a project ([Packages and projects](packages.md)).

All files share one namespace, as in C: a function, type or constant name must be unique in the whole program. There are no modules, qualified names or private items yet.

**The entry point** is `fn main`. It returns `u64` (the exit status on the hosted and user targets) or nothing, and takes as parameters the capabilities its target hands out, each at most once and each as `&mut`:

| Target | `main` may take |
| --- | --- |
| `hosted` | `&mut Serial`, `&mut Pool`, `&mut Host`, `&mut Ports`, `&mut MmioSpace` |
| `kernel` | `&mut Serial`, `&mut Pool`, `&mut Mmu`, `&mut Cpu`, `&mut Ports` (x86-64 only), `&mut MmioSpace` |
| `user` | `&mut Serial` (and must return `u64`) |

The older form `fn main(con: Serial) -> Serial` is still accepted outside user programs.

```bastio
// expect-exit: 0
fn main(con: &mut Serial) -> u64 {
  serial_write(con, 104);   // 'h'
  serial_write(con, 105);   // 'i'
  serial_write(con, 10);
  return 0;
}
```

## Types

**Plain values** are copied freely:

| Type | Meaning |
| --- | --- |
| `u64` | Unsigned 64-bit integer. Arithmetic is proved to stay in range; it never wraps |
| `u8`, `u16`, `u32` | Narrower unsigned integers (C's `uint8_t` and so on), with the same proofs. See "Sized integers" below |
| `i8`, `i16`, `i32`, `i64` | Signed integers (C's `int8_t` and so on), proved to stay in range. See "Sized integers" below |
| `bool` | `true` or `false` |
| `Str` | A string literal: immutable bytes whose length and contents are known at compile time. Never freed |
| A struct or enum with no linear part | See [Structs](#structs) and [Enums](#enums-and-match) |

**Linear values** must be used exactly once (see [Ownership](#ownership-linear-values-and-borrows)):

| Type | Meaning |
| --- | --- |
| `Buf` | An owned buffer of `u64` |
| `Bytes` | An owned buffer of bytes. Its elements are `u8`s |
| `Vec[T]` | A growable array of `T` on the heap |
| `Box[T]` | An owning pointer to a `T` on the heap; the only way a type can contain itself |
| A struct with a linear field, an enum with a linear payload | |
| `Host`, `Child` | The host system, and a child process (hosted target only) |
| `Serial`, `Pool`, `Frame`, `PageTable`, `Mmu`, `Cpu`, `Lease`, `Ports`, `PortIo`, `MmioSpace`, `Mmio`, `Dma` | Capabilities: the console, physical pages, page tables, the MMU, the CPU's user context, a processor's hold on the address space it runs on, I/O ports, memory-mapped device registers and pages lent to devices. See [Kernel capabilities](#kernel-capabilities) |

**Borrows** are allowed only as parameter types, so a borrow can never be stored, returned or outlive the call:

| Type | Meaning |
| --- | --- |
| `&T` | A read-only borrow of any `T`. A borrowed plain value (`&u64`) reads as a copy |
| `&mut T` | A mutable borrow of any `T` |

**Generic types** are written with square brackets: `Vec[u64]`, `Box[Node]`, `Option[Buf]`, `Pair[u64, bool]`.

## Ownership: linear values and borrows

A value of a linear type must be **consumed exactly once** on every path through the program: passed to a function that takes it by value, returned, stored in a struct, enum, box or vector that is itself consumed, or freed. The checker rejects:

- using a value after it has been consumed (`E-LINEAR-USE`): use after free, double free;
- reaching the end of its scope, or a `return`, while still owning it (`E-LINEAR-LEAK`): a leak;
- consuming it on one arm of an `if` or `match` and not the other (`E-LINEAR-BRANCH`).

```bastio
// expect-error: E-LINEAR-USE
fn main() -> u64 {
  let b = alloc(4);
  free(b);
  return len(b);     // b was consumed by free
}
```

```bastio
// expect-error: E-LINEAR-BRANCH
fn main() -> u64 {
  let b = alloc(4);
  if len(b) > 2 {
    free(b);
  }
  return 0;
}
```

Overwriting a linear variable that still holds a value is a leak too. A linear value cannot be consumed where the code may run more than once or not at all: in a `while` condition or on the right of `&&` or `||` (`E-LINEAR-USE`). A loop body must leave every outer linear variable as it found it, owned or consumed (`E-LINEAR-BRANCH`).

**Shared borrows are implicit.** Passing an owned value to a `&T` parameter lends it for the duration of the call; the caller still owns it afterwards. There is no `&x` expression.

**Mutable borrows are explicit.** A `&mut T` parameter is lent with `&mut x`, and `x` must be declared `mut` (or be a `&mut` parameter itself). The callee may read and assign the borrowed value, even move out of it, but it must have put a value back by the time it returns.

```bastio
// expect-exit: 8
fn bump(n: &mut u64)
  requires n < 100
  ensures n == old(n) + 1
{
  n = n + 1;
}

fn replace(b: &mut Buf) -> u64
  ensures result == len(old(b)) && len(b) == 3
{
  let old_len = len(b);
  free(b);              // move out of the borrow ...
  b = alloc(3);         // ... and put a new value back before returning
  return old_len;
}

fn main() -> u64 {
  let mut x = 5;
  bump(&mut x);
  let mut b = alloc(9);
  let n = replace(&mut b);   // 9
  let m = len(b);            // 3
  free(b);
  return x + n - m - 4;      // 6 + 9 - 3 - 4
}
```

One call cannot lend the same place twice if one of the loans is `&mut`, and it cannot both borrow and move the same value (`E-LINEAR-USE`), so a callee never sees two names for memory it may change.

```bastio
// expect-error: E-LINEAR-USE
fn copy_into(dst: &mut Buf, src: &Buf) {}

fn main() -> u64 {
  let mut b = alloc(4);
  copy_into(&mut b, b);   // b lent twice, once mutably
  free(b);
  return 0;
}
```

**Names.** A name cannot shadow another that is still in scope (`E-TYPE`). Once a block ends its names can be used again, so two `match` arms can both bind `v`. Functions (including builtins and spec fns) are named apart from variables: a call `f(..)` always means the function, so a local may be called `len` or `step` even where a function `step` exists. Where a function is passed by name (`locked(l, f)`, `smp_run(f, ..)`), a local of the same name in scope is `E-TYPE`.

## Functions and contracts

```text
fn name(p: T, ...) -> R
  requires <bool specification>     // the caller must prove it
  ensures  <bool specification>     // proved at every return
{ ... }
```

A function may have any number of `requires` and `ensures` clauses. In `ensures`, `result` is the returned value, a by-value parameter means its value on entry, a `&mut` parameter means the value handed back to the caller, and `old(x)` means a `&mut` parameter's value at the call.

Verification is **modular**: a call is checked against the callee's `requires`, and afterwards the caller knows only the callee's `ensures`, never what its body does. A function without an `ensures` therefore tells its callers nothing about its result except its type.

```bastio
// expect-error: E-VERIFY-REQUIRES
fn half(x: u64) -> u64
  requires x % 2 == 0
  ensures result + result == x
{
  return x / 2;
}

fn main() -> u64 {
  return half(7);
}
```

Functions may be recursive. Termination is not checked (see [Soundness scope](#soundness-scope)).

## Statements and expressions

```text
let x = e;          let mut x: T = e;        x = e;       p.field = e;
if c { ... } else if c2 { ... } else { ... }
while c invariant <spec> invariant <spec> { ... }
match e { E::V(x) => { ... }, _ => { ... } }
return e;           return;
assert <spec>;      unreachable;
f(a, b);            // a call whose result is not used
```

- `if` and `while` conditions are `bool`; braces are required, parentheses are not.
- `let` may give a type (`let mut v: u64 = 0;`); otherwise it is the type of the initializer.
- `assert e;` is proved, not checked at run time (`E-VERIFY-ASSERT`); it generates no code.
- `unreachable;` marks a point the prover must show no execution reaches (`E-VERIFY-UNREACHABLE`). The path ends there like a `return`, so linear values in hand need not be consumed. It compiles to `__builtin_unreachable()`, with no check. Its typical use is a `match` arm the specification rules out.
- A loop's `invariant` clauses must hold on entry and be preserved by one iteration (`E-VERIFY-INVARIANT`). After the loop, the variables the body assigns are known only through the invariants and the negated condition. In an invariant, `old(x)` is a parameter's value at the call, which is how a loop says what it leaves alone; it can name only parameters (`E-TYPE`), since locals have no value at the call.

```bastio
// expect-exit: 7
fn append(t: &mut Vec[u64], n: u64)
  requires n <= 100
  ensures vlen(t) == vlen(old(t)) + n
  ensures forall k: u64 :: k < vlen(old(t)) ==> vget(t, k) == vget(old(t), k)
{
  let mut i = 0;
  while i < n
    invariant i <= n && vlen(t) == vlen(old(t)) + i
    invariant forall k: u64 :: k < vlen(old(t)) ==> vget(t, k) == vget(old(t), k)
  {
    vec_push(&mut t, 0);
    i = i + 1;
  }
}

fn main() -> u64 {
  let mut v = vec_new[u64]();
  vec_push(&mut v, 7);
  append(&mut v, 3);
  let first = vec_get(v, 0);     // proved to be 7: append keeps what was there
  vec_free(v);
  return first;
}
```

**Operators**, loosest first:

| Precedence | Operators | Notes |
| --- | --- | --- |
| 1 | `==>` | Implication; specifications only. Right-associative |
| 2 | `\|\|` | Short-circuit |
| 3 | `&&` | Short-circuit |
| 4 | `== != < <= > >=`, `x is E::V` | Do not chain: `a < b < c` is a syntax error |
| 5 | `\|` | Bitwise or; unsigned integers, not in specifications |
| 6 | `^` | Bitwise exclusive or |
| 7 | `&` | Bitwise and (a prefix `&mut` is a borrow) |
| 8 | `<< >>` | Shifts |
| 9 | `+ -` | |
| 10 | `* / %` | |
| 11 | `! -` | Not (bitwise on unsigned integers), and negation (signed integers) |
| 12 | `.field` | Also `.Variant` on enums, in specifications |

Arithmetic is proved safe instead of wrapping or trapping: `+` and `*` must not exceed 2^64 − 1 (`E-VERIFY-OVERFLOW`), `a - b` needs `a >= b` (`E-VERIFY-UNDERFLOW`), and `/` and `%` need a nonzero divisor (`E-VERIFY-DIVZERO`). Facts the code has already tested are used by the proof:

```bastio
// expect-exit: 3
fn avg(a: u64, b: u64, n: u64) -> u64 {
  if n == 0 {
    return 0;
  }
  if a > 1000000 || b > 1000000 {
    return 0;
  }
  return (a + b) / n;      // proved: no overflow, no division by zero
}

fn main() -> u64 {
  return avg(4, 5, 3);
}
```

```bastio
// expect-error: E-VERIFY-UNDERFLOW
fn dist(a: u64, b: u64) -> u64 {
  return a - b;            // fails when b > a
}

fn main() -> u64 {
  return dist(2, 5);
}
```

Call arguments, the operands of an operator and the fields of a struct literal are evaluated left to right, and the proof assumes that order. (One exception: a linear value passed by name or lent as `&mut` is read after the other arguments have run, so `f(b, g(&mut b))` passes `b` as `g` left it.) C leaves this order unspecified, so when an operand has a side effect the generated C evaluates each operand into a temporary first; without side effects it emits the expression as written.

**Sized integers.** `u8`, `u16` and `u32` follow the same rules at their own width: `+` and `*` must stay below 2^8, 2^16 or 2^32. Both operands of an arithmetic operator or a comparison have one integer type; there is no implicit promotion. A literal takes the integer type its position asks for (a typed `let`, an assignment, a `return`, a parameter, a field, a payload, the other operand of an operator) and is `u64` when nothing says otherwise; it must fit (`E-TYPE`). Conversions are written out:
- `cast[T](x)` keeps the value and is proved to fit in `T` (`E-VERIFY-CAST`). Widening always fits, so it needs no annotation.
- `wrap[T](x)` reduces `x` modulo 2^width of `T`, for code that means to drop bits.
- `wrap_add(a, b)`, `wrap_sub(a, b)` and `wrap_mul(a, b)` are modular arithmetic on two operands of one type, for checksums and hashes.
- A builtin's `u64` parameter (an index, a length, a value) accepts any narrower unsigned integer, since widening is free.

```bastio
// expect-exit: 42
fn hex_digit(n: u64) -> u8
  requires n < 16
{
  if n < 10 {
    return cast[u8](n) + 48;     // '0' + n: proved to fit in u8
  }
  return cast[u8](n) + 87;       // 'a' + n - 10
}

fn main() -> u64 {
  let x: u32 = 70000;
  let low: u16 = wrap[u16](x);   // 70000 mod 65536 = 4464
  let s: u8 = wrap_add(hex_digit(15), 200);   // 102 + 200 mod 256 = 46
  if s != 46 || low != 4464 {
    return 1;
  }
  return 42;
}
```

```bastio
// expect-error: E-VERIFY-CAST
fn low_byte(n: u64) -> u8 {
  return cast[u8](n);            // fails: n may be 256 or more; `wrap[u8](n)` drops the bits instead
}

fn main() -> u64 {
  return cast[u64](low_byte(300));
}
```

**Bitwise operators.** `&`, `|`, `^`, `<<`, `>>` and `!x` (bitwise not; the same `!` as for `bool`) work on unsigned integers. Both operands of `&`, `|` and `^` have one type; a shift amount can be any unsigned type and must be proved less than the width (`E-VERIFY-SHIFT`), since C leaves larger shifts undefined. A left shift follows the never-wraps rule: it must lose no bits (`E-VERIFY-OVERFLOW`); mask first to drop them. They bind tighter than comparisons, as in Rust, so `e & 1 == 1` tests a bit. The prover knows a low mask (`x & 255` is `x % 256`) and shifts exactly, and only bounds for the value of other masks, `|` and `^` (`x & y <= x`, `x | y >= x`, ...); what they do to each bit is stated with `has_bit` (see Specifications). They are not available in specifications: write `% 2^k` and `/ 2^k`, or `has_bit`, there.

```bastio
// expect-exit: 42
fn le16(lo: u8, hi: u8) -> u16 {
  return (cast[u16](hi) << 8) | cast[u16](lo);
}

fn main() -> u64 {
  let w = le16(52, 18);              // 0x1234
  let lo: u8 = cast[u8](w & 255);    // proved to fit: a low mask is `% 256`
  let hi: u8 = cast[u8](w >> 8);
  if w != 4660 || lo != 52 || hi != 18 || w & 1 == 1 {
    return 1;
  }
  return 42;
}
```

```bastio
// expect-error: E-VERIFY-SHIFT
fn bit(n: u64) -> u64 {
  return 1 << n;                     // fails: n may be 64 or more
}

fn main() -> u64 {
  return bit(3);
}
```

**Signed integers.** `i8`, `i16`, `i32` and `i64` are proved to stay between their minimum and maximum: an operation that could go below the minimum is `E-VERIFY-UNDERFLOW`, above the maximum `E-VERIFY-OVERFLOW`. Unary minus `-x` works on signed types only (`E-TYPE` on unsigned ones) and is proved not to be applied to the minimum, whose negation does not fit. Division truncates toward zero and `%` takes the sign of the dividend, as in C; `MIN / -1` is `E-VERIFY-OVERFLOW`. `cast[u64](x)` on a signed `x` is proved non-negative; builtins that take `u64` do not accept a signed value without such a conversion. An expression made only of literals is `u64`, so a negative number needs a signed position (`let n: i8 = -7;`).

```bastio
// expect-exit: 42
fn abs(x: i32) -> i32
  requires x != -2147483648
  ensures result >= 0 && (result == x || result == -x)
{
  if x < 0 {
    return -x;
  }
  return x;
}

fn main() -> u64 {
  let n: i8 = -7;
  let q: i8 = n / 2;             // -3: truncates toward zero
  let r: i8 = n % 2;             // -1: sign of the dividend
  if q != -3 || r != -1 {
    return 1;
  }
  return cast[u64](abs(-40)) + 2;
}
```

```bastio
// expect-error: E-VERIFY-OVERFLOW
fn negate(x: i8) -> i8 {
  return -x;                     // fails: x may be -128, and 128 does not fit in i8
}

fn main() -> u64 {
  return cast[u64](negate(-5));
}
```

## Structs

```bastio
// expect-exit: 42
struct Point { x: u64, y: u64 }                 // plain: copied
struct Stack { items: Buf, n: u64 }             // linear: owns a buffer

fn push(s: &mut Stack, v: u64)
  requires s.n < len(s.items)
  ensures s.n == old(s).n + 1 && len(s.items) == len(old(s).items)
  ensures get(s.items, old(s).n) == v
  ensures forall i: u64 :: i < old(s).n ==> get(s.items, i) == get(old(s).items, i)
{
  s.items = set(s.items, s.n, v);
  s.n = s.n + 1;
}

fn main() -> u64 {
  let p = Point { y: 2, x: 40 };               // fields in any order
  let mut s = Stack { items: alloc(8), n: 0 };
  push(&mut s, p.x);
  push(&mut s, p.y);
  let total = get(s.items, 0) + get(s.items, 1);
  free(s.items);                               // consumes the whole struct
  return total;
}
```

- A struct has at least one field, and a field cannot be a borrow. A struct cannot contain itself, except through a `Box`.
- A struct with a linear field is **linear**. A struct without one is a plain value and is copied.
- A struct literal lists every field exactly once, in any order.
- Ownership is tracked **per place** (`p`, `p.space`, `p.space.root`). Moving a linear field out leaves the rest of the struct usable: its scalar fields can still be read and its other fields used. The struct as a whole cannot be moved, borrowed or returned until the field is put back (`E-LINEAR-USE`). A struct counts as consumed once every linear field in it has been moved out, which is why `free(s.items)` above consumes `s`.
- Assigning a linear field that still holds a value is `E-LINEAR-LEAK`, as for variables.
- Fields are places: they can be lent as `&mut` (`push(&mut p.stack, 1)`) or passed to `&T` parameters. A place cannot be borrowed in a call that also lends an overlapping place as `&mut` (`p` and `p.items` overlap).
- A `&mut` struct parameter must be whole again when the function returns.
- Specifications can read fields, including through `old(..)` and `result`: `ensures result.n == old(s).n + 1`. The prover tracks every field, so bounds, overflow and capability state work through fields exactly as through variables.
- In generated code a struct is a C struct passed by value; nothing at run time records which fields have been moved.

### Locks and struct invariants

A `Lock[T]` holds a value that code on several processors can update through a shared borrow `&Lock[T]`. The contents are reachable only inside `locked`, which takes the lock, calls a function on them and releases it.

```bastio
// expect-exit: 3
struct Count { n: u64 } invariant n < 1000;

fn bump(c: &mut Count) requires c.n < 1000 ensures c.n < 1000 {
  if c.n < 999 { c.n = c.n + 1; }
}

fn main() -> u64 {
  let l = lock_new(Count { n: 0 });            // proves the invariant
  locked(l, bump);                             // bump(&mut contents) under the lock
  locked(l, bump);
  locked(l, bump);
  let c = lock_end(l);                         // frees the lock, returns the Count
  return c.n;
}
```

- A struct may end with `invariant e` clauses (then `;`): specifications over its fields.
- `lock_new(v)` proves the invariants of `v` (`E-VERIFY-LOCKINV`) and moves it into a new lock. `lock_end(l)` consumes the lock and returns the value; the invariants are known of it.
- `locked(l, f, args..)` calls `f(&mut contents, args..)` with the lock held. `f` must take `&mut T` first; its result is `locked`'s result. The contents are a fresh value of which only the invariants are known, since another processor may have changed them, so `f`'s `requires` must follow from the invariants. After the call the invariants are proved again (`E-VERIFY-LOCKINV`).
- The invariants of a struct held directly as a field are included; ones behind a box, in a vector or in an enum payload are not yet. Outside a lock, invariants are not enforced: they are what the lock promises about its contents.
- A lock cannot hold a value with a `Cpu`, `Mmu` or `Lease` in it (`E-PIN`): they describe the processor that holds them.
- Lending `&mut` a place that overlaps the lock in the same call is `E-LINEAR-USE`, so the contents have no second way in.
- Deadlock through locks is ruled out at compile time, with nothing written down (`E-LOCK-ORDER`). A lock's *class* is the type of its contents. The checker finds, over the whole call graph, which classes each function may take (`smp_run` counts as a call of its function). While `locked(l, f)` holds a lock of class C, `f` and everything it calls may take only locks of other classes, and the classes must form no cycle: if a `B` lock is ever taken while an `A` lock is held, an `A` lock is never taken while a `B` lock is held. Every processor then takes locks in one global order, so none can wait for another forever, and none spins on a lock it already holds. Two locks with the same contents type cannot be nested; give them different contents types (two structs) to order them. Nothing is generated for this.
- Generated code: a lock is a pointer to a lock word followed by the contents; `locked` spins with an atomic exchange (acquire) and releases with a store (release). Invariants generate nothing.

### Atomics

An `Atomic[T]` holds one word that any number of processors read and write at once through a shared borrow `&Atomic[T]`, with no lock. `T` is an integer type, `bool`, or a struct with a single field of one of those, which is how an atomic value gets an invariant (`E-TYPE` otherwise).

```bastio
// expect-exit: 4
struct Pending { n: u64 } invariant n <= 1024;

fn reserve(a: &Atomic[Pending]) -> bool {
  let x = atomic_load(a);                      // x.n <= 1024 is known
  if x.n < 1024 {
    return atomic_cas(a, x, Pending { n: x.n + 1 });   // proves x.n + 1 <= 1024
  }
  return false;
}

fn main() -> u64 {
  let a = atomic_new(Pending { n: 1020 });
  let mut i = 0;
  while i < 6 invariant i <= 6 {
    let ok = reserve(a);
    i = i + 1;
  }
  let p = atomic_end(a);
  return p.n % 1000 / 5;
}
```

| Builtin | Effect |
| --- | --- |
| `atomic_new(v: T) -> Atomic[T]` | A new atomic holding `v`; proves the invariants of `v` |
| `atomic_end(a: Atomic[T]) -> T` | Consumes the atomic and returns its value |
| `atomic_load(a: &Atomic[T]) -> T` | Reads the value |
| `atomic_store(a: &Atomic[T], v: T)` | Writes `v`; proves its invariants |
| `atomic_swap(a: &Atomic[T], v: T) -> T` | Writes `v` and returns the value it replaced; proves the invariants of `v` |
| `atomic_cas(a: &Atomic[T], expected: T, v: T) -> bool` | Writes `v` if the value is `expected` (compared bit for bit), and says whether it did; proves the invariants of `v` |
| `atomic_wrap_add(a: &Atomic[T], d: T) -> T` | Integer `T` only: adds `d`, wrapping around (as `wrap_add`), and returns the value before |

- Another processor may change the value at any moment, so a value read (`atomic_load`, `atomic_swap`, `atomic_wrap_add`, `atomic_end`) is fresh: only the invariants are known of it. Every value written must satisfy them (`E-VERIFY-LOCKINV`).
- There is no checked `atomic_add`: since the current value is unknown, it would have to be proved not to overflow for every value the invariants allow. Use `atomic_wrap_add` when wrapping is wanted, or a compare-and-swap loop like `reserve` above.
- Every access is sequentially consistent, so a program behaves as if the processors' steps were interleaved one at a time. Weaker orderings are not available (see [concurrency.md](concurrency.md)).
- An atomic is linear: it must be consumed by `atomic_end` (`E-LINEAR-LEAK`). It cannot hand over ownership: it never holds a linear value.
- Generated code: an atomic is a pointer to the word on the heap; the operations are the C compiler's `__atomic` builtins with `__ATOMIC_SEQ_CST`. Invariants generate nothing.

## Enums and `match`

```bastio
// expect-exit: 21
enum Opt  { Some(u64), None }
enum Slot { Full(Buf), Vacant }        // linear: a variant owns a buffer

fn checked_div(a: u64, b: u64) -> Opt
  ensures (b == 0) == (result is Opt::None)
  ensures result is Opt::Some ==> result.Some <= a
{
  if b == 0 {
    return Opt::None;
  }
  return Opt::Some(a / b);
}

fn main() -> u64 {
  let mut r = 0;
  match checked_div(40, 2) {
    Opt::Some(v) => { r = v; },        // proved: v <= 40
    Opt::None => {},
  }
  let s = Slot::Full(alloc(1));
  if s is Slot::Full {                 // tests the tag; consumes nothing
    r = r + 1;
  }
  match s {
    Slot::Full(b) => { free(b); },     // the arm owns b and must consume it
    Slot::Vacant => {},
  }
  return r;
}
```

- A variant has no payload or exactly one (use a struct for several values). An enum cannot contain itself, except through a `Box`.
- Variants are written `Enum::Variant`, with a payload `Enum::Variant(e)`.
- An enum with a linear payload is **linear**: it is consumed exactly once, and dropping it is `E-LINEAR-LEAK`.
- `match` must be **exhaustive**: every variant is handled once, or a final `_` arm takes the rest. A missing variant, a variant matched twice, or an arm after `_` is `E-MATCH`.
- Matching an owned value moves it into the arms. A pattern `E::V(x)` binds the payload to `x` (`E::V(mut x)` to change it), owned by the arm: a linear payload must be consumed there. `E::V(_)` and `_` drop the payload, which is `E-LINEAR-LEAK` for a linear one.
- Matching a borrowed place (a `&E` parameter, or a payload bound out of one) only inspects it. A linear payload is then bound as a shared borrow: it can be read, lent to `&T` parameters and matched again, but not moved (`E-LINEAR-USE`) or declared `mut`. Through a `&mut E` parameter the match moves the value out, and the function must put a value back.
- The arms must agree on what they consume (`E-LINEAR-BRANCH`), like the arms of an `if`. Arms that do not return fall through to the code after the `match`.
- `x is E::V` is a `bool` that reads only the tag. Its operand must be a place or a non-linear value.
- In specifications, `o.V` is the payload of variant `V`. It is meaningful only when `o is E::V`, so write it under an implication, as in `checked_div` above. Inside an arm `E::V(v) => ...` the prover knows that `v` is `o.V`.
- Enum values cannot be built in a specification.
- In generated code an enum is a C struct holding a tag and a union of the payloads. A `match` is an `if` chain on the tag; the last arm needs no test because the match is exhaustive.

```bastio
// expect-error: E-MATCH
enum Color { Red, Green, Blue }

fn main() -> u64 {
  let c = Color::Green;
  match c {
    Color::Red => { return 1; },
    Color::Green => { return 2; },
  }
  return 0;
}
```

## Boxes

```bastio
// expect-exit: 6
struct Node { l: Tree, r: Tree }
enum Tree { Leaf(u64), Branch(Box[Node]) }    // recursive through the box

fn sum(t: &Tree) -> u64 {
  match t {
    Tree::Leaf(v) => {
      if v > 1000 { return 1000; }
      return v;
    },
    Tree::Branch(n) => {                      // n is a borrow
      let a = sum(n.l) % 1000000;
      let b = sum(n.r) % 1000000;
      return a + b;
    },
  }
}

fn drop_tree(t: Tree) {
  match t {
    Tree::Leaf(_) => {},
    Tree::Branch(b) => {
      let n = unbox(b);                       // frees the box, returns the Node
      drop_tree(n.l);
      drop_tree(n.r);
    },
  }
}

fn main() -> u64 {
  let inner = Tree::Branch(box(Node { l: Tree::Leaf(2), r: Tree::Leaf(3) }));
  let t = Tree::Branch(box(Node { l: Tree::Leaf(1), r: inner }));
  let s = sum(t);
  drop_tree(t);
  return s;
}
```

- `box(e)` moves `e` into a new heap allocation; `unbox(b)` consumes the box, frees the allocation and returns the value.
- A box is linear whatever it holds: `box(7)` must still be unboxed.
- Fields of a boxed struct are places, as if the box were not there: `b.n`, `b.items = ..`, `&mut b.l`, `b.l` lent to a `&T` parameter. Ownership is tracked through the box as through a struct. The box itself is a separate resource: moving every field out does not consume it, and it cannot be unboxed, moved or borrowed whole while a field is out.
- Specifications see through boxes the same way: `ensures result.n == 0` for a `Box[Stack]` result.
- The prover unfolds a box that arrives from outside (a parameter, a function's result) two levels deep. Deeper contents are unknown until code looks inside them, and then nothing is known about them: a proof that needs a fact deeper down fails (it is never assumed). That is why `sum` above clamps what it gets back instead of proving a bound for the whole tree. Values a function builds itself are tracked to any depth.
- `box` uses the same allocator as `alloc`. Running out of memory aborts, as for `alloc`.
- `try_box[T](e)` is the box that reports running out of memory instead. It needs `import "core/boxed.bastio"`, which defines what it returns: `enum Boxed[T] { Ok(Box[T]), Full(T) }`. `Boxed::Ok` holds the new box, and `Boxed::Full` gives `e` back when there was no memory for it, so the value is never lost. The prover does not know which one comes back, so the program must handle both:

```bastio
// expect-exit: 5
// A program imports this from core/boxed.bastio; written out here, it is
// the same enum.
enum Boxed[T] { Ok(Box[T]), Full(T) }

fn main() -> u64 {
  let r = try_box[u64](5);
  match r {
    Boxed::Ok(b) => { return unbox(b); },
    Boxed::Full(x) => { return x; },
  }
}
```
- In generated code a `Box[T]` is a `T *`.

## Generics

```bastio
// expect-exit: 5
enum Option[T] { Some(T), None }
struct Pair[A, B] { a: A, b: B }

fn swap[A, B](p: Pair[A, B]) -> Pair[B, A] {
  return Pair[B, A] { a: p.b, b: p.a };
}

fn or_else[T](o: Option[T], d: T) -> T
  ensures o is Option::Some ==> result == o.Some
{
  match o {
    Option::Some(v) => { return v; },          // patterns may omit [T]
    Option::None => { return d; },
  }
}

fn main() -> u64 {
  let p = swap[u64, Buf](Pair[u64, Buf] { a: 5, b: alloc(4) });
  let n = len(p.a);                            // p.a is the Buf now
  free(p.a);
  if n != 4 {
    return 1;
  }
  return or_else[u64](Option[u64]::Some(p.b), 0);   // 5
}
```

- Structs, enums and functions can take type parameters. Generics are **monomorphized**: each definition is copied once per list of type arguments it is used with, and the copies are what is checked, verified and compiled. There is no runtime cost and no boxing.
- Type arguments are written at every use: calls (`swap[u64, Buf](..)`), struct literals and variants (`Option[u64]::None`). They are not inferred. Patterns and `is` tests may leave them out, since the matched value's type has them.
- A type parameter stands for any type, linear or not. Each copy is checked on its own, so a generic function that copies or drops its argument works for `u64` and is rejected for `Buf`, with a note naming the instance (like C++ templates, unlike Rust traits). There are no bounds or traits.
- Specifications in a generic function are verified for every copy.
- A definition that instantiates itself with ever larger arguments (`struct S[T] { next: Box[S[Box[T]]] }`) is rejected.
- `main` cannot be generic.

```bastio
// expect-error: E-LINEAR-LEAK
fn first[T](a: T, b: T) -> T {
  return a;                // drops b: fine for u64, a leak for Buf
}

fn main() -> u64 {
  let x = first[u64](1, 2);
  let b = first[Buf](alloc(1), alloc(2));
  free(b);
  return x;
}
```

## Vectors

`Vec[T]` is built in: a growable array whose slots below the length always hold a value. Bounds, and emptiness before freeing a vector of linear values, are proved.

```bastio
// expect-exit: 10
fn fill(v: &mut Vec[u64], n: u64)
  requires vlen(v) == 0 && n <= 1000
  ensures vlen(v) == n
  ensures forall i: u64 :: i < n ==> vget(v, i) == i
{
  let mut i = 0;
  while i < n
    invariant i <= n && vlen(v) == i
    invariant forall k: u64 :: k < i ==> vget(v, k) == k
  {
    vec_push(&mut v, i);
    i = i + 1;
  }
}

fn main() -> u64 {
  let mut v = vec_new[u64]();
  fill(&mut v, 5);
  let x = vec_get(v, 4);                  // proved: 4 < vlen(v), and x == 4
  vec_free(v);

  let mut bufs = vec_new[Buf]();          // linear elements
  vec_push(&mut bufs, alloc(2));
  let prev = vec_swap(&mut bufs, 0, alloc(6));  // the old element comes back
  let n = len(prev);
  free(prev);
  let b = vec_pop(&mut bufs);
  let m = len(b);
  free(b);
  vec_free(bufs);                         // proved: vlen(bufs) == 0
  return x + n + m - 2;
}
```

| Builtin | Requires (proved) | Effect |
| --- | --- | --- |
| `vec_new[T]() -> Vec[T]` | | An empty vector |
| `vec_len(v: &Vec[T]) -> u64` | | Its length (`vlen(v)` in specifications) |
| `vec_push(v: &mut Vec[T], x: T)` | | Appends `x`, growing the storage when full |
| `vec_try_reserve(v: &mut Vec[T], n: u64) -> bool` | | Makes room for `n` more elements without a push having to allocate, and returns `true`; or returns `false`, with `v` unchanged, when memory runs out. On `true`, `vcap(v) >= vlen(v) + n` |
| `vec_push_within(v: &mut Vec[T], x: T)` | `vlen(v) < vcap(v)` | Appends `x` into room already reserved: never allocates, so it cannot stop the program |
| `vec_pop(v: &mut Vec[T]) -> T` | `vlen(v) > 0` | Removes and returns the last element |
| `vec_get(v: &Vec[T], i: u64) -> T` | `i < vlen(v)`; `T` not linear | A copy of element `i` |
| `vec_set(v: &mut Vec[T], i: u64, x: T)` | `i < vlen(v)`; `T` not linear | Overwrites element `i` |
| `vec_swap(v: &mut Vec[T], i: u64, x: T) -> T` | `i < vlen(v)` | Puts `x` at `i` and returns what was there |
| `vec_free(v: Vec[T])` | for linear `T`, `vlen(v) == 0` (`E-VERIFY-LEAK`) | Frees the storage |
| `vec_with(v: &Vec[T], i: u64, f, args..) -> R` | `i < vlen(v)` | Calls `f(&element, args..)` with element `i` left where it is, and returns what `f` returns (stage1 only). `f` is the name of a function whose first parameter is `&T` and whose others take `args..`; the arguments after `f` may not call anything (the element is lent once they are evaluated), and `v` cannot be lent as `&mut` in the same call. The prover knows the element's value, so `f`'s contract applies to `vget(v, i)` |

- Elements of a vector of linear values leave only through `vec_pop` and `vec_swap`, and `vec_with` lets a function read one in place without taking it out and putting it back (`f` gets a shared borrow of it, so it can look at the element and lend it on but not move it), and the vector can be freed only when nothing is left, so no per-slot ownership is needed.
- In specifications, `vlen(v)` is the length and `vget(v, i)` is element `i`, of the element type, for any element type. So `vget(v, i).page`, `len(vget(v, i).page)`, `vget(v, i) is State::Used` and `vlen(vget(grid, i))` all work, typically under `forall i: u64 :: i < vlen(v) ==> ...`.
- The prover tracks every element's contents: scalars, struct fields, enum tags and payloads, buffer lengths and contents, ghost state of capabilities, nested vectors. A read gives back what was last written at that index. The exception is boxes inside elements: what a box in an element holds is unknown once the element is in the vector.
- The storage doubles when full. A push that cannot get memory aborts; the prover takes this into account by assuming the length stays below 2^60 after a push.
- Code that must not abort when memory runs out reserves first and then pushes within the room:

```bastio
// expect-exit: 3
fn fill(v: &mut Vec[u64], n: u64) -> bool
  requires vlen(v) == 0 && n <= 1000
  ensures result ==> vlen(v) == n
  ensures !result ==> vlen(v) == 0
{
  if !vec_try_reserve(&mut v, n) {
    return false;                      // out of memory: nothing was lost
  }
  let mut i = 0;
  while i < n
    invariant i <= n && vlen(v) == i && vcap(v) >= n
  {
    vec_push_within(&mut v, i);        // proved: vlen(v) < vcap(v)
    i = i + 1;
  }
  return true;
}

fn main() -> u64 {
  let mut v = vec_new[u64]();
  let mut n = 0;
  if fill(&mut v, 3) {
    n = vec_len(v);
  }
  while vec_len(v) > 0
    invariant true
  {
    let x = vec_pop(&mut v);
  }
  vec_free(v);
  return n;
}
```

`vcap(v)` is the capacity, in specifications. The prover tracks it through `vec_push` (which keeps it while there is room), `vec_pop`, `vec_set` and `vec_swap` (which keep it), loops and calls. What it knows across a call is what the callee's contract says. The capacity of a vector stored inside another vector is not tracked. Room is reserved before the element exists, so a vector of linear values works the same way: nothing has to be given back when there is no room.
- In generated code a `Vec[T]` is `{ len, cap, data }` and element access is a plain array access, with no bounds check.

## Buffers, bytes and strings

| Builtin | Requires (proved) | Effect |
| --- | --- | --- |
| `alloc(n: u64) -> Buf` | `n <= 2^60` | A buffer of `n` zeros |
| `len(b: &Buf) -> u64` | | Its length |
| `get(b: &Buf, i: u64) -> u64` | `i < len(b)` | Element `i` |
| `set(b: Buf, i: u64, v: u64) -> Buf` | `i < len(b)` | Consumes `b` and returns it with element `i` set to `v`, updated in place |
| `free(b: Buf)` | | Frees `b` |
| `balloc(n: u64) -> Bytes` | `n <= 2^60` | `n` zero bytes |
| `try_alloc(n: u64) -> Buf` | `n <= 2^60` | Like `alloc`, but when memory runs out it returns an empty buffer instead of stopping the program. The prover knows only that the length is `n` or `0`, so a program must test `len` before indexing |
| `try_balloc(n: u64) -> Bytes` | `n <= 2^60` | Like `balloc`, but returns empty `Bytes` when memory runs out |
| `blen(b: &Bytes) -> u64` | | Its length |
| `bget(b: &Bytes, i: u64) -> u8` | `i < blen(b)` | Byte `i` |
| `bset(b: &mut Bytes, i: u64, v: u8)` | `i < blen(b)` | Sets byte `i` in place |
| `bcopy(dst: &mut Bytes, d: u64, src: &Bytes, s: u64, n: u64)` | `d + n <= blen(dst)` and `s + n <= blen(src)` | Copies bytes `s .. s+n` of `src` over bytes `d .. d+n` of `dst` in one go. The prover knows the result: byte `d + j` of `dst` is byte `s + j` of `src`, and the rest of `dst` is unchanged. `dst` and `src` are always different buffers (one buffer cannot be lent as `&mut` and `&` at once) |
| `bmove(b: &mut Bytes, d: u64, s: u64, n: u64)` | `d + n <= blen(b)` and `s + n <= blen(b)` | The same within one buffer; the ranges may overlap (as if bytes `s .. s+n` were copied out first) |
| `bfree(b: Bytes)` | | Frees `b` |
| `bfrom(s: Str) -> Bytes` | | A copy of a string literal; the prover knows its bytes |
| `slen(s: Str) -> u64` | | Length of a string literal |
| `sget(s: Str, i: u64) -> u8` | `i < slen(s)` | Byte `i`; the prover knows the literal's bytes exactly |
| `print(v: u64)` | | Prints `v` in decimal on a line of its own (for debugging) |

`include_bytes("path")` is a `Str` holding a file's contents, read at compile time; the path is relative to the source file.

```bastio
// expect-exit: 5
fn count(s: &Bytes, c: u8) -> u64
  ensures result <= blen(s)
{
  let mut n = 0;
  let mut i = 0;
  while i < blen(s)
    invariant i <= blen(s) && n <= i
  {
    if bget(s, i) == c {
      n = n + 1;
    }
    i = i + 1;
  }
  return n;
}

fn main() -> u64 {
  let mut b = bfrom("hello, world");
  bset(&mut b, 0, sget("L", 0));          // "Lello, world"
  let n = count(b, 108) + count(b, 76);   // 'l' and 'L'
  bfree(b);
  assert slen("hello") == 5;              // literals are known exactly
  return n + 1;
}
```

## The host: files and processes

On the hosted target, `main` can ask for `&mut Host`: the program's command line and file system. A `Child` is a running child process with a pipe to its standard input and one from its standard output; it is linear, so it must be waited for.

| Builtin | Effect |
| --- | --- |
| `host_argc(h: &Host) -> u64` | Number of command-line arguments, including the program name |
| `host_arg(h: &Host, i: u64) -> Bytes` | Argument `i` (empty if there is none) |
| `host_read(h: &Host, path: &Bytes, out: &mut Bytes) -> bool` | Replaces `out` with the whole file; false (and `out` unchanged) if it cannot be read |
| `host_write(h: &Host, path: &Bytes, data: &Bytes) -> bool` | Creates or replaces the file |
| `host_eprint(h: &Host, text: &Bytes)` | Writes the bytes to standard error |
| `host_env(h: &Host, name: Str, out: &mut Bytes) -> bool` | Replaces `out` with the value of the environment variable `name`; false (and `out` unchanged) if it is not set |
| `host_cpus(h: &Host) -> u64` | Number of processors online (at least 1, which the prover does not know) |
| `host_net(h: &mut Host, op: u64, a: u64, b: u64, buf: &mut Bytes) -> u64` | TCP sockets, one call with an operation number (use `std/net.bastio`, which wraps them). `0` listen on IPv4 address `a` (host order), port `b`; `1` accept on listener `a`; `2` connect to address `a`, port `b`; `3` read up to `b` bytes from `a` into `buf`; `4` send `buf` on `a`, returning the count; `5` close `a`; `6` port of listener `a`; `7` the time in seconds since 1970; `8` replace `buf` with `b` random bytes (at most 256) from the system's source (`std/net.bastio` has `net_now` and `net_random`). A failure is 2^64 - 1; every socket call fails in WebAssembly (the time and random bytes work there) (on Windows the sockets are Winsock, loaded on the first call) |
| `host_spawn(h: &Host, argv: &Vec[Bytes]) -> Child` | Starts `argv[0]` (looked up in `PATH`) with arguments `argv`. A program that cannot be started gives a child that exits with 127 and writes nothing |
| `child_write(c: &mut Child, data: &Bytes) -> bool` | Writes to the child's standard input; false if it can no longer be written |
| `child_close(c: &mut Child)` | Closes the child's standard input, so a program that reads to the end of its input can finish |
| `child_read_line(c: &mut Child, out: &mut Bytes) -> bool` | Replaces `out` with the next line of output, without the newline; false at the end of the output |
| `child_wait(c: Child) -> u64` | Closes the pipes, waits for the child and returns its exit status (128 + the signal number if a signal ended it); the prover knows it is `<= 255` |

```bastio
// expect-exit: 0
// Runs `echo hi` and checks its output.
fn main(host: &mut Host) -> u64 {
  let mut argv = vec_new[Bytes]();
  vec_push(&mut argv, bfrom("echo"));
  vec_push(&mut argv, bfrom("hi"));
  let mut child = host_spawn(host, argv);
  let mut line = balloc(0);
  let got = child_read_line(&mut child, &mut line);
  let status = child_wait(child);
  let mut bad = 0;
  if !got || blen(line) != 2 || status != 0 {
    bad = 1;
  }
  bfree(line);
  while vec_len(argv) > 0 {
    bfree(vec_pop(&mut argv));
  }
  vec_free(argv);
  return bad;
}
```

## Kernel capabilities

Hardware is reached only through capability types. There is no way to read or write memory through an integer address (ROADMAP invariant 1). Capabilities are linear, and `main` is handed the ones its target provides; they cannot be created. They carry **ghost state** that exists only in the proof: in the generated code a `Frame`, `PageTable`, `Dma` or `Lease` is a bare physical address, `Serial`, `Mmu`, `Cpu`, `Ports` and `MmioSpace` are empty, and a `PortIo` or `Mmio` is its base and size.

| Type | Meaning |
| --- | --- |
| `Serial` | The serial console |
| `Pool` | The physical pages the kernel may hand out |
| `Frame` | One physical page (4 KiB), owned by exactly one place at a time |
| `PageTable` | A page-table page at level 1 to 5. It owns the tables and pages its slots point to |
| `Shared` | One reference to a page that several page tables may map at once (`MAP_SHARED` memory): the page stays allocated while any reference is held, and the last `shared_drop` gives it back to the pool |
| `Dirty` | A page taken out of the pool without being zeroed: it may still hold what its last owner wrote, so nothing can read it, map it or give it back until it is overwritten whole and becomes a `Frame` |
| `Mmu` | The right to load CR3 (kernel only) |
| `Cpu` | The CPU's saved user context and the right to enter user mode (kernel only) |
| `Lease` | This processor's hold on the top-level table it loaded with `mmu_enter`: while any is outstanding, other processors may be running on the tree, so nothing is taken out of it (kernel only; pinned to its processor) |
| `Ports` | The x86 I/O port space: the right to claim each of the 65536 ports, once. AArch64 has no port I/O, so an AArch64 kernel is never handed `Ports` and the port builtins are `E-TARGET` there |
| `PortIo` | A claimed range of I/O ports, owned: the only way to read or write them |
| `MmioSpace` | The device window (physical `0x80000000` up to 2^46, which the kernel platform maps uncached, so 64-bit PCI BARs above 4 GiB fit): the right to claim each byte of it, once. RAM above 2 GiB lies inside it and belongs to the page pool instead: `mmio_claim` of a block that overlaps the pool's RAM stops the machine (firmware never puts a device there). On AArch64 the window starts at 0, since devices usually sit below RAM there (QEMU's virt machine has its UART at `0x09000000`); a claim that overlaps any RAM the platform listed, the kernel image's included, stops the machine |
| `Mmio` | A claimed block of memory-mapped device registers, owned: the only way to read or write them |
| `Dma` | A page lent to devices for DMA: while lent it is not a `Frame`, so it cannot be freed, mapped or reused |

Ghost accessors, usable in specifications:

| Accessor | Meaning |
| --- | --- |
| `frame_id(f)`, `pt_id(t)`, `dma_id(d)` | The page number |
| `pt_level(t)` | 1 (maps pages) to 4 (top level, what CR3 holds); 5 is a container for level-4 tables that is never loaded into CR3 |
| `pt_used(t, i)` | Slot `i` owns something |
| `pt_leases(t)` | At least the number of leases outstanding on `t`'s tree: processors that may be running on it. A table you own below level 4 has 0; a table lent by `pt_borrow` has its tree's. Only `mmu_enter` raises it and only `pt_lease_count` lowers it |
| `pt_shared(t, i)` | Slot `i` maps a shared page, through a `Shared` the table holds (also callable at run time, level 1) |
| `shared_id(s)` | The page number of a shared page (also callable at run time) |
| `pool_has(p, i)` | The pool can still hand out page `i` |
| `pool_size(p)` | Number of pages in the pool (also callable at run time) |
| `mmio_free(w, a)` | Nobody has claimed device-window address `a`; `main` receives the whole window free |
| `mmio_base(r)`, `mmio_len(r)` | The registers `r` owns, `mmio_base(r)` up to `mmio_base(r) + mmio_len(r)`, always inside the device window (also callable at run time) |
| `mmu_root(m)` | The id of the table in CR3; `0x8000000` while the kernel's own map is loaded |
| `port_free(p, k)` | Nobody has claimed port `k`; `main` receives every port free |
| `port_base(io)`, `port_count(io)` | The ports `io` owns, `port_base(io)` up to `port_base(io) + port_count(io)`, always within the 65536 ports (also callable at run time) |

**Console** (all targets):

| Builtin | Requires (proved) | Effect |
| --- | --- | --- |
| `serial_write(c: &Serial, b: u8)` | | Writes one byte |
| `serial_read(c: &Serial) -> u8` | | Reads one byte, blocking. Hosted builds read standard input and return 4 (EOT) at its end |
| `serial_poll(c: &Serial) -> u64` | | Kernel only. A received byte, or 256 if none is waiting |

**Pages and page tables** (hosted and kernel; hosted builds simulate the pages with ordinary memory, so page code can be tested under the sanitizers):

| Builtin | Requires (proved) | Effect |
| --- | --- | --- |
| `pool_take(p: &mut Pool, i: u64) -> Frame` | `i < pool_size(p)` and `pool_has(p, i)` | Takes page `i` out of the pool, zeroed |
| `pool_give(p: &mut Pool, f: Frame)` | | Gives a page back |
| `pool_take_dirty(p: &mut Pool, i: u64) -> Dirty` | as `pool_take` | Takes page `i` out of the pool without zeroing it (`dirty_id` is `i`). Cheap where pages are handed out under a lock: the page is overwritten later, outside it |
| `dirty_id(d: &Dirty) -> u64` | | The page number |
| `dirty_zero(d: Dirty) -> Frame` | | Zeroes the page; it is now a frame, the same page |
| `frame_copy(d: Dirty, src: &Frame) -> Frame` | | Copies all of `src` over the page; it is now a frame, the same page. Contents are not tracked |
| `dirty_fill(d: Dirty, src: &Bytes, s: u64) -> Frame` | `s + 4096 <= blen(src)` | Copies bytes `s .. s+4096` of `src` over the page; it is now a frame, the same page. Contents are not tracked |
| `shared_copy(d: Dirty, s: &Shared) -> Frame` | | The same from a shared page: a private copy of it |
| `frame_id(f: &Frame) -> u64` | | The page number |
| `frame_read(f: &Frame, w: u64) -> u64`, `frame_write(f: &mut Frame, w: u64, v: u64)` | `w < 512` | A 64-bit word of the page. Contents are not tracked by the proof |
| `frame_read8(f: &Frame, o: u64) -> u8`, `frame_write8(f: &mut Frame, o: u64, b: u8)` | `o < 4096` | A byte of the page |
| `frame_to_bytes(dst: &mut Bytes, d: u64, f: &Frame, o: u64, n: u64)` | `o + n <= 4096` and `d + n <= blen(dst)` | Copies bytes `o .. o+n` of the page over bytes `d .. d+n` of `dst` in one go. The page's contents are not tracked, so those bytes of `dst` become unknown to the proof; the rest stay as they were |
| `bytes_to_frame(f: &mut Frame, o: u64, src: &Bytes, s: u64, n: u64)` | `o + n <= 4096` and `s + n <= blen(src)` | Copies bytes `s .. s+n` of `src` into the page at `o`. It stays the same page |
| `frame_to_frame(dst: &mut Frame, d: u64, src: &Frame, s: u64, n: u64)` | `d + n <= 4096` and `s + n <= 4096` | Copies bytes `s .. s+n` of `src` into `dst` at `d` in one go. Neither page's contents are tracked, and both stay the same pages |
| `pt_new(f: Frame, level: u64) -> PageTable` | `1 <= level <= 5` | Turns a page into an empty table. A level-4 table gets the kernel's map in slot 0 (not owned, never touched) |
| `pt_free(t: PageTable) -> Frame` | level is not 4; every slot empty | Turns an empty table back into a page |
| `pt_free_root(m: &Mmu, t: PageTable) -> Frame` | level 4; slots 1-255 empty; `pt_id(t) != mmu_root(m)`; `pt_leases(t) == 0` | Kernel only. The same for a top-level table, which must not be in CR3, here or on another processor |
| `pt_has(t: &PageTable, i: u64) -> bool` | `i < 512` | Whether slot `i` owns something (`pt_used(t, i)`) |
| `pt_put(t: &mut PageTable, i: u64, c: PageTable)` | level >= 2; slot `i` empty; `pt_level(c) + 1 == pt_level(t)`; at level 4, `1 <= i < 256` | Moves table `c` into slot `i` |
| `pt_take(t: &mut PageTable, i: u64) -> PageTable` | level >= 2; slot `i` used (same slot rule at level 4); `pt_leases(t) == 0` | Moves the table out of slot `i`; its level is one less. What its slots hold is unknown to the proof, so code that tears it down tests each slot with `pt_has` |
| `pt_put_frame(t: &mut PageTable, i: u64, f: Frame, writable: bool)` | level 1; slot `i` empty | Maps page `f` in slot `i`, user-accessible. On x86-64 a page mapped writable also gets the no-execute bit and a read-only one does not (the kernel sets `EFER.NXE`); `pt_set_access` leaves the bit alone, so a page changes whether it may run by being taken out and mapped again |
| `pt_take_frame(t: &mut PageTable, i: u64) -> Frame` | level 1; slot `i` used; `pt_leases(t) == 0` | Unmaps the page and returns it |
| `pt_writable(t: &PageTable, i: u64) -> bool` | level 1; `i < 512` | Whether the page in slot `i` was mapped writable. The proof does not track it, so the answer is unknown to it; code that moves a page keeps its permission by passing this to `pt_put_frame` |
| `pt_accessible(t: &PageTable, i: u64) -> bool` | level 1; `i < 512` | Whether the program may touch the page in slot `i` at all (see `pt_set_access`). Unknown to the proof, like `pt_writable` |
| `pt_set_access(t: &mut PageTable, i: u64, a: u64)` | level 1; slot `i` used; `a <= 2` | What the program may do with the page in slot `i`: 0 nothing (any access faults, as for `PROT_NONE`), 1 read, 2 read and write. The table still owns the page, so `pt_has` stays true and the page is taken out as before; nothing the proof tracks changes. The kernel must make sure no stale TLB entry still allows more (another processor running on the table) |
| `pt_borrow(t: &mut PageTable, i: u64, f, args..)` | level >= 2; slot `i` used (same slot rule at level 4) | Calls `f(&mut child, args..)` with the table in slot `i` left in place, and returns what `f` returns. `f` must take `&mut PageTable` first and hand back the same table (`E-VERIFY-OWN`); the child's `pt_leases` is `t`'s |
| `pt_borrow_frame(t: &mut PageTable, i: u64, f, args..)` | level 1; slot `i` used | The same for the page in slot `i`: `f` takes `&mut Frame` first |
| `pt_lease_count(t: &mut PageTable) -> u64` | | The leases outstanding on `t` now, counted at run time; `pt_leases(t)` becomes this. Only the holder of `&mut t` could raise it, so a 0 stays 0 while `t` is held |
| `shared_new(f: Frame) -> Shared` | | Makes a page shareable; this is its only reference |
| `shared_dup(s: &Shared) -> Shared` | | Another reference to the same page |
| `shared_drop(p: &mut Pool, s: Shared) -> u64` | | Gives up a reference. If it was the last one, returns the page number and the pool has the page again (`pool_has(p, shared_id(s))`); otherwise returns 2^64 - 1 |
| `pt_put_shared(t: &mut PageTable, i: u64, s: Shared, writable: bool)` | level 1; slot `i` empty | Maps the shared page in slot `i`; the table holds the reference, and `pt_shared(t, i)` is true. Writable pages get the no-execute bit, as for `pt_put_frame` |
| `pt_take_shared(t: &mut PageTable, i: u64) -> Shared` | level 1; `pt_used(t, i) && pt_shared(t, i)`; `pt_leases(t) == 0` | Unmaps it and returns the table's reference |
| `pt_dup_shared(t: &PageTable, i: u64) -> Shared` | level 1; `pt_used(t, i) && pt_shared(t, i)` | A new reference to the shared page in slot `i`, which stays mapped (how `fork` copies a shared mapping) |

`pt_take_frame` also needs `!pt_shared(t, i)` (`E-VERIFY-SHARED`): a shared page is never handed out as a `Frame`, which would let it be freed while another table still maps it. The proof cannot count references, so the count lives at run time (`bst_shared_refs` in the prelude, part of the trusted base); what the proof keeps is that each `Shared` is one reference, used once, so the count is right as long as the prelude counts correctly. The native x86 backend does not support shared pages yet.

```bastio
// expect-exit: 0
// A page mapped by two tables. It goes back to the pool with the last reference.
fn main(pool: &mut Pool) -> u64 {
  if pool_size(pool) < 3 {
    return 1;
  }
  let mut a = pt_new(pool_take(&mut pool, 0), 1);
  let mut b = pt_new(pool_take(&mut pool, 1), 1);
  let s = shared_new(pool_take(&mut pool, 2));
  pt_put_shared(&mut b, 4, shared_dup(s), false);
  pt_put_shared(&mut a, 3, s, true);
  let first = shared_drop(&mut pool, pt_take_shared(&mut a, 3));
  let last = shared_drop(&mut pool, pt_take_shared(&mut b, 4));
  if last != 18446744073709551615 {
    pool_give(&mut pool, pool_take(&mut pool, last));
  }
  pool_give(&mut pool, pt_free(a));
  pool_give(&mut pool, pt_free(b));
  if first == 18446744073709551615 && last == 2 {
    return 0;
  }
  return 2;
}
```

```bastio
// expect-exit: 0
// Build a two-level table, map a page, and tear it all down again.

// Frees a level-1 table and every page it maps.
fn free_l1(pool: &mut Pool, t: PageTable)
  requires pt_level(t) == 1
{
  let mut t1 = t;
  let mut i = 0;
  while i < 512
    invariant i <= 512 && pt_level(t1) == 1
    invariant forall j: u64 :: j < i ==> !pt_used(t1, j)
  {
    if pt_has(t1, i) {
      if pt_shared(t1, i) {
        // A shared page goes back to the pool with its last reference.
        shared_drop(&mut pool, pt_take_shared(&mut t1, i));
      } else {
        pool_give(&mut pool, pt_take_frame(&mut t1, i));
      }
    }
    i = i + 1;
  }
  pool_give(&mut pool, pt_free(t1));
}

fn main(pool: &mut Pool) -> u64 {
  if pool_size(pool) < 3 {
    return 1;
  }
  let mut l2 = pt_new(pool_take(&mut pool, 0), 2);
  let mut l1 = pt_new(pool_take(&mut pool, 1), 1);
  pt_put_frame(&mut l1, 7, pool_take(&mut pool, 2), true);
  pt_put(&mut l2, 3, l1);
  // l2 now maps one page through slot 3. Tear it down:
  free_l1(&mut pool, pt_take(&mut l2, 3));
  pool_give(&mut pool, pt_free(l2));
  return 0;
}
```

```bastio
// expect-error: E-VERIFY-OWN
fn main(pool: &mut Pool) -> u64 {
  if pool_size(pool) < 1 {
    return 1;
  }
  let a = pool_take(&mut pool, 0);
  let b = pool_take(&mut pool, 0);       // page 0 is already taken
  pool_give(&mut pool, a);
  pool_give(&mut pool, b);
  return 0;
}
```

**Address spaces shared between processors.** The threads of one process run on several processors at once, all with the same top-level table in CR3. Each processor takes a `Lease` with `mmu_enter` before it runs on the table and gives it back with `mmu_exit` after; the table itself stays in shared state (a lock), where system calls on any processor keep using it. The rule that makes this safe is that a leased tree only grows:

- Nothing comes out of it: `pt_take` and `pt_take_frame` need `pt_leases(t) == 0`, and so does `pt_free_root`. Code that works on a live tree reaches its lower tables and pages with `pt_borrow` and `pt_borrow_frame`, which lend them in place instead of unlinking them; what they lend has the tree's leases, so it cannot give anything up either.
- Things go into it: `pt_put` and `pt_put_frame` are allowed on a live table. Each writes the entry with one 64-bit store, present bit included, after everything the entry points to is written (release order), so a processor walking the table sees either no entry or a complete one.

Why this needs no TLB shootdown: a TLB caches only present entries, and an entry of a leased tree never stops being present or changes where it points. Entries that were not present are not cached, so a new mapping is seen at once, at worst after one page fault. A mapping goes away only when the tree has no lease left; `mmu_exit` loads the kernel's own table first, which also empties that processor's TLB, so by then no processor holds a translation for it.

Only leases matter at levels below 4 through `pt_borrow`: a table below the top that a program owns was taken out of a tree nobody ran on (or was never in one), so its `pt_leases` is 0 and taking from it is fine, as it always was.

`pt_leases` counts in the proof only up: `mmu_exit` cannot say which table its lease was for (it came from an earlier turn, through a lock), so when a program wants to tear down a table it asks the run-time count with `pt_lease_count`. A count of 0 stays 0 while the program holds `&mut t`, because only `mmu_enter`, which needs `&mut t`, raises it. BastiOS waits for 0 before freeing an address space; a processor holding a lease never waits for anything, so the wait ends.

Every top-level table a processor loads is entered with a lease; there is no other way to put a program's table in CR3.

**MMU and CPU** (kernel only; the kernel's platform code provides the trap entry):

| Builtin | Requires (proved) | Effect |
| --- | --- | --- |
| `mmu_kernel(m: &mut Mmu)` | | Loads the kernel's own map |
| `mmu_enter(m: &mut Mmu, t: &mut PageTable) -> Lease` | level 4; `mmu_root(m)` is the kernel's (`0x8000000`) | Takes a lease on `t` and loads it into CR3: `pt_leases(t)` goes up by one, `mmu_root(m)` becomes `pt_id(t)`. `t` can then go back where it came from (a lock, say) and other processors can enter it too |
| `mmu_exit(m: &mut Mmu, l: Lease)` | | Loads the kernel's own map, then gives the lease back. No table's `pt_leases` drops in the proof (it does not name one): it stays an overcount until `pt_lease_count` reads the true count |
| `cpu_get(c: &Cpu, r: u64) -> u64` | `r < 21` (AArch64: `r < 37`) | Saved user register `r`: rax rbx rcx rdx rsi rdi rbp r8-r15 (0-14), rip (15), rsp (16), rflags (17), error code (18), fault address (19), FS base (20). On AArch64: x0-x30 (0-30), SP_EL0 (31), the return address ELR_EL1 (32), SPSR_EL1 (33), the syndrome ESR_EL1 (34), the fault address FAR_EL1 (35), TPIDR_EL0 (36) |
| `cpu_set(c: &mut Cpu, r: u64, v: u64)` | `r < 18` or `r == 20` (AArch64: `r < 34` or `r == 36`) | Sets a saved register or the FS base (the error code and fault address are only reported by a trap). Whatever is stored, user mode is entered in ring 3 with the user segments, IOPL 0 and interrupts on; the platform replaces a non-canonical address with 0. On AArch64 the platform enters EL0 with interrupts unmasked whatever SPSR holds: only its condition flags are kept |
| `cpu_fpu_save(c: &mut Cpu, f: &mut Frame)` | | Saves the user's x87/SSE registers into the page (`fxsave64`) |
| `cpu_fpu_load(c: &mut Cpu, f: &Frame)` | | Loads them from the page (`fxrstor64`); reserved MXCSR bits are cleared first, so any contents are safe to load. A kernel that runs several user contexts keeps one page per context and swaps them around `cpu_run` |
| `cpu_run(c: &mut Cpu, m: &Mmu) -> u64` | | Runs the saved context in user mode until the next trap; returns its vector (`<= 255`) |
| `cpu_idle(c: &mut Cpu) -> u64` | | Waits for an interrupt; returns its vector |
| `cpu_count(c: &Cpu) -> u64` | | Processors the platform has started, this one included: between 1 and 4096 |
| `cpu_tsc(c: &Cpu) -> u64` | | This processor's time-stamp counter (`rdtsc`). The proof knows nothing about the value, not even that it goes up: how fast it counts and whether the processors' counters agree is the hardware's business |
| `boot_fb(c: &Cpu, f: u64) -> u64` | | The linear framebuffer the boot loader set up (Multiboot), if it is 32-bit RGB and ends at or below 4 GiB: `f` 0 its address, 1 bytes per line, 2 width, 3 height, 4 the bit positions of red, green and blue (a byte each). Every field is 0 without one, and below 2^32. Drawing on it still needs an `Mmio` claim of its block |
| `boot_acpi(c: &Cpu, sig: u64, off: u64) -> u64` | | Byte `off` of the firmware's ACPI table whose signature is `sig`, its four letters as a little-endian number (`0x50434146` is `FACP`, the FADT; `DSDT` is the one the FADT points at), or 256 past the table's end or without one. At most 256. The tables are read, never written, and parsing them (AML included) is the program's job, proved in bounds like any other |
| `boot_info(c: &Cpu, what: u64, i: u64) -> u64` | | Answer `i` to the platform's query `what` about the machine, or 0: the platform defines the queries (BastiOS: its boot command line, the memory map, CPUID, what its processor startup found). Read-only, and nothing is promised about the number, so the program checks it like any other input |
| `smp_run(f, args..)` | `f`'s `requires`, also for the other processors (below) | Runs `f(args..)` on every processor at once and returns when all have returned |

`smp_run(f, args..)` is how kernel code runs on the other processors. `f` returns nothing, and each parameter is `&mut Cpu` or `&mut Mmu` (this processor passes its own; every other processor gets its own, with the kernel's tables in CR3) or a shared borrow `&T` of a value with no `Cpu` or `Mmu` in it (`E-PIN` otherwise). So the processors share only what cannot change, plus locks. `f`'s precondition is proved for this processor's arguments and again with `mmu_root` equal to the kernel's for the others. See [concurrency.md](concurrency.md).

```bastio
// expect-ok
// target: kernel
// A kernel main: echo the console until a 'q' arrives.
fn main(con: &mut Serial, mmu: &mut Mmu) {
  mmu_kernel(&mut mmu);
  let mut c = serial_read(con);
  while c != 113
    invariant c <= 255
  {
    serial_write(con, c);
    c = serial_read(con);
  }
}
```

**I/O ports** (kernel and hosted; hosted builds simulate the ports, each remembering the last value written, so drivers can be tested off the machine):

| Builtin | Requires (proved) | Effect |
| --- | --- | --- |
| `port_claim(p: &mut Ports, base: u64, count: u64) -> PortIo` | `base + count <= 65536`; `port_free(p, k)` for every `k` in the range | Takes the ports out of `p` |
| `port_release(p: &mut Ports, io: PortIo)` | | Gives them back |
| `pio_in8` / `pio_in16` / `pio_in32(io: &mut PortIo, off: u64) -> u8` / `u16` / `u32` | `off + 1` / `2` / `4 <= port_count(io)` | `in` from port `port_base(io) + off` |
| `pio_out8` / `pio_out16` / `pio_out32(io: &mut PortIo, off: u64, v: u8` / `u16` / `u32)` | the same | `out` to that port |

Reads take `&mut` too: reading a device register changes the device (it pops a FIFO or acknowledges an interrupt). Claiming compiles to nothing and each access is one instruction. Ports nobody claimed cannot be touched, and two drivers never own the same port. What a device does with the values is not modelled.

```bastio
// expect-exit: 0
// A driver owns the eight ports of a UART and touches only those.
struct Uart { io: PortIo }

fn uart_send(u: &mut Uart, b: u8)
  requires port_count(u.io) == 8
  ensures port_count(u.io) == 8
{
  pio_out8(&mut u.io, 0, b);
}

fn main(ports: &mut Ports) -> u64 {
  let mut u = Uart { io: port_claim(&mut ports, 0x3F8, 8) };
  uart_send(&mut u, 65);
  // port_claim(&mut ports, 0x3FC, 1) here would be E-VERIFY-OWN.
  port_release(&mut ports, u.io);
  return 0;
}
```

**Memory-mapped registers** (kernel and hosted; hosted builds simulate the window with 1 MiB of memory indexed by the low 20 bits of the address):

| Builtin | Requires (proved) | Effect |
| --- | --- | --- |
| `mmio_claim(w: &mut MmioSpace, base: u64, len: u64) -> Mmio` | `0x80000000 <= base`, `base + len <= 2^46`; `mmio_free(w, a)` for every `a` in the block | Takes the block out of `w` |
| `mmio_release(w: &mut MmioSpace, r: Mmio)` | | Gives it back |
| `mmio_read8` / `16` / `32` / `64(r: &mut Mmio, off: u64) -> u8` / `u16` / `u32` / `u64` | `off + width <= mmio_len(r)`; `mmio_base(r) + off` is a multiple of the width | One volatile load |
| `mmio_write8` / `16` / `32` / `64(r: &mut Mmio, off: u64, v: u8` / `u16` / `u32` / `u64)` | the same | One volatile store |

The rules are those of ports: claiming compiles to nothing, two drivers never own the same register, and reads take `&mut`. Because claims must lie in the device window and the page pool stays below 256 MiB, a driver can never reach memory the allocator owns. What a device does with the values is not modelled.

```bastio
// expect-exit: 0
// The local APIC's version register, read through a claimed block.
fn main(win: &mut MmioSpace) -> u64 {
  let mut apic = mmio_claim(&mut win, 0xFEE00000, 4096);
  let v = mmio_read32(&mut apic, 0x30);
  // mmio_read32(&mut apic, 0x32) would be E-VERIFY-BOUNDS (unaligned).
  mmio_release(&mut win, apic);
  return cast[u64](v % 1);
}
```

**Pages lent to devices** (kernel and hosted). A device that reads or writes memory on its own (DMA) is given a page by turning its `Frame` into a `Dma`:

| Builtin | Requires (proved) | Effect |
| --- | --- | --- |
| `dma_lend(f: Frame) -> Dma` | | Lends the page to devices. A full memory fence first, so earlier stores reach memory before any device can be told where the page is |
| `dma_take(d: Dma) -> Frame` | | Takes the same page back (`frame_id` is `dma_id(d)`), after a fence |
| `dma_id(d: &Dma) -> u64` | | The page number |
| `dma_addr(d: &Dma) -> u64` | | The page's physical address, to program into a device. Only a lent page's address can be read |
| `dma_read8` / `16` / `32` / `64(d: &mut Dma, off: u64) -> u8` / `u16` / `u32` / `u64` | `off + width <= 4096`; `off` is a multiple of the width | One volatile load |
| `dma_write8` / `16` / `32` / `64(d: &mut Dma, off: u64, v: u8` / `u16` / `u32` / `u64)` | the same | One volatile store |
| `dma_to_bytes(dst: &mut Bytes, d: u64, m: &mut Dma, o: u64, n: u64)` | `o + n <= 4096` and `d + n <= blen(dst)` | Copies bytes `o .. o+n` of the page over bytes `d .. d+n` of `dst`, as `frame_to_bytes`; those bytes become unknown to the proof. The copy is a compiler barrier on both sides |
| `bytes_to_dma(m: &mut Dma, o: u64, src: &Bytes, s: u64, n: u64)` | `o + n <= 4096` and `s + n <= blen(src)` | Copies bytes `s .. s+n` of `src` into the page at `o`, as `bytes_to_frame`, with the same barriers |

What is proved: while a page is lent it cannot go back to the pool, into a page table or into anything else that takes a `Frame`, so a device can never write a page that has been reused; the CPU reaches it only with volatile, aligned, in-bounds accesses or with in-bounds bulk copies that are compiler barriers on both sides, so the C compiler cannot cache what the device sees or move an access across a copy (a bulk copy itself may be done in any width, so it suits buffers, not device fields that must not tear); and a `Dma` must be taken back before the program ends (it is linear). What is trusted: that the driver has stopped the device using the page (for example, seen its completion) before `dma_take`, and that the addresses it programs into the device are those of pages it has lent. Devices are outside the proof.

```bastio
// expect-exit: 0
// A one-page buffer handed to a device and taken back.
fn main(pool: &mut Pool) -> u64 {
  if pool_size(pool) == 0 {
    return 0;
  }
  let mut d = dma_lend(pool_take(&mut pool, 0));
  let a = dma_addr(d);
  dma_write64(&mut d, 0, a);
  // dma_write64(&mut d, 4092, 0) would be E-VERIFY-BOUNDS (past the page).
  let f = dma_take(d);
  pool_give(&mut pool, f);
  return 0;
}
```

**System calls** (user target only). Programs built with `--target user` run under BastiOS and have `sys_spawn(b: &Buf, from: u64, to: u64)` (start the program named by elements `from..to` of `b`, proved to be a range within `b`), `sys_wait(pid)`, `sys_kill(pid)`, `sys_yield()`, `sys_getpid()`, `sys_info(k)`, `sys_ps()` and `sys_poweroff()`.

Calls the language does not know by name go through three generic builtins. The call number `n` and what the call does belong to the kernel (BastiOS defines its file system calls this way), so a new system call needs no compiler change:

| Builtin | Requires | Meaning |
| --- | --- | --- |
| `sys_call(n, a, b) -> u64` | none | System call `n` with two integer arguments |
| `sys_send(n, a, buf: &Bytes, len) -> u64` | `len <= blen(buf)` | Lends the kernel bytes `0..len` of `buf` to read |
| `sys_recv(n, a, buf: &mut Bytes) -> u64` | none | Lends the kernel all of `buf` to overwrite. Afterwards its length is the same and every byte is still `<= 255`; nothing else about its contents is known |

The kernel is trusted to stay inside the bytes it is lent; BastiOS proves that it does.

Calling a builtin the target does not have is `E-TARGET`.

## Specifications

Expressions in `requires`, `ensures`, `invariant` and `assert` (and the bodies of `spec fn`s) exist only at compile time and generate no code.

- They are evaluated over **mathematical integers**: `a - b` may be negative and `a + b` cannot overflow. Integers of different types mix freely there, with no conversions.
- They may use `==>` (implication) and `forall x: u64 :: e`.
- They may call `len`, `get`, `slen`, `sget`, `blen`, `bget`, `bdec`, `bhex`, `has_bit`, `vlen`, `vcap`, `vget`, `vcount`, the ghost accessors of the capabilities, and `spec fn`s.
- `vcount(v, x)` is the number of elements of `v` equal to `x`, for a vector of integers. It states what `forall` cannot: that a vector is a permutation of another, or that a counter counts something.
- `bdec(b, from, to)` is the decimal value of bytes `from..to` of `b`, each byte read as a digit (byte - 48): 0 for an empty range, and `10 * bdec(b, from, to - 1) + (bget(b, to - 1) - 48)` otherwise. The prover unfolds that definition one digit for each `bdec` term a proof mentions, so a loop that parses digits states `v == bdec(b, from, i)` as its invariant, and a constant is evaluated by asserting each prefix in turn (`tests/pass/bdec.bastio`).
- `bhex(b, from, to)` is the same in base 16: `16 * bhex(b, from, to - 1)` plus the value of the hex digit `bget(b, to - 1)` (`0-9`, `a-f`, `A-F`; another byte has no meaningful value, so a spec says the bytes are hex digits separately, as `core/num.bastio`'s `all_hex` does).
- `has_bit(x, i)` says whether bit `i` of the integer `x` is set (bit 0 is the lowest; from bit 64 up, never). The prover knows, for every bitwise operator the code evaluates (`&`, `|`, `^`, `!`, shifts and low masks), how each bit of the result follows from the bits of the operands, and that `x & 1` is 1 exactly when bit 0 is set. So a function that sets one bit can promise that the other 63 are unchanged (`forall j: u64 :: j != k ==> has_bit(result, j) == has_bit(w, j)`), and a loop that tests `(w >> i) & 1` can carry `forall j: u64 :: j < i ==> has_bit(w, j)` (`tests/pass/bits_packed.bastio`). Only the bits of values the code computed with these operators are known: a variable's bits are whatever the proof is told about them, and the prover does not turn a statement about every bit back into one about the value (that `x` is 0 when no bit is set, say).
- They may use `is` and enum payloads (`o.Some`), and see through boxes.
- `==` and `!=` also compare structs and enums made of integers and booleans (no buffers, boxes or vectors inside): field by field, and for an enum the variant and then that variant's payload. Both sides must name a value (a variable, a field, `vget(..)` or `old(..)`), since specifications cannot build one. Ordinary code still compares only integers and `bool`.
- They may mention values that have already been consumed. In `ensures`, a by-value parameter means its value on entry, a `&mut` parameter the value handed back, and `old(x)` its value at the call. `result` is the returned value. `old(x)` is also allowed in loop invariants, for parameters only.
- They cannot build enum values or call ordinary functions.

A `spec fn` names a specification. It is inlined where it is used, and it can call only spec fns defined before it, so it cannot be recursive:

```bastio
// expect-exit: 0
spec fn all_small(b: &Buf, n: u64) -> bool {
  forall i: u64 :: i < len(b) ==> get(b, i) < n
}

fn clamp(b: Buf, n: u64) -> Buf
  requires n > 0
  ensures len(result) == len(b) && all_small(result, n)
{
  let mut r = b;
  let mut i = 0;
  while i < len(r)
    invariant i <= len(r) && forall k: u64 :: k < i ==> get(r, k) < n
  {
    if get(r, i) >= n {
      r = set(r, i, n - 1);
    }
    i = i + 1;
  }
  return r;
}

fn main() -> u64 {
  let b = clamp(set(alloc(3), 1, 500), 10);
  assert all_small(b, 10);
  free(b);
  return 0;
}
```

An `opaque spec fn` is inlined only in its own file. Everywhere else it is an uninterpreted function of its arguments: a proof there knows only what contracts say about it, not its body. This lets a library describe its data abstractly and keeps the details out of its users' proofs. `core/ring.bastio` says what a queue holds as `queue_at(q, i)`, its i-th oldest word, computed from the ring's slot arithmetic. Its push and pop are proved against that body, inside the file. A user of the queue sees only that `queue_push` appends to the sequence and `queue_pop` returns its first word and shifts the rest, so a loop that fills and drains a queue proves the order of the words in milliseconds (`tests/pass/queue_fifo.bastio`), where the inlined arithmetic took the solver minutes. This hides nothing that matters for soundness: what a proof concludes about an unknown function also holds for the real one. An opaque spec fn cannot be generic (`E-PARSE`), and its parameters are integers, `bool`, `Buf`, `Bytes`, and structs of those or borrows of one (`E-TYPE`). `opaque` is a keyword only before `spec fn`.

A `spec fn` can be generic. Like a generic function, it is copied once per list of type arguments, which are written at every call (`grew_by[u64](v, w, 1)`), and a generic function's specifications can pass its own type parameters on:

```bastio
// expect-exit: 2
spec fn grew_by[T](v: &Vec[T], w: &Vec[T], n: u64) -> bool {
  vlen(v) == vlen(w) + n
}

fn push_two[T](v: &mut Vec[T], a: T, b: T)
  ensures grew_by[T](v, old(v), 2)
{
  vec_push(&mut v, a);
  vec_push(&mut v, b);
}

fn main() -> u64 {
  let mut v = vec_new[u64]();
  push_two[u64](&mut v, 1, 2);
  let n = vec_len(v);
  vec_pop(&mut v);
  vec_pop(&mut v);
  vec_free(v);
  return n;
}
```

Counting is how a sort states that it loses no element. Each vector operation tells the prover how it changes every count, so a swap of two elements is seen to keep them all:

```bastio
// expect-exit: 3
spec fn same_elems(a: &Vec[u64], b: &Vec[u64]) -> bool {
  vlen(a) == vlen(b) && forall x: u64 :: vcount(a, x) == vcount(b, x)
}

fn swap(v: &mut Vec[u64], i: u64, j: u64)
  requires i < vlen(v) && j < vlen(v)
  ensures same_elems(v, old(v))
{
  let a = vec_get(v, i);
  let b = vec_swap(&mut v, j, a);
  vec_set(&mut v, i, b);
}

fn main() -> u64 {
  let mut v = vec_new[u64]();
  vec_push(&mut v, 3);
  vec_push(&mut v, 4);
  swap(&mut v, 0, 1);
  assert vcount(v, 3) == 1;
  let r = vec_pop(&mut v);
  vec_pop(&mut v);
  vec_free(v);
  return r;
}
```

The facts the prover is given about counts (a count is at most the length, every element is counted, every counted value is an element, and how each vector operation changes the counts) are listed in [the trusted base](trusted-base.md). A program that never uses `vcount` gets none of them.

## How verification works

After the type and linearity checks, the compiler executes each function symbolically. At every point where the program could go wrong it asks Z3 whether the bad case is reachable, and if it is, it rejects the program and shows a counterexample (values of the variables that lead there). A buffer or vector shows its length, and when the failed proof is an `assert`, `requires`, `ensures` or invariant, its elements too if there are at most 8 (`counterexample: vlen(v) = 2, v = [3, 1]`).

- **Modular.** A call uses the callee's `requires` and `ensures`, and a summary of which parts of each `&mut` argument the callee may change (below), never the rest of its body.
- **Loops** are cut with their invariants. After a loop, the variables the body assigns are known only through the invariants and the negated condition.
- **Branches** are followed separately; the condition of an `if` or the variant of a `match` arm is known inside it. So a test the code already makes, as in `if i < len(b) { get(b, i) }`, is enough for the proof.
- **Annotation inference.** What a loop body or a callee provably leaves alone keeps what is known about it, so it need not be restated in an `invariant` or `ensures`:
  - A buffer only ever updated as `x = set(x, ..)` keeps its length, and a vector changed only by `vec_set` and `vec_swap` its length. A capability changed only by builtins that cannot change them keeps its identity: a `Frame` its id, a `PageTable` its id and level, a `Pool` its size.
  - For each `&mut` parameter, the compiler works out from the function's body which places under it the function may change, and how: by assigning them, or only through the shape-keeping updates above. Lending a place to another function counts as that function's summary applied to it, so this goes through any depth of calls, and through recursion. At a call, and in a loop that makes the call, every other part of the argument (scalar fields, enum tags, lengths, capability ids) keeps its value. So a driver function that takes `&mut Dev` and only ever uses `d.page` through builtins needs no `ensures` restating `frame_id(d.page)`, and the loops of its callers no `invariant` either.
  - The contents of buffers and vectors, and capability state such as which slots of a page table are used, are still forgotten when changed: say what is kept of them in `ensures` or `invariant`.
- **Boxes and vectors.** The prover unfolds unknown recursive data two levels deep, and tracks vector contents element by element (see [Boxes](#boxes) and [Vectors](#vectors)).
- **The same answer every time.** Z3 gets a fixed amount of work per query (a resource limit, not a timeout), and each function is verified on a solver in the same fresh state, whichever functions were verified before it and however many run at once (`BASTIO_JOBS`). So whether a program is accepted depends only on the program and the Z3 version, not on the machine's load or the number of processors. A query that runs out of work is reported as "the solver gave up". The limit is counted in Z3's own units, which track time only roughly, and on some nonlinear queries Z3 4.8 does not count its work at all and would search forever. So a query also gives up after 60 s of wall-clock time. That backstop is the one place load can matter: a query that would have been proved after more than 60 s is reported as "the solver gave up" instead.

What is proved, by error code, is listed in the next section.

## Error codes

Every rejection names one of these codes. The tests in `tests/fail/` each break one rule and are checked to be rejected with the right code.

Syntax, types and linearity:

| Code | Rule |
| --- | --- |
| `E-PARSE` | Syntax error, including a constant that is not `u64` or defined twice, and `import` after a definition |
| `E-TYPE` | Type error: wrong argument or field types, an unknown name, a borrow used outside a parameter, shadowing, a bad `main` signature, a generic definition that cannot be instantiated, and so on |
| `E-TARGET` | A builtin the target does not have (such as `host_read` in a kernel) |
| `E-IMPORT` | An imported file cannot be read, or files import each other |
| `E-EXPORT` | An `export fn` breaks a rule of the C boundary: it has a `requires`, takes or returns a type C cannot pass, may free or replace a buffer C lends it, is `main`, or is outside the hosted target |
| `E-MATCH` | A `match` misses a variant, matches one twice, or has an arm after `_` |
| `E-LINEAR-USE` | A consumed value (or a moved-out field, or a struct with a field moved out) is used; one call both borrows and moves a value, or lends it twice with one loan `&mut`; a borrow is returned; a value is consumed where the code may run more than once or not at all (a `while` condition, the right side of `&&` / `\|\|`) |
| `E-LINEAR-LEAK` | A linear value is still owned at the end of its scope or at a `return`, is overwritten, or is dropped by `_` |
| `E-LINEAR-BRANCH` | Only one arm of an `if` or a `match` consumes a value; a loop body changes the ownership of an outer value |
| `E-LOCK-ORDER` | A lock may be taken while a lock of the same class (contents type) is held, or lock classes are taken in opposite orders in two places |
| `E-PIN` | A `Cpu`, `Mmu` or `Lease` (or a value holding one) is put in a lock or shared between processors by `smp_run` |

Proofs:

| Code | What is proved |
| --- | --- |
| `E-VERIFY-BOUNDS` | Every index is below its limit: buffer, byte, string and vector indices; I/O port and register offsets within the claimed range, and register accesses aligned; `vec_pop` on a non-empty vector; page numbers below `pool_size`; words, bytes and slots of a page or table; `cpu_get` / `cpu_set` registers; the `sys_spawn` range; the `sys_send` length |
| `E-VERIFY-LEAK` | A vector of linear elements is empty when it is freed |
| `E-VERIFY-OVERFLOW` | `+` and `*` results fit the operation's type (< 2^64 for `u64`, < 2^8 for `u8`, ...) |
| `E-VERIFY-CAST` | the argument of `cast[T]` fits in `T` |
| `E-VERIFY-SHIFT` | a shift amount is less than the width of the shifted type |
| `E-VERIFY-UNDERFLOW` | `a - b` has `a >= b` |
| `E-VERIFY-DIVZERO` | The divisor of `/` and `%` is not zero |
| `E-VERIFY-ALLOC` | Allocation size <= 2^60 elements |
| `E-VERIFY-OWN` | A page is taken from the pool at most once; an I/O port or device register has at most one owner; a table slot is empty before `pt_put` and used before `pt_take`; a table is empty before it is freed |
| `E-VERIFY-PT` | Table levels line up; frames go only into level-1 tables; slot 0 of a top-level table (the kernel's map) is never touched |
| `E-VERIFY-MMU` | The table in CR3 is never freed; a processor enters one address space at a time |
| `E-VERIFY-LEASE` | Nothing is taken out of a page-table tree, and its top-level table is not freed, while a processor may be running on it |
| `E-VERIFY-SHARED` | A shared page is taken out of a table only as a `Shared` (`pt_take_shared`), never as a `Frame`; `pt_take_shared` and `pt_dup_shared` read a slot that maps one |
| `E-VERIFY-REQUIRES` | The callee's `requires` |
| `E-VERIFY-ENSURES` | The function's own postcondition |
| `E-VERIFY-INVARIANT` | A loop invariant holds on entry and is preserved by one iteration |
| `E-VERIFY-LOCKINV` | A struct invariant holds of a value put in a lock, and again at the end of every `locked` call; of every value written to an atomic |
| `E-VERIFY-ASSERT` | An `assert` |
| `E-VERIFY-UNREACHABLE` | An `unreachable;` the code can reach |

`E-SOLVER` means Z3 could not be run or gave an answer the compiler did not expect. It is not a verdict on the program.

## Targets and generated code

The compiler emits C, which a C compiler turns into machine code. The C contains no runtime checks: every check was discharged by a proof, and `tests/run.sh` verifies this for a sample program by inspecting the C.

- **`--target hosted`** (the default) builds an ordinary program against the C library. `alloc`, `box` and vectors use `malloc`; running out of memory aborts. `Serial` is standard input and output. `Pool` is simulated with ordinary memory.
- **`--target kernel`** emits freestanding C for bare-metal x86-64 that needs no C library. It provides `bst_kmain`, which initialises COM1 (only when `main` takes `Serial`) and calls `main`. The boot code that calls `bst_kmain`, and the trap entry code behind `cpu_run` and `cpu_idle`, live in the kernel's own repository (BastiOS). The prelude implements the console with port I/O on COM1, `alloc` and `free` with a fixed 8 MiB arena and an exact-size free list, then an extension of an eighth of the pool, at least 4 MiB carved from RAM, `print` on the serial port, the pool (every run of RAM in the Multiboot memory map above the end of the kernel image, up to the platform's `bst_ram_limit` and at most 2^27 pages, numbered densely across the holes), page-table entry encoding and CR3 loads. Running out of arena memory halts the machine.
- **`--target user`** emits freestanding C for a user program under BastiOS, entered at `bst_umain`. Its prelude implements the console and the `sys_*` builtins as `int 0x80` system calls, and `alloc` with a 64 KiB arena.

On the freestanding targets `Bytes` come from the same arena as `Buf`.

The hosted C is the same for every processor, so for a hosted program `--arch aarch64` changes only the C compiler: the program is built for AArch64 Linux with `aarch64-linux-gnu-gcc` (or `$CC`).

With `--target kernel` or `--target user`, `--arch aarch64` (bastioc1 only) selects AArch64 preludes. The kernel's console is a PL011 UART at the platform's `bst_uart_base` (QEMU virt's `0x09000000` by default); `bst_kmain`'s argument points to a count of RAM runs followed by each one's base and length, which the platform takes from the device tree; page tables are VMSAv8-64 with a 4 KiB granule, user pages normal memory (MAIR index 0) that EL0 may reach and EL1 may never execute, loaded into TTBR0 (the kernel's map is in top-level slot 0 of every table, as on x86-64); a page mapped for user mode has its data cache cleaned and the instruction cache invalidated, since AArch64's caches are not coherent with each other; MMIO and DMA accesses carry the `dmb` barriers Linux's `readl`/`writel` and `dma_rmb`/`dma_wmb` do, since AArch64 reorders memory accesses that x86-64 keeps in order; `cpu_run` enters EL0 through the platform's `bst_cpu_enter`, which returns 128 for an `svc`, 14 for an abort, 32 + n for interrupt n and 6 for anything else; the kernel's C is built with `-mgeneral-regs-only`, so the FP/SIMD registers keep the user's values until `cpu_fpu_save` stores q0-q31, FPSR and FPCR. A user program makes system calls with `svc #0`, the number in x8 and the arguments in x0-x2. `bastioc1 --arch aarch64 --trusted-base` lists these preludes (`tests/trusted_base_aarch64.txt`).

`tests/aarch64.sh` builds every test program for AArch64, runs it under `qemu-aarch64` and requires the same exit status and output as on x86-64; checks that the compiler built for AArch64 writes the same C for itself as the x86-64 build; builds the user programs for AArch64; and boots a small kernel (`tests/aarch64/`) on QEMU's virt machine that maps a user program, runs it at EL0 through system calls and an abort, and reads a device register. The native backend is x86-64 only.

The native backend (`--backend native`, hosted only, experimental) skips C: it writes x86-64 assembly, so the C compiler leaves the trusted base for the program's own code, while its small runtime (printing, the console and the allocator, written in assembly over the C library) is listed by `--trusted-base`. `tests/native.sh` builds every test and benchmark with both backends and requires the same exit status and output. It loses the C compiler's optimizations (inlining, vectorization), so it is slower; `bench/run.sh` reports the gap.

In the generated C, a struct is a C struct passed by value, an enum is a tag and a union, a `Box[T]` is a `T *`, a `Vec[T]` is `{ len, cap, data }`, and ghost state (capability state, specifications, `assert`) leaves nothing behind.

## Calling Bastio from C

A function marked `export` is also a C function of the same name, so a C program can call proved Bastio code directly. A program with an `export fn` is a library: it needs no `main`. Build it with `--emit-c` and `--header`, and compile the C with the rest of the C program:

```text
bastioc0 --emit-c --header find.h -o find.c find.bastio
cc -c find.c && cc app.c find.o
```

```bastio
// expect-ok
enum Found { At(u64), Missing }

/// Where `c` first occurs in `b`.
export fn find(b: &Bytes, c: u8) -> Found
  ensures result is Found::At ==> result.At < blen(b)
{
  let mut i = 0;
  while i < blen(b)
    invariant i <= blen(b)
  {
    if bget(b, i) == c {
      return Found::At(i);
    }
    i = i + 1;
  }
  return Found::Missing;
}
```

```c
/* in the generated header */
bst_e_Found find(const uint8_t *b, size_t b_len, uint8_t c);
```

What may cross the boundary (otherwise `E-EXPORT`):
- **Parameters:** integers, `bool`, `&Bytes`, which becomes a `const` pointer and a length (`b`, `b_len`), and `&mut Bytes`, a buffer the caller owns and the function writes into (`uint8_t *b, size_t b_len`), so a library need not allocate. Owned values are not accepted.
- **A lent `&mut Bytes` stays the caller's.** The function may write its bytes (`bset`, or by lending it to functions that only do that) but never free or replace it. The compiler proves this from the summary it already computes of what each function may change in its `&mut` arguments ([How verification works](#how-verification-works)), through any depth of calls; a function that might replace the buffer is `E-EXPORT`. So the buffer is the same allocation, of the same length, when the call returns, and there is nothing to copy back.
- **Results:** integers, `bool`, nothing, `Bytes`, and structs and enums built only from those. The header defines the C types (`bst_bytes`, `bst_s_Name`, `bst_e_Name`, with a `bst_e_Name_Variant` constant for each enum tag) exactly as the generated C lays them out. Bytes handed back belong to the caller, who releases `data` with `free()`.
- **No `requires`.** A C caller cannot be proved to meet a precondition, so an exported function has none: it checks the condition itself and returns an error value instead, which is then proved like any other code (`examples/base64/lib.bastio` does this for the encoder's length limit). This keeps the boundary free of generated checks. `ensures` clauses are allowed and are copied into the header as comments: they are proved, so the caller may rely on them.
- Not generic (`E-TYPE`), not `main`, not a name starting with `bst_`, and only on the hosted target.

The proof of an exported function assumes these things about its C caller that nothing checks (they are listed by `--trusted-base` and stated in the header): a `&Bytes` pointer is readable for its length and nothing writes those bytes during the call; a `&mut Bytes` pointer is readable and writable for its length, overlaps no other buffer argument, and nothing else uses it during the call; such a length is at most 2^60 (which follows from the first two, since no x86-64 address space is that large); and every argument is a valid value of its C type. A C caller that breaks them can make the Bastio code misbehave, as with any C library.

The C function is a thin wrapper that converts each pointer and length into the internal `bst_bytes` and calls the proved function; it does no other work. A worked example with a test harness and a benchmark is in `examples/base64`.

## Soundness scope

What a program that `bastioc0` accepts is guaranteed: no out-of-bounds access, no use after free, no double free, no leak, no integer overflow or underflow, no division by zero, and, in kernel code, no page owned twice, no malformed page table and no freeing of the active one. Plus every `requires`, `ensures`, `invariant` and `assert` the programmer wrote. What is not guaranteed:

- **Termination.** Infinite loops and infinite recursion are not detected. This is independent of memory safety and a non-goal for now.
- **Running out of memory.** `alloc`, `balloc`, `box` and `vec_push` abort when memory runs out. Each has a form that returns the failure instead, for code that must not stop: `try_alloc`, `try_balloc`, `try_box` and `vec_try_reserve` with `vec_push_within`.
- **Stack depth.** Deep recursion can overflow the stack.
- **Page contents.** The proof tracks who owns each page, not what is in it.
- **The trusted base.** Z3, the stage0 compiler (Rust) and the C compiler are trusted, as is the hand-written C prelude each target links in (in `stage0/src/codegen.rs`), and the platform's boot and trap entry code. `bastioc0 --trusted-base` lists every prelude part with its size, the parts each target links, and every builtin; the listing is checked into `tests/trusted_base.txt` and `tests/run.sh` fails when it changes, so growth is reviewed (ROADMAP invariant 3). The small per-type helpers the compiler generates for `Box` and `Vec` are part of the compiler, not of this listing. [trusted-base.md](trusted-base.md) goes through every part and why it is there.
- **C callers.** An `export fn` relies on its C caller for the assumptions listed in [Calling Bastio from C](#calling-bastio-from-c).
- **Child processes and files.** What another program writes, or what a file contains, is unknown to the proof, beyond each byte being `<= 255`.

`tests/run.sh` also runs every program in `tests/pass/` under AddressSanitizer and UBSan, to confirm that the proofs match what happens at run time. Those checks exist only in the test build, never in generated code.

## Not in the language yet

Planned, in [ROADMAP.md](../ROADMAP.md): the rest of concurrency (threads, atomics, interrupt-safe locks; designed in [concurrency.md](concurrency.md)), more hardware capabilities (`IrqOff`), allocation failure in the type, and a native x86-64 backend. Inference of type arguments, modules with qualified names, and hand-written lemmas for proofs Z3 cannot find are open questions.
