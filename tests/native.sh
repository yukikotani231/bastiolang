#!/usr/bin/env bash
# Differential test of the native x86-64 backend (`--backend native`): every
# tests/pass program and every bench program is built with both backends,
# and both executables must exit with the same status and print the same
# output (stdin empty). A program using something the native backend does
# not support yet is reported as skipped; anything else that differs fails.
#
# BACKEND=native-ir tests/native.sh builds the native side with
# `--backend native-ir` instead (the functions the IR covers are written from
# it, stage1/irx86.bastio); the C backend stays the reference. (The example
# libraries are built by `native` only.)
#
# NATIVE_OS=macos (with BACKEND=native-ir) runs the comparison on an Intel Mac
# for the Mach-O assembly (`--os macos`), NATIVE_OS=windows on Windows (MSYS2)
# for the MinGW-w64 assembly (`--os windows`); the example libraries and the
# fixed-point check are left out there.
#
# tests/native.sh --stage1 instead runs stage1/difftest.sh with the tools of
# the self-hosted compiler built by the native backend (by the C backend
# when it does not support one yet): large programs whose output is checked
# against stage0's own.
set -u
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
if [ "${1:-}" = --stage1 ]; then
  # difftest's oracle is stage0, whatever compiles the other tests.
  BASTIOC=${BASTIOC0:-stage0/target/release/bastioc0}
  case "$BASTIOC" in /*) ;; *) BASTIOC="$PWD/$BASTIOC" ;; esac
  {
    echo '#!/usr/bin/env bash'
    echo "b='$BASTIOC'"
    echo 'case " $* " in *" --"*) exec "$b" "$@" ;; esac  # a dump or --emit-c'
    echo 'for a in "$@"; do'
    echo '  if [ "$a" = -o ]; then'
    echo '    "$b" --backend native "$@"; s=$?'
    echo '    [ $s = 4 ] || exit $s'
    echo '    echo "native.sh: the C backend builds ${*: -1}" >&2'
    echo '    exec "$b" "$@"'
    echo '  fi'
    echo 'done'
    echo 'exec "$b" "$@"'
  } >"$tmp/bastioc"
  chmod +x "$tmp/bastioc"
  BASTIOC="$tmp/bastioc" stage1/difftest.sh
  exit
fi
BASTIOC=${BASTIOC:-$(tests/bastioc1.sh)} || exit 1
. tests/pfor.sh
. tests/stage1_only.sh
OSARG=
[ "${NATIVE_OS:-linux}" = macos ] && OSARG="--os macos"
[ "${NATIVE_OS:-linux}" = windows ] && OSARG="--os windows"
# GNU_LINK=1 (MINGW_LINK=1 on Windows): the platform's as and cc instead of
# bastioc1's own assembler and linker, which are the default (on Linux, Windows
# and macOS Intel).
[ -n "${MINGW_LINK:-}${GNU_LINK:-}" ] && OSARG="$OSARG --gnu-link"
# SELF_LINK=1 (macOS, x86-64): bastioc1's own assembler and Mach-O writer.
[ -n "${SELF_LINK:-}" ] && OSARG="$OSARG --self-link"
# MUSL_LINK=1 (Linux, x86-64): the same, for an executable that runs on musl's loader.
[ -n "${MUSL_LINK:-}" ] && OSARG="$OSARG --musl-link"
export BASTIOC OSARG
# One program, built by both backends; JOBS of them at a time.
both() {
  local f=$1 cs xs
  if ! "$BASTIOC" -o "$w/c.exe" "$f" 2>/dev/null; then
    echo "FAIL $f: the C backend rejected it"; echo =fail; return
  fi
  "$BASTIOC" --backend "${BACKEND:-native}" ${OSARG:-} -o "$w/x.exe" "$f" 2>"$w/err"; rc=$?
  if [ $rc = 4 ]; then echo "skip $f: $(grep -o 'does not support.*' "$w/err" | head -1)"; echo =skip; return; fi
  if [ $rc != 0 ]; then echo "FAIL $f: native build failed"; sed 's/^/    /' "$w/err" | head -20; echo =fail; return; fi
  "$w/c.exe" </dev/null >"$w/c.out" 2>/dev/null; cs=$?
  "$w/x.exe" </dev/null >"$w/x.out" 2>/dev/null; xs=$?
  # The C build writes text-mode lines on Windows, the native one bytes.
  case "${OSARG:-}" in "--os windows"*) false;; *) true;; esac || { tr -d '\r' <"$w/c.out" >"$w/c.out2"; mv "$w/c.out2" "$w/c.out"; }
  if [ $cs != $xs ]; then echo "FAIL $f: exit $xs, C backend $cs"; echo =fail; return; fi
  if ! cmp -s "$w/c.out" "$w/x.out"; then
    echo "FAIL $f: output differs from the C backend"; diff "$w/c.out" "$w/x.out" | head -10 | sed 's/^/    /'; echo =fail; return
  fi
  echo =pass
}
export -f both
lost=0
pfor both $(s1files tests/pass/*.bastio bench/*.bastio) || lost=1
# The example libraries (`export fn`, no `main`) built by the native backend
# pass the same C harnesses as their C builds.
libs=0
for r in examples/*/run.sh; do
  [ -e "$r" ] || continue
  [ "${BACKEND:-native}" = native ] || continue
  [ -z "$OSARG" ] || continue
  if BACKEND=native BASTIOC="$BASTIOC" "$r" >"$tmp/lib.out" 2>&1; then libs=$((libs+1)); else
    echo "FAIL $r with the native backend"; sed 's/^/    /' "$tmp/lib.out" | tail -20; lost=1
  fi
done
# With the IR emitter, bastioc1 built through it compiles itself to the very
# assembly that the C-built bastioc1 writes with the same backend (a fixed
# point): the code the IR path writes for the compiler is the compiler's.
if [ "${BACKEND:-native}" = native-ir ] && [ -z "$OSARG" ]; then
  if "$BASTIOC" --backend native-ir -o "$tmp/b1" stage1/bastioc1.bastio >/dev/null 2>&1 \
     && "$BASTIOC" --backend native-ir --emit-asm -o "$tmp/a.s" stage1/bastioc1.bastio >/dev/null 2>&1 \
     && "$tmp/b1" --backend native-ir --emit-asm -o "$tmp/b.s" stage1/bastioc1.bastio >/dev/null 2>&1 \
     && cmp -s "$tmp/a.s" "$tmp/b.s"; then
    echo "ok   bastioc1 built through the IR compiles itself to the same assembly"
  else
    echo "FAIL bastioc1 built through the IR does not reproduce its own assembly"; lost=1
  fi
fi
# The assembly has no function that nothing refers to (stage1/prune.bastio).
if [ "${BACKEND:-native}" = native-ir ] && [ -z "$OSARG" ] && [ -s "$tmp/a.s" ]; then
  grep -v '^[[:space:]]*\.\(type\|size\)' "$tmp/a.s" | grep -v '^f_[A-Za-z0-9_]*:' \
    | grep -o 'f_[A-Za-z0-9_]*' | sort -u >"$tmp/refs"
  grep -o '^f_[A-Za-z0-9_]*:' "$tmp/a.s" | tr -d : | sort -u >"$tmp/defs"
  dead=$(comm -23 "$tmp/defs" "$tmp/refs" | head -3 | tr '\n' ' ')
  if [ -z "$dead" ]; then echo "ok   no function in the assembly is unreferenced"; else
    echo "FAIL unreferenced functions are still written: $dead"; lost=1
  fi
fi
pass=$(n pass) skip=$(n skip) fail=$(($(n fail) + lost))
echo "native: $pass same as the C backend, $skip skipped (unsupported), $fail failed; $libs libraries pass their C harnesses"
[ $fail = 0 ]
