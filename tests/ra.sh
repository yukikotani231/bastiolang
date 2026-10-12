#!/usr/bin/env bash
# Register allocation over the IR (docs/ir.md, stage1/irra.bastio): irradump
# must print, for every program in tests/ir, exactly the allocation in
# tests/ra/NAME.expected, and for every program of tests/pass, bench/ and
# core/ an allocation that passes its own validity check, both with the
# normal pools and with the tiny ones (--small) that force spilling.
# usage: tests/ra.sh [--update]    (--update rewrites the .expected files)
set -eu
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-stage0/target/release/bastioc0}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
"$BASTIOC" -o "$tmp/irradump" stage1/irradump.bastio
mkdir -p tests/ra
fail=0
for f in tests/ir/*.bastio; do
  want="tests/ra/$(basename "${f%.bastio}").expected"
  if [ "${1:-}" = --update ]; then
    "$tmp/irradump" "$f" > "$want"
  elif "$tmp/irradump" "$f" | diff -u "$want" - > "$tmp/diff"; then
    echo "ok   $f"
  else
    echo "FAIL $f"; cat "$tmp/diff"; fail=1
  fi
done
[ "${1:-}" = --update ] && exit 0
for f in tests/pass/*.bastio bench/*.bastio core/*.bastio; do
  for flag in "" --small; do
    if "$tmp/irradump" $flag "$f" 2>&1 | grep -q INVALID; then
      echo "FAIL $f $flag: invalid allocation"; fail=1
    fi
  done
done
[ $fail = 0 ] && echo "ok   validity of the allocation over tests/pass, bench and core"
exit $fail
