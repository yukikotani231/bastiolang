#!/usr/bin/env bash
# Runs every test. user/*.bastio must verify and compile for --target user.
# pass/*.bastio must verify, build, and exit with the code in
# their `// expect-exit: N` header (and print the
# first line of `// expect-stdout: TEXT`, if given, with stdin empty); fail/*.bastio must be rejected with the error
# code in their `// expect-error: CODE` header, checked for the target in an
# optional `// target: T` header (a rejection for the wrong
# reason counts as a failure). diag/*.bastio must print exactly diag/*.err.
set -u
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-stage0/target/release/bastioc0}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
. tests/pfor.sh
export BASTIOC
# One check of the lists below; JOBS of them run at a time.
pass_one() {
  local f=$1 want name got out
  want=$(sed -n 's|^// expect-exit: *||p' "$f" | head -1)
  name=$(basename "$f" .bastio)
  if ! "$BASTIOC" -o "$w/$name" "$f" 2>"$w/$name.err"; then
    echo "FAIL $f: rejected"; sed 's/^/    /' "$w/$name.err"; echo =fail; return
  fi
  "$w/$name" </dev/null >"$w/$name.out"; got=$?
  if [ "$got" != "$want" ]; then echo "FAIL $f: exit $got, want $want"; echo =fail; return; fi
  out=$(sed -n 's|^// expect-stdout: *||p' "$f" | head -1)
  if [ -n "$out" ] && [ "$(head -1 "$w/$name.out")" != "$out" ]; then
    echo "FAIL $f: stdout $(head -1 "$w/$name.out" | od -c | head -2), want '$out'"; echo =fail; return
  fi
  echo =pass
}
fail_one() {
  local f=$1 want target missing
  want=$(sed -n 's|^// expect-error: *||p' "$f" | head -1)
  target=$(sed -n 's|^// target: *||p' "$f" | head -1)
  if "$BASTIOC" --check --target "${target:-hosted}" "$f" 2>"$w/err"; then
    echo "FAIL $f: accepted, want $want"; echo =fail; return
  fi
  if ! grep -q "error\[$want\]" "$w/err"; then
    echo "FAIL $f: wrong error, want $want"; sed 's/^/    /' "$w/err"; echo =fail; return
  fi
  # `// expect-text: T` lines: each T must appear in the error output (the
  # sentence or a `  = ` hint), so the wording of diagnostics is tested too.
  missing=$(sed -n 's|^// expect-text: *||p' "$f" | while IFS= read -r t; do
    grep -qF -- "$t" "$w/err" || echo "$t"; done)
  if [ -n "$missing" ]; then
    echo "FAIL $f: error output lacks: $missing"; sed 's/^/    /' "$w/err"; echo =fail; return
  fi
  echo =pass
}
# User programs (for a Bastio kernel such as BastiOS) cannot run here: they
# must verify and their C must compile freestanding.
user_one() {
  local f=$1 name
  name=$(basename "$f" .bastio)
  if "$BASTIOC" --target user -o "$w/$name.u.c" "$f" 2>"$w/$name.err" \
     && ${CC:-cc} -c -ffreestanding -fpie -o "$w/$name.u.o" "$w/$name.u.c" 2>>"$w/$name.err"; then
    echo =pass
  else
    echo "FAIL $f"; sed 's/^/    /' "$w/$name.err"; echo =fail
  fi
}
export -f pass_one fail_one user_one
pass=0
fail=0
tally() {  # after a pfor: add its tallies (a lost job is a failure)
  pass=$((pass + $(n pass))) fail=$((fail + $(n fail) + $1))
}
pfor pass_one tests/pass/*.bastio; tally $?
pfor fail_one tests/fail/*.bastio; tally $?
pfor user_one tests/user/*.bastio; tally $?
# Diagnostics: what the compiler prints for diag/NAME.bastio, run from
# tests/diag, must be diag/NAME.err exactly: the source line with its
# number, the token underlined, the notes. To accept a deliberate change,
# regenerate the .err files with bastioc0 and review the diff.
case "$BASTIOC" in /*) abs=$BASTIOC ;; */*) abs=$PWD/$BASTIOC ;; *) abs=$BASTIOC ;; esac
for f in tests/diag/*.bastio; do
  name=$(basename "$f" .bastio)
  (cd tests/diag && "$abs" --check "$name.bastio" 2>"$tmp/diag.err" >/dev/null)
  if cmp -s "$tmp/diag.err" "tests/diag/$name.err"; then
    pass=$((pass+1))
  else
    echo "FAIL $f: the error output differs from $name.err"; diff "tests/diag/$name.err" "$tmp/diag.err" | sed 's/^/    /'; fail=$((fail+1))
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
# `--lib` or BASTIO_LIB (docs/language.md, "Several files").
mkdir -p "$tmp/away"
printf 'import "core/option.bastio";\nfn main() -> u64 { return opt_or[u64](Option[u64]::Some(7), 0); }\n' >"$tmp/away/prog.bastio"
if BASTIO_LIB= "$BASTIOC" --check "$tmp/away/prog.bastio" 2>"$tmp/away.err"; then
  echo "FAIL --lib: core found without --lib"; fail=$((fail+1))
elif ! grep -q 'error\[E-IMPORT\]' "$tmp/away.err"; then
  echo "FAIL --lib: wrong error without --lib"; sed 's/^/    /' "$tmp/away.err"; fail=$((fail+1))
elif ! BASTIO_LIB= "$BASTIOC" --lib "$PWD" -o "$tmp/away/prog" "$tmp/away/prog.bastio" 2>"$tmp/away.err"; then
  echo "FAIL --lib: rejected"; sed 's/^/    /' "$tmp/away.err"; fail=$((fail+1))
elif "$tmp/away/prog"; [ $? != 7 ]; then
  echo "FAIL --lib: wrong exit"; fail=$((fail+1))
elif ! BASTIO_LIB="/nonexistent:$PWD" "$BASTIOC" --check "$tmp/away/prog.bastio" 2>"$tmp/away.err"; then
  echo "FAIL BASTIO_LIB: rejected"; sed 's/^/    /' "$tmp/away.err"; fail=$((fail+1))
else
  pass=$((pass+1))
fi
# Runtime cross-check: every passing program also runs clean under
# AddressSanitizer + UBSan (out-of-bounds, use-after-free, leaks, overflow).
# This is a test of the prover, not part of the language: the shipped code
# has no checks. Skipped when the C compiler lacks sanitizer support.
if echo 'int main(void){return 0;}' | ${CC:-cc} -fsanitize=address,undefined -x c - -o "$tmp/probe" 2>/dev/null; then
  san_one() {
    local f=$1 name want got
    name=$(basename "$f" .bastio)
    want=$(sed -n 's|^// expect-exit: *||p' "$f" | head -1)
    "$BASTIOC" --emit-c -o "$w/$name.c" "$f" 2>/dev/null
    ${CC:-cc} -fsanitize=address,undefined -fno-sanitize-recover=all -g -o "$w/$name.san" "$w/$name.c"
    # A failed allocation returns NULL, as calloc does without ASan, so the
    # `try_alloc` path that handles it is the one checked.
    ASAN_OPTIONS=allocator_may_return_null=1 \
      "$w/$name.san" </dev/null >/dev/null 2>"$w/$name.san.err"; got=$?
    if [ "$got" = "$want" ]; then echo =pass; else
      echo "FAIL $f under sanitizers: exit $got"; sed 's/^/    /' "$w/$name.san.err" | head -20; echo =fail; fi
  }
  export -f san_one
  pfor san_one tests/pass/*.bastio; tally $?
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
# The library reference (docs/lib/*.md and the module table in
# docs/core.md) is what tests/libref.py generates from core/ and std/, every
# library function is documented and called by an example, and every
# module's examples build, verify and run.
if python3 tests/libref.py --check >"$tmp/libref.out" 2>&1; then pass=$((pass+1)); else
  echo "FAIL tests/libref.py --check"; sed 's/^/    /' "$tmp/libref.out"; fail=$((fail+1)); fi
python3 tests/libref.py --examples "$tmp/libref" >"$tmp/libref.list"
while read -r mod prog names; do
  if ! "$BASTIOC" --lib "$PWD" -o "${prog%.bastio}" "$prog" 2>"$tmp/libref.err"; then
    echo "FAIL examples of $mod: rejected"; sed 's/^/    /' "$tmp/libref.err"; fail=$((fail+1)); continue
  fi
  "${prog%.bastio}" </dev/null >/dev/null 2>&1; got=$?
  if [ "$got" = 0 ]; then pass=$((pass+1)); else
    echo "FAIL examples of $mod: the example of $(echo $names | cut -d' ' -f$got) failed"; fail=$((fail+1)); fi
done <"$tmp/libref.list"
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
