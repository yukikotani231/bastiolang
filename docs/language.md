# Bastio language reference (v0)

v0 is the smallest language that shows the proof pipeline working end to end, plus just enough (string literals and a serial-port capability) to write a kernel shell.

## Types

| Type | Meaning |
| --- | --- |
| `u64` | Unsigned 64-bit integer. Arithmetic is proved to stay in range; it never wraps |
| `bool` | Boolean |
| `Buf` | An owned buffer of u64 elements. **Linear**: must be consumed exactly once |
| `&Buf` | A read-only borrow. Allowed **only as a parameter type**, so it can never be returned or stored, and never outlives the call |
| `Str` | A string literal: immutable bytes with a length known at compile time. Copyable, never freed |
| `Serial` | The capability to use the serial console. **Linear**: it cannot be created, only `main` is handed one, and `main` must return it |
| `&Serial` | A borrowed console, so a helper function can print without taking ownership |

Borrowing at a call site is implicit: passing an owned `b` to a `&Buf` parameter lends it for the duration of the call.

## Built-in functions

| Function | Precondition (proved) | Effect |
| --- | --- | --- |
| `alloc(n: u64) -> Buf` | `n <= 2^60` | A buffer of length n, all zeros |
| `len(b: &Buf) -> u64` | none | Length |
| `get(b: &Buf, i: u64) -> u64` | `i < len(b)` | Element i |
| `set(b: Buf, i: u64, v: u64) -> Buf` | `i < len(b)` | Consumes b and returns it with element i set to v (updated in place) |
| `free(b: Buf)` | none | Consumes and frees b |
| `print(v: u64)` | none | Prints one line to stdout (for debugging) |
| `slen(s: Str) -> u64` | none | Length in bytes |
| `sget(s: Str, i: u64) -> u64` | `i < slen(s)` | Byte i (the prover knows the literal's bytes exactly) |
| `serial_write(c: &Serial, b: u64)` | `b <= 255` | Writes one byte |
| `serial_read(c: &Serial) -> u64` | none | Reads one byte, blocking; the prover knows the result is `<= 255`. Hosted builds return 4 (EOT) at end of input |

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
- The entry point is either `fn main() -> u64`, whose result is the process exit code, or `fn main(con: Serial) -> Serial`, which owns the console and exits with 0.

## Specification expressions (ghost)

Expressions in `requires` / `ensures` / `invariant` / `assert` exist only at compile time and generate no code.

- They are evaluated over mathematical integers (`a - b` may be negative).
- They may use `==>` (implication) and `forall x: u64 :: expr`.
- The only functions they may call are `len`, `get`, `slen` and `sget`.
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
| `E-VERIFY-BOUNDS` | `get` / `set` / `sget` index < length |
| `E-VERIFY-OVERFLOW` | `+` and `*` results < 2^64 |
| `E-VERIFY-UNDERFLOW` | `a - b` has a >= b |
| `E-VERIFY-DIVZERO` | the divisor of `/` and `%` is not zero |
| `E-VERIFY-ALLOC` | allocation size <= 2^60 elements |
| `E-VERIFY-BYTE` | a value passed to `serial_write` is `<= 255` |
| `E-VERIFY-REQUIRES` | the callee's precondition |
| `E-VERIFY-ENSURES` | the function's own postcondition |
| `E-VERIFY-INVARIANT` | a loop invariant holds on entry and is preserved by one iteration |
| `E-VERIFY-ASSERT` | an `assert` |

How verification works:
- **Modular.** A call uses only the callee's `requires` / `ensures`, never its body.
- **Loops** are cut with their invariants. After a loop, the variables the body assigns are known only through the invariants.
- **Annotation inference (1).** A buffer that a loop body only ever updates as `x = set(x, ..)` keeps its length across the loop, so `invariant len(x) == ...` is not needed.
- **Annotation inference (2).** A condition the programmer already tests, as in `if i < len(b) { get(b, i) }`, is used by the proof directly.

## Targets

`bastioc0 --target hosted` (the default) builds a normal program with the C library. `bastioc0 --target kernel` emits freestanding C for bare-metal x86-64 and implies `--emit-c`. That C needs no C library and provides `bst_kmain`, which initialises COM1 and calls `main(con)`; the boot code that calls `bst_kmain` lives in the kernel's own repository (BastiOS). The kernel prelude implements:
- `Serial` with port I/O on COM1;
- `alloc` / `free` with a fixed 8 MiB arena and an exact-size free list;
- `print` on the serial port.

Running out of arena memory halts the machine.

## Soundness scope (what v0 does not guarantee)

- **Termination.** Infinite loops and infinite recursion are not detected. This is independent of memory safety and is a non-goal.
- **Out of memory.** `alloc` aborts when memory runs out. Kernel primitives will return allocation failure in the type instead.
- **Trusted base.** Z3, the stage0 compiler (Rust) and the C compiler are trusted, as is the hand-written C prelude each target links in (in `stage0/src/codegen.rs`; the kernel prelude does port I/O and manages the arena).
- **Argument evaluation order.** C leaves it unspecified. In v0 the only side effects are `print`, `alloc`, `free` and serial I/O, and linearity confines them to distinct buffers, so safety is unaffected; output order is not guaranteed when two calls in one expression both do I/O.
- **Cross-check in the tests.** `tests/run.sh` also runs every passing program under AddressSanitizer and UBSan to confirm that the proofs match runtime behaviour. Those checks exist only in the test build, never in generated code.
