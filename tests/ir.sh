#!/usr/bin/env bash
# The IR (docs/ir.md, stage1/ir.bastio): irdump must print, for every
# program in tests/ir, exactly the IR in the .expected file next to it.
# usage: tests/ir.sh [--update]    (--update rewrites the .expected files)
set -eu
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-stage0/target/release/bastioc0}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
"$BASTIOC" -o "$tmp/irdump" stage1/irdump.bastio
fail=0
for f in tests/ir/*.bastio; do
  want="${f%.bastio}.expected"
  if [ "${1:-}" = --update ]; then
    "$tmp/irdump" "$f" > "$want"
  elif "$tmp/irdump" "$f" | diff -u "$want" - > "$tmp/diff"; then
    echo "ok   $f"
  else
    echo "FAIL $f"; cat "$tmp/diff"; fail=1
  fi
done
exit $fail
