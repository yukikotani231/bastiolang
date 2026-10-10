#!/usr/bin/env bash
# Runs every test. user/*.bastio must verify and compile for --target user.
# pass/*.bastio must verify, build, and exit with the code in
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
  if ! grep -q "error\[$want\]" "$tmp/err"; then
    echo "FAIL $f: wrong error, want $want"; sed 's/^/    /' "$tmp/err"; fail=$((fail+1)); continue
  fi
  # `// expect-text: T` lines: each T must appear in the error output (the
  # sentence or a `  = ` hint), so the wording of diagnostics is tested too.
  missing=$(sed -n 's|^// expect-text: *||p' "$f" | while IFS= read -r t; do
    grep -qF -- "$t" "$tmp/err" || echo "$t"; done)
  if [ -n "$missing" ]; then
    echo "FAIL $f: error output lacks: $missing"; sed 's/^/    /' "$tmp/err"; fail=$((fail+1)); continue
  fi
  pass=$((pass+1))
done
# User programs (for a Bastio kernel such as BastiOS) cannot run here: they
# must verify and their C must compile freestanding.
for f in tests/user/*.bastio; do
  name=$(basename "$f" .bastio)
  if "$BASTIOC" --target user -o "$tmp/$name.u.c" "$f" 2>"$tmp/$name.err" \
     && ${CC:-cc} -c -ffreestanding -fpie -o "$tmp/$name.u.o" "$tmp/$name.u.c" 2>>"$tmp/$name.err"; then
    pass=$((pass+1))
  else
    echo "FAIL $f"; sed 's/^/    /' "$tmp/$name.err"; fail=$((fail+1))
  fi
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
# Library search: a program outside the repository finds core only with
# `--lib` (docs/language.md, "Several files").
mkdir -p "$tmp/away"
printf 'import "core/option.bastio";\nfn main() -> u64 { return opt_or[u64](Option[u64]::Some(7), 0); }\n' >"$tmp/away/prog.bastio"
if "$BASTIOC" --check "$tmp/away/prog.bastio" 2>"$tmp/away.err"; then
  echo "FAIL --lib: core found without --lib"; fail=$((fail+1))
elif ! grep -q 'error\[E-IMPORT\]' "$tmp/away.err"; then
  echo "FAIL --lib: wrong error without --lib"; sed 's/^/    /' "$tmp/away.err"; fail=$((fail+1))
elif ! "$BASTIOC" --lib "$PWD" -o "$tmp/away/prog" "$tmp/away/prog.bastio" 2>"$tmp/away.err"; then
  echo "FAIL --lib: rejected"; sed 's/^/    /' "$tmp/away.err"; fail=$((fail+1))
elif "$tmp/away/prog"; [ $? != 7 ]; then
  echo "FAIL --lib: wrong exit"; fail=$((fail+1))
else
  pass=$((pass+1))
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
# Error transcripts in the docs match what the compiler prints, and blocks
# marked `<!-- file: PATH -->` match that file (tests/doc_check.py).
python3 tests/doc_check.py "$BASTIOC" docs/*.md >"$tmp/doccheck.out"
dc=$(sed -n 's/^doc_check: \([0-9]*\) passed, \([0-9]*\) failed$/\1 \2/p' "$tmp/doccheck.out")
if [ -n "$dc" ]; then
  grep -v '^doc_check: ' "$tmp/doccheck.out"
  pass=$((pass + ${dc% *})); fail=$((fail + ${dc#* }))
else
  echo "FAIL tests/doc_check.py did not finish"; sed 's/^/    /' "$tmp/doccheck.out"; fail=$((fail+1))
fi
# Single-file examples (examples/*.bastio) build and run, exiting with 0.
for f in examples/*.bastio; do
  name=$(basename "$f" .bastio)
  if ! "$BASTIOC" -o "$tmp/ex_$name" "$f" 2>"$tmp/ex.err"; then
    echo "FAIL $f: rejected"; sed 's/^/    /' "$tmp/ex.err"; fail=$((fail+1)); continue
  fi
  if "$tmp/ex_$name" </dev/null >/dev/null; then pass=$((pass+1)); else
    echo "FAIL $f: exit $?, want 0"; fail=$((fail+1)); fi
done
echo "-- $pass passed, $fail failed"
[ "$fail" = 0 ]
