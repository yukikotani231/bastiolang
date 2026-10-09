# Bastio language reference (v0)

v0 is the smallest language that shows the proof pipeline working end to end: four types and six built-in functions.

## Types

| Type | Meaning |
| --- | --- |
| `u64` | Unsigned 64-bit integer. Arithmetic is proved to stay in range; it never wraps |
| `bool` | Boolean |
| `Buf` | An owned buffer of u64 elements. **Linear**: must be consumed exactly once |
| `&Buf` | A read-only borrow. Allowed **only as a parameter type**, so it can never be returned or stored, and never outlives the call |

## Built-in functions

| Function | Precondition (proved) | Effect |
| --- | --- | --- |
| `alloc(n: u64) -> Buf` | `n <= 2^60` | A buffer of length n, all zeros |
| `len(b: &Buf) -> u64` | none | Length |
| `get(b: &Buf, i: u64) -> u64` | `i < len(b)` | Element i |
| `set(b: Buf, i: u64, v: u64) -> Buf` | `i < len(b)` | Consumes b and returns it with element i set to v (updated in place) |
| `free(b: Buf)` | none | Consumes and frees b |
| `print(v: u64)` | none | Prints one line to stdout (for debugging) |

## Statements and expressions

```
fn name(p: T, mut q: T) -> R
  requires <bool expr>     // the caller proves it
  ensures  <bool expr>     // proved at every return; `result` is the return value
{ ... }

let x = e;   let mut x: T = e;   x = e;
if c { ... } else { ... }
while c invariant <expr> invariant <expr> { ... }
return e;   assert <expr>;
```

- Operators: `+ - * / %`, `== != < <= > >=`, `&& || !`. `&&` and `||` short-circuit.
- A name cannot be redeclared (no shadowing) within a function.
- The entry point is `fn main() -> u64`; its result is the process exit code.

## Specification expressions (ghost)

Expressions in `requires` / `ensures` / `invariant` / `assert` exist only at compile time and generate no code.

- They are evaluated over mathematical integers (`a - b` may be negative).
- They may use `==>` (implication) and `forall x: u64 :: expr`.
- The only functions they may call are `len` and `get`.
- They may mention buffers that have already been consumed. In `ensures`, a parameter name refers to **its value on entry** to the function.

## Linearity rules (error codes)

| Code | Rule |
| --- | --- |
| `E-LINEAR-USE` | A consumed buffer is used; one call both borrows and moves the same buffer; a borrow is returned; a buffer is consumed where the code may run more than once or not at all (a `while` condition, the right side of `&&` / `\|\|`) |
| `E-LINEAR-LEAK` | A buffer is still owned at the end of its scope or at a `return`; an owned buffer is overwritten |
| `E-LINEAR-BRANCH` | Only one arm of an `if` consumes a buffer; a loop body changes the ownership of an outer buffer |

## Verification (error codes)

The compiler executes each function symbolically. At every point where the program could go wrong, it asks Z3 whether the bad case is reachable, and if it is, rejects the program with a counterexample.

| Code | What is proved |
| --- | --- |
| `E-VERIFY-BOUNDS` | `get` / `set` index < length |
| `E-VERIFY-OVERFLOW` | `+` and `*` results < 2^64 |
| `E-VERIFY-UNDERFLOW` | `a - b` has a >= b |
| `E-VERIFY-DIVZERO` | the divisor of `/` and `%` is not zero |
| `E-VERIFY-ALLOC` | allocation size <= 2^60 elements |
| `E-VERIFY-REQUIRES` | the callee's precondition |
| `E-VERIFY-ENSURES` | the function's own postcondition |
| `E-VERIFY-INVARIANT` | a loop invariant holds on entry and is preserved by one iteration |
| `E-VERIFY-ASSERT` | an `assert` |

How verification works:
- **Modular.** A call uses only the callee's `requires` / `ensures`, never its body.
- **Loops** are cut with their invariants. After a loop, the variables the body assigns are known only through the invariants.
- **Annotation inference (1).** A buffer that a loop body only ever updates as `x = set(x, ..)` keeps its length across the loop, so `invariant len(x) == ...` is not needed.
- **Annotation inference (2).** A condition the programmer already tests, as in `if i < len(b) { get(b, i) }`, is used by the proof directly.

## Soundness scope (what v0 does not guarantee)

- **Termination.** Infinite loops and infinite recursion are not detected. This is independent of memory safety and is a non-goal.
- **Out of memory.** `alloc` aborts when memory runs out. Kernel primitives will return allocation failure in the type instead.
- **Trusted base.** Z3, the stage0 compiler (Rust) and the C compiler are trusted.
- **Argument evaluation order.** C leaves it unspecified. In v0 the only side effects are `print`, `alloc` and `free`, and linearity confines them to distinct buffers, so safety is unaffected; output order is not guaranteed.
- **Cross-check in the tests.** `tests/run.sh` also runs every passing program under AddressSanitizer and UBSan to confirm that the proofs match runtime behaviour. Those checks exist only in the test build, never in generated code.
