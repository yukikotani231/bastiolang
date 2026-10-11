#!/usr/bin/env bash
# Prints the path of bastioc1, the self-hosted compiler and the default
# compiler of the tests (ROADMAP v2, "bastioc1 is the default compiler").
# stage0 builds it first when it is missing or older than stage0 or any
# stage1 source, so a run builds it once and later runs reuse it.
#
# usage: BASTIOC=$(tests/bastioc1.sh) || exit 1
set -eu
cd "$(dirname "$0")/.."
b0=${BASTIOC0:-stage0/target/release/bastioc0}
out=stage0/target/release/bastioc1
if [ ! -x "$out" ] || [ -n "$(find stage1 -name '*.bastio' -newer "$out" | head -1)" ] \
   || [ "$b0" -nt "$out" ]; then
  echo "tests/bastioc1.sh: building $out with $b0" >&2
  # Built under another name and moved, so a reader never sees half a file.
  "$b0" -o "$out.tmp.$$" stage1/bastioc1.bastio >&2
  mv -f "$out.tmp.$$" "$out"
fi
echo "$PWD/$out"
