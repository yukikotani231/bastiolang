# The core and std libraries

Bastio's library comes in two parts, as ROADMAP.md (design consequence 1) decided:

- **`core/`** needs no operating system. It runs on every target (`hosted`, `kernel`, `user`): it uses only allocation, vectors and the serial console, which every target has.
- **`std/`** uses the host's services (files, the command line, standard error) and works on the hosted target only. Importing it from a kernel or user program is rejected with `E-TARGET`.

Everything in both is proved memory safe like any other Bastio code, and most functions also state what they compute (`ensures`), so callers get facts for their own proofs. The library is young: the modules below are what the compiler, BastiOS and the test programs were writing again and again by hand (see [Where it came from](#where-it-came-from)).

## Using it

Import a module by its name under the library, wherever the importing file is. A module imports what it needs itself, and a file imported twice is loaded once.

```text
import "core/text.bastio";     // also brings in core/bytes.bastio and core/option.bastio
import "core/num.bastio";
```

An import that does not start with `.` or `/` is looked for next to the importing file, then in each directory above it, then in each `--lib DIR` given to the compiler and each directory of `BASTIO_LIB` (see [Programs and files](language.md#programs-and-files)). Inside this repository the walk up finds `core/`; a program elsewhere, such as BastiOS, passes `--lib path/to/bastiolang`.

All files share one namespace (see [Programs and files](language.md#programs-and-files)), so every function is prefixed by its module (`bytes_`, `text_`, `hmap_`, `bmap_`, ...) except the few whose names say what they are (`parse_u64`, `fmt_u64`, `print_str`, `sort_u64`). Two names are taken without a prefix: the types `Option[T]` and `Span`. A program that imports core must not define its own.

For a whole program, see [`examples/wordfreq.bastio`](../examples/wordfreq.bastio): it splits a text, counts words in a `BytesMap`, sorts them by count and prints the top five, and every library proof it leans on is named in its comments. The tests are the worked examples of each function: `tests/pass/core_*.bastio` and `tests/pass/std_host.bastio` use every function, and `tests/user/core_user.bastio` builds core for a BastiOS user program.

## Conventions

- **Spans, not copies.** A `Span { start, end }` names bytes `start..end` of some byte string. Splitting and trimming return spans; parsing takes a range `from..to`. Copy only when a separate value is needed (`bytes_slice`).
- **`Option[T]` for "maybe".** `Some(v)` or `None`: a key that is not in a map, text that does not parse, a substring that does not occur. Searches that scan for one byte return the length of the string instead, so that a scan loop needs no `match`.
- **Text is a `Vec[u8]`.** Building a string pushes bytes onto a vector, which grows as needed; `text_finish` turns it into `Bytes`. `fmt_*` functions append to a text, `print_*` functions write to the console.
- **Preconditions are proved at each call.** `bytes_slice(b, 2, 9)` on five bytes is rejected at compile time (`tests/fail/core_slice_bounds.bastio`), and so is a binary search of a vector not known to be sorted (`tests/fail/core_search_unsorted.bastio`).

## Modules

### `core/option.bastio`

| Item | Meaning |
| --- | --- |
| `enum Option[T] { Some(T), None }` | A value that may be missing |
| `opt_or[T](o, d) -> T` | The payload, or `d` (proved) |
| `struct Span { start: u64, end: u64 }` | Bytes `start..end` of some byte string |

### `core/boxed.bastio`

| Item | Meaning |
| --- | --- |
| `enum Boxed[T] { Ok(Box[T]), Full(T) }` | What `try_box[T](x)` returns: `x` in a new box, or `x` itself when memory ran out (see [Boxes](language.md#boxes)) |

### `core/bytes.bastio`

| Function | Proved |
| --- | --- |
| `bytes_eq(a, b)`, `bytes_eq_str(a, s: Str)`, `bytes_span_eq_str(a, from, to, s)` | The result is exactly "same length and same bytes" (spec fns `bytes_equal`, `bytes_equal_str`) |
| `bytes_starts_with(a, p)`, `bytes_ends_with(a, p)` | The result is exactly the prefix or suffix relation |
| `bytes_cmp(a, b) -> i8` | Lexicographic order, -1, 0 or 1; 0 exactly when the bytes are equal |
| `bytes_find_byte(b, c, from) -> u64` | The first index at or after `from` holding `c`, or `blen(b)`: every byte skipped differs from `c` |
| `bytes_find(hay, needle, from) -> Option[u64]` | A `Some(i)` is a real occurrence at or after `from`. Not proved: that it is the first one, or that `None` means there is none |
| `bytes_slice(b, from, to) -> Bytes` | A copy of exactly those bytes |
| `bytes_concat(a, b) -> Bytes` | `a` then `b`. Needs `blen(a) + blen(b) <= 2^60`: every `Bytes` is known to be at most 2^60 long, but two of them together are not, since nothing bounds the memory of all live values at once. For strings of unknown length, build a text instead |
| `bytes_copy_into(dst, at, src)` | `src` lands at `at`, and the rest of `dst` is untouched |
| `bytes_copy_span(dst, at, src, from, to)` | `src[from..to]` lands at `at` in one `bcopy`, and the rest of `dst` is untouched |
| `bytes_span_eq(a, i, b, j, n)` | Exactly whether `a[i..i+n]` and `b[j..j+n]` are equal, stated over the positions `p` of the first span (`bget(a, p) == bget(b, p - i + j)`); `a` and `b` may be the same string |
| `bytes_put_byte(dst, at, c)`, `bytes_put_str(dst, at, s)` | Write into a fixed buffer as much as fits and return the position after it, so writes chain and the overflow is dropped; nothing outside what was written changes |
| `bytes_trim(b, s: Span) -> Span` | Only white space was cut, and the result does not start or end with white space |
| `bytes_split(b, sep: u8) -> Vec[Span]` | At least one piece, every piece a span of `b` |
| `bytes_hash(b) -> u64` | (FNV-1a; nothing to prove beyond safety) |
| `bytes_vec_free(v: Vec[Bytes])` | Frees the vector and every string in it |
| `is_space(c)` | Space, tab, newline or carriage return |

### `core/text.bastio`

`text_new()`, `text_push(t, c)`, `text_push_str(t, s: Str)`, `text_push_bytes(t, b)`, `text_push_span(t, b, from, to)`, `text_to_bytes(&t) -> Bytes`, `text_finish(t) -> Bytes`. Each push is proved to add exactly the given bytes at the end and to leave the earlier bytes alone (spec fn `text_keeps`), and `text_to_bytes` to copy every byte. The `fmt_*` functions keep the earlier bytes too.

### `core/num.bastio`

| Function | Proved |
| --- | --- |
| `parse_u64(b, from, to) -> Option[u64]` | Decimal. `Some` only if the range is non-empty and all digits (spec fn `all_digits`), and then its value is `bdec(b, from, to)`, the number the digits denote; never overflows |
| `parse_u8`, `parse_u16`, `parse_u32` | The same (value included), and the value fits the type |
| `parse_i64` and `parse_i8` ... `parse_i32` | An optional `-`, then digits; the value is `bdec` of the digits, negated after a `-` (spec fn `dec_signed`), and fits the type, down to the minimum |
| `parse_hex_u64(b, from, to)` | Hexadecimal digits, either case, no `0x`; `Some` only if all are hex digits (spec fn `all_hex`), and then its value is `bhex(b, from, to)` |
| `fmt_u64(t, x)` | Appends 1 to 20 bytes, every one a decimal digit |
| `fmt_i64(t, x)` | A `-` when negative, then the digits; 1 to 21 bytes |
| `fmt_u64_pad(t, x, width)` | Left-padded with spaces |
| `fmt_hex_u64(t, x, min)` | Lowercase hex, at least `min` and at most 16 digits |
| `bytes_put_u64(dst, at, x)` | Writes `x` in decimal into a fixed buffer, as many leading digits as fit (at least one when there is room), and returns the position after them; every byte written is a digit and nothing else changes |
| `min_u64(a, b)`, `max_u64(a, b)` | The smaller or larger of the two |
| `dec_digits(x)`, `is_digit(c)`, `hex_value(c)`, `hex_char(d)` | Helpers |

Narrower integers format through `fmt_u64(t, cast[u64](x))`: widening needs no proof. The tests check that formatting and parsing are inverse on every power of two and its predecessor and on the extremes of `i64`. Parsing is proved to return the digits' value; that `fmt_u64` writes the digits of `x` is tested, not proved (it writes into a `Vec[u8]`, and `bdec` reads `Bytes`).

### `core/wtext.bastio`

Text kept one character a word, in a `Vec[u64]`, as a compiler keeps source and output it mixes with other words (bastioc1 does). Every function is total: a position or span out of range is no match, so a caller needs no bounds facts.

| Function | Proved |
| --- | --- |
| `wtext_push_str(t, s)` | Appends exactly the characters of `s` (spec fn `wtext_is_str_at`) and keeps what was there (`wtext_keeps`) |
| `wtext_push_u64(t, x)`, `wtext_push_hex(t, x, min)` | Append 1 to 20 decimal digits, or lower-case hex digits padded to `min`, keeping what was there |
| `wtext_push_span(t, src, from, to)`, `wtext_push_all(t, src)` | Append `src[from..to]` (when in range) or all of `src`, word for word |
| `wtext_has_str_at(t, at, s)` | Exactly whether `t` holds `s` from `at` |
| `wtext_same(t, i, j, n)` | Exactly whether `t[i..i+n]` and `t[j..j+n]` are in range and equal |

### `core/print.bastio`

`print_str(con, s: Str)`, `print_bytes(con, b)`, `print_span(con, b, from, to)`, `print_text(con, t)`, `print_u64`, `print_u64_pad`, `print_i64`, `print_hex` (with `0x`) and `print_nl`. They take `&Serial`, which is standard output on the hosted target and COM1 under BastiOS.

### `core/endian.bastio`

`load_le16/32/64`, `load_be16/32/64` and `store_le16/32/64`, `store_be16/32/64` on `Bytes` at an offset. Every access is proved in bounds; the 16-bit loads and stores also state their value in terms of the two bytes.

### `core/hashmap.bastio`

A hash map from `u64` keys to values of a type `V` that can be copied (`u64`, `Span`, a struct of numbers): open addressing, linear probing, tombstones, rebuilt (and doubled when half full of keys) at three quarters use. Every function takes `V` as its type argument, since type arguments are not inferred.

```text
let mut m = hmap_new[u64]();
while ... invariant hmap_ok[u64](m) {          // a loop that changes a map says it stays well formed
  let prev = hmap_insert[u64](&mut m, key, value);
}
match hmap_get[u64](m, key) { Option::Some(v) => { ... }, Option::None => {} }
hmap_free[u64](m);
```

`hmap_new`, `hmap_with_cap`, `hmap_len`, `hmap_insert` (returns the replaced value), `hmap_get`, `hmap_get_or`, `hmap_contains`, `hmap_remove`, `hmap_keys`, `hmap_free`, and `hash_u64`, the mixing function it uses.

Proved, for every `V`: every index is in bounds and no counter overflows; `len` is the number of keys and `used` the number of slots that are not empty (both from `hmap_ok`, which every function needs and keeps, and which states them with `vcount`); after `hmap_insert(m, k, v)` the map holds `k` with `v` (spec fn `hmap_maps`, which compares struct values with `==`); a value `hmap_get` returns is one the map holds for that key. Not proved: that a lookup finds every key present. Inserting a new key falls back to a plain scan for an empty slot if its probe sequence finds none; the probe sequence visits every slot, but the prover does not follow it around the table, so the scan is there and its failure is what is proved unreachable.

**Linear values.** A `HashMap[Buf]` is rejected: `hmap_get` copies the value out. To map keys to buffers or other linear values, keep the values in a `Vec[T]` and map each key to its index (`tests/pass/core_hashmap.bastio` does this).

### `core/bytesmap.bastio`

A hash map from byte strings to values of a type `V` that can be copied, for symbol tables, paths and names. The map owns copies of its keys, packed into one byte arena (slot `i`'s key is `arena[starts[i] .. starts[i] + lens[i]]`), so a key costs no allocation of its own and lookups take a `&Bytes` without copying it. The table is managed as in `core/hashmap.bastio`. Growing it also compacts the arena, which drops the bytes of removed keys.

```text
let mut m = bmap_new[u64]();
bmap_insert[u64](&mut m, name, 7);             // copies `name` into the map
match bmap_get[u64](m, name) { Option::Some(v) => { ... }, Option::None => {} }
bmap_free[u64](m);
```

`bmap_new`, `bmap_with_cap`, `bmap_len`, `bmap_insert` (returns the replaced value), `bmap_get`, `bmap_get_or`, `bmap_contains`, `bmap_remove`, `bmap_key` (a copy of the key in a slot that holds one, for walking the map), `bmap_free`.

Proved, for every `V`:
- Every index is in bounds; in particular every slot that holds a key names bytes inside the arena (part of `bmap_ok`), so comparing and copying keys needs no checks.
- `len` and `used` count, as for `HashMap`.
- A slot a lookup returns holds exactly the bytes asked for (spec fn `bmap_key_at`).
- After `bmap_insert(m, k, v)` the map holds `k` with `v` (spec fn `bmap_maps`).

Not proved: that a lookup finds every key present.

### `core/sort.bastio`

| Function | Proved |
| --- | --- |
| `sort_u64(v)` | Insertion sort. Afterwards the vector is sorted (spec fn `sorted`) and a permutation of what it was (spec fn `same_elems`: same length, and each value as many times) |
| `merge_sort_u64(v)` | The same guarantee as `sort_u64`, in O(n log n) time: merge sort, with insertion sort below 16 elements. Uses n words of extra memory |
| `sort_by_key_u64(v, keys)` | Needs every element of `v` to be an index into `keys` (spec fn `all_indices_below`). Afterwards `v` is a permutation of what it was and ascends by key, indices with equal keys ascending too (spec fns `sorted_by_key` and `key_le`), so the indices it returns read `keys` with no further check. Sorting `0..n` this way is a stable sort of n records by a field. Merge sort |
| `lower_bound_u64(v, x)` | Needs `sorted(v)`. The first index whose element is `>= x`: everything before it is smaller, everything from it on is not |
| `sorted_contains_u64(v, x)` | Needs `sorted(v)`. Exactly whether `x` occurs |
| `vec_contains_u64(v, x)` | A linear scan; exactly whether `x` occurs |
| `vec_extend_u64(dst, src)`, `vec_clone_u64(v)` | Append every element of `src`, or copy `v`, element for element |

Insertion sort is quadratic in the worst case; on 50,000 random words it takes 4.8 s where `merge_sort_u64` takes 0.02 s. The merge sort deals the vector out into two, sorts each, and merges them by popping the larger tail, so every step of its proof is about the ends of vectors and the counts in them (`vcount`); no lemma about index arithmetic is needed. To sort records by a field, put the field in `keys` and sort the indices 0..n.

### `core/ring.bastio`

A `Ring { h, n, cap }` holds only the indices of a ring buffer: element `i` (0 the oldest) is in slot `(h + i) % cap`, and the elements live wherever the caller keeps them (a console's `Bytes`, a `Buf` of command records, a slice of a bigger buffer). Every function needs and keeps `ring_ok(r)`: `0 < cap <= 2^63`, `h < cap`, `n <= cap`.

| Function | Proved |
| --- | --- |
| `ring_new(cap)` | Empty, head at slot 0 |
| `ring_slot(r, i)` | Needs `i < cap`. The slot of element `i`, below `cap` (spec fn `ring_at`) |
| `ring_push(r)` | Needs room. Claims and returns the slot after the last element; `n` grows by one, `h` stays |
| `ring_pop(r)` | Needs an element. Returns the head's slot; the head moves on by one, wrapping |
| `ring_push_n(r, k)`, `ring_pop_n(r, k)` | The same for `k` elements at once, for callers that copy runs of bytes |
| `ring_room`, `ring_empty`, `ring_full`, `ring_clear` | Exactly `cap - n`, `n == 0`, `n == cap`; empties it |

`Queue { r: Ring, q: Buf }` is the common case, a FIFO of `u64` words (`queue_new(cap)`, `queue_push`, `queue_pop`, `queue_peek`, `queue_len`, `queue_free`): `queue_pop` and `queue_peek` return the word in the head's slot. The contents are the sequence `queue_at(q, 0)`, `queue_at(q, 1)`, ... (oldest first). `queue_push` appends to it and `queue_pop` returns its first word and shifts the rest, so words come out in the order they went in. `queue_at` is an opaque spec fn, so users' proofs never see the slot arithmetic (`tests/pass/queue_fifo.bastio`). A full queue is a precondition, so the caller decides whether to drop (a console) or wait (a pipe).

### `core/bitmap.bastio`

An allocation map is a `Bytes` with one byte per entry (a disk block, a page, a slot): 0 free, anything else in use. Functions take a range `lo..hi`, so a map can reserve its first entries.

| Function | Proved |
| --- | --- |
| `map_find_free(m, lo, hi, hint)` | Searches from `hint` to `hi`, then from `lo` (a hint outside the range starts at `lo`). Returns a free entry in `lo..hi`, or `hi` only when all of `lo..hi` is in use (spec fn `map_full`) |
| `map_take(m, lo, hi, hint)` | The same, and marks the entry it returns in use; no other byte changes (spec fn `map_same_but`) |
| `map_give(m, i)` | Marks entry `i` free and nothing else |
| `map_fill(m, lo, hi, v)` | Sets `lo..hi` to `v` and leaves the rest |
| `map_is_free(m, i)`, `map_count_free(m, lo, hi)` | Exactly whether byte `i` is 0; a count of at most `hi - lo` |

`Bits` packs 64 entries in each `u64` word (`bits_new(n)`, all free; a set bit is in use), an eighth of the memory, with the same operations and guarantees, stated per bit with `has_bit` (spec fn `bits_at(b, i)`):

| Function | Proved |
| --- | --- |
| `bits_find_clear(b, lo, hi, hint)` | As `map_find_free`: a free entry in `lo..hi`, or `hi` only when all of `lo..hi` is in use (spec fn `bits_full`). A word whose 64 entries are all in use is skipped with one comparison |
| `bits_take(b, lo, hi, hint)` | The same, and sets the entry it returns; no other bit of any word changes (spec fn `bits_same_but`) |
| `bits_set(b, i)`, `bits_clear(b, i)`, `bits_give(b, i)` | Set or clear entry `i` and nothing else |
| `bits_test(b, i)`, `bits_count_clear(b, lo, hi)` | Exactly whether entry `i` is in use; a count of at most `hi - lo` |
| `bits_grow(b, n)` | Grows the map to `n` entries, keeping every old one; the new ones are clear. Returns false, with the map unchanged, when memory runs out |
| `bits_free(b)` | |

### `core/slots.bastio`

A growable table of numbered slots, for ids a program hands out and takes back: process ids, file descriptors, pipe and socket numbers. A `Slots[V]` is a `Vec[Option[V]]`; slot `i` is in use when it holds `Some`, and ids at or past the end are free. `V` must be a type that can be copied, as for `HashMap`; to number linear values, keep them in a `Vec[T]` and store the index.

```text
let mut fds = slots_new[u64]();
let fd = slots_alloc[u64](&mut fds, 0, nofile, row);   // the lowest free descriptor below the limit
if fd == nofile { /* EMFILE */ } else if fd == SLOTS_NOMEM { /* ENOMEM */ }
let pid = slots_alloc[u64](&mut procs, last + 1, max, p); // search past the last id, so a dead one waits
match slots_take[u64](&mut fds, fd) { Option::Some(row) => { ... }, Option::None => { /* EBADF */ } }
slots_free[u64](fds);
```

| Function | Proved |
| --- | --- |
| `slots_alloc(s, from, limit, v)` | Puts `v` in the lowest free slot of `from..limit` and returns it, adding slots when it is past the end; every slot from `from` up to it was in use. Returns `limit` only when all of `from..limit` is in use (spec fn `slots_full`), and `SLOTS_NOMEM` when the table had to grow and memory ran out; either way it changes nothing and never stops the program. No other slot changes (spec fn `slots_same_but`) |
| `slots_find_free(s, from, limit)` | The same search, without taking the slot |
| `slots_take(s, i)` | Frees slot `i` and returns what it held (`None` if free or past the end); nothing else changes |
| `slots_set(s, i, v)` | Replaces the value of a slot in use (a free one is rejected, `E-VERIFY-REQUIRES`) |
| `slots_get(s, i)`, `slots_used(s, i)` | Exactly what slot `i` holds, for any `i` (spec fns `slots_maps`, `slots_has`) |
| `slots_cap(s)`, `slots_count(s)`, `slots_new()`, `slots_free(s)` | The number of slots; a count of those in use, at most the capacity |

The table never shrinks: a closed descriptor's slot stays, free, for the next one.

### `core/rows.bastio`

A table kept as one `Buf` with a row of `stride` words per entry, indexed as `get(t, id * stride + field)`, is what a kernel's process table and descriptor rows look like. `rows_grow(t, stride, k)` makes it `k` rows longer without touching the code that indexes it.

| Function | Proved |
| --- | --- |
| `rows_grow(t, stride, k)` | The length grows by `k * stride`; every old word is unchanged; every new word is 0 (so 0 should be the "free" value of the field that says a row is in use). Returns false, with `t` unchanged, when memory runs out, so a kernel can answer ENOMEM instead of stopping |

### `core/csum.bastio`

| Function | Proved |
| --- | --- |
| `csum_add(s, v)` | One step of a ones'-complement sum of 16-bit words: `s + v`, less 65535 when it carries |
| `csum_sum16(b, off, n, s0)` | The sum of bytes `off..off+n` as big-endian words, added to `s0` (an odd byte padded); a 16-bit value |
| `csum_inet(b, off, n)` | The Internet checksum (RFC 1071): 0 over a header that holds its own |
| `csum_pseudo4(src, dst, proto, size)` | The IPv4 pseudo-header's sum, to start a UDP or TCP sum |
| `crc32(b, off, n)`; `crc32_start`, `crc32_update`, `crc32_end` | CRC-32 (IEEE 802.3), in one call or in pieces; bit by bit, no table |

That the sums are the checksums the standards define is checked by the tests on known values (`tests/pass/core_csum.bastio`: the CRC-32 check value of "123456789" and an IPv4 header), not proved.

### `std/host.bastio`

`eprint_str(h, s)`, `eprint_text(h, t)`, `host_arg_is(h, i, s)`, `read_file(h, path) -> Option[Bytes]`, `read_file_str(h, path: Str)`, `write_file_str(h, path: Str, data)`.

## Gaps

- **Proofs in a program that counts.** When a struct invariant uses `vcount` (BastiOS's kernel invariant does, through a `HashMap`), every function in the program gets the counting lemmas, and with them Z3 no longer instantiates some quantifiers by itself. Every core function is verified in that setting too (`tests/pass/core_all_counting.bastio`); a contract that relates two strings at two offsets is stated over the positions of one of them, and `rows_grow` fills new rows with zeros instead of copying a template row (the nested quantifier did not prove there).

What the library would say if the language let it. Each is a language or verifier change, not a library change. (Earlier gaps, all closed: `old(x)` is allowed in loop invariants, which the text functions and `bytes_copy_into` use; a local variable may share a function's name, so importing core no longer reserves its local names; a `spec fn` may be generic; `vcount` counts the elements of a vector, which proves the sort a permutation and the map's counters; and imports are searched for, so `import "core/text.bastio"` works from anywhere.)

No language gap is open at the moment. One library proof is missing:

- **A lookup finds every key present.** `HashMap` and `BytesMap` prove that a slot a lookup returns holds the key asked for, not that a lookup finds a key the map holds. That needs an invariant about runs of probed slots (no empty slot between a key's home and the key), which no one has written yet.

(Closed last: `has_bit` states what each bitwise operator does to every bit, so `Bits` packs 64 entries in a word with the same guarantees as the byte maps; an `opaque spec fn` (`queue_at`) gives `Queue` an abstract view of its contents, so it is proved to be a FIFO; `bdec` and `bhex` name the value of a digit string, so every `parse_*` is proved to return the value of its digits; and an existing `Bytes` is known to be at most 2^60 long, which is all a single value can say.)

## Where it came from

A survey of the hand-written helpers in `stage1/`, BastiOS (`src/` and `user/`) and `tests/pass/`, on 2026-10-10, found these copies. They are the candidates for moving onto core; stage1 and BastiOS are changed by their own owners. BastiOS's endian loads and stores and its range comparison moved onto core first (bastios #101).

| Helper | Copies | core |
| --- | --- | --- |
| Print a literal / a decimal number to the console (`puts`, `put`, `put_num`) | 7 and 7 (BastiOS user programs, `stage1/out.bastio`, tests) | `print_str`, `print_u64` |
| Padded number (`put_num_pad`) | 2 | `print_u64_pad` |
| Print a byte range (`put_text`, `put_bytes`) | 4 | `print_span` |
| Byte-string equality: range vs literal, range vs range, bytes vs bytes (`spells`, `same`, `name_is`, `is_cmd`, ...) | about 11 | `bytes_span_eq_str`, `bytes_eq`, `bytes_eq_str` |
| Prefix and suffix (`starts_text`, `ends_text`) | 2 | `bytes_starts_with`, `bytes_ends_with` |
| Trim (`trim`, `trim_rng`, `skip_spaces`) | 3 | `bytes_trim` |
| Concatenate, copy, slice (`concat`, `b1_concat`, `copy_bytes`, `line_bytes`) | about 6 | `bytes_concat`, `bytes_slice`, text |
| Append a literal / a number to a text buffer (`cg_s`, `out_str`, `push_str`; `cg_n`, `push_num`, `out_num`) | 4 and 3 | `text_push_str`, `fmt_u64` (stage1's texts are `Vec[u64]`, not `Vec[u8]`) |
| Decimal and hex parsing (`parse_num`, the lexer's literal, `hex_val`) | 3 | `parse_u64`, `parse_hex_u64`, `hex_value` |
| Hex output (`put_hex`, `put_hex2`, `cg_hex2`) | about 4 | `fmt_hex_u64`, `print_hex` |
| Endian loads and stores (`be16`, `be32`, `put16`, `rd16`, `rd32`, `get_u64`, `put_u64`) | about 7 (BastiOS network, ELF and disk code) | `core/endian.bastio` |
| Free a `Vec[Bytes]` (`drop_argv`, `drop_args`) | 2 | `bytes_vec_free` |
| Vector contains (`has_id`, `has_u`) | 2 | `vec_contains_u64` |
| Minimum (`smaller`) | 2 (121 uses in BastiOS) | `min_u64` |
| Span against span (`same`, `same_name`, `name_is`, `x86_same_text`, ...) | about 9 | `bytes_span_eq`, `wtext_same` |
| Writes into a fixed reply buffer that drop the overflow (`put_s`, `put_c`, `put_n` in /proc) | 1 (about 146 uses) | `bytes_put_str`, `bytes_put_byte`, `bytes_put_u64` |
| Text as `Vec[u64]`: append a literal, a number, hex; prefix and span compare (`cm_s`, `out_str`, `x86_num`, `cm_num`, `x86_has_prefix`, ...) | about 20 (bastioc1, about 760 calls) | `core/wtext.bastio` |
| Vector append and copy (`x86_append`, `copy_set`, `x86_copy_text`) | 3 | `vec_extend_u64`, `vec_clone_u64`, `wtext_push_span` |
| Insertion sort | 2 (stage1's sorts compare records by a key) | `sort_u64`, `merge_sort_u64`, `sort_by_key_u64` (when the key is a number) |
| Lookup by key: every table is a linear scan (`find_inst`, `env_find`, `find_file`, ...) | many | `HashMap`, `BytesMap` (for names and paths) |
| Ring buffers: head and count over a fixed capacity (console output, tty input, pipes, AF_UNIX channels, the disk command queue) | 6 (BastiOS) | `Ring`, `Queue` |
| Allocation maps, a byte or word per entry with a wrap-around search (disk blocks, pages) | 2 (BastiOS) | `core/bitmap.bastio` (`map_*` bytes, `Bits` packed) |
| Fixed-size id tables: process table, descriptors per process, pipes, sockets (`NPROC`, `NFD`, ...) | several (BastiOS) | `Slots`, `rows_grow`, `bits_grow` |
| The Internet checksum (`cadd`, `sum16`, `csum`, `l4_sum`) | 1 (BastiOS network) | `core/csum.bastio` |
