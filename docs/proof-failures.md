# Common proof failures

This page collects the proof failures people meet first, each with the program that triggers it, what the compiler prints, why, and a fix. It assumes you have read the [tutorial](tutorial.md), which explains requires, ensures and invariants. Each fix is one of four moves:

- **Fix a real bug.** The counterexample is a state the program can reach.
- **Check first.** Add an `if` that rules the state out, so the code after it is safe.
- **State an assumption.** Add a `requires` and let the compiler check every caller.
- **Pass on a fact.** Add an `ensures` or an `invariant`, so a fact the compiler cannot see at a call or after a loop is carried there.

As in the tutorial, every program on this page is compiled by `tests/run.sh`, and the error messages are what the compiler prints. Only the values in a counterexample can differ between Z3 versions.

| Error | Sections |
| --- | --- |
| `E-VERIFY-UNDERFLOW` | [1](#1-the-last-element-of-a-buffer-that-may-be-empty) |
| `E-VERIFY-BOUNDS` | [2](#2-looking-at-the-next-element), [5](#5-a-value-that-comes-back-from-a-call) |
| `E-VERIFY-INVARIANT` | [3](#3-a-loop-that-steps-by-more-than-one) |
| `E-VERIFY-REQUIRES` | [4](#4-a-caller-that-cannot-meet-a-requires) |
| `E-VERIFY-ENSURES` | [6](#6-a-promise-about-every-element) |
| `E-VERIFY-OVERFLOW` | [7](#7-multiplying-before-dividing) |
| `E-VERIFY-CAST` | [8](#8-narrowing-a-number) |
| `E-VERIFY-DIVZERO` | [9](#9-dividing-by-a-value-read-from-memory) |
| `E-LINEAR-LEAK` | [10](#10-an-early-return-that-leaks) |
| `E-VERIFY-SHIFT` | [11](#11-a-shift-by-an-unknown-amount) |
| `E-VERIFY-ASSERT` | [12](#12-an-assertion-that-is-not-known) |
| `E-LINEAR-USE` | [13](#13-using-a-value-after-it-was-consumed) |
| `E-MATCH` | [14](#14-a-match-that-skips-a-variant) |
| `E-TYPE` | [15](#15-a-value-of-the-wrong-type) |
| `the solver gave up` | [16](#16-the-solver-gave-up) |

## 1. The last element of a buffer that may be empty

```bastio
// expect-error: E-VERIFY-UNDERFLOW
fn last(b: &Buf) -> u64 {
  return get(b, len(b) - 1);
}

fn main() -> u64 {
  let b = alloc(3);
  let x = last(b);
  free(b);
  return x;
}
```

```text
last.bastio:2:24: error[E-VERIFY-UNDERFLOW]: could not prove that `len(b) - 1` does not go below zero
 2 |   return get(b, len(b) - 1);
   |                        ^
   = counterexample: len(b) = 0
   = check the operands with an `if` or a `requires` first, or use `wrap_sub` if wrapping around is meant
```

This is a real bug: `last` is checked for every buffer it could be given, and an empty one has no last element. **Check first** and decide what an empty buffer means, or, if no caller ever passes one, **state the assumption** with `requires len(b) > 0`:

```bastio
// expect-exit: 0
fn last(b: &Buf) -> u64 {
  if len(b) == 0 {
    return 0;
  }
  return get(b, len(b) - 1);
}

fn main() -> u64 {
  let b = alloc(3);
  let x = last(b);
  free(b);
  return x;
}
```

## 2. Looking at the next element

```bastio
// expect-error: E-VERIFY-BOUNDS
fn rises(b: &Buf) -> u64 {
  let mut n = 0;
  let mut i = 0;
  while i < len(b)
    invariant n <= i
  {
    if get(b, i) < get(b, i + 1) {
      n = n + 1;
    }
    i = i + 1;
  }
  return n;
}

fn main() -> u64 {
  let b = alloc(3);
  let n = rises(b);
  free(b);
  return n;
}
```

```text
rises.bastio:7:20: error[E-VERIFY-BOUNDS]: could not prove that the index `i + 1` is within `b` (`i + 1 < len(b)`)
 7 |     if get(b, i) < get(b, i + 1) {
   |                    ^^^
   = counterexample: len(b) = 1152921504606846976 (2^60), i = 1152921504606846975 (2^60 - 1), n = 0
   = check the index against the length first, with an `if` or a `requires`, or keep it in range with a loop `invariant`
```

Another real bug: in the last iteration `i + 1` is one past the end (here `i = len(b) - 1`; the size of the buffer in the counterexample does not matter). Start one later and look back instead:

```bastio
// expect-exit: 0
fn rises(b: &Buf) -> u64 {
  let mut n = 0;
  let mut i = 1;
  while i < len(b)
    invariant 1 <= i && n <= i
  {
    if get(b, i - 1) < get(b, i) {
      n = n + 1;
    }
    i = i + 1;
  }
  return n;
}

fn main() -> u64 {
  let b = alloc(3);
  let n = rises(b);
  free(b);
  return n;
}
```

The obvious repair, `while i + 1 < len(b)`, is rejected in turn: at the top of the loop the compiler knows only the invariant, which says nothing about how large `i` is, so `i + 1` could overflow. Comparing each element with the one before it needs no addition at all. The invariant now says `1 <= i` too: inside the loop the compiler knows only the invariant and the condition, and without it `i - 1` could go below zero.

## 3. A loop that steps by more than one

```bastio
// expect-error: E-VERIFY-INVARIANT
fn mark_evens(b: &mut Buf)
  ensures len(b) == len(old(b))
{
  let mut i = 0;
  while i < len(b)
    invariant i <= len(b)
  {
    b = set(b, i, 1);
    i = i + 2;
  }
}

fn main() -> u64 {
  let mut b = alloc(5);
  mark_evens(&mut b);
  let x = get(b, 4);
  free(b);
  return x;
}
```

```text
evens.bastio:6:17: error[E-VERIFY-INVARIANT]: could not prove that the loop body preserves the invariant `i <= len(b)`
 6 |     invariant i <= len(b)
   |                 ^
   = counterexample after one iteration: len(b) = 1152921504606846976 (2^60), i = 1152921504606846977 (2^60 + 1)
   = an invariant must hold when the loop starts, and again after any iteration that begins with it and the loop condition true
```

The counterexample is the state after one iteration, as its label says: `i` was `len(b) - 1`, the condition held, and `i + 2` went one past the length. `i <= len(b)` is simply not true of this loop, which ends with `i` one past the end when the length is odd. Write what is true. Here the invariant is not needed at all: the index is in bounds because of the loop condition, and the length is kept because the buffer is changed only with `b = set(b, ...)`. Removing it is enough:

```bastio
// expect-exit: 1
fn mark_evens(b: &mut Buf)
  ensures len(b) == len(old(b))
{
  let mut i = 0;
  while i < len(b) {
    b = set(b, i, 1);
    i = i + 2;
  }
}

fn main() -> u64 {
  let mut b = alloc(5);
  mark_evens(&mut b);
  let x = get(b, 4);
  free(b);
  return x;
}
```

`i + 2` cannot overflow either: a buffer's length is far below 2^64.

## 4. A caller that cannot meet a `requires`

```bastio
// expect-error: E-VERIFY-REQUIRES
fn percent(part: u64, whole: u64) -> u64
  requires whole > 0 && part <= whole && whole <= 1000000
{
  return part * 100 / whole;
}

fn main() -> u64 {
  let mut b = alloc(2);
  b = set(b, 0, 3);
  let p = percent(get(b, 0), get(b, 1));
  free(b);
  return p;
}
```

```text
percent.bastio:10:11: error[E-VERIFY-REQUIRES]: could not prove the precondition `((whole > 0) && (part <= whole)) && (whole <= 1000000)` of `percent`
 10 |   let p = percent(get(b, 0), get(b, 1));
    |           ^^^^^^^
    = counterexample: len(b) = 2, b = [3, 0]
    = `percent` requires it on line 2
    = the caller must make it hold: check it with an `if` before the call, or ask for it in the caller's own `requires`
```

The error is at the call, not in `percent`: `percent` is fine for every input its `requires` allows, and this caller passes values read from a buffer, which could be anything (here `b = [3, 0]`: `0` for `whole`). **Check first**, at the call:

```bastio
// expect-exit: 0
fn percent(part: u64, whole: u64) -> u64
  requires whole > 0 && part <= whole && whole <= 1000000
{
  return part * 100 / whole;
}

fn main() -> u64 {
  let mut b = alloc(2);
  b = set(b, 0, 3);
  let part = get(b, 0);
  let whole = get(b, 1);
  let mut p = 0;
  if whole > 0 && part <= whole && whole <= 1000000 {
    p = percent(part, whole);
  }
  free(b);
  return p;
}
```

## 5. A value that comes back from a call

```bastio
// expect-error: E-VERIFY-BOUNDS
fn middle(b: &Buf) -> u64 {
  return len(b) / 2;
}

fn main() -> u64 {
  let mut b = alloc(4);
  b = set(b, 2, 7);
  let m = middle(b);
  let x = get(b, m);
  free(b);
  return x;
}
```

```text
middle.bastio:9:11: error[E-VERIFY-BOUNDS]: could not prove that the index `m` is within `b` (`m < len(b)`)
 9 |   let x = get(b, m);
   |           ^^^
   = counterexample: len(b) = 4, m = 4
   = check the index against the length first, with an `if` or a `requires`, or keep it in range with a loop `invariant`
```

`middle(b)` is 2 here, so why is the index not known to be in bounds? Because the compiler checks `main` without looking inside `middle`: of a call it knows only what the callee promises, and `middle` promises nothing. **Pass on the fact** with an `ensures`, which the compiler then checks against `middle`'s body. The promise needs a `requires` too, since an empty buffer has no middle:

```bastio
// expect-exit: 7
fn middle(b: &Buf) -> u64
  requires len(b) > 0
  ensures result < len(b)
{
  return len(b) / 2;
}

fn main() -> u64 {
  let mut b = alloc(4);
  b = set(b, 2, 7);
  let m = middle(b);
  let x = get(b, m);
  free(b);
  return x;
}
```

This is what keeps verification fast and each function's contract honest: changing `middle`'s body cannot break `main` unless it breaks the `ensures`. One kind of fact is carried across calls without an `ensures`: when a function changes a `&mut` argument only by updating it in place (such as `b = set(b, i, v)`), the caller still knows its length and every other part the function does not change ([how verification works](language.md#how-verification-works)).

## 6. A promise about every element

```bastio
// expect-error: E-VERIFY-ENSURES
fn max(b: &Buf) -> u64
  ensures forall j: u64 :: j < len(b) ==> get(b, j) <= result
{
  let mut m = 0;
  let mut i = 0;
  while i < len(b)
    invariant i <= len(b)
  {
    if get(b, i) > m {
      m = get(b, i);
    }
    i = i + 1;
  }
  return m;
}

fn main() -> u64 {
  let mut b = alloc(3);
  b = set(b, 1, 9);
  let m = max(b);
  free(b);
  return m;
}
```

```text
max.bastio:14:3: error[E-VERIFY-ENSURES]: could not prove the postcondition `forall j: u64 :: (j < len(b)) ==> (get(b, j) <= result)` of `max` at this return
 14 |   return m;
    |   ^^^^^^
    = counterexample: len(b) = 1152921504606846976 (2^60), i = 1152921504606846976 (2^60), m = 0
    = `max` promises it on line 2
    = every `return` must make it hold for each input that meets the `requires`
```

The function is right, but after the loop the compiler knows only the invariant (`i <= len(b)`) and the negated condition, so `m = 0` is as good as any other value. A loop that builds up a fact about every element it has seen needs an invariant that says so, in the same words as the `ensures`, for the elements before `i`:

```bastio
// expect-exit: 9
fn max(b: &Buf) -> u64
  ensures forall j: u64 :: j < len(b) ==> get(b, j) <= result
{
  let mut m = 0;
  let mut i = 0;
  while i < len(b)
    invariant i <= len(b)
    invariant forall j: u64 :: j < i ==> get(b, j) <= m
  {
    if get(b, i) > m {
      m = get(b, i);
    }
    i = i + 1;
  }
  return m;
}

fn main() -> u64 {
  let mut b = alloc(3);
  b = set(b, 1, 9);
  let m = max(b);
  free(b);
  return m;
}
```

When the loop ends, `i == len(b)`, so the invariant becomes the `ensures`.

## 7. Multiplying before dividing

```bastio
// expect-error: E-VERIFY-OVERFLOW
fn scale(x: u64, num: u64, den: u64) -> u64
  requires den > 0
{
  return x * num / den;
}

fn main() -> u64 {
  return scale(10, 3, 2);
}
```

```text
scale.bastio:4:12: error[E-VERIFY-OVERFLOW]: could not prove that `x * num` does not overflow u64
 4 |   return x * num / den;
   |            ^
   = counterexample: den = 1, num = 2, x = 9223372036854775808 (2^63)
   = bound the operands with a `requires` or an `if`, or use `wrap_add`, `wrap_sub` or `wrap_mul` if wrapping around is meant
```

The division comes too late to help: the product must fit before it is divided. Decide how large the inputs really are and **state it**. A bound on each operand is enough when their product stays below 2^64:

```bastio
// expect-exit: 15
fn scale(x: u64, num: u64, den: u64) -> u64
  requires den > 0 && x <= 4294967295 && num <= 4294967295
{
  return x * num / den;
}

fn main() -> u64 {
  return scale(10, 3, 2);
}
```

`wrap_mul` is not the fix here: it would compile, and give a wrong answer for large inputs.

## 8. Narrowing a number

```bastio
// expect-error: E-VERIFY-CAST
fn digit(n: u64) -> u8 {
  return cast[u8](n) + 48;
}

fn main() -> u64 {
  return cast[u64](digit(7));
}
```

```text
digit.bastio:2:10: error[E-VERIFY-CAST]: could not prove that `n` fits in u8 (`<= 255`)
 2 |   return cast[u8](n) + 48;
   |          ^^^^
   = counterexample: n = 256
   = check the value first, with an `if` or a `requires`, or use `wrap[T](..)` if wrapping around is meant
```

`cast[T](x)` keeps the value, so it must be proved to fit. Here the right bound is tighter than the cast needs, since `n + 48` must also fit in a `u8` and only `0` to `9` are digits:

```bastio
// expect-exit: 55
fn digit(n: u64) -> u8
  requires n <= 9
{
  return cast[u8](n) + 48;
}

fn main() -> u64 {
  return cast[u64](digit(7));
}
```

When you do want the low bits of a larger number, write `wrap[u8](n)`, or `cast[u8](n % 256)`, which is proved to fit.

## 9. Dividing by a value read from memory

```bastio
// expect-error: E-VERIFY-DIVZERO
fn mean(b: &Buf) -> u64
  requires len(b) == 2
{
  return get(b, 0) / get(b, 1);
}

fn main() -> u64 {
  let mut b = alloc(2);
  b = set(b, 0, 8);
  b = set(b, 1, 4);
  let q = mean(b);
  free(b);
  return q;
}
```

```text
mean.bastio:4:20: error[E-VERIFY-DIVZERO]: could not prove that the divisor `get(b, 1)` is not zero
 4 |   return get(b, 0) / get(b, 1);
   |                    ^
   = counterexample: len(b) = 2
   = check that the divisor is not zero first, with an `if` or a `requires`
```

Elements of a buffer can hold anything unless something says otherwise. Read the value once and check it:

```bastio
// expect-exit: 2
fn mean(b: &Buf) -> u64
  requires len(b) == 2
{
  let d = get(b, 1);
  if d == 0 {
    return 0;
  }
  return get(b, 0) / d;
}

fn main() -> u64 {
  let mut b = alloc(2);
  b = set(b, 0, 8);
  b = set(b, 1, 4);
  let q = mean(b);
  free(b);
  return q;
}
```

## 10. An early return that leaks

```bastio
// expect-error: E-LINEAR-LEAK
fn first(n: u64) -> u64
  requires n <= 1000
{
  let b = alloc(n);
  if n == 0 {
    return 0;
  }
  let x = get(b, 0);
  free(b);
  return x;
}

fn main() -> u64 {
  return first(3);
}
```

```text
first.bastio:6:5: error[E-LINEAR-LEAK]: `b` is still owned at this `return`
 6 |     return 0;
   |     ^^^^^^
   = every linear value must be consumed exactly once: return it, pass it to a function that takes it by value, or `free` it
```

Every `return` is a place where everything still owned must already be gone. Free it on that path too:

```bastio
// expect-exit: 0
fn first(n: u64) -> u64
  requires n <= 1000
{
  let b = alloc(n);
  if n == 0 {
    free(b);
    return 0;
  }
  let x = get(b, 0);
  free(b);
  return x;
}

fn main() -> u64 {
  return first(3);
}
```

The same error at the end of an `if` or a `match` arm, instead of a `return`, is `E-LINEAR-BRANCH`: one branch frees the value and another does not.

## 11. A shift by an unknown amount

```bastio
// expect-error: E-VERIFY-SHIFT
fn bit(n: u64) -> u64 {
  return 1 << n;
}

fn main() -> u64 {
  return bit(3);
}
```

```text
bit.bastio:2:12: error[E-VERIFY-SHIFT]: could not prove that the shift amount `n` is less than 64, the width of u64
 2 |   return 1 << n;
   |            ^
   = counterexample: n = 64
   = check the shift amount first, with an `if` or a `requires`
```

In C, shifting a 64-bit value by 64 or more is undefined, so the compiler wants a proof that the amount is below the width. A caller could pass any value, and the counterexample is the smallest one that breaks it. Check the amount first (or state it as a `requires`, as in section 4):

```bastio
// expect-exit: 8
fn bit(n: u64) -> u64 {
  if n >= 64 {
    return 0;
  }
  return 1 << n;
}

fn main() -> u64 {
  return bit(3);
}
```

## 12. An assertion that is not known

```bastio
// expect-error: E-VERIFY-ASSERT
fn main() -> u64 {
  let b = alloc(3);
  assert get(b, 0) == 1;
  free(b);
  return 0;
}
```

```text
check.bastio:3:3: error[E-VERIFY-ASSERT]: could not prove `get(b, 0) == 1`
 3 |   assert get(b, 0) == 1;
   |   ^^^^^^
   = counterexample: len(b) = 3, b = [0, 0, 0]
   = an `assert` must follow from what is known there: the `requires`, the conditions of the enclosing `if`s and loops, and loop invariants
```

An `assert` is a claim the compiler must prove, not a runtime check: nothing is added to the generated code. A fresh buffer holds zeros, so the claim is false here, and the counterexample shows the contents. Make the program establish the fact before it asserts it:

```bastio
// expect-exit: 1
fn main() -> u64 {
  let mut b = alloc(3);
  b = set(b, 0, 1);
  assert get(b, 0) == 1;
  let x = get(b, 0);
  free(b);
  return x;
}
```

## 13. Using a value after it was consumed

```bastio
// expect-error: E-LINEAR-USE
fn main() -> u64 {
  let b = alloc(4);
  free(b);
  return get(b, 0);
}
```

```text
reuse.bastio:4:14: error[E-LINEAR-USE]: `b` is borrowed after it was consumed
 4 |   return get(b, 0);
   |              ^
   = `b` was consumed at 3:8
```

`free` consumes the buffer, so it can no longer be read. This is the ownership rule that section 10 enforces at each `return`; here it is enforced at each use. Use the value first and consume it last:

```bastio
// expect-exit: 0
fn main() -> u64 {
  let b = alloc(4);
  let x = get(b, 0);
  free(b);
  return x;
}
```

## 14. A match that skips a variant

```bastio
// expect-error: E-MATCH
enum Shape { Dot, Line(u64) }

fn size(s: Shape) -> u64 {
  match s {
    Shape::Line(n) => { return n; },
  }
  return 0;
}

fn main() -> u64 {
  return size(Shape::Line(3));
}
```

```text
pick.bastio:4:3: error[E-MATCH]: this `match` does not handle `Shape::Dot`
 4 |   match s {
   |   ^^^^^
```

Every variant of an `enum` needs an arm, so a case cannot be forgotten when a variant is added later. Add the missing arm. (Once every arm of a `match` returns, nothing may follow it.)

```bastio
// expect-exit: 3
enum Shape { Dot, Line(u64) }

fn size(s: Shape) -> u64 {
  match s {
    Shape::Dot => { return 0; },
    Shape::Line(n) => { return n; },
  }
}

fn main() -> u64 {
  return size(Shape::Line(3));
}
```

## 15. A value of the wrong type

```bastio
// expect-error: E-TYPE
fn double(x: u64) -> u64
  requires x <= 1000
{
  return x + x;
}

fn main() -> u64 {
  let b = alloc(1);
  let r = double(b);
  free(b);
  return r;
}
```

```text
mix.bastio:9:18: error[E-TYPE]: argument of `double` must be u64, found Buf
 9 |   let r = double(b);
   |                  ^
```

Types are checked before any proof is attempted, so this error has no counterexample: the message names the expected and the found type. Pass a value of the right type, here the length of the buffer:

```bastio
// expect-exit: 0
fn double(x: u64) -> u64
  requires x <= 1000
{
  return x + x;
}

fn main() -> u64 {
  let b = alloc(1);
  let r = double(len(b));
  free(b);
  return r - 2;
}
```

## 16. The solver gave up

Instead of a counterexample, some errors end with:

```text
   = the solver gave up (its work limit, or quantifiers); add an `assert` or a stronger invariant as a hint
```

Z3 could neither prove the fact nor find a counterexample within a fixed budget of work. The budget is counted in steps, not seconds, so running the compiler again gives the same answer. The fact may well be true; the proof needs a step that Z3 does not take on its own. Two things usually help:

- **Split it.** Before the failing line, `assert` an intermediate fact the goal follows from. Each `assert` is proved on its own and then assumed, so a hard step becomes two easier ones.
- **Remove what the proof does not need.** Facts with `forall` in invariants and `ensures` are the usual cause. Keep each one as narrow as the caller needs.

`bastioc0 --dump-smt` prints every question sent to Z3, if you want to see exactly what it was asked.
