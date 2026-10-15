# Cookbook: small proved programs

Short programs for things people write first, each showing a technique the compiler needs help with. They are not tutorials: if a term is unfamiliar, the [tutorial](tutorial.md) introduces it. Every `bastio` block is compiled and run by `tests/run.sh`, so they are known to work.

## Binary search without overflow

The classic bug in this algorithm is `(lo + hi) / 2` overflowing. The compiler would flag it, so write `lo + (hi - lo) / 2`, which it can prove stays in range. The contract says that a result below the length is a real hit:

```bastio
// expect-exit: 42
/// An index of `key` in the sorted buffer `b`, or `len(b)` if there is none.
fn search(b: &Buf, key: u64) -> u64
  ensures result <= len(b)
  ensures result < len(b) ==> get(b, result) == key
{
  let mut lo = 0;
  let mut hi = len(b);
  while lo < hi
    invariant lo <= hi && hi <= len(b)
  {
    let mid = lo + (hi - lo) / 2;      // lo <= mid < hi, so get(b, mid) is in bounds
    let v = get(b, mid);
    if v == key {
      return mid;
    }
    if v < key {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  return len(b);
}

fn main() -> u64 {
  let mut b = alloc(5);
  b = set(b, 0, 3);
  b = set(b, 1, 8);
  b = set(b, 2, 13);
  b = set(b, 3, 21);
  b = set(b, 4, 34);
  let hit = search(b, 21);
  let miss = search(b, 5);
  let n = len(b);
  free(b);
  if hit == 3 && miss == n {
    return 42;
  }
  return 1;
}
```

The `ensures` lines state the part that matters for memory safety and for callers (a hit is a valid index and holds the key). Proving that a miss means the key is absent needs the buffer to be sorted, which would be a `forall` precondition over the buffer; it is the next step if you want it.

## A checked sum that reports overflow

Arithmetic never wraps, so a function that adds user-supplied numbers has to say what happens when the total does not fit. An enum makes the answer part of the type:

```bastio
// expect-exit: 42
enum Total { Sum(u64), TooBig }

fn add_all(b: &Buf) -> Total {
  let mut sum = 0;
  let mut i = 0;
  while i < len(b)
    invariant i <= len(b)
  {
    let x = get(b, i);
    if x > 18446744073709551615 - sum {      // sum + x would not fit
      return Total::TooBig;
    }
    sum = sum + x;
    i = i + 1;
  }
  return Total::Sum(sum);
}

fn main() -> u64 {
  let mut b = alloc(3);
  b = set(b, 0, 40);
  b = set(b, 1, 1);
  b = set(b, 2, 1);
  let t = add_all(b);
  b = set(b, 0, 18446744073709551615);
  let u = add_all(b);
  free(b);
  let mut r = 1;
  match t {
    Total::Sum(n) => {
      match u {
        Total::TooBig => { if n == 42 { r = 42; } }
        Total::Sum(m) => {}
      }
    }
    Total::TooBig => {}
  }
  return r;
}
```

The test `x > MAX - sum` is exactly the condition the compiler needs: after it, `sum + x <= MAX` is known, and the addition is proved.

## A state machine

An enum with a payload per state, and a `match` that must cover every state (`E-MATCH` if one is missing), keeps a protocol honest:

```bastio
// expect-exit: 42
enum Conn { Closed, Open(u64), Failed }

/// One step of a toy connection: opening, sending bytes, closing.
fn step(c: Conn, event: u64) -> Conn {
  match c {
    Conn::Closed => {
      if event == 0 {
        return Conn::Open(0);
      }
      return Conn::Failed;                    // anything but "open" is an error
    }
    Conn::Open(sent) => {
      if event == 1 && sent < 1000 {
        return Conn::Open(sent + 1);          // sent < 1000: cannot overflow
      }
      if event == 2 {
        return Conn::Closed;
      }
      return Conn::Failed;
    }
    Conn::Failed => {
      return Conn::Failed;
    }
  }
}

fn main() -> u64 {
  let mut c = Conn::Closed;
  c = step(c, 0);
  c = step(c, 1);
  c = step(c, 1);
  match c {
    Conn::Open(n) => {
      if n < 100 {
        return n * 21;
      }
      return 1;
    }
    Conn::Closed => { return 0; }
    Conn::Failed => { return 1; }
  }
}
```

## A stack on `Vec`

`vec_pop` needs a nonempty vector, so the caller has to show it. The usual way is to test first; the test is the proof:

```bastio
// expect-exit: 42
/// Adds the top two elements and pushes the result. False if there are fewer than two.
fn add_top(stack: &mut Vec[u64]) -> bool
  ensures result ==> vlen(stack) >= 1
{
  if vec_len(stack) < 2 {
    return false;
  }
  let a = vec_pop(&mut stack);       // vlen >= 2: proved by the test above
  let b = vec_pop(&mut stack);       // and one left after the first pop
  if a > 18446744073709551615 - b {
    vec_push(&mut stack, 18446744073709551615);   // saturate
  } else {
    vec_push(&mut stack, a + b);
  }
  return true;
}

fn main() -> u64 {
  let mut s = vec_new[u64]();
  vec_push(&mut s, 40);
  vec_push(&mut s, 2);
  if !add_top(&mut s) {
    vec_free(s);
    return 1;
  }
  let top = vec_pop(&mut s);        // add_top promised a nonempty stack
  vec_free(s);
  return top;
}
```

`vec_free` needs the vector to be empty only for vectors of linear values; for a `Vec[u64]` you can free it with elements left.

## A ring buffer index

An index that wraps is a remainder, and the compiler knows `x % n < n` when `n` is nonzero. That is all the bounds proof needs:

```bastio
// expect-exit: 42
struct Ring { buf: Buf, head: u64 }

fn ring_new(n: u64) -> Ring
  requires 0 < n && n <= 1000000
  ensures result.head < len(result.buf) && len(result.buf) == n
{
  return Ring { buf: alloc(n), head: 0 };
}

fn ring_push(r: &mut Ring, v: u64)
  requires r.head < len(r.buf)
  ensures r.head < len(r.buf) && len(r.buf) == len(old(r).buf)
{
  r.buf = set(r.buf, r.head, v);
  r.head = (r.head + 1) % len(r.buf);
}

fn main() -> u64 {
  let mut r = ring_new(4);
  let mut i = 0;
  while i < 10
    invariant i <= 10 && r.head < len(r.buf) && len(r.buf) == 4
  {
    ring_push(&mut r, i);
    i = i + 1;
  }
  let at = r.head;
  let last = get(r.buf, 1);
  free(r.buf);
  if at == 2 && last == 9 {
    return 42;
  }
  return 1;
}
```

The `requires` and `ensures` on `ring_push` say that the head stays inside the buffer and the buffer keeps its length; `main`'s loop carries the same fact as an invariant. If you want the compiler to enforce such a fact on a shared structure, put it on the struct as an `invariant` and keep the value in a `Lock`, as in [sharing state](tutorial-shared.md).
