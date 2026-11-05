#!/usr/bin/env bash
# The example libraries (`export fn`, no `main`) built by the native backend
# for macOS: each is a Mach-O object that its C harness (examples/*/run.sh)
# links with Apple's cc and runs. Run on a Mac: Apple Silicon uses
# `--arch aarch64`, Intel `--backend native-ir`.
#
# usage: tests/macos_lib.sh [--gnu-link]
#   --gnu-link: Apple's as writes the object instead of bastioc1 itself. Without
#   it, each object is also compared with the one Apple's as writes
#   (tests/machoobj.py).
set -u
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-$(tests/bastioc1.sh)} || exit 1
link=${1:-}
case "$(uname -m)" in
  arm64|aarch64) flags="--arch aarch64 --os macos $link" ;;
  *) flags="--backend native-ir --os macos $link" ;;
esac
n=0
bad=0
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
if [ -z "$link" ]; then
  for f in $(grep -l '^export fn' examples/*/*.bastio); do
    "$BASTIOC" $flags -o "$tmp/own.o" "$f" >/dev/null 2>&1 || { echo "FAIL $f: no object"; bad=1; continue; }
    "$BASTIOC" $flags --gnu-link -o "$tmp/gnu.o" "$f" >/dev/null 2>&1 || { echo "FAIL $f: no object from Apple's as"; bad=1; continue; }
    tests/machoobj.py "$tmp/own.o" "$tmp/gnu.o" >/dev/null || { echo "FAIL $f: the object differs from Apple's"; tests/machoobj.py "$tmp/own.o" "$tmp/gnu.o" 2>&1 | tail -3; bad=1; }
  done
fi
for r in examples/*/run.sh; do
  [ -e "$r" ] || continue
  d=$(dirname "$r")
  grep -q '^export fn' "$d"/*.bastio || continue
  if out=$(BACKEND=native BASTIOC="$BASTIOC" BFLAGS="$flags" "$r" 2>&1); then
    n=$((n+1))
  else
    echo "FAIL $r"; echo "$out" | tail -20 | sed 's/^/    /'; bad=1
  fi
done
echo "macos_lib: $n libraries pass their C harnesses ($flags)"
[ $bad = 0 ] && [ $n -gt 0 ]
