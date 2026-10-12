#!/usr/bin/env bash
# Builds the Bastio UTF-8 library into C, links it with the C test harness
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
  "$BASTIOC" --backend native --header "$tmp/utf8.h" -o "$tmp/utf8.o" "$here/utf8.bastio"
else
  "$BASTIOC" --emit-c --header "$tmp/utf8.h" -o "$tmp/utf8.c" "$here/utf8.bastio"
fi
# The generated C is compiled with -w: its prelude defines helpers this
# library does not use. The harness gets -Wall.
build() {  # C compiler flags...
  [ "$BACKEND" = native ] || "$CC" -std=c11 "$@" -w -c -o "$tmp/utf8.o" "$tmp/utf8.c"
  "$CC" -std=c11 "$@" -Wall -I"$tmp" -o "$tmp/test" "$here/test.c" "$tmp/utf8.o"
}
if [ "${1:-}" = bench ]; then
  build -O2
  "$tmp/test" bench
  exit
fi
build -O1 -g -fsanitize=address,undefined -fno-sanitize-recover=all
"$tmp/test" "$@"
