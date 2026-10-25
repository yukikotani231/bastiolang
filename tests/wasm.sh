#!/usr/bin/env bash
# Differential test of the WebAssembly backend (`--arch wasm32`): every
# tests/pass program and every bench program is built with the C backend and
# as a wasm module, and the module, run by Node.js (tests/wasm_run.js), must
# exit with the same status and print the same output. A program using
# something the backend does not cover yet is reported as skipped; anything
# else that differs fails. Needs node (18 or later).
set -u
cd "$(dirname "$0")/.."
if ! command -v node >/dev/null; then echo "wasm: node is missing, SKIPPED"; exit 0; fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
BASTIOC=${BASTIOC:-$(tests/bastioc1.sh)} || exit 1
. tests/pfor.sh
. tests/stage1_only.sh
export BASTIOC
both() {
  local f=$1 cs xs rc
  if ! "$BASTIOC" -o "$w/c.exe" "$f" 2>/dev/null; then
    echo "FAIL $f: the C backend rejected it"; echo =fail; return
  fi
  "$BASTIOC" --arch wasm32 -o "$w/x.wasm" "$f" 2>"$w/err"; rc=$?
  if [ $rc = 4 ]; then echo "skip $f: $(grep -o 'does not .*' "$w/err" | head -1)"; echo =skip; return; fi
  if [ $rc != 0 ]; then echo "FAIL $f: wasm build failed"; sed 's/^/    /' "$w/err" | head -20; echo =fail; return; fi
  "$w/c.exe" </dev/null >"$w/c.out" 2>/dev/null; cs=$?
  node tests/wasm_run.js "$w/x.wasm" </dev/null >"$w/x.out" 2>"$w/x.err"; xs=$?
  if [ $cs != $xs ]; then echo "FAIL $f: exit $xs, C backend $cs"; head -3 "$w/x.err" | sed 's/^/    /'; echo =fail; return; fi
  if ! cmp -s "$w/c.out" "$w/x.out"; then
    echo "FAIL $f: output differs from the C backend"; diff "$w/c.out" "$w/x.out" | head -10 | sed 's/^/    /'; echo =fail; return
  fi
  echo =pass
}
export -f both
lost=0
pfor both $(s1files tests/pass/*.bastio bench/*.bastio) || lost=1
pass=$(n pass) skip=$(n skip) fail=$(($(n fail) + lost))
echo "wasm: $pass same as the C backend, $skip skipped (unsupported), $fail failed"
[ $fail = 0 ]
