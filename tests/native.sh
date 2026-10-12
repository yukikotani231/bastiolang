#!/usr/bin/env bash
# Differential test of the native x86-64 backend (`--backend native`): every
# tests/pass program and every bench program is built with both backends,
# and both executables must exit with the same status and print the same
# output (stdin empty). A program using something the native backend does
# not support yet is reported as skipped; anything else that differs fails.
#
# BACKEND=native-ir tests/native.sh builds the native side with
# `--backend native-ir` instead (the functions the IR covers are written from
# it, stage1/irx86.bastio); the C backend stays the reference.
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
export BASTIOC
# One program, built by both backends; JOBS of them at a time.
both() {
  local f=$1 cs xs
  if ! "$BASTIOC" -o "$w/c.exe" "$f" 2>/dev/null; then
    echo "FAIL $f: the C backend rejected it"; echo =fail; return
  fi
  "$BASTIOC" --backend "${BACKEND:-native}" -o "$w/x.exe" "$f" 2>"$w/err"; rc=$?
  if [ $rc = 4 ]; then echo "skip $f: $(grep -o 'does not support.*' "$w/err" | head -1)"; echo =skip; return; fi
  if [ $rc != 0 ]; then echo "FAIL $f: native build failed"; sed 's/^/    /' "$w/err" | head -20; echo =fail; return; fi
  "$w/c.exe" </dev/null >"$w/c.out" 2>/dev/null; cs=$?
  "$w/x.exe" </dev/null >"$w/x.out" 2>/dev/null; xs=$?
  if [ $cs != $xs ]; then echo "FAIL $f: exit $xs, C backend $cs"; echo =fail; return; fi
  if ! cmp -s "$w/c.out" "$w/x.out"; then
    echo "FAIL $f: output differs from the C backend"; diff "$w/c.out" "$w/x.out" | head -10 | sed 's/^/    /'; echo =fail; return
  fi
  echo =pass
}
export -f both
lost=0
pfor both $(s1files tests/pass/*.bastio bench/*.bastio) || lost=1
pass=$(n pass) skip=$(n skip) fail=$(($(n fail) + lost))
echo "native: $pass same as the C backend, $skip skipped (unsupported), $fail failed"
[ $fail = 0 ]
