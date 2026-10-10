# Bastio language reference

The language as stage0 implements it today: the v0 proof pipeline, plus `&mut` borrows and the capability types BastiOS is written with (physical pages, page tables, the MMU and the CPU's user context).

## Types

| Type | Meaning |
| --- | --- |
| `u64` | Unsigned 64-bit integer. Arithmetic is proved to stay in range; it never wraps |
| `bool` | Boolean |
| `Buf` | An owned buffer of u64 elements. **Linear**: must be consumed exactly once |
| `Bytes` | An owned buffer of bytes. **Linear**. Every element is proved to be `<= 255` |
| `Host` | The host operating system: command-line arguments and files. **Linear**; hosted programs only, and only `main` is handed one |
| `Child` | A child process started with `host_spawn`, with a pipe to its standard input and one from its standard output. **Linear**: it must be waited for. Hosted only |
| `&T` | A read-only borrow of any `T`. Allowed **only as a parameter type**, so it can never be returned or stored, and never outlives the call. A borrowed plain value (`&u64`) reads as a copy |
| `Str` | A string literal: immutable bytes with a length known at compile time. Copyable, never freed |
| `Serial` | The capability to use the serial console. **Linear**: it cannot be created, only `main` is handed one, and `main` must return it |
| `&Serial` | A borrowed console, so a helper function can print without taking ownership |
| `&mut T` | A mutable borrow of any `T`, again only as a parameter type. Lent with `&mut x`. The callee may move out of the borrow, but must have put a value back by the time it returns |
| `Pool` | The physical pages the kernel may hand out. Linear; only `main` is handed one |
| `Frame` | One physical page (4 KiB). Linear: owned by exactly one place at a time |
| `PageTable` | A page-table page at level 1 to 5. Linear; it owns the tables and pages its slots point to |
| `Mmu` | The right to load CR3. Linear; only `main` (kernel target) is handed one |
| `Cpu` | The CPU's saved user context and the right to enter user mode. Linear; kernel target only |
| `Ports` | The x86 I/O port space: the right to claim each of the 65536 ports once. Linear; only `main` is handed one (kernel and hosted targets; hosted simulates the ports) |
| `PortIo` | A claimed range of I/O ports, owned: the only way to read or write them. Linear |

| `S` (a struct) | A user-defined record. Linear when any field is linear; otherwise a plain value that is copied |
| `E` (an enum) | One of several variants, each with at most one payload. Linear when any payload is linear |
| `Vec[T]` | A growable array of `T` on the heap. Always **linear** |
| `Box[T]` | An owning pointer to a `T` on the heap. Always **linear** (it owns the allocation). The only way a type can contain itself |

Borrowing at a call site is implicit: passing an owned `b` to a `&Buf` parameter lends it for the duration of the call. A `&mut` parameter needs an explicit `&mut b`, and `b` must be declared `mut` (or itself be a `&mut` parameter). One call cannot lend the same variable twice if one of the loans is `&mut`.

The capability types carry **ghost state** that exists only in the proof and costs nothing at run time (a `Frame` or `PageTable` is a bare physical address in the generated code; `Serial`, `Mmu`, `Cpu` and `Ports` are empty; a `PortIo` is its base and count):

| Accessor (specifications only) | Meaning |
| --- | --- |
| `frame_id(f)`, `pt_id(t)` | The page number |
| `pt_level(t)` | 1 (maps pages) to 4 (top level, what CR3 holds); 5 is a container for level-4 tables that is never loaded into CR3 |
| `pt_used(t, i)` | Slot `i` owns something |
| `pool_has(p, i)` | The pool can still hand out page `i` |
| `pool_size(p)` | Number of pages in the pool (also callable at run time) |
| `mmu_root(m)` | The id of the table in CR3; `0x10000` while the kernel's own map is loaded |
| `port_free(p, k)` | Nobody has claimed port `k` (`main` receives every port free) |
| `port_base(io)`, `port_count(io)` | The ports `io` owns: `port_base(io) ..< port_base(io) + port_count(io)`, always within the 65536 ports (also callable at run time) |

## Structs

```
struct Space { root: PageTable, pages: u64 }
struct Proc  { space: Space, regs: Buf, state: u64 }

let p = Proc { space: Space { root: t, pages: 0 }, regs: alloc(18), state: 1 };
let n = p.space.pages;                 // read a scalar field
pt_put(&mut p.space.root, 1, l3);      // lend a field as &mut
let r = sum(p.regs);                   // lend a field as &Buf
let t = p.space.root;                  // move a linear field out ...
p.space.root = t;                      // ... and put one back
```

- A struct has at least one field, and a field cannot be a borrow. A struct cannot contain itself, except through a `Box`.
- A struct with a linear field is **linear**: it must be consumed exactly once, like a `Buf`. A struct without one is a plain value and is copied.
- Ownership is tracked **per place** (`p`, `p.space`, `p.space.root`). Moving a linear field out leaves the rest of the struct usable: its scalar fields can still be read and its other fields used. The struct as a whole cannot be moved, borrowed or returned until the field is put back (`E-LINEAR-USE`). A struct counts as consumed once every linear field in it has been moved out, so `free(s.items)` consumes a `Stack { items: Buf, n: u64 }`.
- Assigning a linear field that still holds a value is `E-LINEAR-LEAK`, as for variables.
- A place cannot be borrowed in a call that also lends an overlapping place as `&mut` (`p` and `p.regs` overlap).
- A `&mut` struct parameter must be whole again when the function returns.
- Struct literals list every field once, in any order. Fields are evaluated in an unspecified order (as with call arguments).
- Specifications can read fields, including through `old(..)` and `result`: `ensures result.n == old(s).n + 1`, `pt_level(s.root) == 1`. The proof tracks every field, so bounds, overflow and capability state work through fields exactly as through variables.
- In generated code a struct is a C struct passed by value; there is no runtime representation of which fields have been moved.

## Enums and `match`

```
enum Opt  { Some(u64), None }
enum Slot { Full(Buf), Vacant }        // linear: a variant owns a buffer

let o = Opt::Some(5);
let s = Slot::Full(alloc(16));
if o is Opt::None { ... }              // test the variant; consumes nothing
match s {
  Slot::Full(b) => { free(b); },       // the arm owns `b` and must consume it
  Slot::Vacant => {},
}
```

- A variant has no payload or exactly one (use a struct for several values). An enum cannot contain itself, except through a `Box`.
- An enum with a linear payload is **linear**: it is consumed exactly once, and dropping it is `E-LINEAR-LEAK`.
- `match` must be **exhaustive**: every variant is handled once, or a final `_` arm takes the rest. A missing variant, a variant matched twice, or an arm after `_` is `E-MATCH`.
- Matching an owned value moves it into the arms. A pattern `E::V(x)` binds the payload to `x` (`E::V(mut x)` to change it), owned by the arm: a linear payload must be consumed there. `E::V(_)` and `_` drop the payload, which is `E-LINEAR-LEAK` for a linear one.
- Matching a borrowed place (a `&E` parameter, or a payload bound out of one) only inspects it. A linear payload is then bound as a shared borrow: it can be read, lent to `&T` parameters and matched again, but not moved (`E-LINEAR-USE`) or declared `mut`. Through a `&mut E` parameter the match moves the value out, and the function must put a new one back.
- The arms must agree on what they consume (`E-LINEAR-BRANCH`), like the arms of an `if`.
- `x is E::V` is a `bool` that reads only the tag. Its operand must be a place or a non-linear value, since it consumes nothing.
- Specifications can use `is`, and `o.V` means the payload of variant `V`. It is meaningful only when `o is E::V`, so write it under an implication: `ensures result is Opt::Some ==> result.Some <= a`. The prover knows which arm of a `match` it is in, so inside `Opt::Some(v) => ...` it knows `v` is `o.Some`.
- Enum values cannot be built in a specification.
- In generated code an enum is a C struct holding a tag and a union of the payloads. A `match` is an `if` chain on the tag; the last arm needs no test because the match is exhaustive.

## Boxes

```
struct Node { l: Tree, r: Tree }
enum Tree { Leaf(u64), Branch(Box[Node]) }    // recursive through the box

let t = Tree::Branch(box(Node { l: Tree::Leaf(1), r: Tree::Leaf(2) }));
fn leftmost(t: &Tree) -> u64 {
  match t {
    Tree::Leaf(v) => { return v; },
    Tree::Branch(n) => { return leftmost(n.l); },   // `n` is a borrow
  }
}
let n = unbox(b);        // frees the box, hands back what it held
```

- `box(e)` moves `e` into a new heap allocation; `unbox(b)` consumes the box, frees the allocation and returns the value.
- A box is linear whatever it holds: `box(7)` must still be unboxed.
- Fields of a boxed struct are places, as if the box were not there: `b.n`, `b.items = ..`, `&mut b.l`, `b.l` lent to a `&T` parameter. Ownership is tracked through the box as through a struct. The box itself is a separate resource: moving every field out does not consume it, and it cannot be unboxed, moved or borrowed whole while a field is out.
- Specifications see through boxes the same way: `ensures result.n == 0` for a `Box[Stack]` result.
- The prover unfolds a box that arrives from outside (a parameter, a function's result) two levels deep. Deeper contents are unknown until code looks inside them, and then nothing is known about them: a proof that needs a fact deeper down fails (it is never assumed). Values the function builds itself are tracked to any depth.
- `box` uses the same allocator as `alloc`: `malloc` on the hosted target, the arena on the freestanding ones. Running out of memory aborts, as for `alloc`.
- In generated code a `Box[T]` is a `T *`.

## Generics

```
enum Option[T] { Some(T), None }
struct Pair[A, B] { a: A, b: B }

fn swap[A, B](p: Pair[A, B]) -> Pair[B, A] {
  return Pair[B, A] { a: p.b, b: p.a };
}

let p = swap[u64, Buf](Pair[u64, Buf] { a: 1, b: alloc(4) });
let o = Option[u64]::Some(5);
match o { Option::Some(v) => { .. }, Option::None => { .. } }
```

- Structs, enums and functions can take type parameters. Generics are **monomorphized**: each definition is copied once per list of type arguments it is used with (`Option[u64]`, `swap[u64, Buf]`), and the copies are what is checked, verified and compiled. There is no runtime cost and no boxing.
- Type arguments are written at every use: calls (`swap[u64, Buf](..)`), struct literals and variants (`Option[u64]::None`). They are not inferred yet. Patterns and `is` tests may leave them out, since the matched value's type has them.
- A type parameter can stand for any type, linear or not. Each copy is checked on its own, so a generic function that copies or drops its argument works for `u64` and is rejected for `Buf`, with a note naming the copy (like C++ templates, unlike Rust traits).
- Specifications in a generic function are verified for every copy.
- A definition that instantiates itself with ever larger arguments (`struct S[T] { next: Box[S[Box[T]]] }`) is rejected.

## Vectors

```
let mut v = vec_new[u64]();
vec_push(&mut v, 7);
let x = vec_get(v, 0);           // proved: 0 < vlen(v)
vec_free(v);

let mut ps = vec_new[Proc]();    // linear elements
vec_push(&mut ps, p);
let q = vec_swap(&mut ps, 0, p2);   // exchange: the old element comes back
let r = vec_pop(&mut ps);
vec_free(ps);                    // proved: vlen(ps) == 0
```

| Builtin | Meaning |
| --- | --- |
| `vec_new[T]() -> Vec[T]` | An empty vector |
| `vec_len(v: &Vec[T]) -> u64` | Its length (`vlen(v)` in specifications) |
| `vec_push(v: &mut Vec[T], x: T)` | Appends `x`, growing the storage when full |
| `vec_pop(v: &mut Vec[T]) -> T` | Removes and returns the last element. Requires `vlen(v) > 0` |
| `vec_get(v: &Vec[T], i: u64) -> T` | A copy of element `i`. Requires `i < vlen(v)`; only for non-linear `T` |
| `vec_set(v: &mut Vec[T], i: u64, x: T)` | Overwrites element `i`. Requires `i < vlen(v)`; only for non-linear `T` |
| `vec_swap(v: &mut Vec[T], i: u64, x: T) -> T` | Puts `x` at `i` and returns what was there. Requires `i < vlen(v)` |
| `vec_free(v: Vec[T])` | Frees the storage. For linear `T`, requires `vlen(v) == 0` (`E-VERIFY-LEAK`) |

- Every slot below the length holds a value, so a vector of linear values needs no per-slot ownership: elements leave only through `vec_pop` and `vec_swap`, and freeing is allowed only when nothing is left.
- Specifications: `vlen(v)` is the length and `vget(v, i)` is element `i`, of the element type, for any vector. So `vget(v, i).page`, `len(vget(v, i).page)`, `pt_level(vget(v, i).root)`, `vget(v, i) is State::Used` and `vlen(vget(grid, i))` all work, typically under `forall i: u64 :: i < vlen(v) ==> ...`.
- The prover tracks every element's contents: scalars, struct fields, enum tags and payloads, buffer lengths and contents, ghost fields of linear objects, nested vectors. A read gives back what was last written at that index. The exception is boxes inside elements: what a box in an element holds is unknown once the element is in the vector (as for a box that arrives from outside, see Boxes).
- The storage doubles when full. A push that cannot get memory aborts, as `alloc` does; the prover takes this into account by assuming the length stays below 2^60 after a push.
- In generated code a `Vec[T]` is `{ len, cap, data }` and element access is a plain array access, with no bounds checks.

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
| `serial_poll(c: &Serial) -> u64` | none | Kernel only. A received byte, or 256 if none is waiting |
| `balloc(n: u64) -> Bytes` | `n <= 2^60` | n zero bytes |
| `blen(b: &Bytes) -> u64` | none | Length |
| `bget(b: &Bytes, i: u64) -> u64` | `i < blen(b)` | Byte i; the prover knows it is `<= 255` |
| `bset(b: &mut Bytes, i: u64, v: u64)` | `i < blen(b)`, `v <= 255` | Sets byte i in place |
| `bfree(b: Bytes)` | none | Frees b |
| `bfrom(s: Str) -> Bytes` | none | A copy of a string literal (the prover knows its bytes) |
| `host_argc(h: &Host) -> u64` | none | Hosted only. Number of command-line arguments, including the program name |
| `host_arg(h: &Host, i: u64) -> Bytes` | none | Hosted only. Argument i (empty if there is none) |
| `host_read(h: &Host, path: &Bytes, out: &mut Bytes) -> bool` | none | Hosted only. Replaces `out` with the whole file; false (and `out` unchanged) if it cannot be read |
| `host_write(h: &Host, path: &Bytes, data: &Bytes) -> bool` | none | Hosted only. Creates or replaces the file |
| `host_spawn(h: &Host, argv: &Vec[Bytes]) -> Child` | none | Hosted only. Starts `argv[0]` (looked up in `PATH`) with arguments `argv`. A program that cannot be started gives a child that exits with 127 and writes nothing |
| `child_write(c: &mut Child, data: &Bytes) -> bool` | none | Hosted only. Writes to the child's standard input; false if it can no longer be written (the child has exited or the input is closed) |
| `child_close(c: &mut Child)` | none | Hosted only. Closes the child's standard input, so a program that reads to the end of its input can finish |
| `child_read_line(c: &mut Child, out: &mut Bytes) -> bool` | none | Hosted only. Replaces `out` with the next line of the child's output, without the newline; false at the end of the output |
| `child_wait(c: Child) -> u64` | none | Hosted only. Closes the pipes, waits for the child and returns its exit status (128 + the signal number if a signal ended it); the prover knows the result is `<= 255` |

Physical pages and page tables (hosted and kernel; hosted builds simulate the pages with ordinary memory, so tests run page code under the sanitizers):

| Function | Precondition (proved) | Effect |
| --- | --- | --- |
| `pool_take(p: &mut Pool, i: u64) -> Frame` | `i < pool_size(p)` and `pool_has(p, i)` | Takes page i out of the pool, zeroed |
| `pool_give(p: &mut Pool, f: Frame)` | none | Gives a page back |
| `frame_id(f: &Frame) -> u64` | none | The page number |
| `frame_read(f: &Frame, w: u64) -> u64` / `frame_write(f: &mut Frame, w, v)` | `w < 512` | A 64-bit word of the page. Contents are not tracked by the proof |
| `frame_read8(f: &Frame, o: u64) -> u64` / `frame_write8(f: &mut Frame, o, b)` | `o < 4096`; writing: `b <= 255` | A byte of the page |
| `pt_new(f: Frame, level: u64) -> PageTable` | `1 <= level <= 5` | Turns a page into an empty table. A level-4 table gets the kernel's map in slot 0 (not owned, never touched) |
| `pt_free(t: PageTable) -> Frame` | level is not 4; every slot empty | Turns an empty table back into a page |
| `pt_free_root(m: &Mmu, t: PageTable) -> Frame` | level 4; slots 1-255 empty; `pt_id(t) != mmu_root(m)` | Same for a top-level table, which must not be in CR3 |
| `pt_has(t: &PageTable, i: u64) -> bool` | `i < 512` | Whether slot i owns something (the result is `pt_used(t, i)`) |
| `pt_put(t: &mut PageTable, i, c: PageTable)` | level >= 2; slot i empty; `pt_level(c) + 1 == pt_level(t)`; at level 4, `1 <= i < 256` | Moves table c into slot i |
| `pt_take(t: &mut PageTable, i) -> PageTable` | level >= 2; slot i used (same slot rule at level 4) | Moves the table out of slot i; its level is one less |
| `pt_put_frame(t: &mut PageTable, i, f: Frame, writable: bool)` | level 1; slot i empty | Maps page f in slot i, user-accessible |
| `pt_take_frame(t: &mut PageTable, i) -> Frame` | level 1; slot i used | Unmaps the page and returns it |

MMU and CPU (kernel only; the platform provides the trap entry code):

| Function | Precondition (proved) | Effect |
| --- | --- | --- |
| `mmu_switch(m: &mut Mmu, t: &PageTable)` | level 4 | Loads t into CR3; `mmu_root(m)` becomes `pt_id(t)` |
| `mmu_kernel(m: &mut Mmu)` | none | Loads the kernel's own map |
| `cpu_get(c: &Cpu, r: u64) -> u64` | `r < 20` | Saved user register r: rax rbx rcx rdx rsi rdi rbp r8-r15 (0-14), rip (15), rsp (16), rflags (17), error code (18), fault address (19) |
| `cpu_set(c: &mut Cpu, r, v)` | `r < 18` | Sets a saved register. Whatever is stored, user mode is entered in ring 3 with the user segments, IOPL 0 and interrupts on |
| `cpu_run(c: &mut Cpu, m: &Mmu) -> u64` | none | Runs the saved context in user mode until the next trap; returns its vector (`<= 255`) |
| `cpu_idle(c: &mut Cpu) -> u64` | none | Waits for an interrupt; returns its vector |

I/O ports (kernel and hosted):

| Function | Precondition (proved) | Effect |
| --- | --- | --- |
| `port_claim(p: &mut Ports, base, count) -> PortIo` | `base + count <= 65536`; `port_free(p, k)` for every k in the range | Takes the ports out of `p` |
| `port_release(p: &mut Ports, io: PortIo)` | none | Gives them back |
| `pio_in8/16/32(io: &mut PortIo, off) -> u64` | `off + 1/2/4 <= port_count(io)` | `in` from port `port_base(io) + off`; the result fits in 8/16/32 bits |
| `pio_out8/16/32(io: &mut PortIo, off, v)` | same, and v fits in 8/16/32 bits | `out` to that port |

Both directions take `&mut`: reading a device register changes the device (it pops a FIFO, acknowledges an interrupt), so a read is not a shared, side-effect-free access. Claiming and releasing compile to nothing; an access is one instruction. Ports that are never claimed (or devices whose drivers were not written) cannot be touched at all, and two drivers can never own the same port. What a device does with the values written to it is not modelled.

User programs (`--target user`) instead have system calls: `sys_spawn(b: &Buf, from, to)` (the program named by bytes `from..to` of b; proved to be a range within b), `sys_wait(pid)`, `sys_kill(pid)`, `sys_yield()`, `sys_getpid()`, `sys_info(k)`, `sys_ps()`, `sys_poweroff()`. Calling a builtin the target does not have is `E-TARGET`.

## Statements and expressions

```
fn name(p: T, mut q: T) -> R
  requires <bool expr>     // the caller proves it
  ensures  <bool expr>     // proved at every return; `result` is the return value
{ ... }

let x = e;   let mut x: T = e;   x = e;
if c { ... } else { ... }
match e { E::V(x) => { ... }, _ => { ... } }
while c invariant <expr> invariant <expr> { ... }
return e;   assert <expr>;   unreachable;

spec fn name(p: T, ...) -> bool { <spec expr> }   // a named specification, inlined where used
const NAME: u64 = <constant expr>;                 // folded at compile time; define before use
include_bytes("path")                              // a file's bytes as a Str, path relative to the source file
```

- Operators: `+ - * / %`, `== != < <= > >=`, `&& || !`. `&&` and `||` short-circuit.
- Integer literals are decimal or hex (`0x1F`).
- A program can span several files. A file may start with `import "path";` lines (the path is relative to the importing file); each imported file is loaded, with its own imports, before the rest of the file is read, so its constants can be used there. A file imported more than once is loaded once; files that import each other are `E-IMPORT`, as is a file that cannot be read. Files given together on the command line also form one program, in command-line order.
- All files share one namespace, as in C: a function, type or constant name must be unique in the whole program.
- `unreachable;` marks a point the prover must show no execution reaches (`E-VERIFY-UNREACHABLE`). The path ends there like a `return`, so linear values in hand need not be consumed. Its typical use is a `match` arm the specification rules out, such as the `Full` arm after swapping a value into a slot known to be empty. It compiles to `__builtin_unreachable()`, with no check.
- A name cannot shadow one that is still in scope. Once a block ends, its names can be used again (two `match` arms can both bind `v`).
- The entry point is `fn main(...) -> u64` (or with no result), taking any of the capabilities its target hands out, each at most once and as `&mut`: hosted `&mut Serial`, `&mut Pool`, `&mut Host`, `&mut Ports`; kernel `&mut Serial`, `&mut Pool`, `&mut Mmu`, `&mut Cpu`, `&mut Ports`; user `&mut Serial` only. A user program's result is its exit code. The older `fn main(con: Serial) -> Serial` is still accepted outside user programs.

## Specification expressions (ghost)

Expressions in `requires` / `ensures` / `invariant` / `assert` exist only at compile time and generate no code.

- They are evaluated over mathematical integers (`a - b` may be negative).
- They may use `==>` (implication) and `forall x: u64 :: expr`.
- They may call `len`, `get`, `slen`, `sget`, `blen`, `bget`, the ghost accessors above, and `spec fn`s.
- They may mention buffers that have already been consumed. In `ensures`, a by-value parameter name refers to **its value on entry** to the function; a `&mut` parameter refers to the value handed back, and `old(x)` to its value at the call.

## Linearity rules (error codes)

| Code | Rule |
| --- | --- |
| `E-LINEAR-USE` | A consumed buffer (or a moved-out field, or a struct with a field moved out) is used; one call both borrows and moves the same buffer; a borrow is returned; a buffer is consumed where the code may run more than once or not at all (a `while` condition, the right side of `&&` / `\|\|`) |
| `E-LINEAR-LEAK` | A buffer is still owned at the end of its scope or at a `return`; an owned buffer is overwritten |
| `E-LINEAR-BRANCH` | Only one arm of an `if` or a `match` consumes a buffer; a loop body changes the ownership of an outer buffer |
| `E-MATCH` | A `match` misses a variant, matches one twice, or has an arm after `_` |
| `E-IMPORT` | An imported file cannot be read, or files import each other |

The same rules apply to every linear type (`Frame`, `PageTable`, the capabilities), and a `&mut` parameter must hold a value again when the function returns.

## Verification (error codes)

The compiler executes each function symbolically. At every point where the program could go wrong, it asks Z3 whether the bad case is reachable, and if it is, rejects the program with a counterexample.

| Code | What is proved |
| --- | --- |
| `E-VERIFY-BOUNDS` | `get` / `set` / `sget` / `vec_get` / `vec_set` / `vec_swap` index < length; `vec_pop` on a non-empty vector; an I/O port access stays inside the claimed range |
| `E-VERIFY-LEAK` | a vector of linear elements is empty when it is freed |
| `E-VERIFY-OVERFLOW` | `+` and `*` results < 2^64 |
| `E-VERIFY-UNDERFLOW` | `a - b` has a >= b |
| `E-VERIFY-DIVZERO` | the divisor of `/` and `%` is not zero |
| `E-VERIFY-ALLOC` | allocation size <= 2^60 elements |
| `E-VERIFY-BYTE` | a value passed to `serial_write` or `frame_write8` is `<= 255`; a value passed to `pio_out16` / `pio_out32` fits in 16 / 32 bits |
| `E-VERIFY-OWN` | a page is taken from the pool at most once; an I/O port is claimed by at most one owner; a table slot is empty before `pt_put` and used before `pt_take`; a table is empty before it is freed |
| `E-VERIFY-PT` | table levels line up; frames go only into level-1 tables; slot 0 of a top-level table (the kernel's map) is never touched |
| `E-VERIFY-MMU` | the table in CR3 is never freed |
| `E-VERIFY-REQUIRES` | the callee's precondition |
| `E-VERIFY-ENSURES` | the function's own postcondition |
| `E-VERIFY-INVARIANT` | a loop invariant holds on entry and is preserved by one iteration |
| `E-VERIFY-ASSERT` | an `assert` |
| `E-VERIFY-UNREACHABLE` | an `unreachable;` the code can reach |

How verification works:
- **Modular.** A call uses only the callee's `requires` / `ensures`, never its body.
- **Loops** are cut with their invariants. After a loop, the variables the body assigns are known only through the invariants.
- **Annotation inference (1).** A buffer that a loop body only ever updates as `x = set(x, ..)` keeps its length across the loop, so `invariant len(x) == ...` is not needed.
- **Annotation inference (2).** A condition the programmer already tests, as in `if i < len(b) { get(b, i) }`, is used by the proof directly.

## Targets

`bastioc0 --target hosted` (the default) builds a normal program with the C library. `bastioc0 --target kernel` emits freestanding C for bare-metal x86-64 and implies `--emit-c`. That C needs no C library and provides `bst_kmain`, which initialises COM1 (only if `main` asks for `Serial`) and calls `main`; the boot code that calls `bst_kmain`, and the trap entry code behind `cpu_run` / `cpu_idle`, live in the kernel's own repository (BastiOS). `--target user` emits freestanding C for a user program under BastiOS, entered at `bst_umain`. The kernel prelude implements:
- `Serial` with port I/O on COM1;
- `pio_in*` / `pio_out*` as single `in` / `out` instructions;
- `alloc` / `free` with a fixed 8 MiB arena and an exact-size free list;
- `print` on the serial port;
- the pool (every page from the end of the kernel image to the end of RAM, at most 2^16), page-table entry encoding and CR3 loads.

The user prelude implements the console and the `sys_*` builtins as `int 0x80` system calls, and `alloc` with a 64 KiB arena. On the freestanding targets `Bytes` come from the same arena as `Buf`.

Running out of arena memory halts the machine.

## Soundness scope (what is not guaranteed)

- **Termination.** Infinite loops and infinite recursion are not detected. This is independent of memory safety and is a non-goal.
- **Out of memory.** `alloc` aborts when memory runs out. Kernel primitives will return allocation failure in the type instead.
- **Trusted base.** Z3, the stage0 compiler (Rust) and the C compiler are trusted, as is the hand-written C prelude each target links in (in `stage0/src/codegen.rs`; the kernel prelude does port I/O, manages the arena, encodes page-table entries and loads CR3), and the platform's trap entry code. `bastioc0 --trusted-base` lists every prelude part with its size, the parts each target links, and every builtin; the listing is checked into `tests/trusted_base.txt` and `tests/run.sh` fails when it changes, so growth is reviewed. (The small per-type helpers the compiler generates for `Box` and `Vec` are part of the compiler, not of this listing.)
- **Page contents.** The proof tracks who owns each page, not what is in it.
- **Argument evaluation order.** C leaves it unspecified. In v0 the only side effects are `print`, `alloc`, `free` and serial I/O, and linearity confines them to distinct buffers, so safety is unaffected; output order is not guaranteed when two calls in one expression both do I/O.
- **Cross-check in the tests.** `tests/run.sh` also runs every passing program under AddressSanitizer and UBSan to confirm that the proofs match runtime behaviour. Those checks exist only in the test build, never in generated code.
