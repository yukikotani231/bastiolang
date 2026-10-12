#!/usr/bin/env bash
# Builds the Bastio DER library into C, links it with the C test harness
# under AddressSanitizer + UBSan, and runs the vectors and the differential
# fuzz. `run.sh bench` builds with -O2 alone and measures throughput.
# Needs z3 and cc; BASTIOC overrides the compiler (default: stage0's build).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
BASTIOC=${BASTIOC:-$here/../../stage0/target/release/bastioc0}
CC=${CC:-cc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# BACKEND=native builds the library with bastioc1's native backend (no C
# compiler for the library itself) instead of translating it to C.
BACKEND=${BACKEND:-c}
if [ "$BACKEND" = native ]; then
  "$BASTIOC" --backend native --header "$tmp/der.h" -o "$tmp/der.o" "$here/der.bastio"
else
  "$BASTIOC" --emit-c --header "$tmp/der.h" -o "$tmp/der.c" "$here/der.bastio"
fi
# The generated C is compiled with -w: its prelude defines helpers this
# library does not use. The harness gets -Wall.
build() {  # C compiler flags...
  [ "$BACKEND" = native ] || "$CC" -std=c11 "$@" -w -c -o "$tmp/der.o" "$tmp/der.c"
  "$CC" -std=c11 "$@" -Wall -I"$tmp" -o "$tmp/test" "$here/test.c" "$tmp/der.o"
}
if [ "${1:-}" = bench ]; then
  build -O2
  "$tmp/test" bench
  exit
fi
build -O1 -g -fsanitize=address,undefined -fno-sanitize-recover=all
"$tmp/test" "$@"
