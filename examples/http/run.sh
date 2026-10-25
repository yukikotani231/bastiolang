#!/usr/bin/env bash
# Builds the Bastio HTTP head parser into C and runs the C test harness on it
# under AddressSanitizer + UBSan. Needs z3 and cc; BASTIOC overrides the
# compiler (default: stage0's build).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
BASTIOC=${BASTIOC:-$here/../../stage0/target/release/bastioc0}
CC=${CC:-cc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
"$BASTIOC" --emit-c --header "$tmp/http.h" -o "$tmp/http.c" "$here/http.bastio"
# The sanitizers, where the C compiler has them (not MinGW-w64).
SAN="-fsanitize=address,undefined -fno-sanitize-recover=all"
echo 'int main(void){return 0;}' | "$CC" $SAN -x c - -o "$tmp/san_probe" 2>/dev/null && ( ${RUNNER:-} "$tmp/san_probe" && : ) 2>/dev/null || SAN=
"$CC" -std=c11 -O1 -g $SAN -w -c -o "$tmp/http.o" "$tmp/http.c"
"$CC" -std=c11 -O1 -g $SAN -Wall -I"$tmp" -o "$tmp/test" "$here/test.c" "$tmp/http.o"
${RUNNER:-} "$tmp/test"
