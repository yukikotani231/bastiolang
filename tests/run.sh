#!/usr/bin/env bash
# Runs every test. pass/*.bastio must verify, build, and exit with the code in
# their `// expect-exit: N` header; fail/*.bastio must be rejected with the error
# code in their `// expect-error: CODE` header (a rejection for the wrong
# reason counts as a failure).
set -u
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-stage0/target/release/bastioc0}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
pass=0
fail=0
for f in tests/pass/*.bastio; do
  want=$(sed -n 's|^// expect-exit: *||p' "$f" | head -1)
  name=$(basename "$f" .bastio)
  if ! "$BASTIOC" -o "$tmp/$name" "$f" 2>"$tmp/$name.err"; then
    echo "FAIL $f: rejected"; sed 's/^/    /' "$tmp/$name.err"; fail=$((fail+1)); continue
  fi
  "$tmp/$name" >/dev/null; got=$?
  if [ "$got" = "$want" ]; then pass=$((pass+1)); else echo "FAIL $f: exit $got, want $want"; fail=$((fail+1)); fi
done
for f in tests/fail/*.bastio; do
  want=$(sed -n 's|^// expect-error: *||p' "$f" | head -1)
  if "$BASTIOC" --check "$f" 2>"$tmp/err"; then
    echo "FAIL $f: accepted, want $want"; fail=$((fail+1)); continue
  fi
  if grep -q "error\[$want\]" "$tmp/err"; then pass=$((pass+1)); else
    echo "FAIL $f: wrong error, want $want"; sed 's/^/    /' "$tmp/err"; fail=$((fail+1)); fi
done
# Zero overhead: the generated C must index buffers directly, with no checks.
"$BASTIOC" --emit-c -o "$tmp/z.c" tests/pass/fill_sum.bastio 2>/dev/null
if grep -q 'v_b.data\[v_i\]' "$tmp/z.c" && ! sed -n '/^static uint64_t f_sum/,/^}/p' "$tmp/z.c" | grep -q 'if ('; then
  pass=$((pass+1))
else
  echo "FAIL zero-overhead: generated C for sum() is not a plain loop"; fail=$((fail+1))
fi
# Runtime cross-check: every passing program also runs clean under
# AddressSanitizer + UBSan (out-of-bounds, use-after-free, leaks, overflow).
# This is a test of the prover, not part of the language: the shipped code
# has no checks. Skipped when the C compiler lacks sanitizer support.
if echo 'int main(void){return 0;}' | ${CC:-cc} -fsanitize=address,undefined -x c - -o "$tmp/probe" 2>/dev/null; then
  for f in tests/pass/*.bastio; do
    name=$(basename "$f" .bastio)
    want=$(sed -n 's|^// expect-exit: *||p' "$f" | head -1)
    "$BASTIOC" --emit-c -o "$tmp/$name.c" "$f" 2>/dev/null
    ${CC:-cc} -fsanitize=address,undefined -fno-sanitize-recover=all -g -o "$tmp/$name.san" "$tmp/$name.c"
    "$tmp/$name.san" >/dev/null 2>"$tmp/$name.san.err"; got=$?
    if [ "$got" = "$want" ]; then pass=$((pass+1)); else
      echo "FAIL $f under sanitizers: exit $got"; sed 's/^/    /' "$tmp/$name.san.err" | head -20; fail=$((fail+1)); fi
  done
else
  echo "(sanitizer cross-check skipped: no -fsanitize support)"
fi
echo "-- $pass passed, $fail failed"
[ "$fail" = 0 ]
