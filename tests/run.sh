#!/usr/bin/env bash
# Runs every test. user/*.bastio must verify and compile for --target user.
# pass/*.bastio must verify, build, and exit with the code in
# their `// expect-exit: N` header (and print the
# first line of `// expect-stdout: TEXT`, if given, with stdin empty); fail/*.bastio must be rejected with the error
# code in their `// expect-error: CODE` header, checked for the target in an
# optional `// target: T` header and the architecture in an optional
# `// arch: A` header (a rejection for the wrong
# reason counts as a failure). diag/*.bastio must print exactly diag/*.err.
# A program with a `// stage1-only: <feature>` header is skipped when
# BASTIOC is stage0 (tests/stage1_only.sh).
#
# ARCH=aarch64 builds the programs that run (pass/*.bastio and the
# single-file examples) with `--arch aarch64` instead, compiling the C with
# AARCH64_CC (default: aarch64-linux-gnu-gcc), and runs them under
# qemu-aarch64 unless this machine is an AArch64 one. The other checks do
# not depend on the processor and are left to the default run.
set -u
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-$(tests/bastioc1.sh)} || exit 1
ARCH=${ARCH:-x86_64}
case "$ARCH" in
  x86_64) RUN= ;;
  aarch64)
    export CC=${AARCH64_CC:-aarch64-linux-gnu-gcc}
    if [ "$(uname -m)" = aarch64 ]; then RUN=; else
      RUN=qemu-aarch64
      # The cross toolchain's libraries, for dynamically linked programs.
      export QEMU_LD_PREFIX=${QEMU_LD_PREFIX:-/usr/aarch64-linux-gnu}
    fi ;;
  *) echo "tests/run.sh: unknown ARCH $ARCH (expected x86_64 or aarch64)"; exit 2 ;;
esac
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
. tests/pfor.sh
. tests/stage1_only.sh
export BASTIOC ARCH RUN
# One check of the lists below; JOBS of them run at a time.
pass_one() {
  local f=$1 want name got out
  # Sockets are not implemented on Windows (every call fails).
  case "$f:$(uname -s)" in tests/pass/host_net.bastio:MINGW*|tests/pass/host_net.bastio:MSYS*) echo =pass; return ;; esac
  want=$(sed -n 's|^// expect-exit: *||p' "$f" | head -1)
  name=$(basename "$f" .bastio)
  if ! "$BASTIOC" --arch "$ARCH" -o "$w/$name" "$f" 2>"$w/$name.err"; then
    echo "FAIL $f: rejected"; sed 's/^/    /' "$w/$name.err"; echo =fail; return
  fi
  $RUN "$w/$name" </dev/null >"$w/$name.out"; got=$?
  if [ "$got" != "$want" ]; then echo "FAIL $f: exit $got, want $want"; echo =fail; return; fi
  out=$(sed -n 's|^// expect-stdout: *||p' "$f" | head -1)
  if [ -n "$out" ] && [ "$(head -1 "$w/$name.out")" != "$out" ]; then
    echo "FAIL $f: stdout $(head -1 "$w/$name.out" | od -c | head -2), want '$out'"; echo =fail; return
  fi
  echo =pass
}
fail_one() {
  local f=$1 want target arch missing
  want=$(sed -n 's|^// expect-error: *||p' "$f" | head -1)
  target=$(sed -n 's|^// target: *||p' "$f" | head -1)
  arch=$(sed -n 's|^// arch: *||p' "$f" | head -1)
  if "$BASTIOC" --check --target "${target:-hosted}" --arch "${arch:-x86_64}" "$f" 2>"$w/err"; then
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
  # The C of a user program has x86 inline assembly, so it is only compiled
  # on an x86-64 host.
  if "$BASTIOC" --target user -o "$w/$name.u.c" "$f" 2>"$w/$name.err" \
     && { [ "$(uname -m)" != x86_64 ] \
          || ${CC:-cc} -c -ffreestanding -fpie -o "$w/$name.u.o" "$w/$name.u.c" 2>>"$w/$name.err"; }; then
    echo =pass
  else
    echo "FAIL $f"; sed 's/^/    /' "$w/$name.err"; echo =fail
  fi
}
# Single-file examples (examples/*.bastio) build and run, exiting with 0.
example_one() {
  local f=$1 name
  # Sockets are not implemented on Windows (every call fails).
  case "$f:$(uname -s)" in examples/httpd.bastio:MINGW*|examples/httpd.bastio:MSYS*) echo =pass; return ;; esac
  name=$(basename "$f" .bastio)
  if ! "$BASTIOC" --arch "$ARCH" -o "$w/ex_$name" "$f" 2>"$w/ex.err"; then
    echo "FAIL $f: rejected"; sed 's/^/    /' "$w/ex.err"; echo =fail; return
  fi
  if $RUN "$w/ex_$name" </dev/null >/dev/null; then echo =pass; else
    echo "FAIL $f: exit $?, want 0"; echo =fail; fi
}
export -f pass_one fail_one user_one example_one
pass=0
fail=0
tally() {  # after a pfor: add its tallies (a lost job is a failure)
  pass=$((pass + $(n pass))) fail=$((fail + $(n fail) + $1))
}
pfor pass_one $(s1files tests/pass/*.bastio); tally $?
if [ "$ARCH" != x86_64 ]; then
  pfor example_one $(s1files examples/*.bastio); tally $?
  echo "-- $pass passed, $fail failed (ARCH=$ARCH: the programs that run)"
  [ "$fail" = 0 ]
  exit
fi
pfor fail_one $(s1files tests/fail/*.bastio); tally $?
pfor user_one $(s1files tests/user/*.bastio); tally $?
# Diagnostics: what the compiler prints for diag/NAME.bastio, run from
# tests/diag, must be diag/NAME.err exactly: the source line with its
# number, the token underlined, the notes. To accept a deliberate change,
# regenerate the .err files with bastioc0 and review the diff.
case "$BASTIOC" in /*) abs=$BASTIOC ;; */*) abs=$PWD/$BASTIOC ;; *) abs=$BASTIOC ;; esac
for f in tests/diag/*.bastio; do
  name=$(basename "$f" .bastio)
  (cd tests/diag && "$abs" --check "$name.bastio" 2>"$tmp/diag.err" >/dev/null)
  # The numbers of a counterexample are whatever model the solver picks,
  # which differs between Z3 versions: compare their names, not their values.
  nums='/= counterexample:/s/= [-]*[0-9][0-9]*/= N/g'
  if cmp -s <(sed "$nums" "$tmp/diag.err") <(sed "$nums" "tests/diag/$name.err"); then
    pass=$((pass+1))
  else
    echo "FAIL $f: the error output differs from $name.err"; diff "tests/diag/$name.err" "$tmp/diag.err" | sed 's/^/    /'; fail=$((fail+1))
  fi
done
# `--help` prints the options on standard output and succeeds; a solver
# that cannot be started is one error about no line of the program.
if "$BASTIOC" --help 2>/dev/null | grep -q '^usage: ' && [ "${PIPESTATUS[0]}" = 0 ]; then
  pass=$((pass+1))
else
  echo "FAIL --help: no usage on standard output, or a nonzero exit"; fail=$((fail+1))
fi
want='examples/hello.bastio: error[E-SOLVER]: could not start the SMT solver `/nonexistent/z3` (install Z3 and put `z3` on PATH)'
got=$(MSYS2_ENV_CONV_EXCL=BASTIO_SOLVER BASTIO_SOLVER=/nonexistent/z3 "$BASTIOC" --check examples/hello.bastio 2>&1)
if [ "$got" = "$want" ]; then
  pass=$((pass+1))
else
  echo "FAIL a missing solver: got"; printf '%s\n' "$got" | sed 's/^/    /'; fail=$((fail+1))
fi
# Zero overhead: the generated C must index buffers directly, with no checks.
"$BASTIOC" --emit-c -o "$tmp/z.c" tests/pass/fill_sum.bastio 2>/dev/null
if grep -q 'v_b.data\[v_i\]' "$tmp/z.c" && ! sed -n '/^static uint64_t f_sum/,/^}/p' "$tmp/z.c" | grep -q 'if ('; then
  pass=$((pass+1))
else
  echo "FAIL zero-overhead: generated C for sum() is not a plain loop"; fail=$((fail+1))
fi
# The trusted base is listed and counted (ROADMAP invariant 3): a change to
# it must come with an updated tests/trusted_base.txt, so it shows in review.
# (A builtin marked "stage1 only" is not in bastioc0's list.)
if [ -n "$stage0_compiler" ]; then
  grep -v 'stage1 only' tests/trusted_base.txt >"$tmp/tb.expect"
else
  cp tests/trusted_base.txt "$tmp/tb.expect"
fi
# (Line ends are not compared: a Windows checkout may have CR LF in the files.)
tr -d '\r' <"$tmp/tb.expect" >"$tmp/tb.expect.lf"
if "$BASTIOC" --trusted-base | tr -d '\r' | diff -u "$tmp/tb.expect.lf" - >"$tmp/tb.diff"; then
  pass=$((pass+1))
else
  echo "FAIL trusted base changed; review it, then: $BASTIOC --trusted-base > tests/trusted_base.txt"
  sed 's/^/    /' "$tmp/tb.diff"; fail=$((fail+1))
fi
# The same for AArch64's preludes (stage1 only: stage0 has no AArch64 kernel
# or user target).
if [ -z "$stage0_compiler" ]; then
  tr -d '\r' <tests/trusted_base_aarch64.txt >"$tmp/tb.a64.lf"
  if "$BASTIOC" --arch aarch64 --trusted-base | tr -d '\r' | diff -u "$tmp/tb.a64.lf" - >"$tmp/tb.diff"; then
    pass=$((pass+1))
  else
    echo "FAIL AArch64 trusted base changed; review it, then: $BASTIOC --arch aarch64 --trusted-base > tests/trusted_base_aarch64.txt"
    sed 's/^/    /' "$tmp/tb.diff"; fail=$((fail+1))
  fi
fi
# Library search: a program outside the repository finds core only with
# `--lib` or BASTIO_LIB (docs/language.md, "Several files").
mkdir -p "$tmp/away"
printf 'import "core/option.bastio";\nfn main() -> u64 { return opt_or[u64](Option[u64]::Some(7), 0); }\n' >"$tmp/away/prog.bastio"
if case "$(uname -s)" in MINGW*|MSYS*) true ;; *) false ;; esac; then
  # BASTIO_LIB is a ';'-separated list of C:/ paths there, not ':'-separated.
  echo "(--lib search skipped on Windows)"
elif BASTIO_LIB= "$BASTIOC" --check "$tmp/away/prog.bastio" 2>"$tmp/away.err"; then
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
  pfor san_one $(s1files tests/pass/*.bastio); tally $?
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
# cannot run here), with an optional `// target: T` line after it. They may
# import the library (core/, std/). Sketches
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
      if ! "$BASTIOC" --lib "$PWD" -o "$tmp/docex" "$f" 2>"$tmp/doc.err"; then
        echo "FAIL $at: rejected"; sed 's/^/    /' "$tmp/doc.err"; fail=$((fail+1)); continue
      fi
      "$tmp/docex" </dev/null >/dev/null; got=$?
      if [ "$got" = "$want" ]; then pass=$((pass+1)); else echo "FAIL $at: exit $got, want $want"; fail=$((fail+1)); fi ;;
    "// expect-error: "*)
      want=${head1#// expect-error: }
      if "$BASTIOC" --lib "$PWD" --check --target "${target:-hosted}" "$f" 2>"$tmp/doc.err"; then
        echo "FAIL $at: accepted, want $want"; fail=$((fail+1))
      elif grep -q "error\[$want\]" "$tmp/doc.err"; then pass=$((pass+1)); else
        echo "FAIL $at: wrong error, want $want"; sed 's/^/    /' "$tmp/doc.err"; fail=$((fail+1)); fi ;;
    "// expect-ok")
      if "$BASTIOC" --lib "$PWD" --check --target "${target:-hosted}" "$f" 2>"$tmp/doc.err"; then pass=$((pass+1)); else
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
  case "$mod:$(uname -s)" in std/net.bastio:MINGW*|std/net.bastio:MSYS*|std/http.bastio:MINGW*|std/http.bastio:MSYS*) pass=$((pass+1)); continue ;; esac
  if ! "$BASTIOC" --lib "$PWD" -o "${prog%.bastio}" "$prog" 2>"$tmp/libref.err"; then
    echo "FAIL examples of $mod: rejected"; sed 's/^/    /' "$tmp/libref.err"; fail=$((fail+1)); continue
  fi
  "${prog%.bastio}" </dev/null >/dev/null 2>&1; got=$?
  if [ "$got" = 0 ]; then pass=$((pass+1)); else
    echo "FAIL examples of $mod: the example of $(echo $names | cut -d' ' -f$got) failed"; fail=$((fail+1)); fi
done <"$tmp/libref.list"
pfor example_one $(s1files examples/*.bastio); tally $?
s1skip=$(s1left tests/pass/*.bastio tests/fail/*.bastio tests/user/*.bastio)
if [ "$s1skip" -gt 0 ]; then
  echo "-- $pass passed, $fail failed, $s1skip skipped (stage1-only, and BASTIOC is stage0)"
else
  echo "-- $pass passed, $fail failed"
fi
[ "$fail" = 0 ]
