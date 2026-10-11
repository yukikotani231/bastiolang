#!/usr/bin/env bash
# AArch64 through the C backend:
#  1. every tests/pass program and single-file example, built with
#     `--arch aarch64`, exits (and prints) as on x86-64 (tests/run.sh with
#     ARCH=aarch64);
#  2. bastioc1 built for AArch64 writes byte for byte the same C for itself
#     as bastioc1 built for x86-64;
#  3. every tests/user program verifies and its C compiles freestanding for
#     `--target user --arch aarch64`;
#  4. tests/aarch64/kernel.bastio, built for `--target kernel --arch
#     aarch64` with the small platform beside it, boots on QEMU's virt
#     machine and prints what it should: page tables, a user program at EL0
#     making system calls, an abort, MMIO.
# The programs run under qemu-aarch64 unless this machine is an AArch64 one
# (where step 2 compares two native builds and proves less).
# Needs z3, an AArch64 C compiler (AARCH64_CC, default aarch64-linux-gnu-gcc)
# with binutils, qemu-system-aarch64 and, on other machines, qemu-aarch64
# (Debian/Ubuntu: gcc-aarch64-linux-gnu qemu-user qemu-system-arm).
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

BASTIOC=${BASTIOC:-$(tests/bastioc1.sh)}
# The kernel and user C: freestanding, no FP/SIMD in kernel code (the user's
# registers stay untouched until cpu_fpu_save), atomics inline.
CFLAGS="-ffreestanding -fno-builtin -nostdlib -mgeneral-regs-only -mno-outline-atomics -fno-pie -fno-stack-protector -O2"
fails=0
for f in tests/user/*.bastio; do
  n=$(basename "$f" .bastio)
  if ! "$BASTIOC" --target user --arch aarch64 -o "$tmp/$n.c" "$f" 2>"$tmp/$n.err" \
     || ! $AARCH64_CC $CFLAGS -c -o "$tmp/$n.o" "$tmp/$n.c" 2>>"$tmp/$n.err"; then
    echo "FAIL $f for --target user --arch aarch64"; sed 's/^/    /' "$tmp/$n.err"; fails=$((fails+1))
  fi
done
[ "$fails" = 0 ] && echo "aarch64: every user program builds for AArch64"

k=tests/aarch64
"$BASTIOC" --target kernel --arch aarch64 -o "$tmp/kernel.c" $k/kernel.bastio 2>/dev/null
$AARCH64_CC $CFLAGS -w -c -o "$tmp/kernel.o" "$tmp/kernel.c"
$AARCH64_CC $CFLAGS -c -o "$tmp/platform.o" $k/platform.c
$AARCH64_CC -c -o "$tmp/boot.o" $k/boot.S
${AARCH64_CC%gcc}ld -nostdlib -static --no-warn-rwx-segments -T $k/linker.ld \
  -o "$tmp/kernel.elf" "$tmp/boot.o" "$tmp/kernel.o" "$tmp/platform.o"
# The kernel powers the machine off when bst_kmain returns.
timeout 60 qemu-system-aarch64 -M virt -cpu cortex-a57 -m 128M -nographic -no-reboot \
  -kernel "$tmp/kernel.elf" </dev/null >"$tmp/boot.out" 2>&1 || true
tr -d '\r' <"$tmp/boot.out" >"$tmp/boot.txt"
want="aarch64: uart flags 0x90
US
aarch64: exit 7
aarch64: abort at 0x0 pc 0x8000000024
aarch64: done after 4 traps"
if grep -q '^aarch64: kernel up, [0-9]* pages$' "$tmp/boot.txt" \
   && [ "$(sed -n '2,$p' "$tmp/boot.txt")" = "$want" ]; then
  echo "aarch64: the kernel boots on QEMU virt, runs a user program at EL0 and powers off"
else
  echo "FAIL aarch64: the kernel's output on QEMU virt:"; sed 's/^/    /' "$tmp/boot.txt"; fails=$((fails+1))
fi
[ "$fails" = 0 ]
