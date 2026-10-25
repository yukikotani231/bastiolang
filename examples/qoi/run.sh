#!/usr/bin/env bash
# Builds the Bastio QOI library into C, links it with the C test harness
# under AddressSanitizer + UBSan, and runs the round trips, the differential
# fuzz and the out-of-memory test. `run.sh bench` builds with -O2 alone and
# measures throughput.
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
  "$BASTIOC" --backend native ${BFLAGS:-} --header "$tmp/qoi.h" -o "$tmp/qoi.o" "$here/lib.bastio"
else
  "$BASTIOC" --emit-c --header "$tmp/qoi.h" -o "$tmp/qoi.c" "$here/lib.bastio"
fi
# The generated C is compiled with -w: its prelude defines helpers this
# library does not use. The harness gets -Wall.
build() {  # C compiler flags...
  [ "$BACKEND" = native ] || "$CC" -std=c11 "$@" -w -c -o "$tmp/qoi.o" "$tmp/qoi.c"
  "$CC" -std=c11 "$@" -Wall -I"$tmp" -o "$tmp/test" "$here/test.c" "$tmp/qoi.o"
}
if [ "${1:-}" = bench ]; then
  build -O2
  ${RUNNER:-} "$tmp/test" bench
  exit
fi
# The sanitizers, where the C compiler has them (not MinGW-w64).
SAN="-fsanitize=address,undefined -fno-sanitize-recover=all"
echo 'int main(void){return 0;}' | "$CC" $SAN -x c - -o "$tmp/san_probe" 2>/dev/null && ( ${RUNNER:-} "$tmp/san_probe" && : ) 2>/dev/null || SAN=
build -O1 -g $SAN
# Allocations over 64 MiB fail (malloc returns NULL), so the test can check
# that running out of memory is reported, not a crash.
[ -z "$SAN" ] || export ASAN_OPTIONS=allocator_may_return_null=1:max_allocation_size_mb=64
${RUNNER:-} "$tmp/test" "$@"
