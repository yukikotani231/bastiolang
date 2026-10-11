# A tour of Bastio

This tutorial walks through Bastio by writing small programs and reading what the compiler says about them. It assumes you know C or a language like it. It does not assume you know anything about proofs or SMT solvers: the compiler does the proving, and you mostly tell it what you mean.

The full rules are in the [language reference](language.md). Every program in a `bastio` block below is compiled by `tests/run.sh`, so they are known to behave as described. The first line of each says what the compiler does with it: `// expect-exit: N` means it compiles, runs and exits with `N`; `// expect-error: CODE` means it is rejected with that error. The error messages shown after them are checked too: they are what the compiler prints, except that the values in a counterexample can differ between Z3 versions.

## 1. Building the compiler

You need [rustup](https://rustup.rs), Z3 and a C compiler. On Ubuntu or Debian: `sudo apt-get install z3 build-essential`. CI uses the Z3 in Ubuntu 24.04 (4.8.12); 5.1 works too. Then:

```text
$ git clone https://github.com/yukikotani231/bastiolang && cd bastiolang
$ (cd stage0 && cargo build --release)
$ export PATH=$PWD/stage0/target/release:$PATH
```

The first `cargo` run installs the Rust release the repository pins (`rust-toolchain.toml`). The compiler is `bastioc0`. It checks a program, proves it safe with Z3, turns it into C and calls `cc` on that C:

```text
$ bastioc0 -o hello examples/hello.bastio && ./hello
bastioc0: examples/hello.bastio: 12 proof obligation(s) discharged
1
4
9
16
25
```

The first line says how many facts the compiler proved: here, that each `set` and `get` is in bounds and that `(i + 1) * (i + 1)` and the counters do not overflow. To check the whole setup, run `tests/run.sh`. It compiles every test and every program in this tutorial.

## 2. A first proof

```bastio
// expect-exit: 42
fn average(a: u64, b: u64) -> u64 {
  return a / 2 + b / 2;
}

fn main() -> u64 {
  return average(40, 44);
}
```

`main`'s result is the program's exit status. Nothing unusual yet. Now write `average` the obvious way:

```bastio
// expect-error: E-VERIFY-OVERFLOW
fn average(a: u64, b: u64) -> u64 {
  return (a + b) / 2;
}

fn main() -> u64 {
  return average(40, 44);
}
```

```text
$ bastioc0 average.bastio
average.bastio:2:13: error[E-VERIFY-OVERFLOW]: could not prove that `a + b` does not overflow u64
 2 |   return (a + b) / 2;
   |             ^
   = counterexample: a = 18446744073709551615 (2^64 - 1), b = 1
   = bound the operands with a `requires` or an `if`, or use `wrap_add`, `wrap_sub` or `wrap_mul` if wrapping around is meant
```

This is the central idea of Bastio. `u64` arithmetic never wraps around and never traps: instead, the compiler must prove, before the program runs, that it cannot go out of range. Here it cannot, and it says why: the **counterexample** is a call, `average(2^64 - 1, 1)`, for which `a + b` overflows. The program is rejected even though `main` only ever passes 40 and 44, because `average` is checked on its own, for every argument it could be given.

There are two ways out. Change the code so that it cannot overflow (as in the first version), or narrow what `average` accepts:

```bastio
// expect-exit: 42
fn average(a: u64, b: u64) -> u64
  requires a <= 1000000 && b <= 1000000
{
  return (a + b) / 2;
}

fn main() -> u64 {
  return average(40, 44);
}
```

`requires` is a **precondition**. Inside `average` the compiler now assumes it, so `a + b` is proved not to overflow. In exchange, every caller must prove it: `average(40, 44)` is fine, and a call the compiler cannot show meets it is rejected with `E-VERIFY-REQUIRES`.

The same goes for subtraction (`a - b` needs `a >= b`), multiplication, and division and remainder (the divisor must not be zero). If you write a check yourself, the proof uses it:

```bastio
// expect-exit: 3
fn ratio(a: u64, b: u64) -> u64 {
  if b == 0 {
    return 0;
  }
  return a / b;          // fine: b != 0 here
}

fn main() -> u64 {
  return ratio(9, 3);
}
```

None of this costs anything when the program runs. The C that `bastioc0` emits for `ratio` contains the `if` you wrote and a plain division; there are no hidden checks, because everything that would have needed one was proved.

## 3. Owning memory

A `Buf` is a buffer of `u64` on the heap. `alloc(n)` makes one, filled with zeros, and `free(b)` releases it. A `Buf` is **linear**: every buffer must be used up exactly once, on every path through the program. Forgetting to free it is a compile error:

```bastio
// expect-error: E-LINEAR-LEAK
fn main() -> u64 {
  let b = alloc(8);
  let n = len(b);
  return n;
}
```

```text
leak.bastio:4:3: error[E-LINEAR-LEAK]: `b` is still owned at this `return`
 4 |   return n;
   |   ^^^^^^
   = every linear value must be consumed exactly once: return it, pass it to a function that takes it by value, or `free` it
```

So is using it after it is gone (`E-LINEAR-USE`), which rules out use after free and double free:

```bastio
// expect-error: E-LINEAR-USE
fn main() -> u64 {
  let b = alloc(8);
  free(b);
  return len(b);
}
```

and so is freeing it on one path but not another:

```bastio
// expect-error: E-LINEAR-BRANCH
fn main() -> u64 {
  let b = alloc(8);
  if len(b) > 4 {
    free(b);
  }
  return 0;
}
```

How do you work with a buffer without using it up? Three ways:

- **Lend it read-only.** A parameter of type `&Buf` borrows the buffer for the duration of the call. You pass the buffer itself, `len(b)`, and you still own it afterwards.
- **Pass it through.** `set(b, i, v)` takes the buffer and gives it back with element `i` changed, so you write `b = set(b, i, v)`. (It is updated in place; nothing is copied.)
- **Lend it mutably.** A parameter of type `&mut Buf` is lent with `&mut b`, and the variable must be declared `let mut`. The function can change the buffer, or even replace it, but must leave a buffer there when it returns.

```bastio
// expect-exit: 12
fn fill(b: &mut Buf, v: u64)
  ensures len(b) == len(old(b))
{
  let mut i = 0;
  while i < len(b) {
    b = set(b, i, v);
    i = i + 1;
  }
}

fn main() -> u64 {
  let mut b = alloc(4);
  fill(&mut b, 3);
  let x = get(b, 3);   // in bounds: fill promised to keep the length
  free(b);
  if x == 3 {
    return 12;
  }
  return 0;
}
```

`fill`'s `ensures` line is a promise to its callers that the length does not change, and the compiler checks that `fill` keeps it. Here `main` would know it even without the promise: the compiler sees that `fill` changes `b` only with `set`, which keeps the length. For any other fact about what a function did, such as the value it returned, the caller knows only what an `ensures` says; section 5 explains why. Borrows exist only as parameters: you cannot store a borrow in a variable, return one or put one in a struct. That is what lets Bastio do without lifetime annotations. A borrow can never outlive the call it was made for.

Other heap types follow the same rules: `Bytes` (byte buffers), `Vec[T]` (growable arrays), `Box[T]` (pointers to a single value), and any struct or enum that holds one of them.

## 4. Loops and invariants

Reading a buffer needs an index, and every index must be proved in bounds. Here is a loop with an off-by-one bug:

```bastio
// expect-error: E-VERIFY-BOUNDS
fn sum(b: &Buf) -> u64 {
  let mut total = 0;
  let mut i = 0;
  while i <= len(b) {
    total = total + get(b, i);
    i = i + 1;
  }
  return total;
}

fn main() -> u64 {
  let b = alloc(3);
  let s = sum(b);
  free(b);
  return s;
}
```

```text
sum.bastio:5:21: error[E-VERIFY-BOUNDS]: could not prove that the index `i` is within `b` (`i < len(b)`)
 5 |     total = total + get(b, i);
   |                     ^^^
   = counterexample: len(b) = 0, i = 0, total = 0
   = check the index against the length first, with an `if` or a `requires`, or keep it in range with a loop `invariant`
```

With `i < len(b)` the index is fine. But that is not enough here: `total` could still overflow, since the compiler knows nothing about the values in the buffer. Count something instead, and a new problem shows up:

```bastio
// expect-error: E-VERIFY-OVERFLOW
fn count_zeros(b: &Buf) -> u64 {
  let mut n = 0;
  let mut i = 0;
  while i < len(b) {
    if get(b, i) == 0 {
      n = n + 1;
    }
    i = i + 1;
  }
  return n;
}

fn main() -> u64 {
  let b = alloc(3);
  let n = count_zeros(b);
  free(b);
  return n;
}
```

```text
count.bastio:6:13: error[E-VERIFY-OVERFLOW]: could not prove that `n + 1` does not overflow u64
 6 |       n = n + 1;
   |             ^
   = counterexample: len(b) = 39, i = 38, n = 18446744073709551615 (2^64 - 1)
   = bound the operands with a `requires` or an `if`, or use `wrap_add`, `wrap_sub` or `wrap_mul` if wrapping around is meant
```

`n` counts at most one per element, so it cannot overflow; why can't the compiler see that? Because it does not run the loop. It looks at one iteration, starting from *any* state the loop could be in, and in "any state" `n` can be as large as it likes. You describe the states the loop can actually be in with an **invariant**, a condition that holds every time the loop is about to test its condition:

```bastio
// expect-exit: 3
fn count_zeros(b: &Buf) -> u64 {
  let mut n = 0;
  let mut i = 0;
  while i < len(b)
    invariant i <= len(b) && n <= i
  {
    if get(b, i) == 0 {
      n = n + 1;
    }
    i = i + 1;
  }
  return n;
}

fn main() -> u64 {
  let b = alloc(3);
  let n = count_zeros(b);
  free(b);
  return n;
}
```

The compiler proves two things about it: it holds when the loop is first reached (`0 <= len(b)` and `0 <= 0`), and if it holds before an iteration, it holds after (`n` and `i` grow together, and `i` stops at `len(b)`). Then it may assume the invariant at the top of every iteration, and `n <= i <= len(b)` is far below 2^64. If you write an invariant that is not true, it is rejected with `E-VERIFY-INVARIANT` and a counterexample.

Finding an invariant is usually a matter of saying what the loop has done so far: "`n` is at most the number of elements looked at". You never need one for a loop that only uses its index to read a buffer; the compiler works out that the index stays in bounds from the `while` condition, and that a buffer updated only with `b = set(b, ...)` keeps its length, as in `fill` above.

## 5. Contracts between functions

Use `count_zeros` (with its invariant) to compute how many elements are *not* zero:

```bastio
// expect-error: E-VERIFY-UNDERFLOW
fn count_zeros(b: &Buf) -> u64 {
  let mut n = 0;
  let mut i = 0;
  while i < len(b)
    invariant i <= len(b) && n <= i
  {
    if get(b, i) == 0 {
      n = n + 1;
    }
    i = i + 1;
  }
  return n;
}

fn main() -> u64 {
  let mut b = alloc(5);
  b = set(b, 1, 7);
  let zeros = count_zeros(b);
  let nonzero = len(b) - zeros;
  free(b);
  return nonzero;
}
```

```text
nonzero.bastio:19:24: error[E-VERIFY-UNDERFLOW]: could not prove that `len(b) - zeros` does not go below zero
 19 |   let nonzero = len(b) - zeros;
    |                        ^
    = counterexample: len(b) = 5, zeros = 6
    = check the operands with an `if` or a `requires` first, or use `wrap_sub` if wrapping around is meant
```

The compiler checks each function **on its own**. When `main` calls `count_zeros`, it does not look inside; it only knows what the function promises, and so far it promises nothing beyond returning a `u64`, which could be 6. The promise goes in an `ensures` clause, a **postcondition**, where `result` names the value returned:

```bastio
// expect-exit: 1
fn count_zeros(b: &Buf) -> u64
  ensures result <= len(b)
{
  let mut n = 0;
  let mut i = 0;
  while i < len(b)
    invariant i <= len(b) && n <= i
  {
    if get(b, i) == 0 {
      n = n + 1;
    }
    i = i + 1;
  }
  return n;
}

fn main() -> u64 {
  let mut b = alloc(5);
  b = set(b, 1, 7);
  let zeros = count_zeros(b);
  let nonzero = len(b) - zeros;
  free(b);
  return nonzero;
}
```

Now `count_zeros` must prove `result <= len(b)` at its `return` (it follows from the invariant), and `main` may rely on it.

This is how a whole Bastio program is verified: each function is checked against its `requires` and `ensures`, and each call against the callee's. Checking stays fast however large the program gets, and a function's contract is also its documentation. You do not have to say everything a function does, only what its callers need to know to be safe. (You can say more, and then it is proved too. Proving that a function computes the right answer is optional in Bastio; proving that it is memory safe is not.)

Contracts can use `forall`. This one says that every element of the result is below a bound, so a caller can add elements up without overflow:

```bastio
// expect-exit: 9
fn clamp(b: Buf, max: u64) -> Buf
  ensures len(result) == len(b)
  ensures forall i: u64 :: i < len(result) ==> get(result, i) <= max
{
  let mut r = b;
  let mut i = 0;
  while i < len(r)
    invariant i <= len(r)
    invariant forall k: u64 :: k < i ==> get(r, k) <= max
  {
    if get(r, i) > max {
      r = set(r, i, max);
    }
    i = i + 1;
  }
  return r;
}

fn main() -> u64 {
  let mut b = alloc(3);
  b = set(b, 0, 100);
  b = set(b, 2, 4);
  b = clamp(b, 5);
  let s = get(b, 0) + get(b, 1) + get(b, 2);   // at most 15: no overflow
  free(b);
  return s;
}
```

`==>` means "implies", and `len(b)` in the `ensures` means the length of `b` as it was passed in.

## 6. Data types

Structs and enums work much as in Rust; the [reference](language.md#structs) has the details. A struct that owns a buffer is linear itself. An enum is the usual way to return "a value, or a reason there isn't one", and `match` must handle every variant:

```bastio
// expect-exit: 20
struct Stack { items: Buf, n: u64 }

enum Popped { Value(u64), Empty }

fn push(s: &mut Stack, v: u64) -> bool
  ensures len(s.items) == len(old(s).items) && s.n <= len(s.items)
  requires s.n <= len(s.items)
{
  if s.n == len(s.items) {
    return false;                              // full
  }
  s.items = set(s.items, s.n, v);
  s.n = s.n + 1;
  return true;
}

fn pop(s: &mut Stack) -> Popped
  requires s.n <= len(s.items)
  ensures len(s.items) == len(old(s).items) && s.n <= len(s.items)
{
  if s.n == 0 {
    return Popped::Empty;
  }
  s.n = s.n - 1;
  return Popped::Value(get(s.items, s.n));
}

fn main() -> u64 {
  let mut s = Stack { items: alloc(2), n: 0 };
  push(&mut s, 10);
  push(&mut s, 20);
  let full = push(&mut s, 30);                 // false: no room
  let mut total = 0;
  match pop(&mut s) {
    Popped::Value(v) => { total = v; },        // 20
    Popped::Empty => {},
  }
  if full {
    total = 0;
  }
  free(s.items);                               // frees the stack
  return total;
}
```

`old(s)` in an `ensures` is the value `s` had when the function was called; `s` alone is the value handed back. Generic types (`Option[T]`), `Box[T]` for recursive data and `Vec[T]` are in the reference.

## 7. Reading the compiler's errors

Every error has the same shape: where, a code, a sentence, the source line, and either a counterexample or a hint. The code tells you what kind of problem it is:

| Code starts with | Meaning | What to do |
| --- | --- | --- |
| `E-PARSE`, `E-TYPE` | Not a valid program | As in any compiler |
| `E-LINEAR-` | A value is leaked, used twice or used after it is gone | Find the path that leaks or reuses it; free, return or pass it on exactly once on every path |
| `E-VERIFY-` | Something could go wrong for some input | Read the counterexample |

For an `E-VERIFY-` error, the counterexample is a state the compiler believes the program can reach in which the operation fails. Ask whether that state is really possible:

- **Yes, it can happen.** It is a bug: an index one past the end, a missing check for zero. Fix the code. The counterexample is often the test case you would have written.
- **No, the caller never does that.** Then say so with a `requires`, and the compiler will check that the callers really don't.
- **No, an earlier function guarantees it.** Then that function needs an `ensures` saying so. Values from a call are known only through the callee's `ensures`, as in section 5.
- **No, the loop can't get there.** Then the loop needs an invariant, as in section 4. Inside and after a loop, variables the loop changes are known only through its invariants. A counterexample with absurd values (a counter near 2^64 after a short loop) usually means a missing invariant.

Some errors have a hint instead of a counterexample. The one to know is `the solver gave up (its work limit, or quantifiers)`: Z3 could neither prove the fact nor find a counterexample within its fixed budget of work. That is not a timeout, and running the compiler again will not change the answer. It usually means the fact needs a step Z3 does not find on its own, often one involving `forall`. Help it with an `assert` of an intermediate fact, or a stronger invariant.

`bastioc0 --dump-smt` prints the questions the compiler sends to Z3, if you want to see exactly what it was asked. [Common proof failures](proof-failures.md) goes through the errors people meet first, one at a time, with a fix for each.

## 8. Calling Bastio from C

Bastio compiles to C, so a proved Bastio function can replace one piece of a C program, such as a parser or decoder that reads untrusted input. This section builds the smallest such library, [`examples/lines`](../examples/lines). It is a line counter whose C side reads standard input and asks Bastio about it.

A function marked `export` is also a C function. Here is a first try at one that measures the line starting at a given offset:

```bastio
// expect-error: E-EXPORT
enum Line { Len(u64), OutOfRange }

export fn line_len(b: &Bytes, start: u64) -> Line
  requires start <= blen(b)
{
  let mut i = start;
  while i < blen(b)
    invariant start <= i && i <= blen(b)
  {
    if bget(b, i) == 10 {
      return Line::Len(i - start);
    }
    i = i + 1;
  }
  return Line::Len(i - start);
}
```

```text
$ bastioc0 lines.bastio
lines.bastio:4:18: error[E-EXPORT]: `line_len` is exported, so it cannot have a precondition
 4 |   requires start <= blen(b)
   |                  ^
   = a C caller cannot be proved to meet it; check the condition in the function and return an error instead
```

Inside Bastio, a `requires` is checked at every call. A C caller is not checked, so the compiler will not take its word for it. The function has to check `start` itself, and the check becomes ordinary code that is proved like the rest. Here is the whole library:

<!-- file: examples/lines/lines.bastio -->
```bastio
// expect-ok
enum Line { Len(u64), OutOfRange }

/// How many times `c` occurs in `b`.
export fn count_byte(b: &Bytes, c: u8) -> u64
  ensures result <= blen(b)
{
  let mut n = 0;
  let mut i = 0;
  while i < blen(b)
    invariant i <= blen(b) && n <= i
  {
    if bget(b, i) == c {
      n = n + 1;
    }
    i = i + 1;
  }
  return n;
}

/// The length of the line that starts at offset `start` of `b`, not
/// counting its '\n'. The last line need not end in one.
export fn line_len(b: &Bytes, start: u64) -> Line
  ensures result is Line::Len ==> start + result.Len <= blen(b)
{
  if start > blen(b) {
    return Line::OutOfRange;
  }
  let mut i = start;
  while i < blen(b)
    invariant start <= i && i <= blen(b)
  {
    if bget(b, i) == 10 {
      return Line::Len(i - start);
    }
    i = i + 1;
  }
  return Line::Len(i - start);
}
```

`&Bytes` is a read-only byte buffer; at the C boundary it becomes a pointer and a length. Build the library into C with a header:

```text
$ bastioc0 --emit-c --header lines.h -o lines.c lines.bastio
bastioc0: lines.bastio: 15 proof obligation(s) discharged
```

The header declares the two functions and copies each `ensures` into a comment above it. Those promises are proved, so C code may rely on them:

```c
typedef struct bst_e_Line bst_e_Line;
struct bst_e_Line { uint64_t tag; union { uint64_t Len; } u; };
enum { bst_e_Line_Len = 0, bst_e_Line_OutOfRange = 1 };

/* ensures result <= blen(b) */
uint64_t count_byte(const uint8_t *b, size_t b_len, uint8_t c);

/* ensures (result is Line::Len) ==> ((start + result.Len) <= blen(b)) */
bst_e_Line line_len(const uint8_t *b, size_t b_len, uint64_t start);
```

The C program is ordinary C:

<!-- file: examples/lines/main.c -->
```c
/* Reads standard input and prints how many lines it has and how long the
 * longest one is, using the two proved functions in lines.bastio. */
#include <stdio.h>
#include <stdlib.h>
#include "lines.h"

int main(void) {
    size_t cap = 1 << 16, len = 0, got;
    uint8_t *buf = malloc(cap);
    if (!buf) return 1;
    while ((got = fread(buf + len, 1, cap - len, stdin)) > 0) {
        len += got;
        if (len == cap) {
            uint8_t *bigger = realloc(buf, cap *= 2);
            if (!bigger) { free(buf); return 1; }
            buf = bigger;
        }
    }

    uint64_t lines = count_byte(buf, len, '\n');
    uint64_t longest = 0, start = 0;
    while (start < len) {
        bst_e_Line l = line_len(buf, len, start);
        if (l.tag != bst_e_Line_Len) break;   /* cannot happen: start <= len */
        if (l.u.Len > longest) longest = l.u.Len;
        start += l.u.Len + 1;                 /* the header promises start + Len <= len */
    }
    printf("lines: %llu, longest: %llu bytes\n",
           (unsigned long long)lines, (unsigned long long)longest);
    free(buf);
    return 0;
}
```

```text
$ cc -c -w lines.c && cc -o lines main.c lines.o
$ printf 'one\ntwo\nthree\n' | ./lines
lines: 3, longest: 5 bytes
```

The header's opening comment lists what the proofs assume about the C caller and cannot check, for example that the pointer is readable for the length passed with it. Nothing else is trusted: the generated C has no runtime checks, and the wrapper that turns a pointer and a length into a Bastio buffer is generated too. The rules for what may cross the boundary are in the reference, under [Calling Bastio from C](language.md#calling-bastio-from-c).

`examples/lines/run.sh` builds this example under AddressSanitizer and UBSan and checks its output, and CI runs it. The library and the C program shown above are checked to be the same as the files in the repository. Larger libraries for untrusted input, each with a differential fuzz against a C implementation and a benchmark, are [base64](../examples/base64), [UTF-8](../examples/utf8) and [DER](../examples/der).

## Where to go next

- [Common proof failures](proof-failures.md) shows the errors you are likely to meet next and how to fix each.
- The [language reference](language.md) has every type, builtin and error code, and the [soundness scope](language.md#soundness-scope): exactly what is and is not guaranteed.
- `tests/pass/` has larger programs, and `tests/fail/` one program for each rule, each rejected with the error it breaks.
- [ROADMAP.md](../ROADMAP.md) says where the language is going: a compiler written in Bastio, concurrency, and the BastiOS kernel.
