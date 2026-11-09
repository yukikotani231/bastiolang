#!/usr/bin/env bash
# bastioc1 links libm (-lm) only into a program that calls an f64 builtin.
# A C compiler whose C library has no separate libm (tcc on BastiOS, with
# musl's single libc.so) must still build every other program.
#
# usage: tests/nolibm.sh
set -euo pipefail
cd "$(dirname "$0")/.."
BASTIOC=$(tests/bastioc1.sh)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/cc" <<'SH'
#!/bin/sh
for a in "$@"; do
  [ "$a" = -lm ] && { echo "cc: library 'm' not found" >&2; exit 1; }
done
exec cc "$@"
SH
chmod +x "$tmp/cc"
echo 'fn main() { }' > "$tmp/plain.bastio"
CC="$tmp/cc" "$BASTIOC" --backend c -o "$tmp/plain" "$tmp/plain.bastio" >/dev/null
if CC="$tmp/cc" "$BASTIOC" --backend c -o "$tmp/f64" tests/pass/f64.bastio >/dev/null 2>&1; then
  echo "tests/nolibm.sh: an f64 program built without -lm" >&2
  exit 1
fi
"$BASTIOC" --backend c -o "$tmp/f64" tests/pass/f64.bastio >/dev/null
"$tmp/f64" >/dev/null
echo "nolibm: ok"
