#!/usr/bin/env bash
# Builds the Bastio leb128 library into C, links it with the C test harness
# under AddressSanitizer + UBSan, and runs the vectors and the differential
# fuzz.
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
  "$BASTIOC" --backend native ${BFLAGS:-} --header "$tmp/leb128.h" -o "$tmp/leb128.o" "$here/lib.bastio"
else
  "$BASTIOC" --emit-c --header "$tmp/leb128.h" -o "$tmp/leb128.c" "$here/lib.bastio"
fi
# The generated C is compiled with -w: its prelude defines helpers this
# library does not use. The harness gets -Wall.
build() {  # C compiler flags...
  [ "$BACKEND" = native ] || "$CC" -std=c11 "$@" -w -c -o "$tmp/leb128.o" "$tmp/leb128.c"
  "$CC" -std=c11 "$@" -Wall -I"$tmp" -o "$tmp/test" "$here/test.c" "$tmp/leb128.o"
}
# The sanitizers, where the C compiler has them (not MinGW-w64).
SAN="-fsanitize=address,undefined -fno-sanitize-recover=all"
echo 'int main(void){return 0;}' | "$CC" $SAN -x c - -o "$tmp/san_probe" 2>/dev/null && ( ${RUNNER:-} "$tmp/san_probe" && : ) 2>/dev/null || SAN=
build -O1 -g $SAN
${RUNNER:-} "$tmp/test" "$@"
