# Changelog

Notable changes to Bastio, the language and its compilers. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Bastio is
pre-release: before 1.0, any minor version may change the language
incompatibly.

The version is the one in `stage0/Cargo.toml`. A release is a tag `vX.Y.Z` on
`main`; `.github/workflows/release.yml` builds it (see
[docs/releasing.md](docs/releasing.md)).

## [Unreleased]

What will be 0.2.0.

### Tools

- `host_net` operation 2 takes a time limit in the bits of `b` above the port (`std/net`'s `net_connect_timeout`): a connect that does not finish in time fails, and reads and sends on that socket give up after the same time. `dns_lookup` and `https_fetch` use it. Windows ignores the limit.

- `tools/bastiofmt`: a source formatter written in Bastio (`bastiofmt file.bastio`,
  `--write`, `--check`, `-` for standard input). It lays a file out from its
  tokens and refuses to print anything that does not lex to the same tokens as
  its input. See [tools/README.md](tools/README.md).
- `tools/bastio-lsp`: a language server written in Bastio. Diagnostics from
  `bastioc1 --check` (and the lexer while typing), go to definition and hover
  in the file and its imports, references, highlights and rename, completion
  and signature help, workspace symbols, semantic tokens, document symbols,
  formatting, quick fixes (code actions) for the common compiler errors and
  inlay hints (parameter names at calls, the type of a `let` from its value), folding
  ranges and links from imports to their files.
  The compiler checks the text of the buffer, not the saved file; with the
  option `checkOnChange` it runs on every edit.
- `editors/vim`: file type, syntax, indentation and a Neovim plugin that
  starts the language server.

### Compiler

- `bastio`, a project tool (`stage1/bastio.bastio`), and `bastioc1 --pkg NAME=DIR`.
  A directory with a `bastio.toml` is a project; its `[dependencies]` are
  local directories or git repositories (checked out at a tag or commit and
  pinned in `bastio.lock`), imported by name: `import "util/math.bastio"`.
  `bastio build`, `check` and `update`. See docs/packages.md.

- The page-table prelude marks a page mapped writable (`pt_put_frame`,
  `pt_put_shared` with `writable`) no-execute on x86-64, and leaves a
  read-only one executable, so a kernel that sets `EFER.NXE` gets W^X from
  how it maps pages. `pt_set_access` leaves the bit alone.

- Diagnostics say what to do about the slips a newcomer meets first: `=`
  where `==` is meant, `+=`, `var` and `for`, a parameter without its type, a
  block that is never closed, a function that can reach its end without
  returning, an integer of the wrong type (with the `cast` and `wrap`
  spelling for the type wanted), a `return` of a value from a function with
  no return type, a linear value consumed in one branch of an `if`, a variant
  matched twice, a call with the wrong number of arguments (the parameter types are listed), and a failed proof of `vec_free` or `unreachable`. Each
  message has a golden test in `tests/diag`.
- `BASTIO_PROOF_CACHE=<dir>`: bastioc1 records each function it has proved
  (by a hash of its verification conditions, the preamble, the solver and the
  solver's version) and does not send it to the solver again. Checking
  `stage1/bastioc1.bastio` takes about 3.3 s with a warm cache instead of
  13.9 s; changing one function re-proves only that function. Off unless the
  variable is set; the cache is trusted like the solver (see
  docs/trusted-base.md). The key is a hash of the conditions' term structure,
  each shared term hashed once, so a warm check of the BastiOS kernel takes
  about 14 s instead of 27 s.

### Language

- `try_alloc` and `try_balloc`: allocation that returns an empty buffer when
  memory runs out instead of stopping the program. The prover makes the
  program check the length before using it.
- `vec_try_reserve` and `vec_push_within`: room for elements is reserved
  ahead of time, or the reservation reports that memory ran out, and pushes
  into reserved room never allocate. The prover tracks a vector's capacity
  (`vcap` in specifications). `alloc`, `balloc`, `vec_push` and `box` still
  stop the program on out of memory.
- `try_box[T](x)`: a box that reports running out of memory. It returns
  `Boxed::Ok` with the box, or `Boxed::Full` with `x` given back, from
  `enum Boxed[T]` in the new `core/boxed.bastio`.
- Page tables shared between processors: a processor takes a `Lease` on a
  top-level table with `mmu_enter` and gives it back with `mmu_exit`, and
  while a tree has leases nothing can be taken out of it, only added
  (`pt_leases`, `pt_lease_count`, `E-VERIFY-LEASE`). `pt_borrow` reaches
  lower tables and pages in place. `mmu_switch` is removed (#83, #88).
- Bulk copies proved in bounds, with no run-time checks: `bcopy` and `bmove`
  between and within `Bytes`, `frame_to_bytes` / `bytes_to_frame` for pages
  and `dma_to_bytes` / `bytes_to_dma` for lent pages. The prover knows what
  `bcopy` and `bmove` copy; page contents stay untracked (#91).
- `frame_to_frame` copies a range of one page into another in a single copy,
  proved in bounds like the other bulk copies, so a kernel can move file data
  from a cached page to a user page without staging it in a buffer.
- `Dirty` pages: `pool_take_dirty` takes a page without zeroing it, and it
  becomes a `Frame` only by being overwritten whole (`dirty_zero`,
  `frame_copy`, `shared_copy`, or `dirty_fill` from a page's worth of
  bytes), so a kernel can take pages under a lock and fill them outside it,
  or fill a page from a file without zeroing it first, without ever
  exposing what they held.
- `pt_writable`: whether a level-1 page-table entry maps its frame
  writable (#81).
- `pt_set_access` and `pt_accessible`: a level-1 slot can keep its page
  while the program may not touch it (`PROT_NONE`), or only read it. The
  table still owns the page (`pt_has` stays true), so it is taken out and
  freed as before. The present bit is cleared and PTE bit 10 marks the
  slot, in the runtime prelude (trusted). Not in the native x86 backend.
- `boot_fb`, the framebuffer the boot loader reported, and a device window
  from 2 GiB (#67) up to 2^46 (#77).
- `boot_acpi`, a byte of one of the firmware's ACPI tables, so a kernel
  parses them (and the AML it needs) in Bastio, proved in bounds.
- `boot_info`, the platform's answers about the machine (BastiOS: the boot
  command line, the memory map, CPUID), unchecked input to the program.
- `old(x)` in loop invariants (#73).
- Generic `spec fn`s, and `vcount` for counting in specifications; local
  variables may share a function's name (#74).
- `==` on structs and enums in specifications (#86).
- `opaque spec fn`: a spec fn whose body is used only in its own file and
  is an uninterpreted function everywhere else, so a library can describe
  its data abstractly. `core/ring`'s `Queue` is now proved to be a FIFO
  (`queue_at`).
- `has_bit(x, i)` in specifications, and what every bitwise operator does
  to each bit, so packed bitmaps are proved to change only the bit they
  set or clear.
- Imports are searched for in the library path (`--lib`), so
  `import "core/text.bastio"` works from anywhere (#85).

### Library

- `core/json`: `JsonOut` builds a whole JSON document (`json_out_begin_obj`, `json_out_key`,
  `json_out_str`, `json_out_u64`, `json_out_end`, `json_out_finish`); it tracks commas, keys and nesting,
  so the result is well-formed or `None`.
- `std/http`: `http_post_request` and `http_post` send a `POST` with a body and a content type;
  `http_get`'s response reader reads the answer.
- `examples/todoapi`: a to-do list service over HTTP and JSON (list, add, mark done, delete), proved
  memory-safe whatever a client sends; `GET /todos?done=0|1` filters through `core/url`; `todoapi serve PORT` serves it, and with no arguments it checks itself.
- `core/json`: the scanner refuses a string whose bytes are not valid UTF-8 (a sequence cut off by the end
  of the input is `Partial`), and `json_push_string` writes a byte that starts no valid sequence as U+FFFD,
  so what it writes is always a string the scanner accepts.
- `core/utf8`: `utf8_seq_len`, `utf8_valid` and `utf8_count` check UTF-8 by RFC 3629 (no overlong forms,
  surrogates or values above U+10FFFF); reads are proved in bounds.
- `core/sha256`: `sha256` and `hmac_sha256` (FIPS 180-4, RFC 2104). Reads are proved in bounds and a digest is 32 bytes;
  that it is SHA-256's is checked by the FIPS 180-4 and RFC 4231 vectors, the lengths around a block and a million `a`s.
- `core/chacha20`, `core/poly1305`, `core/chacha20poly1305`: the ChaCha20 stream cipher, the Poly1305 authenticator and
  the AEAD built from them (RFC 8439): `chacha20_xor`, `poly1305`, `aead_seal`, `aead_open` (the tag is compared in
  full before anything is decrypted). Reads are proved in bounds; that the output is RFC 8439's is checked by its
  vectors, a differential run against a Python reference on 25 lengths up to 4097, and every one-bit change of a
  sealed message being refused.
- `core/hkdf`: `hkdf_extract` and `hkdf_expand` (HKDF with HMAC-SHA-256, RFC 5869), the key derivation of TLS 1.3. Reads are proved in
  bounds and `hkdf_expand` returns exactly the number of bytes asked for (up to 8160); that the output is RFC 5869's is
  checked by its three test cases and by digests of outputs from 0 to 8160 bytes against a Python reference.
- `core/x25519`: `x25519` (RFC 7748), the Diffie-Hellman function on Curve25519, with 16-limb field elements. Reads are proved
  in bounds and a result is 32 bytes; that it is RFC 7748's is checked by its vectors, the iterated function after 1 and 1000
  steps, and the exchange of section 6.1.
- `core/der`: `der_item` reads one element of ASN.1 DER (one-byte tags, definite lengths in their shortest form) and `der_content_eq`
  compares its content; an item it returns is proved to lie inside the span it was asked to read. Tested on lengths from 0 to
  65536, on the encodings DER forbids and on a real Ed25519 X.509 certificate walked down to its fields.
- `core/sha512`: `sha512` (FIPS 180-4). Reads are proved in bounds and a digest is 64 bytes; that it is SHA-512's is checked by the
  FIPS 180-4 vectors, the lengths around a block and a million `a`s.
- `core/ed25519`: `ed25519_verify` (RFC 8032), on the field arithmetic of `core/x25519` and `core/sha512`. Reads are proved in bounds;
  that it accepts the signatures RFC 8032 accepts is checked by the RFC's vectors, signatures made by OpenSSL, a signature whose S is
  not below the group order, a key that is not a point, and signatures, keys and messages with a bit changed.
- `core/tls13`: the key schedule of TLS 1.3 with SHA-256 (RFC 8446 section 7): `tls13_expand_label`, `tls13_derive_secret`,
  `tls13_early_secret`, `tls13_next_secret` (handshake and master secrets), `tls13_traffic_key`, `tls13_traffic_iv`, `tls13_nonce` and
  `tls13_finished`. Output lengths are proved; the values are checked against RFC 8448 (early, `derived`, handshake secret) and a
  Python reference.
- `core/httpsurl`, `https_fetch`: `https_fetch(host, url, roots, redirects)` takes an `https://` URL, looks the host up, checks the server against the roots and returns the status and body of the answer, following up to `redirects` redirects (301, 302, 303, 307, 308) to a path on the same server or to another `https://` URL; a redirect to `http://`, to `//host/...` or to a relative path is not followed. `core/httpsurl` splits URLs strictly (bounds, leaks, host 1 to 255 bytes of letters, digits, dots and hyphens, port 1 to 65535, a path that cannot carry a line break into the request are proved or checked; the rest is tested). The answer is complete if it has a `Content-Length` and that many bytes (the server need not send a close_notify) or, without one, if it ended with a close_notify; `Transfer-Encoding` answers are refused. `https_exchange` gives the whole result of an exchange. `examples/httpsget URL ROOTS` is a command-line fetch, tested with a python3 HTTPS server in `tests/tls.sh`.
- `core/dns`, `std/dns`: host name lookup for IPv4. `dns_query` and `dns_parse_a` write the query and read the response (bounds and leaks proved, whatever the resolver sends; the query is checked against dnspython's, responses against dnspython's too, with every too-short prefix refused); `dns_resolv_conf` reads the first IPv4 nameserver of a resolv.conf; `dns_lookup` returns a dotted quad as it is, then reads /etc/hosts (`dns_hosts`; `localhost` is always 127.0.0.1) and then asks that resolver over TCP port 53 (RFC 7766); `dns_lookup_via`/`dns_lookup_at` ask a given one. `std/https` gets `https_get_by_name`. The message id is random and is the only check on the answer; there is no timeout (an unreachable resolver blocks), no AAAA and no cache. `tests/dns.sh` runs it against a small server. `examples/lookup.bastio` is a command-line lookup.
- `core/aes`, `core/aesgcm`: AES-128 (`aes128_new`, `aes128_encrypt`; the S-box is computed, not typed in) and AES-128-GCM (`gcm_seal`, `gcm_open`; the tag is compared in constant time). Bounds and leaks are proved; the values are checked against NIST vectors and ciphertexts made by OpenSSL. The S-box lookups are table lookups, so AES here is not constant time with respect to cache timing. The TLS 1.3 layers (`core/tls13hs`, `core/tls13record`, `core/tls13client`) offer and accept TLS_AES_128_GCM_SHA256 as well as ChaCha20-Poly1305; `tls13_seal` and `tls13_open` take the suite as their first argument.
- `core/tls13record`: the TLS 1.3 record layer with ChaCha20-Poly1305 (RFC 8446 section 5): `tls13_seal` makes a record from a message,
  a content type and padding; `tls13_open` checks the header and the tag and returns the content type and message. Lengths are proved;
  the records are checked against ones made by Python's `cryptography` (sequence numbers, padding, 16 KiB) and every bit change refused.
- `core/tls13hs`: the first two TLS 1.3 handshake messages: `tls13_client_hello` builds a ClientHello (X25519, TLS_CHACHA20_POLY1305_SHA256, ed25519 / ECDSA P-256 / RSA-PSS schemes, server name) and `tls13_parse_server_hello` returns the server key share, refusing a HelloRetryRequest, other suites, groups or extensions and any length that leaves its part. Bounds and lengths are proved; the ClientHello is compared with one assembled in Python, and malformed and truncated ServerHellos are refused.
- `core/p256`: ECDSA verification over P-256 with SHA-256 (`ecdsa_p256_verify`: an uncompressed key, a message and a DER signature), with Montgomery multiplication on 16-bit limbs, Jacobian points and a check that the key is on the curve. Bounds are proved; the arithmetic is checked against signatures made by OpenSSL, about 300 one-bit changes of signature, key and message, out-of-range r and s, and non-minimal DER.
- `core/rsa`: RSA signature verification with SHA-256 for keys of 2048 to 4096 bits: `rsa_pss_sha256_verify` (RSASSA-PSS with MGF1-SHA-256 and a 32-byte salt, as TLS 1.3's rsa_pss_rsae_sha256) and `rsa_pkcs1_sha256_verify` (the sha256WithRSAEncryption padding of most certificates), with Montgomery multiplication on 16-bit limbs. Bounds are proved; the arithmetic and the padding rules are checked against signatures made by OpenSSL (2048, 2049, 3072 and 4096 bits, exponents 65537 and 3) and against every padding rule being broken in turn.
- `core/x509`: reading X.509 certificates: `x509_parse` gives the signed part, signature, names, validity times, key (RSA, ECDSA P-256 or Ed25519) and the extensions basicConstraints, keyUsage and subjectAltName, and refuses a critical extension it does not know; `x509_signed_by` checks a certificate's signature with its issuer's key (sha256WithRSAEncryption, ecdsa-with-SHA256, Ed25519), `x509_can_sign`, `x509_valid_at` and `x509_has_name` (case-insensitive, one-label wildcards) complete a chain check. Spans are proved to stay inside the certificate; the tests use certificates made by OpenSSL (a three-level chain, Ed25519, RSA, both time forms), wrong signers, unknown critical extensions, SHA-384 signatures, bad wildcards and a sample of one-bit changes of each certificate.
- `core/tls13client`: a TLS 1.3 client that does no I/O (`tls13c_new`, `tls13c_feed`, `tls13c_send`, `tls13c_close`): it makes the ClientHello, runs the key schedule, decrypts and parses the server's flight, checks the certificate chain against the caller's roots (`x509`), the CertificateVerify signature (Ed25519, ECDSA P-256, RSA-PSS), the Finished MAC, and then carries application data, KeyUpdate and NewSessionTicket. Suite TLS_CHACHA20_POLY1305_SHA256 and group X25519 only. Bounds are proved; `tests/tls.sh` runs `examples/httpsget` against OpenSSL's `s_server` with ECDSA, RSA and Ed25519 certificates, intermediates, wildcards, and refusal of a wrong name, untrusted root, expired or not-yet-valid certificate, broken chain and a non-CA signer. Not done: the caller supplies the random bytes and the time (there is no host entropy or clock call), no path-length or name constraints, no extended key usage or revocation, no SHA-384 or P-384, no session resumption.
- `host_net` operations 7 and 8 and `std/net`'s `net_now` and `net_random`: the system clock (seconds since 1970) and up to 256 random bytes (`/dev/urandom`, `rand_s` on Windows, `crypto.getRandomValues` in WebAssembly, which needs no sockets), on the C backend, the x86-64 and AArch64 native backends and WebAssembly; the length of a draw is checked by `net_random` and proved by its callers. This is what `core/tls13client` needs for its `random` and `now`. The net prelude grows by 34 lines of C and the native runtime by a routine per target; not tested here: the Windows and macOS runtimes (the CI jobs are the check).
- `std/https`: HTTPS over `std/net`: `https_get` and `https_request` connect to an IPv4 address, run the `core/tls13client` handshake against the trusted roots the caller gives, send the request and return everything the server sends up to its close_notify (a connection cut off without one, or any failure, is `None`); `https_run` takes an open socket, the random bytes and the time. The random bytes and the time come from `net_random` and `net_now`. `examples/httpsget` and `tests/tls.sh` use it against OpenSSL's `s_server`. Not done: a name lookup (servers are by IPv4 address), redirects, TLS suites other than TLS_AES_128_GCM_SHA256 and TLS_CHACHA20_POLY1305_SHA256 with X25519, so a server that offers only AES-256 or SHA-384 suites cannot be reached.
- `core/url`: `url_encode`, `url_decode`/`url_decode_span` (percent-decoding, `+` as a space on request) and `url_query_get`/`url_query_start` for the query string of a request target; reads are proved in bounds, a decoded string is never longer than its source and an encoded one at most three times as long.
- `std/http`: `http_header` reads a header field of a response (the name is matched ignoring case,
  the value has its blanks removed), for `Location`, `Content-Type` and the like.
- `core/json`: `json_i64` reads a signed integer (an optional `-` and digits, within `i64`), and
  `json_out_i64` writes one.
- `examples/leb128`: a proved unsigned LEB128 (varint) decoder and encoder called from C, fuzzed
  against a C decoder under AddressSanitizer.
- `core/bytes`: `bytes_rfind` finds the last occurrence of a byte string (a `Some` is a
  proved occurrence). `core/json` is a strict RFC 8259 scanner with readers and a string
  writer; `std/http` reads chunked responses and keeps a connection alive after a
  `Content-Length` or chunked request body; `host_net` works on macOS and Windows.
- `core/strsort`: `spans_order` sorts pieces of a text in byte order by
  returning the indices `0..n` in order, proved a permutation with every
  neighbouring pair in order; `core/spancmp` has `span_cmp` for one pair.
- Six new modules. `core/path` splits file names (`path_split`,
  `path_ext`), joins them (`path_join`) and cleans them lexically
  (`path_clean`); every span it returns is proved to lie inside the path.
  `core/set` is a `HashSet` of `u64`. `core/ascii` has character classes and
  case conversion, proved byte for byte. `core/lines` splits text into
  lines (`\n` and `\r\n`), proved to give spans with no newline in them.
  `core/format` pads columns, joins, replaces, quotes and prints sizes
  (`fmt_pad_left`, `fmt_join`, `fmt_replace_all`, `fmt_quoted`, `fmt_size`).
  `core/scan` splits at white space (`scan_words`), at a separator string
  (`scan_split_str`) and once at a byte (`scan_split_once`, for `key=value`).
  `std/host` gained `write_file`, `file_exists`, `env_get` and `host_args`.
- A reference page for every library module (docs/lib/, and in the site):
  each function with its contract as the compiler checks it, what it is for,
  and an example. `tests/libref.py` generates the pages from the doc
  comments in core/ and std/, and `tests/run.sh` checks that they are up to
  date, that every function has a doc comment and an example, and that
  every example builds, verifies and runs.
- `checked_add_u64`, `checked_sub_u64`, `checked_mul_u64` (`None` when the
  result does not fit), `sat_add_u64`, `sat_sub_u64`, `div_ceil_u64`,
  `align_down_u64` and `align_up_u64`, each proved to compute exactly that.
- `vec_find_u64` and `sorted_find_u64`: the first index of a value, with
  `None` proved to mean it does not occur; `vec_min_u64`, `vec_max_u64` and
  `vec_reverse_u64`; `bytes_rfind_byte`, the last index of a byte.
- A first `core/` and `std/`: byte-string comparison, search, trimming and
  splitting; text building on `Vec[u8]`; integer parsing and formatting for
  every sized type; console printing; endian loads and stores; a hash map;
  insertion sort and binary search; host files and standard error (#69).
  The hash map is generic, `HashMap[V]` (#86).
- Every `parse_*` is proved to return the value of its digits (`bdec`,
  `bhex`) (#93, #98).
- `core/ring` (`Ring`, `Queue`), `core/bitmap` (allocation maps) and
  `core/csum` (Internet checksum, CRC-32) (#98). The bitmap also comes
  packed, 64 entries a word (`Bits`).
- `core/slots`: `Slots[V]`, a growable table of numbered slots for process
  ids, file descriptors and the like, proved to hand out the lowest free
  id at or above a starting point, under a limit; `core/rows`'s `rows_grow`
  for tables of fixed-size rows in one `Buf` (new rows are zero); `bits_grow`. All three report
  running out of memory instead of stopping the program.
- Helpers BastiOS and bastioc1 each wrote by hand: `min_u64`, `max_u64`,
  `bytes_span_eq`, `bytes_copy_span`, writes into a fixed buffer that drop
  what does not fit (`bytes_put_byte`, `bytes_put_str`, `bytes_put_u64`),
  `vec_extend_u64`, `vec_clone_u64`, and `core/wtext` for text kept one
  character a word in a `Vec[u64]`.
- `merge_sort_u64`, O(n log n) with the insertion sort's proof, and
  `sort_by_key_u64`, a proved stable sort of indices by a key vector;
  `BytesMap[V]`, a hash map from byte strings; `examples/wordfreq.bastio`
  (#107).

### Compilers

- `--arch wasm32`: `host_net` is a host import, so programs that use `std/net` and `std/http` (and
  `examples/todoapi`) run as a module under Node.js. `web/netsync.js` does the sockets in a worker thread
  while the module waits on a shared buffer; `tests/wasm.sh` runs `std_http` and `todoapi` that way.
- `--arch aarch64` builds for AArch64 with the C backend. Hosted programs
  build for AArch64 Linux in both compilers (#121). In bastioc1, the kernel
  and user targets get AArch64 preludes too: a PL011 console, VMSAv8-64
  page tables in TTBR0, EL0 entry through the platform, `svc` system calls,
  and the barriers AArch64's weaker ordering needs around MMIO and DMA. An
  AArch64 kernel has no `Ports`, its device window starts at 0, and
  `cpu_get`/`cpu_set` use AArch64's saved registers (#122, #123).
  `tests/aarch64.sh` runs the tests under `qemu-aarch64` and boots a small
  kernel on QEMU's virt machine.
- Clearer messages, the same in both compilers: `E-VERIFY-CAST`,
  `E-VERIFY-SHIFT` and `E-VERIFY-ALLOC` end with a hint like the other
  proof failures; a loop body that breaks an invariant shows its
  counterexample as `counterexample after one iteration:`, since the values
  are those after the iteration; a solver that cannot be started is one
  line naming the file, with no source line; `--help` prints the options.
- A counterexample shows the elements of a buffer or vector of at most 8
  scalars, after its length, when the failed proof is an `assert`,
  `requires`, `ensures` or invariant: `vlen(v) = 2, v = [3, 1]` says why a
  vector is not sorted. Bounds and arithmetic errors still show only the
  length.
- bastioc1 is the compiler the tests and benchmarks use: `tests/bastioc1.sh`
  builds it with stage0 and rebuilds it only when stage0 or stage1 changes.
  `BASTIOC` still picks another compiler.
- `bastioc1 --dump-smt` prints every query sent to Z3, as bastioc0 does.
- A test program marked `// stage1-only: <feature>` is skipped when the tests
  run with stage0, for features that land after the stage0 freeze.
- stage0 is frozen: new features land in bastioc1 only. A branch that
  changes `stage0/` must say why in a commit message, with `Stage0-only:`
  (no stage1 counterpart) or `Stage0-shared:` (a bug fix made in both
  compilers, mirror required); `tests/stage0_mirror.sh` checks it. The
  `// stage1-only:` header in BastiOS's `src/main.bastio` leaves its kernel
  out of difftest's comparisons with stage0.
- A native x86-64 backend in the self-hosted compiler (`stage1/x86.bastio`),
  ported from stage0's and checked byte for byte against it (#70). It
  passes up to four argument words in registers (#72) and covers `Child`
  (#76), `Lock`, `Atomic` and the device types (#80). bastioc1 builds
  itself with it (#76), and so do BastiOS's user programs (#82) and kernel
  (#84). It is driven by `stage1/asmdump.bastio`; `bastioc1 --backend
  native` is not exposed yet.
- bastioc1 prints every error sentence and hint that bastioc0 does, except
  a failed proof's sentence (#68); it has `--header` and `--trusted-base`
  (#64), proves in parallel and reads `$CC` (#71).
- Diagnostics name tokens as written, report a missing `;` at the end of the
  line, say where a linear value was consumed, and suggest a close name for
  an unknown variable or function (#87).
- An error shows its source line with the line number, the whole token at
  the error underlined, and its notes aligned under it; `tests/diag` holds
  golden transcripts, checked with both compilers (#102).
- bastioc1 checks its own source in 8.5 s instead of 11.5 s (hash indexes
  instead of linear scans by name), and proves in batches, freeing each
  batch's terms: 175 MB instead of 497 MB on its own source, 325 MB
  instead of 953 MB on the BastiOS kernel (#102).
- Z3 runs with a fixed work limit per query (`:rlimit`) instead of a 10 s
  timeout, so a proof no longer fails on a busy machine (#89).
- Each function is verified on a reset solver, so whether a proof near
  the work limit succeeds no longer depends on which functions shared its
  solver: the same program could be
  accepted with one `BASTIO_JOBS` and rejected with another. Found by the
  fuzzer below.
- `tests/fuzz.py`: random programs whose meaning the fuzzer computes
  itself (a proof that holds while some input overflows, divides by zero,
  shifts too far, indexes out of bounds or breaks an `assert` or `ensures`
  is a false proof; proved programs are also run, through both backends,
  against that meaning), and damaged copies of the tests (no compiler may
  crash or hang; one that is still accepted runs under ASan and UBSan).
  bastioc1 must agree with bastioc0 on every input. CI runs a short
  deterministic run.
- A change to `stage0/src` needs its stage1 mirror, or a `Stage0-only:`
  reason in a commit message; CI checks this on every pull request (#78).
- The kernel prelude (C and native) reads the boot loader's information
  defensively: a framebuffer whose address would wrap `addr + pitch * h`,
  or lies at 4 GiB, is not reported (the verifier takes each `boot_fb`
  field as below 2^32, which such an address broke), and a memory-map entry
  whose size is 0xFFFFFFFC no longer makes the page pool's walk loop forever
  at boot. `tests/prelude_boot.sh` feeds both.
- A 60 s wall-clock backstop on each Z3 query, on top of `:rlimit`: Z3 4.8.12
  (Ubuntu's, and CI's) does not count its work on some nonlinear queries and
  searched them forever, so `bastioc0 --check` hung on a 15-line function
  (a guarded `x * x` on an `i16`). Such a query now gives up. The fuzzers
  kill a timed-out compiler's z3 too, instead of leaving it spinning.
- `tests/progfuzz.py`: random whole programs (helpers, recursion, `&mut`
  calls, a struct, an enum, `Buf`, `Bytes` and `Vec` through bounded loops,
  every integer type) run by an exact reference interpreter, then built by
  bastioc0 and bastioc1 with both backends and by bastioc0's C under ASan
  and UBSan; every build must print what the interpreter printed and exit
  the same way, and a proof the interpreter's run breaks is a false proof.

### Fixed

- `http_read_request` answers an HTTP/1.1 request with no `Host` field, or more than one, with 400 (RFC 9112, section 3.2); it used to pass it on.
- `json_member` compares a key written with escapes as the string it stands for (RFC 8259, section 4): `{"a\u0062": 1}` has the key `ab`. It compared the bytes as written.
- `wrap[T](..)`, `wrap_add`, `wrap_sub` and `wrap_mul` are written for the
  solver without `mod`: the result is a fresh value in the type's range
  that differs from the exact one by a multiple of 2^width. Z3 4.8.12 (the
  version Ubuntu ships, which CI installs) searched past its resource
  limit, forever, on a `mod` of a wrapped value under another `mod` or a
  product; `tests/fuzz.py --seed 1` hit 9 such inputs and every one now
  takes under a second (`tests/fail/wrap_twice_overflow.bastio`).
- A false proof in both compilers. A loop or call that changes a signed
  integer (also inside a struct, enum or box) forgot its value but gave the
  new one the range of `u64`, which leaves out the negative numbers, so an
  `i64` that went negative in a loop could be proved to be 0 after it
  (`tests/fail/loop_signed_range.bastio`, `call_signed_range.bastio`). A
  changed `u8`, `u16` or `u32` got the range of `u64` too, which only made
  true facts about it unprovable. Each now keeps its type's range.
- Another false proof in both compilers: a loop or call did not look
  inside the argument of `cast[T](..)` or `wrap[T](..)`, so a variable
  changed by a call written there kept its value from before
  (`tests/fail/loop_cast_call.bastio`).
- Another false proof in both compilers: what came out of a lock built by
  `lock_new` from an integer was taken for a `u64` whatever its type, so an
  `i64` that a `locked` call made negative could be proved to be at least 0
  after `lock_end` (`tests/fail/lock_signed_contents.bastio`,
  `lock_signed_box.bastio`). The lock now keeps the checker's type of its
  contents, which also makes a `u8` that comes out known to be at most 255.

### Documentation

- Tutorial part 5, `docs/tutorial-web.md`: untrusted JSON, building well-formed JSON, an invariant on server state and serving requests, with every example compiled and run by `tests/run.sh`.
- For contributors: `tools/setup-dev.sh` sets up a Debian or Ubuntu machine in one command (and a dev container runs it), `docs/architecture.md` maps the compiler's phases to files, and CONTRIBUTING.md explains the issue labels and where to find a first issue. A feature-request issue template and `.github/labels.yml` (synced by a workflow) were added.
- Releases: the release workflow also builds for AArch64 Linux (on an arm64 runner, tested there), makes the archives byte-for-byte repeatable, records build provenance attestations, and `docs/releasing.md` says how to verify a download and rebuild a release.
- A documentation index (`docs/README.md`), linked from the README, and a README layout table that lists every document. `docs/integers.md` says the sized integers are implemented. CONTRIBUTING.md has a "Getting oriented" section.

- `docs/bootstrap.md`: how stage0 builds bastioc1, how to check the
  bootstrap's fixed point, and what the stage0 freeze allows.
- The trusted base is listed and explained (`docs/trusted-base.md`, #66).
- `examples/qoi`: a QOI image codec called from C. Its buffers are sized by
  an untrusted header, and running out of memory is a result C sees, not an
  abort. "Calling Bastio from C" in the book is now a guide: building,
  vendoring the generated C, ownership, errors and out of memory, several
  libraries in one program, and what the proofs assume of the C side.
- The tutorial's section on calling Bastio from C walks through a new
  example, `examples/lines`, built on `export fn` and `--header` (#92).
- `tests/run.sh` checks that error messages shown in `docs/` are what the
  compiler prints, that file copies in `docs/` match the files, and that
  `examples/*.bastio` build and run (#92).
- `docs/proof-failures.md`, "Common proof failures": the errors newcomers
  meet first, each with the program, the message, why, and a fix; on the
  book's site after the tutorial.

### Added

- Shared pages: `Shared`, one counted reference to a page several page
  tables map at once (`MAP_SHARED` memory after `fork`), with `shared_new`,
  `shared_dup`, `shared_drop`, `shared_id`, `pt_put_shared`,
  `pt_take_shared`, `pt_dup_shared` and the ghost `pt_shared(t, i)`. The
  proof keeps each reference used once; the count itself is in the runtime
  prelude (trusted). Not in the native x86 backend yet.

### Changed

- `pt_take_frame` now needs `!pt_shared(t, i)` (`E-VERIFY-SHARED`). Code that
  tears down a table it did not build itself tests `pt_shared` and takes
  such slots with `pt_take_shared`.

## [0.1.0] - not yet tagged

The first version: everything up to the self-hosting fixed point.

### Language

- Linear ownership: `Buf`, `Bytes`, structs with linear fields, `Box[T]`,
  `Vec[T]`, generic structs, enums and functions (monomorphized), enums with
  exhaustive `match`, `&T` and `&mut T` borrows. Use-after-free, double free
  and leaks are compile errors.
- Proofs by Z3, with counterexamples on failure: indices in bounds, no
  overflow, underflow or division by zero, `requires` / `ensures` /
  `invariant` / `assert`, and `unreachable;`.
- Integers `u8` to `u64` and `i8` to `i64`, proved never to wrap at any
  width; literals typed by context; `cast[T]` (proved to fit) and `wrap[T]`;
  bitwise operators with shift amounts proved in range.
- `import "path";` for programs that span files.
- Hosted programs: the `Host` capability for files and arguments, and child
  processes with pipes (`Child`).
- Kernel and user targets (`--target kernel`, `--target user`): capabilities
  for physical pages and page tables (`Pool`, `Frame`, `PageTable`, `Mmu`,
  `Cpu`), the serial console, I/O ports (`Ports`, `PortIo`), memory-mapped
  registers (`MmioSpace`, `Mmio`), DMA buffers (`Dma`), several processors
  (`cpu_count`, `smp_run`), `Lock[T]` with struct invariants, lock order
  checked at compile time (`E-LOCK-ORDER`), and `Atomic[T]`.
- The C boundary: `export fn` with a generated header (`--header`), taking
  `&Bytes` and `&mut Bytes` from C.

### Compilers

- `stage0` (Rust): the bootstrap compiler. C backend with no runtime checks,
  cross-checked under sanitizers; one solver scope per function and
  functions proved in parallel (`BASTIO_JOBS`); `--trusted-base` lists what
  the proofs assume; an experimental native x86-64 backend
  (`--backend native`, hosted target only).
- `stage1`: the self-hosted compiler, `bastioc1`, written in Bastio. Lexer,
  parser, checker, verifier and C code generator each produce the same
  output as stage0 (`stage1/difftest.sh`). bastioc1 builds itself and
  reaches a fixed point: gen1, gen2 and gen3 are identical, byte for byte.
  Not yet ported: stage0's error sentences, `--header`, `--trusted-base`,
  `--backend native` and parallel proving.

### Tooling and documentation

- A tutorial, a language reference, and design documents for sized integers
  and concurrency; every `bastio` example in them is compiled by the tests.
- Editor support: a tree-sitter grammar and a VS Code extension.
- `bench/`, comparing generated code with hand-written C.
- Proved C-callable libraries in `examples/`: base64, UTF-8 and DER.
- The documentation website (`site/`).
- Dual licensed Apache-2.0 OR MIT.

[Unreleased]: https://github.com/yukikotani231/bastiolang/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/yukikotani231/bastiolang/releases/tag/v0.1.0
