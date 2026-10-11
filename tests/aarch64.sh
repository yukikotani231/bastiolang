#!/usr/bin/env bash
# Hosted AArch64 through the C backend (#121):
#  1. every tests/pass program and single-file example, built with
#     `--arch aarch64`, exits (and prints) as on x86-64 (tests/run.sh with
#     ARCH=aarch64);
#  2. bastioc1 built for AArch64 writes byte for byte the same C for itself
#     as bastioc1 built for x86-64.
# The programs run under qemu-aarch64 unless this machine is an AArch64 one
# (where step 2 compares two native builds and proves less).
# Needs z3, an AArch64 C compiler (AARCH64_CC, default aarch64-linux-gnu-gcc)
# and, on other machines, qemu-aarch64 (Debian/Ubuntu: gcc-aarch64-linux-gnu
# qemu-user).
set -euo pipefail
cd "$(dirname "$0")/.."
# stage0 builds the two bastioc1s; the tests use the default compiler.
B0=${BASTIOC0:-stage0/target/release/bastioc0}
AARCH64_CC=${AARCH64_CC:-aarch64-linux-gnu-gcc}
export AARCH64_CC
if [ "$(uname -m)" = aarch64 ]; then RUN=; else
  RUN=qemu-aarch64
  export QEMU_LD_PREFIX=${QEMU_LD_PREFIX:-/usr/aarch64-linux-gnu}
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

ARCH=aarch64 tests/run.sh

# bastioc1 built with `cc` for this machine and with --arch aarch64; each
# writes its own C, the AArch64 one under $RUN.
"$B0" -o "$tmp/b1-x86_64" stage1/bastioc1.bastio 2>/dev/null
CC=$AARCH64_CC "$B0" --arch aarch64 -o "$tmp/b1-aarch64" stage1/bastioc1.bastio 2>/dev/null
"$tmp/b1-x86_64" --emit-c -o "$tmp/x86_64.c" stage1/bastioc1.bastio 2>/dev/null
$RUN "$tmp/b1-aarch64" --emit-c -o "$tmp/aarch64.c" stage1/bastioc1.bastio 2>/dev/null
if cmp -s "$tmp/x86_64.c" "$tmp/aarch64.c"; then
  echo "aarch64: bastioc1 built for AArch64 writes the same C for itself as for x86-64"
else
  echo "FAIL aarch64: bastioc1's C for itself differs between its AArch64 and x86-64 builds"
  diff "$tmp/x86_64.c" "$tmp/aarch64.c" | head -20 | sed 's/^/    /'
  exit 1
fi
