# Concurrency in Bastio (design)

Status: design, not implemented. Written 2026-10-10 to settle the "how to express concurrency in types" question in [ROADMAP.md](../ROADMAP.md). Nothing here is in stage0 yet; [language.md](language.md) describes only what exists.

## Goals

1. **Data-race freedom is part of memory safety**, so it is mandatory and automatic, like bounds and ownership. A program with a data race does not compile.
2. **Zero overhead.** The only run-time cost is the synchronization the programmer asked for (a lock word, an atomic instruction, a thread). Who owns what, which lock protects what, lock order and interrupt state are ghost and erased.
3. **No `unsafe` escape hatch.** Kernel concurrency (several cores, interrupt handlers, locks shared with handlers) is expressed with capabilities and proofs, not by trusting the programmer. This is where Rust kernels still rely on human review; the first CVE in Rust-for-Linux code (CVE-2025-68260) was a race inside an `unsafe` block.
4. **Verification stays sequential.** The prover never explores interleavings. Each function is still verified on its own, top to bottom, as today.

Non-goals, as for termination: deadlock freedom in general, starvation, fairness. One common class of deadlock (lock order, and spinning on a lock the interrupted code holds) is ruled out cheaply, see "Lock ranks".

## The key observation

Today a value in Bastio is either:
- **owned** by exactly one place (linear values), or
- **borrowed read-only** (`&T`), or
- **borrowed exclusively** (`&mut T`), for the length of one call.

Bastio has no interior mutability: nothing can change through a `&T`. So the existing rules already say everything data-race freedom needs. A race is two accesses to the same memory, at least one a write, with no synchronization between them. Two threads can only write the same memory if both own it or both hold `&mut` to it, and linearity forbids that. Two threads may both hold `&T` to the same value, but then nobody writes it.

So the design adds **ways to run code on another thread that respect these rules**, plus **a small set of types that allow mutation through a shared reference on purpose** (`Lock`, `Atomic`), each with a proof obligation that keeps the sequential reasoning sound.

Consequences:
- Bastio needs no `Send`/`Sync` traits. Every type can be sent (moved) to another thread and every `&T` can be shared across threads, with one exception: **pinned** capabilities that describe *this* core or *this* thread (see "Pinned types").
- The sequential verifier stays sound. Its one assumption that concurrency could break is "a value behind `&T` does not change during the call". It still holds: the only things that change behind a shared reference are the contents of `Lock` and `Atomic`, and the prover never knows those contents except through the invariant (below).
- Because every compiled program is data-race-free and its only synchronization is locks and sequentially consistent atomics, the C11 memory model guarantees it behaves as if threads were interleaved one step at a time (the "DRF-SC" theorem). The C backend therefore stays valid: non-atomic accesses compile to plain loads and stores, and the C compiler keeps its optimizations.

## Layer 1: running code in parallel

### Fork-join: `par`

```
par {
  let a = sum(left);              // runs on this thread
  let b = sum(right);             // may run on another thread
  fill(&mut out, &table);         // another branch
}
// a and b are in scope here; every branch has finished
```

- Each statement in a `par` block is one call (optionally bound with `let`). The calls run in parallel and the block ends when all of them have returned.
- **The arguments of all the calls are checked as if they were the arguments of one call.** That is the existing rule: a variable may be lent `&` to any number of branches, but a variable lent `&mut` or moved in one branch cannot appear in another (`E-LINEAR-USE`). No new aliasing rule is needed.
- Borrows into `par` are safe without lifetimes because the block joins before the borrowed variables go out of scope.
- Verification: the prover checks each callee's `requires`, then assumes all the `ensures`, exactly as for calls in sequence. This is sound because the branches touch disjoint state (the "disjoint parallel composition" rule of concurrent separation logic).
- The first branch runs on the current thread, so it is the only one that may receive pinned capabilities.
- Generated code: the first call runs inline, the others on worker threads (pthreads on the hosted target; on the kernel target, a per-core work slot the kernel provides).

### Threads: `spawn` and `join`

```
let t = spawn worker(job, shared_clone);   // arguments are moved in
...
let r = join(t);                           // consumes the handle
```

- `spawn f(args)` names a function statically (Bastio has no function values) and starts it on a new thread. Every argument must be owned and is moved in; borrows cannot be passed, since the thread may outlive the caller's variables.
- The result is a linear `Thread[R]`, where `R` is `f`'s return type. It must be consumed by `join(t) -> R`, so a thread cannot be forgotten and whatever it owns comes back. Leaking a thread handle is `E-LINEAR-LEAK`.
- `join` gives back `f`'s result with `f`'s `ensures` applied.

### Sharing for an unbounded time: `Share[T]`

`par` shares values for one block. A thread or another core that lives indefinitely needs a different way to read shared data, but without a garbage collector or a run-time reference count.

```
let s = share_new(state);          // Share[State]: one heap allocation
let s2 = share_clone(&mut s);      // a second handle; no run-time count
let t = spawn worker(s2);
...
share_merge(&mut s, join(t));      // the worker returned its handle
let state = share_end(s);          // all handles are back: own it again
```

- `Share[T]` is a linear handle to a read-only `T` on the heap. Through a `Share` the value reads like a box: fields are places, and they can be lent to `&T` parameters, but never moved or lent `&mut`.
- Each handle carries a ghost **fraction** of the whole (`share_part(s)`, a rational number, as in fractional permissions). `share_new` gives 1. `share_clone` splits a handle's fraction in half. `share_merge(&mut a, b)` requires `share_id(a) == share_id(b)` and adds the fractions. `share_end(s) -> T` requires `share_part(s) == 1` (`E-VERIFY-SHARE`), frees the allocation and returns the value.
- So the program can always get its value back and free it, and the proof shows when that is safe, but at run time a `Share` is one pointer and cloning copies it. Nothing is counted.
- Mutation of shared state goes through a `Lock` or an `Atomic` inside the shared value.

## Layer 2: shared mutable state

### Type invariants

A shared value can change while a thread is not looking, so a thread can only rely on facts that every thread preserves. Those facts are written once, on the type:

```
struct Queue { items: Vec[u64], max: u64 }
  invariant vlen(items) <= max && max <= 4096;
```

An invariant is a specification expression over the fields (ghost, erased). It is not checked continuously. It is **proved where a value enters shared storage** (creating a lock, releasing it, storing to an atomic) and **assumed where a value comes out** (acquiring, loading). Between those points the owner may break it, which is what makes multi-step updates possible.

### Locks

```
let l = lock_new(q, 1);                // Lock[Queue]; proves the invariant of q; rank 1
...
let mut h = acquire(&mut ctx, &l);     // Held[Queue]; assumes the invariant
vec_push(&mut h.items, 7);             // h's fields are places, as through a Box
release(&mut ctx, &l, h);              // proves the invariant again
```

| Builtin | Precondition (proved) | Effect |
| --- | --- | --- |
| `lock_new(v: T, rank: u64) -> Lock[T]` | `inv(v)`; `rank >= 1` | A lock holding `v` |
| `acquire(c: &mut Ctx, l: &Lock[T]) -> Held[T]` | `lock_rank(l) > ctx_rank(c)` | Waits for the lock; the result holds the value, and `inv` holds of it |
| `release(c: &mut Ctx, l: &Lock[T], h: Held[T])` | `held_lock(h) == lock_id(l)`; `inv` of the contents; `ctx_rank(c) == lock_rank(l)` | Puts the value back and frees the lock |
| `lock_end(l: Lock[T]) -> T` | none | The owner of the whole lock takes the value out. Needs the `Lock` itself, so nobody else can be using it |

- `Held[T]` is linear and behaves like a `Box[T]` that must go back to the lock it came from. Holding a lock is therefore visible in the code: you cannot touch the queue without a `Held`, and you cannot forget to release, because dropping a `Held` is `E-LINEAR-LEAK`.
- A missing invariant at release is `E-VERIFY-LOCKINV`, with a counterexample like any other verification error.
- The race in CVE-2025-68260 (a list manipulated outside the lock that protects it) cannot be written: the list is only reachable through the `Held`.
- Generated code: a `Lock[T]` is `{ lock word, T }`; `acquire` is a spin (kernel targets) or a `pthread_mutex` (hosted), with acquire/release ordering; `Held[T]` is a `T *`. `Ctx`, ids, ranks and invariants generate nothing.

### Lock ranks (cheap deadlock avoidance)

A **`Ctx`** is a pinned, linear, zero-size capability for the current thread of execution. `main`, every spawned function and every interrupt handler receives one. Its ghost state is `ctx_rank(c)`: the rank of the most recently acquired lock still held, 0 when none.

- `acquire` requires the lock's rank to be strictly greater than `ctx_rank`, and raises `ctx_rank` to it. `release` requires the lock to be the last one taken and restores the previous rank. So locks are taken in increasing rank order and released in reverse, and no two threads can wait for each other through locks (`E-VERIFY-RANK`).
- The default rank 1 for every lock means "never hold two locks at once", which is already deadlock-free. Code that needs nesting gives its locks distinct ranks.
- `spawn` and `par` give the new threads a `Ctx` whose rank starts at the parent's current rank (`ctx_split`), so a child cannot block on a lock its parent holds while the parent waits to join it.
- Linux checks lock order at run time (lockdep). Here it is a proof and costs nothing.

### Atomics

```
struct Pending { n: u64 } invariant n <= 1024;

let a = atomic_new(Pending { n: 0 });
let x = atomic_load(&a);                  // x.n <= 1024 is known
if x.n < 1024 {
  let ok = atomic_cas(&a, x, Pending { n: x.n + 1 });   // proves x.n + 1 <= 1024
}
```

- `Atomic[T]` holds a plain value that fits in one machine word: `u64`, `bool`, or a struct with a single such field (which is how an atomic gets an invariant).
- `atomic_load` assumes the invariant of the result; `atomic_store(&a, v)` and `atomic_cas(&a, expected, v)` prove it of `v`. An in-place update such as `atomic_add(&a, d)` must prove that the update preserves the invariant and cannot overflow for **every** value satisfying it (`forall x :: inv(x) ==> inv(x + d) && x + d < 2^64`), since the current value is unknown.
- All atomic operations are sequentially consistent. Weaker orderings (relaxed, acquire/release) are left out: with them the "behaves as interleaved" argument no longer holds, and the prover would need a weak-memory model.
- Atomics cannot transfer ownership of linear values. Lock-free data structures that hand over ownership (a lock-free queue of `Buf`s) belong in the trusted prelude until there is a proof technique for them.

## Layer 3: the kernel

### Pinned types

Some capabilities describe the core or thread that holds them: `Ctx`, `Cpu` (this core's saved user context), `Mmu` (this core's CR3), `IrqSaved` (below) and `Held` (its release must match a `Ctx`). Moving one to another thread would make it describe the wrong core. A **pinned** type, or a struct, enum, vector or box containing one, cannot be an argument of `spawn`, cannot be passed to any `par` branch but the first, and cannot be put in a `Lock`, `Atomic`, `Share` or `Thread` result (`E-PIN`).

### Multiple cores

Each core runs its own copy of the kernel loop, with its own `Cpu`, `Mmu` and `Ctx`:

```
fn ap_main(cpu: Cpu, mmu: Mmu, ctx: Ctx, k: Share[KernelShared]) -> Share[KernelShared] { ... }

let t = smp_start(ap, ap_main, k_clone);   // kernel target: starts application processor `ap`
```

- `smp_start` is the kernel-target form of `spawn`: the boot code brings up the core, and the function receives that core's pinned capabilities, created there and never moved. The core's handle is a `Thread`, so at power off the bootstrap core asks the others to stop (through an atomic flag), joins them and gets every `Share` back, and BastiOS can keep proving that every page returns.
- State all cores use (the process table, the page allocator) lives in `Lock`s inside a `Share`. A first version can use one lock for the whole kernel state; finer locks are an optimization with the same proofs.
- **Page tables are already SMP-safe in BastiOS's design.** A process's address space is taken out of the table of spaces before it runs and put back afterwards (`take_space` / `put_space`). On several cores the table of spaces is behind a lock, and a running space is owned by the core running it, so no other core can free or change it. Since each process has one thread and every switch reloads CR3 (no PCIDs), a core leaving a process keeps no TLB entries for it, so unmapping needs no TLB shootdown. Multi-threaded user processes (one address space on several cores at once) will need shootdowns; see open questions.

### Interrupts

**Model A: interrupts as events (what BastiOS does now, and the recommended default).** Interrupts are off whenever kernel code runs. A trap or interrupt ends `cpu_run` / `cpu_idle`, which return its vector, and the kernel loop handles it as an ordinary event. There are no handlers to race with, so nothing in this section is needed. With several cores, each core does the same and shares state through locks.

**Model B: interrupt handlers**, for drivers that must react while kernel code runs:

```
irq fn on_rx(ctx: &mut Ctx, dev: &Uart) { ... }    // runs with interrupts off

let s = irq_off(&mut ctx);            // IrqSaved: interrupts off on this core
let h = acquire(&mut ctx, &dev.rx);   // allowed: dev.rx is an irq lock and interrupts are off
...
release(&mut ctx, &dev.rx, h);
irq_restore(&mut ctx, s);             // linear: interrupts cannot be left off by accident
```

- `irq_off(&mut Ctx) -> IrqSaved` and `irq_restore(&mut Ctx, IrqSaved)` set the ghost flag `ctx_irq(c)`. `IrqSaved` is linear and pinned, so interrupts are always restored, on the same core.
- A lock created with `irq_lock_new` may be taken by handlers. Outside a handler it can only be acquired while `ctx_irq(c)` holds (`E-VERIFY-IRQ`). This rules out the classic deadlock in which a handler spins on a lock that the code it interrupted holds on the same core.
- A handler receives a `Ctx` with interrupts off and may take only irq locks.
- Handlers have no global variables to reach (Bastio has none). Their state is attached when the handler is installed: `irq_attach(vector, on_rx, s: Share[Uart]) -> IrqSlot`, and `irq_detach(slot) -> Share[Uart]` gives the share back, so teardown is still proved complete.

### Devices and DMA (direction only)

The same ownership idea extends to hardware that writes memory on its own. A buffer given to a device for DMA is moved into a linear "in flight" token and comes back only when the device reports completion, so the kernel cannot read or free it meanwhile. This belongs with the `Mmio` and `PortIo` capabilities in v3 and is not designed further here.

## What gets proved, and what is trusted

Proved (mandatory, like the rest of memory safety):
- no data races on non-atomic memory;
- shared state is only touched with its lock held, and its type invariant holds whenever it is unlocked;
- every `Held`, `IrqSaved`, `Thread` and `Share` is accounted for (released, restored, joined, merged), so teardown can still be shown to return every resource;
- locks are taken in rank order, irq locks only with interrupts off, and pinned capabilities stay on their core.

Not proved: absence of deadlock through other means (for example joining a thread that waits on a lock-protected flag nobody sets), progress, fairness, and termination.

The trusted base grows by: the lock and atomic primitives and the thread start code in the C prelude (pthreads on the hosted target, spinlocks and C11 atomics on the kernel targets); the application-processor startup and the interrupt entry in BastiOS's assembly; and the C compiler's implementation of the C11 memory model. Each part is listed by `--trusted-base`, as invariant 3 requires.

## Error codes (planned)

| Code | Rule |
| --- | --- |
| `E-PIN` | A pinned capability crosses to another thread (`spawn`, a later `par` branch, a lock, a share) |
| `E-VERIFY-LOCKINV` | A type invariant is not proved where a value enters a lock or an atomic |
| `E-VERIFY-RANK` | A lock is acquired out of rank order, or released out of order |
| `E-VERIFY-IRQ` | An irq lock is acquired with interrupts on, outside a handler |
| `E-VERIFY-SHARE` | `share_end` without every handle merged back, or merging handles of different shares |

Aliasing between `par` branches and leaked `Held` / `Thread` / `Share` values reuse `E-LINEAR-USE` and `E-LINEAR-LEAK`. Each code gets programs in `tests/fail/` that trigger it, per invariant 4.

## Implementation plan

Concurrency touches the checker (pinned types, `par` aliasing, `spawn`) and the verifier (invariants, fractions, ranks), which is exactly where the self-hosted compiler is being written. To avoid implementing everything twice, the features land in the self-hosted compiler after the fixed point, in this order:

1. **Fork-join and threads, hosted:** `par`, `spawn`/`join`, `Share`, pinned types. Benchmarks: a parallel array sum and a parallel sieve against C with pthreads.
2. **Locks and atomics:** type invariants, `Lock`, `Held`, `Atomic`, `Ctx` with ranks. Example: a work queue shared by several threads.
3. **BastiOS SMP:** `smp_start`, a big kernel lock first, then per-core run queues. Tests: processes running on two cores in QEMU (`-smp 2`), and the page count at power off still equal to the count at boot.
4. **Interrupt handlers:** `irq_off`, irq locks, `irq fn`, with the first driver that needs them (most likely with `Mmio`).

Performance (invariant 5): measure an uncontended acquire/release against `pthread_mutex` and a spinlock in C, and the parallel benchmarks against C. The expected cost is exactly the C cost, since nothing else is generated.

## Open questions

- **Monotonic facts.** A counter that only grows lets a thread keep "it was at least 5 when I looked". Needs two-state invariants on atomics. Useful for sequence numbers and reference counts.
- **Blocking.** Condition variables or wait queues, so a thread can sleep until shared state changes. The kernel can do without them for now (the scheduler blocks processes), but hosted programs will want them.
- **Channels.** A typed channel of linear values can be written on top of a lock and a blocking primitive. Decide whether it is built in.
- **Multi-threaded user processes** in BastiOS: one address space running on several cores, which brings TLB shootdowns. A linear "shootdown owed" token returned only after all cores acknowledge is one possible shape.
- **Weaker atomic orderings** and lock-free ownership transfer, if measurements show sequential consistency costs too much.
- **Read-write locks and RCU**, both common in kernels. RCU in particular frees memory only after every reader is done, which would need a grace-period capability.
