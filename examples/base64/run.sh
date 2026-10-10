#!/usr/bin/env bash
# Builds the Bastio base64 library into C, links it with the C test harness
# under AddressSanitizer + UBSan, and runs the vectors and the differential
# fuzz. `run.sh bench` builds with -O2 alone and measures throughput.
# Needs z3 and cc; BASTIOC overrides the compiler (default: stage0's build).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
BASTIOC=${BASTIOC:-$here/../../stage0/target/release/bastioc0}
CC=${CC:-cc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
"$BASTIOC" --emit-c -o "$tmp/base64.gen.c" "$here/lib.bastio"
cflags=(-std=c11 -Wall -Wno-unused-function -I"$tmp" -I"$here")
if [ "${1:-}" = bench ]; then
  "$CC" "${cflags[@]}" -O2 -o "$tmp/test" "$here/test.c" "$here/base64_capi.c"
  "$tmp/test" bench
  exit
fi
"$CC" "${cflags[@]}" -O1 -g -fsanitize=address,undefined -fno-sanitize-recover=all \
  -o "$tmp/test" "$here/test.c" "$here/base64_capi.c"
"$tmp/test" "$@"
