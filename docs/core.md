# The core and std libraries

Bastio's library comes in two parts, as ROADMAP.md (design consequence 1) decided:

- **`core/`** needs no operating system. It runs on every target (`hosted`, `kernel`, `user`): it uses only allocation, vectors and the serial console, which every target has.
- **`std/`** uses the host's services (files, the command line, standard error) and works on the hosted target only. Importing it from a kernel or user program is rejected with `E-TARGET`.

Everything in both is proved memory safe like any other Bastio code, and most functions also state what they compute (`ensures`), so callers get facts for their own proofs. The library is young: the modules below are what the compiler, BastiOS and the test programs were writing again and again by hand (see [Where it came from](#where-it-came-from)).

## Using it

There are no library search paths yet: import a module by its path relative to the importing file. A module imports what it needs itself, and a file imported twice is loaded once.

```text
import "../core/text.bastio";     // also brings in core/bytes.bastio and core/option.bastio
import "../core/num.bastio";
```

All files share one namespace (see [Programs and files](language.md#programs-and-files)), so every function is prefixed by its module (`bytes_`, `text_`, `hmap_`, ...) except the few whose names say what they are (`parse_u64`, `fmt_u64`, `print_str`, `sort_u64`). Two names are taken without a prefix: the types `Option[T]` and `Span`. A program that imports core must not define its own.

The tests are the worked examples: `tests/pass/core_*.bastio` and `tests/pass/std_host.bastio` use every function, and `tests/user/core_user.bastio` builds core for a BastiOS user program.

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

### `core/bytes.bastio`

| Function | Proved |
| --- | --- |
| `bytes_eq(a, b)`, `bytes_eq_str(a, s: Str)`, `bytes_span_eq_str(a, from, to, s)` | The result is exactly "same length and same bytes" (spec fns `bytes_equal`, `bytes_equal_str`) |
| `bytes_starts_with(a, p)`, `bytes_ends_with(a, p)` | The result is exactly the prefix or suffix relation |
| `bytes_cmp(a, b) -> i8` | Lexicographic order, -1, 0 or 1; 0 exactly when the bytes are equal |
| `bytes_find_byte(b, c, from) -> u64` | The first index at or after `from` holding `c`, or `blen(b)`: every byte skipped differs from `c` |
| `bytes_find(hay, needle, from) -> Option[u64]` | A `Some(i)` is a real occurrence at or after `from`. Not proved: that it is the first one, or that `None` means there is none |
| `bytes_slice(b, from, to) -> Bytes` | A copy of exactly those bytes |
| `bytes_concat(a, b) -> Bytes` | `a` then `b`. Needs `blen(a) + blen(b) <= 2^60`; for strings of unknown length, build a text instead |
| `bytes_copy_into(dst, at, src)` | `src` lands at `at`, and the rest of `dst` is untouched |
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
| `parse_u64(b, from, to) -> Option[u64]` | Decimal. `Some` only if the range is non-empty and all digits (spec fn `all_digits`); never overflows |
| `parse_u8`, `parse_u16`, `parse_u32` | The same, and the value fits the type |
| `parse_i64` and `parse_i8` ... `parse_i32` | An optional `-`, then digits; the value fits the type, down to the minimum |
| `parse_hex_u64(b, from, to)` | Hexadecimal digits, either case, no `0x` |
| `fmt_u64(t, x)` | Appends 1 to 20 bytes, every one a decimal digit |
| `fmt_i64(t, x)` | A `-` when negative, then the digits; 1 to 21 bytes |
| `fmt_u64_pad(t, x, width)` | Left-padded with spaces |
| `fmt_hex_u64(t, x, min)` | Lowercase hex, at least `min` and at most 16 digits |
| `dec_digits(x)`, `is_digit(c)`, `hex_value(c)`, `hex_char(d)` | Helpers |

Narrower integers format through `fmt_u64(t, cast[u64](x))`: widening needs no proof. The tests check that formatting and parsing are inverse on every power of two and its predecessor and on the extremes of `i64`; that is tested, not proved (see [Gaps](#gaps)).

### `core/print.bastio`

`print_str(con, s: Str)`, `print_bytes(con, b)`, `print_span(con, b, from, to)`, `print_text(con, t)`, `print_u64`, `print_u64_pad`, `print_i64`, `print_hex` (with `0x`) and `print_nl`. They take `&Serial`, which is standard output on the hosted target and COM1 under BastiOS.

### `core/endian.bastio`

`load_le16/32/64`, `load_be16/32/64` and `store_le16/32/64`, `store_be16/32/64` on `Bytes` at an offset. Every access is proved in bounds; the 16-bit loads and stores also state their value in terms of the two bytes.

### `core/hashmap.bastio`

A hash map from `u64` keys to `u64` values: open addressing, linear probing, tombstones, rebuilt (and doubled when half full of keys) at three quarters use.

```text
let mut m = hmap_new();
while ... invariant hmap_ok(m) {          // a loop that changes a map says it stays well formed
  let prev = hmap_insert(&mut m, key, value);
}
match hmap_get(m, key) { Option::Some(v) => { ... }, Option::None => {} }
hmap_free(m);
```

`hmap_new`, `hmap_with_cap`, `hmap_len`, `hmap_insert` (returns the replaced value), `hmap_get`, `hmap_get_or`, `hmap_contains`, `hmap_remove`, `hmap_keys`, `hmap_free`, and `hash_u64`, the mixing function it uses.

Proved: every index is in bounds and no counter overflows (from `hmap_ok`, which every function needs and keeps); after `hmap_insert(m, k, v)` the map holds `k` with `v` (spec fn `hmap_maps`); a value `hmap_get` returns is one the map holds for that key. Not proved: that a lookup finds every key present, and that `len` counts the keys. One decrement in `hmap_remove` is guarded by a test rather than proved, for that reason.

**Linear values.** The map holds only numbers. To map keys to buffers or other linear values, keep the values in a `Vec[T]` and map each key to its index (`tests/pass/core_hashmap.bastio` does this). A generic `HashMap[V]` was tried first and dropped: spec fns cannot be generic, so its well-formedness could not be named in a caller's loop invariant.

### `core/sort.bastio`

| Function | Proved |
| --- | --- |
| `sort_u64(v)` | Insertion sort. Afterwards the vector is sorted (spec fn `sorted`) and has the same length. Not proved: that it holds the same elements |
| `lower_bound_u64(v, x)` | Needs `sorted(v)`. The first index whose element is `>= x`: everything before it is smaller, everything from it on is not |
| `sorted_contains_u64(v, x)` | Needs `sorted(v)`. Exactly whether `x` occurs |
| `vec_contains_u64(v, x)` | A linear scan; exactly whether `x` occurs |

Insertion sort is quadratic in the worst case. It was chosen because its proof is short; a heap sort or merge sort with the same guarantee is future work.

### `std/host.bastio`

`eprint_str(h, s)`, `eprint_text(h, t)`, `host_arg_is(h, i, s)`, `read_file(h, path) -> Option[Bytes]`, `read_file_str(h, path: Str)`, `write_file_str(h, path: Str, data)`.

## Gaps

What the library would say if the language let it. Each is a language or verifier change, not a library change. (Two earlier gaps are closed: `old(x)` is allowed in loop invariants, which the text functions and `bytes_copy_into` use, and a local variable may share a function's name, so importing core no longer reserves its local names.)

1. **Generic spec fns.** `spec fn` cannot take type parameters, so a property of a `HashMap[V]` cannot be named; the hash map therefore holds `u64` values only.
2. **Counting.** Specifications have `forall` but no way to count elements, so "the map has `len` keys" and "the sort output is a permutation of the input" cannot be stated. Recursive spec fns or a multiset theory would allow both, and also "this parse gives the value of these digits".
3. **Library search paths.** Imports are relative to the importing file, so where core lives is written into every import. A `--lib` path or an `import core "text"` form would fix it.
4. **Length bounds on `Bytes`.** Nothing says an existing `Bytes` is at most 2^60 long (a `Vec` push gets such a fact), so `bytes_concat` must ask for it.

## Where it came from

A survey of the hand-written helpers in `stage1/`, BastiOS (`src/` and `user/`) and `tests/pass/`, on 2026-10-10, found these copies. They are the candidates for moving onto core. Nothing has been moved yet: stage1 and BastiOS are changed by their own owners.

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
| Insertion sort | 2 (stage1's sort records by a key, which core cannot do yet) | `sort_u64` |
| Lookup by key: every table is a linear scan (`find_inst`, `env_find`, `find_file`, ...) | many | `HashMap` |
