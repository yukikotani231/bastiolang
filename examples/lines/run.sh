#!/usr/bin/env bash
# Builds lines.bastio into C with its header, links it with main.c under
# AddressSanitizer + UBSan, and checks the output on a few inputs. This is the
# program that docs/tutorial.md, section 8, walks through.
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
  "$BASTIOC" --backend native --header "$tmp/lines.h" -o "$tmp/lines.o" "$here/lines.bastio"
else
  "$BASTIOC" --emit-c --header "$tmp/lines.h" -o "$tmp/lines.c" "$here/lines.bastio"
fi
# The sanitizers, where the C compiler has them (not MinGW-w64).
SAN="-fsanitize=address,undefined -fno-sanitize-recover=all"
echo 'int main(void){return 0;}' | "$CC" $SAN -x c - -o "$tmp/san_probe" 2>/dev/null || SAN=
[ "$BACKEND" = native ] || "$CC" -std=c11 -O1 -g $SAN \
  -w -c -o "$tmp/lines.o" "$tmp/lines.c"
"$CC" -std=c11 -O1 -g $SAN \
  -Wall -Werror -I"$tmp" -o "$tmp/lines" "$here/main.c" "$tmp/lines.o"
check() {  # input, expected output
  got=$(printf '%b' "$1" | "$tmp/lines")
  if [ "$got" != "$2" ]; then
    echo "lines: input '$1': got '$got', want '$2'" >&2
    exit 1
  fi
}
check '' 'lines: 0, longest: 0 bytes'
check 'one\ntwo\nthree\n' 'lines: 3, longest: 5 bytes'
check 'no newline at the end' 'lines: 0, longest: 21 bytes'
check '\n\n\nx\n' 'lines: 4, longest: 1 bytes'
# More than one read() worth, so main.c's buffer has to grow.
head -c 200000 /dev/zero | tr '\0' 'a' >"$tmp/big"
printf '\nb\n' >>"$tmp/big"
got=$("$tmp/lines" <"$tmp/big")
[ "$got" = 'lines: 2, longest: 200000 bytes' ] || { echo "lines: big input: got '$got'" >&2; exit 1; }
echo "lines: all checks passed"
