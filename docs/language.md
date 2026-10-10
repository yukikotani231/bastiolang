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
| `&Buf` | A read-only borrow. Allowed **only as a parameter type**, so it can never be returned or stored, and never outlives the call |
| `Str` | A string literal: immutable bytes with a length known at compile time. Copyable, never freed |
| `Serial` | The capability to use the serial console. **Linear**: it cannot be created, only `main` is handed one, and `main` must return it |
| `&Serial` | A borrowed console, so a helper function can print without taking ownership |
| `&mut T` | A mutable borrow of any linear `T`, again only as a parameter type. Lent with `&mut x`. The callee may move out of the borrow, but must have put a value back by the time it returns |
| `Pool` | The physical pages the kernel may hand out. Linear; only `main` is handed one |
| `Frame` | One physical page (4 KiB). Linear: owned by exactly one place at a time |
| `PageTable` | A page-table page at level 1 to 5. Linear; it owns the tables and pages its slots point to |
| `Mmu` | The right to load CR3. Linear; only `main` (kernel target) is handed one |
| `Cpu` | The CPU's saved user context and the right to enter user mode. Linear; kernel target only |

| `S` (a struct) | A user-defined record. Linear when any field is linear; otherwise a plain value that is copied |

Borrowing at a call site is implicit: passing an owned `b` to a `&Buf` parameter lends it for the duration of the call. A `&mut` parameter needs an explicit `&mut b`, and `b` must be declared `mut` (or itself be a `&mut` parameter). One call cannot lend the same variable twice if one of the loans is `&mut`.

The capability types carry **ghost state** that exists only in the proof and costs nothing at run time (a `Frame` or `PageTable` is a bare physical address in the generated code; `Serial`, `Mmu` and `Cpu` are empty):

| Accessor (specifications only) | Meaning |
| --- | --- |
| `frame_id(f)`, `pt_id(t)` | The page number |
| `pt_level(t)` | 1 (maps pages) to 4 (top level, what CR3 holds); 5 is a container for level-4 tables that is never loaded into CR3 |
| `pt_used(t, i)` | Slot `i` owns something |
| `pool_has(p, i)` | The pool can still hand out page `i` |
| `pool_size(p)` | Number of pages in the pool (also callable at run time) |
| `mmu_root(m)` | The id of the table in CR3; `0x10000` while the kernel's own map is loaded |

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

- A struct has at least one field, and a field cannot be a borrow. A struct cannot contain itself.
- A struct with a linear field is **linear**: it must be consumed exactly once, like a `Buf`. A struct without one is a plain value and is copied.
- Ownership is tracked **per place** (`p`, `p.space`, `p.space.root`). Moving a linear field out leaves the rest of the struct usable: its scalar fields can still be read and its other fields used. The struct as a whole cannot be moved, borrowed or returned until the field is put back (`E-LINEAR-USE`). A struct counts as consumed once every linear field in it has been moved out, so `free(s.items)` consumes a `Stack { items: Buf, n: u64 }`.
- Assigning a linear field that still holds a value is `E-LINEAR-LEAK`, as for variables.
- A place cannot be borrowed in a call that also lends an overlapping place as `&mut` (`p` and `p.regs` overlap).
- A `&mut` struct parameter must be whole again when the function returns.
- Struct literals list every field once, in any order. Fields are evaluated in an unspecified order (as with call arguments).
- Specifications can read fields, including through `old(..)` and `result`: `ensures result.n == old(s).n + 1`, `pt_level(s.root) == 1`. The proof tracks every field, so bounds, overflow and capability state work through fields exactly as through variables.
- In generated code a struct is a C struct passed by value; there is no runtime representation of which fields have been moved.

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

User programs (`--target user`) instead have system calls: `sys_spawn(b: &Buf, from, to)` (the program named by bytes `from..to` of b; proved to be a range within b), `sys_wait(pid)`, `sys_kill(pid)`, `sys_yield()`, `sys_getpid()`, `sys_info(k)`, `sys_ps()`, `sys_poweroff()`. Calling a builtin the target does not have is `E-TARGET`.

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

spec fn name(p: T, ...) -> bool { <spec expr> }   // a named specification, inlined where used
const NAME: u64 = <constant expr>;                 // folded at compile time; define before use
include_bytes("path")                              // a file's bytes as a Str, path relative to the source file
```

- Operators: `+ - * / %`, `== != < <= > >=`, `&& || !`. `&&` and `||` short-circuit.
- Integer literals are decimal or hex (`0x1F`).
- Several source files can be given on the command line; they form one program, and constants are visible in later files.
- A name cannot be redeclared (no shadowing) within a function.
- The entry point is `fn main(...) -> u64` (or with no result), taking any of the capabilities its target hands out, each at most once and as `&mut`: hosted `&mut Serial`, `&mut Pool`, `&mut Host`; kernel those plus `&mut Mmu`, `&mut Cpu`; user `&mut Serial` only. A user program's result is its exit code. The older `fn main(con: Serial) -> Serial` is still accepted outside user programs.

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
| `E-LINEAR-BRANCH` | Only one arm of an `if` consumes a buffer; a loop body changes the ownership of an outer buffer |

The same rules apply to every linear type (`Frame`, `PageTable`, the capabilities), and a `&mut` parameter must hold a value again when the function returns.

## Verification (error codes)

The compiler executes each function symbolically. At every point where the program could go wrong, it asks Z3 whether the bad case is reachable, and if it is, rejects the program with a counterexample.

| Code | What is proved |
| --- | --- |
| `E-VERIFY-BOUNDS` | `get` / `set` / `sget` index < length |
| `E-VERIFY-OVERFLOW` | `+` and `*` results < 2^64 |
| `E-VERIFY-UNDERFLOW` | `a - b` has a >= b |
| `E-VERIFY-DIVZERO` | the divisor of `/` and `%` is not zero |
| `E-VERIFY-ALLOC` | allocation size <= 2^60 elements |
| `E-VERIFY-BYTE` | a value passed to `serial_write` or `frame_write8` is `<= 255` |
| `E-VERIFY-OWN` | a page is taken from the pool at most once; a table slot is empty before `pt_put` and used before `pt_take`; a table is empty before it is freed |
| `E-VERIFY-PT` | table levels line up; frames go only into level-1 tables; slot 0 of a top-level table (the kernel's map) is never touched |
| `E-VERIFY-MMU` | the table in CR3 is never freed |
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

`bastioc0 --target hosted` (the default) builds a normal program with the C library. `bastioc0 --target kernel` emits freestanding C for bare-metal x86-64 and implies `--emit-c`. That C needs no C library and provides `bst_kmain`, which initialises COM1 and calls `main`; the boot code that calls `bst_kmain`, and the trap entry code behind `cpu_run` / `cpu_idle`, live in the kernel's own repository (BastiOS). `--target user` emits freestanding C for a user program under BastiOS, entered at `bst_umain`. The kernel prelude implements:
- `Serial` with port I/O on COM1;
- `alloc` / `free` with a fixed 8 MiB arena and an exact-size free list;
- `print` on the serial port;
- the pool (every page from the end of the kernel image to the end of RAM, at most 2^16), page-table entry encoding and CR3 loads.

The user prelude implements the console and the `sys_*` builtins as `int 0x80` system calls, and `alloc` with a 64 KiB arena. On the freestanding targets `Bytes` come from the same arena as `Buf`.

Running out of arena memory halts the machine.

## Soundness scope (what is not guaranteed)

- **Termination.** Infinite loops and infinite recursion are not detected. This is independent of memory safety and is a non-goal.
- **Out of memory.** `alloc` aborts when memory runs out. Kernel primitives will return allocation failure in the type instead.
- **Trusted base.** Z3, the stage0 compiler (Rust) and the C compiler are trusted, as is the hand-written C prelude each target links in (in `stage0/src/codegen.rs`; the kernel prelude does port I/O, manages the arena, encodes page-table entries and loads CR3), and the platform's trap entry code.
- **Page contents.** The proof tracks who owns each page, not what is in it.
- **Argument evaluation order.** C leaves it unspecified. In v0 the only side effects are `print`, `alloc`, `free` and serial I/O, and linearity confines them to distinct buffers, so safety is unaffected; output order is not guaranteed when two calls in one expression both do I/O.
- **Cross-check in the tests.** `tests/run.sh` also runs every passing program under AddressSanitizer and UBSan to confirm that the proofs match runtime behaviour. Those checks exist only in the test build, never in generated code.
