# Sharing state between processors

This is the fourth part of the [tutorial](tutorial.md). Part 3 gave one processor ownership of pages and devices. A kernel has many processors, and they must share some state: a run queue, a free-page count, a table of processes. In C this is where data races, forgotten locks and deadlocks come from. Bastio does not let a program have them, and still generates the code you would write by hand: a spinlock and an atomic exchange.

You should know contracts and loop invariants from the [first part](tutorial.md). The [concurrency design page](concurrency.md) has the full rationale, and says which parts (`par`, `spawn`, `Share`) are not built yet; this page covers what exists.

As before, every `bastio` block is compiled by `tests/run.sh`. The hosted target runs a lock on one thread, which is enough to check the logic; running on several processors is a kernel feature, so those examples are only checked (`// expect-ok`).

## 1. The problem: the shared value has no owner

Ownership gave a rule for memory: one owner at a time, so no two places write the same data. Shared state breaks that on purpose, so Bastio adds exactly one way back in. A `Lock[T]` owns a `T`. Any number of processors hold a *shared borrow* `&Lock[T]`, and none of them can reach the `T` except inside `locked`, which takes the lock, runs a function on `&mut T`, and releases the lock.

```bastio
// expect-error: E-VERIFY-OVERFLOW
struct Count { n: u64 }

fn bump(c: &mut Count) {
  c.n = c.n + 1;            // c.n is whatever the last processor left
}

fn main() -> u64 {
  let l = lock_new(Count { n: 0 });
  locked(l, bump);
  let c = lock_end(l);
  return c.n;
}
```

This is rejected, and the reason is the heart of this part. Inside `locked`, the contents are a *fresh* value: another processor may have changed them since this one last looked, so the compiler knows nothing about `c.n`, and the addition might overflow. On one thread you would know `n` is 0; with many you do not.

## 2. Invariants: what the lock promises

What a lock can promise is a fact that *every* processor keeps true. You state it on the struct as an `invariant`:

```bastio
// expect-exit: 3
struct Count { n: u64 } invariant n < 1000;

fn bump(c: &mut Count)
  requires c.n < 1000
  ensures c.n < 1000
{
  if c.n < 999 {
    c.n = c.n + 1;
  }
}

fn main() -> u64 {
  let l = lock_new(Count { n: 0 });      // proves the invariant holds for 0
  locked(l, bump);                       // assumes it before bump, proves it after
  locked(l, bump);
  locked(l, bump);
  let c = lock_end(l);                   // the invariant is known of the result
  return c.n;
}
```

Three proofs are involved, and together they are the argument that the program is race-free:

1. `lock_new` proves the invariant for the initial value.
2. Inside `locked`, the invariant is *assumed* on entry (the previous holder left it true) and the function's `requires` must follow from it.
3. When the function returns, the invariant is *proved* again before the lock is released.

An invariant that a locked section breaks is `E-VERIFY-LOCKINV`, even though no other processor is running at that moment: the next one to take the lock would find it false.

```bastio
// expect-error: E-VERIFY-LOCKINV
struct Ring { buf: Buf, at: u64 } invariant at < len(buf);

fn step(r: &mut Ring)
  requires r.at < len(r.buf)
{
  r.at = r.at + 1;          // may step past the end of the buffer
}

fn main() -> u64 {
  let l = lock_new(Ring { buf: alloc(4), at: 0 });
  locked(l, step);
  let r = lock_end(l);
  free(r.buf);
  return 0;
}
```

Outside a lock, invariants are not enforced; they are what the lock promises about its contents. That is why the contents cannot escape: `locked` hands `f` a `&mut`, a borrow, and borrows cannot be stored or returned.

## 3. Deadlock is a type error

A lock invites the other classic bug: processor 1 holds A and waits for B while processor 2 holds B and waits for A. Bastio rules it out without any annotation. A lock's *class* is the type of its contents. The checker looks at the whole call graph, and requires that the classes in which locks are nested never form a cycle, so every processor takes locks in one global order.

```bastio
// expect-error: E-LOCK-ORDER
struct A { n: u64 }
struct B { n: u64 }

fn touch_b(b: &mut B) { b.n = 1; }
fn touch_a(a: &mut A) { a.n = 1; }
fn under_a(a: &mut A, lb: &Lock[B]) { locked(lb, touch_b); }
fn under_b(b: &mut B, la: &Lock[A]) { locked(la, touch_a); }
fn a_then_b(la: &Lock[A], lb: &Lock[B]) { locked(la, under_a, lb); }
fn b_then_a(la: &Lock[A], lb: &Lock[B]) { locked(lb, under_b, la); }

fn main() -> u64 {
  let la = lock_new(A { n: 0 });
  let lb = lock_new(B { n: 0 });
  a_then_b(la, lb);
  b_then_a(la, lb);
  let a = lock_end(la);
  let b = lock_end(lb);
  return 0;
}
```

`a_then_b` takes A then B, `b_then_a` takes B then A. Either alone is fine; having both is the cycle, and the compiler reports it. Two locks with the same contents type cannot be nested either (a processor could wait on itself), so if you need two, give their contents two struct types.

## 4. Atomics: sharing one word with no lock

For a counter or a flag, a lock is more than you need. An `Atomic[T]` is one word read and written by any number of processors through `&Atomic[T]`. `T` is an integer, a `bool`, or a struct with a single such field, which is how an atomic gets an invariant.

The contents are again fresh after every read, so the idiom is a compare-and-swap loop: read, compute, and write only if nobody changed it meanwhile.

```bastio
// expect-exit: 4
struct Pending { n: u64 } invariant n <= 1024;

/// Reserves one slot, or says there is none left.
fn reserve(a: &Atomic[Pending]) -> bool {
  let x = atomic_load(a);                              // x.n <= 1024 is known
  if x.n < 1024 {
    return atomic_cas(a, x, Pending { n: x.n + 1 });   // x.n + 1 <= 1024: proved
  }
  return false;
}

fn main() -> u64 {
  let a = atomic_new(Pending { n: 1020 });
  let mut i = 0;
  while i < 6
    invariant i <= 6
  {
    let ok = reserve(a);
    i = i + 1;
  }
  let p = atomic_end(a);
  return p.n % 1000 / 5;
}
```

Here the invariant is what makes `x.n + 1` safe even though `x` was read from a shared word. If another processor wins the race, `atomic_cas` returns false and the caller retries; no processor ever writes a value outside the invariant, because each write proves it. When you want wrapping (a statistics counter), `atomic_wrap_add` says so by name. All operations are sequentially consistent: a program behaves as if the processors' steps were interleaved one at a time.

## 5. Running on every processor

In the kernel, `smp_run(f, args..)` calls `f` on every processor at once and returns when all have returned. The rules are what keep it safe: `f` receives each processor's own `&mut Cpu` and `&mut Mmu`, and everything else only as a shared borrow `&T`. A `&mut` buffer would be written by all of them, so it is `E-PIN`. Shared state therefore has to be a `Lock` or an `Atomic`.

```bastio
// expect-ok
// target: kernel
struct Count { n: u64 } invariant n < 1000000;

fn bump(c: &mut Count)
  requires c.n < 1000000
  ensures c.n < 1000000
{
  if c.n < 999999 {
    c.n = c.n + 1;
  }
}

/// Every processor adds one to the shared count.
fn work(cpu: &mut Cpu, l: &Lock[Count]) {
  locked(l, bump);
}

fn main(cpu: &mut Cpu) -> u64 {
  let l = lock_new(Count { n: 0 });
  smp_run(work, &mut cpu, l);
  let c = lock_end(l);
  return c.n;
}
```

`Cpu` and `Mmu` describe one processor, so they are *pinned*: a lock cannot hold one, and `smp_run` gives each processor its own. Interrupt handlers are not covered yet; BastiOS keeps interrupts off while it holds a lock.

## What you can rely on

- Shared mutable data is reachable only inside `locked` or through an atomic operation. A data race is not expressible.
- Every lock has an invariant that is assumed on entry and proved on exit, so a processor never sees a half-updated structure.
- Lock cycles are a compile error. Nothing is annotated and nothing is generated for it.
- Generated code is a spinlock (`xchg` to take, a store to release) and `__atomic` builtins. The proofs cost nothing at run time.

What is not covered: the memory model is sequentially consistent only, and `par`, `spawn` and `Share` from the [design](concurrency.md) are not built.
