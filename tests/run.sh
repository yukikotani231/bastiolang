#!/usr/bin/env bash
# Runs every test. pass/*.bastio must verify, build, and exit with the code in
# their `// expect-exit: N` header (and print the
# first line of `// expect-stdout: TEXT`, if given, with stdin empty); fail/*.bastio must be rejected with the error
# code in their `// expect-error: CODE` header, checked for the target in an
# optional `// target: T` header (a rejection for the wrong
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
  "$tmp/$name" </dev/null >"$tmp/$name.out"; got=$?
  if [ "$got" != "$want" ]; then echo "FAIL $f: exit $got, want $want"; fail=$((fail+1)); continue; fi
  out=$(sed -n 's|^// expect-stdout: *||p' "$f" | head -1)
  if [ -n "$out" ] && [ "$(head -1 "$tmp/$name.out")" != "$out" ]; then
    echo "FAIL $f: stdout $(head -1 "$tmp/$name.out" | od -c | head -2), want '$out'"; fail=$((fail+1)); continue
  fi
  pass=$((pass+1))
done
for f in tests/fail/*.bastio; do
  want=$(sed -n 's|^// expect-error: *||p' "$f" | head -1)
  target=$(sed -n 's|^// target: *||p' "$f" | head -1)
  if "$BASTIOC" --check --target "${target:-hosted}" "$f" 2>"$tmp/err"; then
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
# The trusted base is listed and counted (ROADMAP invariant 3): a change to
# it must come with an updated tests/trusted_base.txt, so it shows in review.
if "$BASTIOC" --trusted-base | diff -u tests/trusted_base.txt - >"$tmp/tb.diff"; then
  pass=$((pass+1))
else
  echo "FAIL trusted base changed; review it, then: $BASTIOC --trusted-base > tests/trusted_base.txt"
  sed 's/^/    /' "$tmp/tb.diff"; fail=$((fail+1))
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
    "$tmp/$name.san" </dev/null >/dev/null 2>"$tmp/$name.san.err"; got=$?
    if [ "$got" = "$want" ]; then pass=$((pass+1)); else
      echo "FAIL $f under sanitizers: exit $got"; sed 's/^/    /' "$tmp/$name.san.err" | head -20; fail=$((fail+1)); fi
  done
else
  echo "(sanitizer cross-check skipped: no -fsanitize support)"
fi
# Examples with a C side: each builds its Bastio code into C, links it with a
# C harness and runs it (examples/*/run.sh).
for r in examples/*/run.sh; do
  [ -e "$r" ] || continue
  if BASTIOC="$BASTIOC" "$r" >"$tmp/ex.out" 2>&1; then pass=$((pass+1)); else
    echo "FAIL $r"; sed 's/^/    /' "$tmp/ex.out" | tail -20; fail=$((fail+1)); fi
done
# Every ```bastio block in docs/*.md is a whole program, checked
# like a test: its first line is `// expect-exit: N` (built and run on the
# hosted target), `// expect-error: CODE` (rejected with CODE), or
# `// expect-ok` (accepted by --check; for kernel and user examples that
# cannot run here), with an optional `// target: T` line after it. Sketches
# that are not whole programs use ```text instead.
mkdir -p "$tmp/doc"
for md in docs/*.md; do
  awk -v dir="$tmp/doc" -v doc="$(basename "$md" .md)" '
    /^```bastio$/ { n = NR; f = dir "/" doc "@" NR ".bastio"; next }
    /^```/ && n { n = 0; close(f); next }
    n { print > f }
  ' "$md"
done
for f in "$tmp"/doc/*.bastio; do
  [ -e "$f" ] || continue
  at="docs/$(basename "$f" .bastio | sed 's/@/.md:/')"
  head1=$(head -1 "$f")
  target=$(sed -n 's|^// target: *||p' "$f" | head -1)
  case "$head1" in
    "// expect-exit: "*)
      want=${head1#// expect-exit: }
      if ! "$BASTIOC" -o "$tmp/docex" "$f" 2>"$tmp/doc.err"; then
        echo "FAIL $at: rejected"; sed 's/^/    /' "$tmp/doc.err"; fail=$((fail+1)); continue
      fi
      "$tmp/docex" </dev/null >/dev/null; got=$?
      if [ "$got" = "$want" ]; then pass=$((pass+1)); else echo "FAIL $at: exit $got, want $want"; fail=$((fail+1)); fi ;;
    "// expect-error: "*)
      want=${head1#// expect-error: }
      if "$BASTIOC" --check --target "${target:-hosted}" "$f" 2>"$tmp/doc.err"; then
        echo "FAIL $at: accepted, want $want"; fail=$((fail+1))
      elif grep -q "error\[$want\]" "$tmp/doc.err"; then pass=$((pass+1)); else
        echo "FAIL $at: wrong error, want $want"; sed 's/^/    /' "$tmp/doc.err"; fail=$((fail+1)); fi ;;
    "// expect-ok")
      if "$BASTIOC" --check --target "${target:-hosted}" "$f" 2>"$tmp/doc.err"; then pass=$((pass+1)); else
        echo "FAIL $at: rejected"; sed 's/^/    /' "$tmp/doc.err"; fail=$((fail+1)); fi ;;
    *)
      echo "FAIL $at: a \`\`\`bastio block must start with // expect-exit:, // expect-error: or // expect-ok"; fail=$((fail+1)) ;;
  esac
done
echo "-- $pass passed, $fail failed"
[ "$fail" = 0 ]
