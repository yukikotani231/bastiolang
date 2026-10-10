#!/usr/bin/env bash
# Differential test of the native x86-64 backend (`--backend native`): every
# tests/pass program and every bench program is built with both backends,
# and both executables must exit with the same status and print the same
# output (stdin empty). A program using something the native backend does
# not support yet is reported as skipped; anything else that differs fails.
#
# tests/native.sh --stage1 instead runs stage1/difftest.sh with the tools of
# the self-hosted compiler built by the native backend (by the C backend
# when it does not support one yet): large programs whose output is checked
# against stage0's own.
set -u
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-stage0/target/release/bastioc0}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
if [ "${1:-}" = --stage1 ]; then
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
. tests/stage1_only.sh
pass=0; fail=0; skip=0
for f in tests/pass/*.bastio bench/*.bastio; do
  if [ -n "$stage0_compiler" ] && stage1_only "$f"; then
    echo "skip $f: stage1-only, and BASTIOC is stage0"; skip=$((skip+1)); continue
  fi
  n=$(basename "$f" .bastio)
  [ "${f#bench/}" != "$f" ] && n="bench_$n"
  if ! "$BASTIOC" -o "$tmp/$n.c.exe" "$f" 2>/dev/null; then
    echo "FAIL $f: the C backend rejected it"; fail=$((fail+1)); continue
  fi
  "$BASTIOC" --backend native -o "$tmp/$n.x.exe" "$f" 2>"$tmp/$n.err"; rc=$?
  if [ $rc = 4 ]; then echo "skip $f: $(grep -o 'does not support.*' "$tmp/$n.err" | head -1)"; skip=$((skip+1)); continue; fi
  if [ $rc != 0 ]; then echo "FAIL $f: native build failed"; sed 's/^/    /' "$tmp/$n.err" | head -20; fail=$((fail+1)); continue; fi
  "$tmp/$n.c.exe" </dev/null >"$tmp/$n.c.out" 2>/dev/null; cs=$?
  "$tmp/$n.x.exe" </dev/null >"$tmp/$n.x.out" 2>/dev/null; xs=$?
  if [ $cs != $xs ]; then echo "FAIL $f: exit $xs, C backend $cs"; fail=$((fail+1)); continue; fi
  if ! cmp -s "$tmp/$n.c.out" "$tmp/$n.x.out"; then
    echo "FAIL $f: output differs from the C backend"; diff "$tmp/$n.c.out" "$tmp/$n.x.out" | head -10 | sed 's/^/    /'; fail=$((fail+1)); continue
  fi
  pass=$((pass+1))
done
echo "native: $pass same as the C backend, $skip skipped (unsupported), $fail failed"
[ $fail = 0 ]
