#!/usr/bin/env bash
# The macOS executables bastioc1 writes itself (stage1/machold.bastio), checked
# without a Mac: tests/macho_check.py reads the headers, the segments and the
# arm64 code signature of a few programs for each architecture, and
# the SHA-256 the signature uses is compared with sha256sum.
#
# usage: tests/macho.sh
set -euo pipefail
cd "$(dirname "$0")/.."
BASTIOC=$(tests/bastioc1.sh)
b0=${BASTIOC0:-stage0/target/release/bastioc0}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for f in tests/pass/accrec.bastio tests/pass/add_nonzero.bastio bench/fib.bastio; do
  n=$(basename "$f" .bastio)
  "$BASTIOC" --backend native-ir --os macos --self-link -o "$work/$n.x86" "$f" >/dev/null 2>&1
  "$BASTIOC" --arch aarch64 --backend native --os macos --self-link -o "$work/$n.arm" "$f" >/dev/null 2>&1
done
tests/macho_check.py "$work"/*
"$b0" -o "$work/as86" stage1/as86.bastio >/dev/null
for f in "$work"/accrec.arm stage1/elf.bastio /dev/null; do
  [ "$("$work/as86" --sha "$f")" = "$(sha256sum "$f" | cut -d' ' -f1)" ] || { echo "macho.sh: SHA-256 differs from sha256sum on $f" >&2; exit 1; }
done
echo "macho.sh: SHA-256 equals sha256sum"
