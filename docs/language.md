# Bastio language reference

Bastio is a systems language whose memory safety is proved at compile time. This document describes the language that the bootstrap compiler, `bastioc0` (in `stage0/`), accepts on the current `main` branch. The self-hosted compiler in `stage1/` is being written to accept exactly the same language; see [ROADMAP.md](../ROADMAP.md) for how far it has got and for what is planned.

Bastio is pre-release. Anything here may change.

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
21. [Soundness scope](#soundness-scope)
22. [Not in the language yet](#not-in-the-language-yet)

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
bastioc0 [--check] [--emit-c] [--target hosted|kernel|user] [-o out] <file.bastio>...
```

| Option | Effect |
| --- | --- |
| (none) | Check, verify, emit C and compile it with `cc -O2` into an executable (`a.out` unless `-o` is given) |
| `--check` | Check and verify only |
| `--emit-c` | Write the generated C (`out.c` unless `-o` is given) instead of an executable |
| `--target T` | `hosted` (the default), `kernel` or `user`; see [Targets](#targets-and-generated-code). `kernel` and `user` imply `--emit-c` |
| `--dump-smt` | Also print the queries sent to Z3 |
| `--trusted-base` | Print the hand-written C each target links in, and every builtin with the targets that have it |
| `--dump-tokens`, `--dump-ast`, `--dump-mono`, `--dump-check` | Print the result of one compiler phase. These exist so the self-hosted compiler can be tested against stage0 |

`bastioc0` runs `z3` (found on `PATH`) as a subprocess and `cc` (or `$CC`) to compile the C it emits. Errors are printed with the source line, a caret and, for proof failures, a counterexample. Exit status: 0 on success, 1 when the program is rejected, 2 for a usage or file error, 3 when the C compiler fails (which is a compiler bug).

## Lexical structure

- **Comments** run from `//` to the end of the line. There are no block comments.
- **Identifiers** are letters, digits and `_`, not starting with a digit.
- **Integer literals** are decimal (`4096`) or hexadecimal (`0x1000`, `0X1000`). `_` may separate digits (`1_000_000`, `0xffff_0000`). A literal must fit in 64 bits. There are no negative literals and no unary minus: integers are unsigned.
- **String literals** are written in double quotes and cannot span lines. Escapes: `\n`, `\r`, `\t`, `\0`, `\\`, `\"`.
- **Keywords:** `assert` `const` `else` `ensures` `enum` `false` `fn` `forall` `if` `import` `invariant` `is` `let` `match` `mut` `requires` `return` `spec` `struct` `true` `unreachable` `while`. The names `result`, `old` and `include_bytes`, and the names of builtins, have special meanings described below.

## Programs and files

A program is a list of definitions in any order, except that a constant must be defined before it is used:

```text
fn name[T, ...](p: T, mut q: T, ...) -> R  requires ...  ensures ...  { ... }
spec fn name(p: T, ...) -> bool { <specification expression> }
struct Name[T, ...] { field: T, ... }
enum Name[T, ...] { Variant(T), Variant, ... }
const NAME: u64 = <constant expression>;
```

A function with no `-> R` returns nothing. Parameters and `let` bindings are immutable unless declared `mut`.

**Constants** have type `u64`. The expression is folded at compile time and may use literals, other constants and arithmetic.

**Several files.** A file may begin with `import "path";` lines, before any definition. The path is relative to the importing file. Each imported file is loaded, with its own imports, before the rest of the importing file is read, so its constants can be used there. A file imported more than once is loaded once. Files that import each other, or an import that cannot be read, are `E-IMPORT`. Files named together on the command line also form one program, in command-line order.

All files share one namespace, as in C: a function, type or constant name must be unique in the whole program. There are no modules, qualified names or private items yet.

**The entry point** is `fn main`. It returns `u64` (the exit status on the hosted and user targets) or nothing, and takes as parameters the capabilities its target hands out, each at most once and each as `&mut`:

| Target | `main` may take |
| --- | --- |
| `hosted` | `&mut Serial`, `&mut Pool`, `&mut Host` |
| `kernel` | `&mut Serial`, `&mut Pool`, `&mut Mmu`, `&mut Cpu` |
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
| `u64` | Unsigned 64-bit integer, the only integer type. Arithmetic is proved to stay in range; it never wraps |
| `bool` | `true` or `false` |
| `Str` | A string literal: immutable bytes whose length and contents are known at compile time. Never freed |
| A struct or enum with no linear part | See [Structs](#structs) and [Enums](#enums-and-match) |

**Linear values** must be used exactly once (see [Ownership](#ownership-linear-values-and-borrows)):

| Type | Meaning |
| --- | --- |
| `Buf` | An owned buffer of `u64` |
| `Bytes` | An owned buffer of bytes. Every element is proved to be `<= 255` |
| `Vec[T]` | A growable array of `T` on the heap |
| `Box[T]` | An owning pointer to a `T` on the heap; the only way a type can contain itself |
| A struct with a linear field, an enum with a linear payload | |
| `Host`, `Child` | The host system, and a child process (hosted target only) |
| `Serial`, `Pool`, `Frame`, `PageTable`, `Mmu`, `Cpu` | Capabilities: the console, physical pages, page tables, the MMU and the CPU's user context. See [Kernel capabilities](#kernel-capabilities) |

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

**Names.** A name cannot shadow another that is still in scope (`E-TYPE`). Once a block ends its names can be used again, so two `match` arms can both bind `v`.

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
- A loop's `invariant` clauses must hold on entry and be preserved by one iteration (`E-VERIFY-INVARIANT`). After the loop, the variables the body assigns are known only through the invariants and the negated condition.

**Operators**, loosest first:

| Precedence | Operators | Notes |
| --- | --- | --- |
| 1 | `==>` | Implication; specifications only. Right-associative |
| 2 | `\|\|` | Short-circuit |
| 3 | `&&` | Short-circuit |
| 4 | `== != < <= > >=`, `x is E::V` | Do not chain: `a < b < c` is a syntax error |
| 5 | `+ -` | |
| 6 | `* / %` | |
| 7 | `!` | |
| 8 | `.field` | Also `.Variant` on enums, in specifications |

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

Call arguments, and the fields of a struct literal, are evaluated in an unspecified order.

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
| `vec_pop(v: &mut Vec[T]) -> T` | `vlen(v) > 0` | Removes and returns the last element |
| `vec_get(v: &Vec[T], i: u64) -> T` | `i < vlen(v)`; `T` not linear | A copy of element `i` |
| `vec_set(v: &mut Vec[T], i: u64, x: T)` | `i < vlen(v)`; `T` not linear | Overwrites element `i` |
| `vec_swap(v: &mut Vec[T], i: u64, x: T) -> T` | `i < vlen(v)` | Puts `x` at `i` and returns what was there |
| `vec_free(v: Vec[T])` | for linear `T`, `vlen(v) == 0` (`E-VERIFY-LEAK`) | Frees the storage |

- Elements of a vector of linear values leave only through `vec_pop` and `vec_swap`, and the vector can be freed only when nothing is left, so no per-slot ownership is needed.
- In specifications, `vlen(v)` is the length and `vget(v, i)` is element `i`, of the element type, for any element type. So `vget(v, i).page`, `len(vget(v, i).page)`, `vget(v, i) is State::Used` and `vlen(vget(grid, i))` all work, typically under `forall i: u64 :: i < vlen(v) ==> ...`.
- The prover tracks every element's contents: scalars, struct fields, enum tags and payloads, buffer lengths and contents, ghost state of capabilities, nested vectors. A read gives back what was last written at that index. The exception is boxes inside elements: what a box in an element holds is unknown once the element is in the vector.
- The storage doubles when full. A push that cannot get memory aborts; the prover takes this into account by assuming the length stays below 2^60 after a push.
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
| `blen(b: &Bytes) -> u64` | | Its length |
| `bget(b: &Bytes, i: u64) -> u64` | `i < blen(b)` | Byte `i`; the prover knows it is `<= 255` |
| `bset(b: &mut Bytes, i: u64, v: u64)` | `i < blen(b)`, `v <= 255` | Sets byte `i` in place |
| `bfree(b: Bytes)` | | Frees `b` |
| `bfrom(s: Str) -> Bytes` | | A copy of a string literal; the prover knows its bytes |
| `slen(s: Str) -> u64` | | Length of a string literal |
| `sget(s: Str, i: u64) -> u64` | `i < slen(s)` | Byte `i`; the prover knows the literal's bytes exactly |
| `print(v: u64)` | | Prints `v` in decimal on a line of its own (for debugging) |

`include_bytes("path")` is a `Str` holding a file's contents, read at compile time; the path is relative to the source file.

```bastio
// expect-exit: 5
fn count(s: &Bytes, c: u64) -> u64
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

Hardware is reached only through capability types. There is no way to read or write memory through an integer address (ROADMAP invariant 1). Capabilities are linear, and `main` is handed the ones its target provides; they cannot be created. They carry **ghost state** that exists only in the proof: in the generated code a `Frame` or `PageTable` is a bare physical address, and `Serial`, `Mmu` and `Cpu` are empty.

| Type | Meaning |
| --- | --- |
| `Serial` | The serial console |
| `Pool` | The physical pages the kernel may hand out |
| `Frame` | One physical page (4 KiB), owned by exactly one place at a time |
| `PageTable` | A page-table page at level 1 to 5. It owns the tables and pages its slots point to |
| `Mmu` | The right to load CR3 (kernel only) |
| `Cpu` | The CPU's saved user context and the right to enter user mode (kernel only) |

Ghost accessors, usable in specifications:

| Accessor | Meaning |
| --- | --- |
| `frame_id(f)`, `pt_id(t)` | The page number |
| `pt_level(t)` | 1 (maps pages) to 4 (top level, what CR3 holds); 5 is a container for level-4 tables that is never loaded into CR3 |
| `pt_used(t, i)` | Slot `i` owns something |
| `pool_has(p, i)` | The pool can still hand out page `i` |
| `pool_size(p)` | Number of pages in the pool (also callable at run time) |
| `mmu_root(m)` | The id of the table in CR3; `0x10000` while the kernel's own map is loaded |

**Console** (all targets):

| Builtin | Requires (proved) | Effect |
| --- | --- | --- |
| `serial_write(c: &Serial, b: u64)` | `b <= 255` | Writes one byte |
| `serial_read(c: &Serial) -> u64` | | Reads one byte, blocking; the result is `<= 255`. Hosted builds read standard input and return 4 (EOT) at its end |
| `serial_poll(c: &Serial) -> u64` | | Kernel only. A received byte, or 256 if none is waiting |

**Pages and page tables** (hosted and kernel; hosted builds simulate the pages with ordinary memory, so page code can be tested under the sanitizers):

| Builtin | Requires (proved) | Effect |
| --- | --- | --- |
| `pool_take(p: &mut Pool, i: u64) -> Frame` | `i < pool_size(p)` and `pool_has(p, i)` | Takes page `i` out of the pool, zeroed |
| `pool_give(p: &mut Pool, f: Frame)` | | Gives a page back |
| `frame_id(f: &Frame) -> u64` | | The page number |
| `frame_read(f: &Frame, w: u64) -> u64`, `frame_write(f: &mut Frame, w: u64, v: u64)` | `w < 512` | A 64-bit word of the page. Contents are not tracked by the proof |
| `frame_read8(f: &Frame, o: u64) -> u64`, `frame_write8(f: &mut Frame, o: u64, b: u64)` | `o < 4096`; writing: `b <= 255` | A byte of the page |
| `pt_new(f: Frame, level: u64) -> PageTable` | `1 <= level <= 5` | Turns a page into an empty table. A level-4 table gets the kernel's map in slot 0 (not owned, never touched) |
| `pt_free(t: PageTable) -> Frame` | level is not 4; every slot empty | Turns an empty table back into a page |
| `pt_free_root(m: &Mmu, t: PageTable) -> Frame` | level 4; slots 1-255 empty; `pt_id(t) != mmu_root(m)` | Kernel only. The same for a top-level table, which must not be in CR3 |
| `pt_has(t: &PageTable, i: u64) -> bool` | `i < 512` | Whether slot `i` owns something (`pt_used(t, i)`) |
| `pt_put(t: &mut PageTable, i: u64, c: PageTable)` | level >= 2; slot `i` empty; `pt_level(c) + 1 == pt_level(t)`; at level 4, `1 <= i < 256` | Moves table `c` into slot `i` |
| `pt_take(t: &mut PageTable, i: u64) -> PageTable` | level >= 2; slot `i` used (same slot rule at level 4) | Moves the table out of slot `i`; its level is one less. What its slots hold is unknown to the proof, so code that tears it down tests each slot with `pt_has` |
| `pt_put_frame(t: &mut PageTable, i: u64, f: Frame, writable: bool)` | level 1; slot `i` empty | Maps page `f` in slot `i`, user-accessible |
| `pt_take_frame(t: &mut PageTable, i: u64) -> Frame` | level 1; slot `i` used | Unmaps the page and returns it |

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
      pool_give(&mut pool, pt_take_frame(&mut t1, i));
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

**MMU and CPU** (kernel only; the kernel's platform code provides the trap entry):

| Builtin | Requires (proved) | Effect |
| --- | --- | --- |
| `mmu_switch(m: &mut Mmu, t: &PageTable)` | level 4 | Loads `t` into CR3; `mmu_root(m)` becomes `pt_id(t)` |
| `mmu_kernel(m: &mut Mmu)` | | Loads the kernel's own map |
| `cpu_get(c: &Cpu, r: u64) -> u64` | `r < 20` | Saved user register `r`: rax rbx rcx rdx rsi rdi rbp r8-r15 (0-14), rip (15), rsp (16), rflags (17), error code (18), fault address (19) |
| `cpu_set(c: &mut Cpu, r: u64, v: u64)` | `r < 18` | Sets a saved register. Whatever is stored, user mode is entered in ring 3 with the user segments, IOPL 0 and interrupts on |
| `cpu_run(c: &mut Cpu, m: &Mmu) -> u64` | | Runs the saved context in user mode until the next trap; returns its vector (`<= 255`) |
| `cpu_idle(c: &mut Cpu) -> u64` | | Waits for an interrupt; returns its vector |

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

**System calls** (user target only). Programs built with `--target user` run under BastiOS and have `sys_spawn(b: &Buf, from: u64, to: u64)` (start the program named by elements `from..to` of `b`, proved to be a range within `b`), `sys_wait(pid)`, `sys_kill(pid)`, `sys_yield()`, `sys_getpid()`, `sys_info(k)`, `sys_ps()` and `sys_poweroff()`.

Calling a builtin the target does not have is `E-TARGET`.

## Specifications

Expressions in `requires`, `ensures`, `invariant` and `assert` (and the bodies of `spec fn`s) exist only at compile time and generate no code.

- They are evaluated over **mathematical integers**: `a - b` may be negative and `a + b` cannot overflow.
- They may use `==>` (implication) and `forall x: u64 :: e`.
- They may call `len`, `get`, `slen`, `sget`, `blen`, `bget`, `vlen`, `vget`, the ghost accessors of the capabilities, and `spec fn`s.
- They may use `is` and enum payloads (`o.Some`), and see through boxes.
- They may mention values that have already been consumed. In `ensures`, a by-value parameter means its value on entry, a `&mut` parameter the value handed back, and `old(x)` its value at the call. `result` is the returned value.
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

## How verification works

After the type and linearity checks, the compiler executes each function symbolically. At every point where the program could go wrong it asks Z3 whether the bad case is reachable, and if it is, it rejects the program and shows a counterexample (values of the variables that lead there).

- **Modular.** A call uses only the callee's `requires` and `ensures`, never its body.
- **Loops** are cut with their invariants. After a loop, the variables the body assigns are known only through the invariants and the negated condition.
- **Branches** are followed separately; the condition of an `if` or the variant of a `match` arm is known inside it. So a test the code already makes, as in `if i < len(b) { get(b, i) }`, is enough for the proof.
- **Annotation inference.** A buffer that a loop body only ever updates as `x = set(x, ..)` keeps its length across the loop, so `invariant len(x) == ...` is not needed.
- **Boxes and vectors.** The prover unfolds unknown recursive data two levels deep, and tracks vector contents element by element (see [Boxes](#boxes) and [Vectors](#vectors)).

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
| `E-MATCH` | A `match` misses a variant, matches one twice, or has an arm after `_` |
| `E-LINEAR-USE` | A consumed value (or a moved-out field, or a struct with a field moved out) is used; one call both borrows and moves a value, or lends it twice with one loan `&mut`; a borrow is returned; a value is consumed where the code may run more than once or not at all (a `while` condition, the right side of `&&` / `\|\|`) |
| `E-LINEAR-LEAK` | A linear value is still owned at the end of its scope or at a `return`, is overwritten, or is dropped by `_` |
| `E-LINEAR-BRANCH` | Only one arm of an `if` or a `match` consumes a value; a loop body changes the ownership of an outer value |

Proofs:

| Code | What is proved |
| --- | --- |
| `E-VERIFY-BOUNDS` | Every index is below its limit: buffer, byte, string and vector indices; `vec_pop` on a non-empty vector; page numbers below `pool_size`; words, bytes and slots of a page or table; `cpu_get` / `cpu_set` registers; the `sys_spawn` range |
| `E-VERIFY-LEAK` | A vector of linear elements is empty when it is freed |
| `E-VERIFY-OVERFLOW` | `+` and `*` results < 2^64 |
| `E-VERIFY-UNDERFLOW` | `a - b` has `a >= b` |
| `E-VERIFY-DIVZERO` | The divisor of `/` and `%` is not zero |
| `E-VERIFY-ALLOC` | Allocation size <= 2^60 elements |
| `E-VERIFY-BYTE` | A value stored as a byte (`serial_write`, `bset`, `frame_write8`) is `<= 255` |
| `E-VERIFY-OWN` | A page is taken from the pool at most once; a table slot is empty before `pt_put` and used before `pt_take`; a table is empty before it is freed |
| `E-VERIFY-PT` | Table levels line up; frames go only into level-1 tables; slot 0 of a top-level table (the kernel's map) is never touched |
| `E-VERIFY-MMU` | The table in CR3 is never freed |
| `E-VERIFY-REQUIRES` | The callee's `requires` |
| `E-VERIFY-ENSURES` | The function's own postcondition |
| `E-VERIFY-INVARIANT` | A loop invariant holds on entry and is preserved by one iteration |
| `E-VERIFY-ASSERT` | An `assert` |
| `E-VERIFY-UNREACHABLE` | An `unreachable;` the code can reach |

`E-SOLVER` means Z3 could not be run or gave an answer the compiler did not expect. It is not a verdict on the program.

## Targets and generated code

The compiler emits C, which a C compiler turns into machine code. The C contains no runtime checks: every check was discharged by a proof, and `tests/run.sh` verifies this for a sample program by inspecting the C.

- **`--target hosted`** (the default) builds an ordinary program against the C library. `alloc`, `box` and vectors use `malloc`; running out of memory aborts. `Serial` is standard input and output. `Pool` is simulated with ordinary memory.
- **`--target kernel`** emits freestanding C for bare-metal x86-64 that needs no C library. It provides `bst_kmain`, which initialises COM1 and calls `main`. The boot code that calls `bst_kmain`, and the trap entry code behind `cpu_run` and `cpu_idle`, live in the kernel's own repository (BastiOS). The prelude implements the console with port I/O on COM1, `alloc` and `free` with a fixed 8 MiB arena and an exact-size free list, `print` on the serial port, the pool (every page from the end of the kernel image to the end of RAM, at most 2^16), page-table entry encoding and CR3 loads. Running out of arena memory halts the machine.
- **`--target user`** emits freestanding C for a user program under BastiOS, entered at `bst_umain`. Its prelude implements the console and the `sys_*` builtins as `int 0x80` system calls, and `alloc` with a 64 KiB arena.

On the freestanding targets `Bytes` come from the same arena as `Buf`.

In the generated C, a struct is a C struct passed by value, an enum is a tag and a union, a `Box[T]` is a `T *`, a `Vec[T]` is `{ len, cap, data }`, and ghost state (capability state, specifications, `assert`) leaves nothing behind.

## Soundness scope

What a program that `bastioc0` accepts is guaranteed: no out-of-bounds access, no use after free, no double free, no leak, no integer overflow or underflow, no division by zero, and, in kernel code, no page owned twice, no malformed page table and no freeing of the active one. Plus every `requires`, `ensures`, `invariant` and `assert` the programmer wrote. What is not guaranteed:

- **Termination.** Infinite loops and infinite recursion are not detected. This is independent of memory safety and a non-goal for now.
- **Running out of memory.** `alloc`, `box` and `vec_push` abort when memory runs out. Allocation failure is planned to be returned in the type instead.
- **Stack depth.** Deep recursion can overflow the stack.
- **Page contents.** The proof tracks who owns each page, not what is in it.
- **The trusted base.** Z3, the stage0 compiler (Rust) and the C compiler are trusted, as is the hand-written C prelude each target links in (in `stage0/src/codegen.rs`), and the platform's boot and trap entry code. `bastioc0 --trusted-base` lists every prelude part with its size, the parts each target links, and every builtin; the listing is checked into `tests/trusted_base.txt` and `tests/run.sh` fails when it changes, so growth is reviewed (ROADMAP invariant 3). The small per-type helpers the compiler generates for `Box` and `Vec` are part of the compiler, not of this listing.
- **Evaluation order.** C leaves the order of call arguments unspecified. Linearity keeps side effects on distinct values, so safety is unaffected, but output order is not guaranteed when two calls in one expression both do I/O.
- **Child processes and files.** What another program writes, or what a file contains, is unknown to the proof, beyond each byte being `<= 255`.

`tests/run.sh` also runs every program in `tests/pass/` under AddressSanitizer and UBSan, to confirm that the proofs match what happens at run time. Those checks exist only in the test build, never in generated code.

## Not in the language yet

Planned, in [ROADMAP.md](../ROADMAP.md): concurrency (designed in [concurrency.md](concurrency.md)), integer types narrower than `u64`, `export fn` for calling Bastio from C, more hardware capabilities (`PortIo`, `Mmio`), allocation failure in the type, and a native x86-64 backend. Inference of type arguments, modules with qualified names, and hand-written lemmas for proofs Z3 cannot find are open questions.
