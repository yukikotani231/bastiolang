#!/usr/bin/env bash
# AArch64 through the C backend:
#  1. every tests/pass program and single-file example, built with
#     `--arch aarch64`, exits (and prints) as on x86-64 (tests/run.sh with
#     ARCH=aarch64);
#  2. bastioc1 built for AArch64 writes byte for byte the same C for itself
#     as bastioc1 built for x86-64;
#  3. every tests/user program verifies and its C compiles freestanding for
#     `--target user --arch aarch64`, and its native assembly (`--backend
#     native`) assembles and links on its own;
#  4. tests/aarch64/kernel.bastio, built for `--target kernel --arch
#     aarch64` with the small platform beside it, boots on QEMU's virt
#     machine and prints what it should: page tables, a user program at EL0
#     making system calls, an abort, MMIO;
#  5. kernels built by the native AArch64 backend (tests/aarch64/hello_kernel, hello_pool, hello_pt, hello_smp and kernel itself)
#     boot the same way and print what they should;
#  6. the programs the native AArch64 backend (`--arch aarch64 --backend
#     native`, no C compiler on the proved code) covers behave as under the C
#     backend (tests/aarch64_native.sh).
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
# The same programs through the native backend: assembly with no C compiler
# on the proved code, which must assemble and link on its own (the entry
# `bst_umain` and the runtime are all it needs).
nfails=0
for f in tests/user/*.bastio; do
  n=$(basename "$f" .bastio)
  if ! "$BASTIOC" --target user --arch aarch64 --backend native --emit-asm -o "$tmp/$n.s" "$f" 2>"$tmp/$n.err" \
     || ! ${AARCH64_CC%gcc}as -o "$tmp/$n.s.o" "$tmp/$n.s" 2>>"$tmp/$n.err" \
     || ! ${AARCH64_CC%gcc}ld -nostdlib -static -e bst_umain -o "$tmp/$n.s.elf" "$tmp/$n.s.o" 2>>"$tmp/$n.err"; then
    echo "FAIL $f for --target user --arch aarch64 --backend native"; sed 's/^/    /' "$tmp/$n.err"; nfails=$((nfails+1))
  fi
done
[ "$nfails" = 0 ] && echo "aarch64: every user program builds natively for AArch64"
fails=$((fails+nfails))

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
# The same platform boots a kernel built by the native backend (`--backend
# native`, no C compiler on the proved code): the UART console and the
# allocator in its own runtime so far (stage1/irarm64.bastio).
"$BASTIOC" --target kernel --arch aarch64 --backend native --emit-asm -o "$tmp/hello.s" $k/hello_kernel.bastio 2>/dev/null
${AARCH64_CC%gcc}as -o "$tmp/hello.o" "$tmp/hello.s"
${AARCH64_CC%gcc}ld -nostdlib -static --no-warn-rwx-segments -T $k/linker.ld \
  -o "$tmp/hello.elf" "$tmp/boot.o" "$tmp/hello.o" "$tmp/platform.o"
timeout 60 qemu-system-aarch64 -M virt -cpu cortex-a57 -m 128M -nographic -no-reboot \
  -kernel "$tmp/hello.elf" </dev/null >"$tmp/hello.out" 2>&1 || true
tr -d '\r' <"$tmp/hello.out" >"$tmp/hello.txt"
want="hello from the native AArch64 kernel
sum of squares: 332833500
ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMN
0xdeadbeef"
if [ "$(cat "$tmp/hello.txt")" = "$want" ]; then
  echo "aarch64: a kernel built by the native backend boots on QEMU virt and prints"
else
  echo "FAIL aarch64: the native kernel's output on QEMU virt:"; sed 's/^/    /' "$tmp/hello.txt"; fails=$((fails+1))
fi
# The same kernel built by the native backend (pool, tables, MMIO, the processor and the EL0 program) prints the same.
"$BASTIOC" --target kernel --arch aarch64 --backend native --emit-asm -o "$tmp/nk.s" $k/kernel.bastio 2>/dev/null
${AARCH64_CC%gcc}as -o "$tmp/nk.o" "$tmp/nk.s"
${AARCH64_CC%gcc}ld -nostdlib -static --no-warn-rwx-segments -T $k/linker.ld \
  -o "$tmp/nk.elf" "$tmp/boot.o" "$tmp/nk.o" "$tmp/platform.o"
timeout 60 qemu-system-aarch64 -M virt -cpu cortex-a57 -m 128M -nographic -no-reboot \
  -kernel "$tmp/nk.elf" </dev/null 2>&1 | tr -d '\r' >"$tmp/nk.txt" || true
# (The pool's size depends on how big the kernel image is, so the page count is masked.)
mask() { sed 's/^aarch64: kernel up, [0-9]* pages$/aarch64: kernel up, N pages/' "$1"; }
if [ -s "$tmp/nk.txt" ] && [ "$(mask "$tmp/nk.txt")" = "$(mask "$tmp/boot.txt")" ]; then
  echo "aarch64: the kernel built by the native backend runs a user program at EL0 and prints what the C backend's build does"
else
  echo "FAIL aarch64: the native kernel's output differs from the C backend's:"
  diff <(mask "$tmp/boot.txt") <(mask "$tmp/nk.txt") | sed 's/^/    /'; fails=$((fails+1))
fi
# With a pool: the runs of RAM the platform lists, pages taken and given back, and the
# allocator's second arena (carved from the pool) once the first is full.
"$BASTIOC" --target kernel --arch aarch64 --backend native --emit-asm -o "$tmp/pool.s" $k/hello_pool.bastio 2>/dev/null
${AARCH64_CC%gcc}as -o "$tmp/pool.o" "$tmp/pool.s"
${AARCH64_CC%gcc}ld -nostdlib -static --no-warn-rwx-segments -T $k/linker.ld \
  -o "$tmp/pool.elf" "$tmp/boot.o" "$tmp/pool.o" "$tmp/platform.o"
timeout 60 qemu-system-aarch64 -M virt -cpu cortex-a57 -m 128M -nographic -no-reboot \
  -kernel "$tmp/pool.elf" </dev/null >"$tmp/pool.out" 2>&1 || true
tr -d '\r' <"$tmp/pool.out" >"$tmp/pool.txt"
want="more than 1000 pages: yes
ids 0 7
zeroed 0
words 0x1234 0x5678
distinct yes
big 199999"
if [ "$(cat "$tmp/pool.txt")" = "$want" ]; then
  echo "aarch64: a native kernel takes pages from the pool and grows past its first arena"
else
  echo "FAIL aarch64: the native pool kernel's output on QEMU virt:"; sed 's/^/    /' "$tmp/pool.txt"; fails=$((fails+1))
fi
# With page tables: AArch64 descriptors (valid, read-only, hidden, shared bits, the instruction
# cache cleaned before a page is mapped); the same kernel built by the C backend must print the same.
"$BASTIOC" --target kernel --arch aarch64 --backend native --emit-asm -o "$tmp/pt.s" $k/hello_pt.bastio 2>/dev/null
${AARCH64_CC%gcc}as -o "$tmp/pt.o" "$tmp/pt.s"
${AARCH64_CC%gcc}ld -nostdlib -static --no-warn-rwx-segments -T $k/linker.ld \
  -o "$tmp/pt.elf" "$tmp/boot.o" "$tmp/pt.o" "$tmp/platform.o"
"$BASTIOC" --target kernel --arch aarch64 -o "$tmp/ptc.c" $k/hello_pt.bastio 2>/dev/null
$AARCH64_CC $CFLAGS -w -c -o "$tmp/ptc.o" "$tmp/ptc.c"
${AARCH64_CC%gcc}ld -nostdlib -static --no-warn-rwx-segments -T $k/linker.ld \
  -o "$tmp/ptc.elf" "$tmp/boot.o" "$tmp/ptc.o" "$tmp/platform.o"
for v in pt ptc; do
  timeout 60 qemu-system-aarch64 -M virt -cpu cortex-a57 -m 128M -nographic -no-reboot \
    -kernel "$tmp/$v.elf" </dev/null 2>&1 | tr -d '\r' >"$tmp/$v.txt" || true
done
if [ "$(cat "$tmp/pt.txt")" = "$(cat "$tmp/ptc.txt")" ] && grep -q '^drops 0xffffffffffffffff 3$' "$tmp/pt.txt"; then
  echo "aarch64: a native kernel builds page tables and prints what the C backend's does"
else
  echo "FAIL aarch64: the native page table kernel's output differs from the C backend's:"
  diff "$tmp/ptc.txt" "$tmp/pt.txt" | sed 's/^/    /'; fails=$((fails+1))
fi
# smp_run and locked calls: the thunk the native backend writes runs once on this platform (it has no other processors).
"$BASTIOC" --target kernel --arch aarch64 --backend native --emit-asm -o "$tmp/smp.s" $k/hello_smp.bastio 2>/dev/null
${AARCH64_CC%gcc}as -o "$tmp/smp.o" "$tmp/smp.s"
${AARCH64_CC%gcc}ld -nostdlib -static --no-warn-rwx-segments -T $k/linker.ld \
  -o "$tmp/smp.elf" "$tmp/boot.o" "$tmp/smp.o" "$tmp/platform.o"
timeout 60 qemu-system-aarch64 -M virt -cpu cortex-a57 -m 128M -nographic -no-reboot \
  -kernel "$tmp/smp.elf" </dev/null 2>&1 | tr -d '\r' >"$tmp/smp.txt" || true
if [ "$(cat "$tmp/smp.txt")" = "processors that ran: 1" ]; then
  echo "aarch64: a native kernel runs a function on every processor with smp_run and takes a lock"
else
  echo "FAIL aarch64: the native smp_run kernel's output on QEMU virt:"; sed 's/^/    /' "$tmp/smp.txt"; fails=$((fails+1))
fi
BASTIOC="$BASTIOC" tests/aarch64_native.sh || fails=$((fails+1))
# The same programs through bastioc1's own assembler and ELF linker.
SELF_LINK=1 BASTIOC="$BASTIOC" tests/aarch64_native.sh || fails=$((fails+1))
# The example libraries (`export fn`, no `main`), built by the native backend
# either way (GNU as, or bastioc1's own object writer) and driven by their C
# harnesses under qemu-aarch64.
if [ "$(uname -m)" = aarch64 ]; then lrun=; else lrun=qemu-aarch64; fi
export QEMU_LD_PREFIX=${QEMU_LD_PREFIX:-/usr/aarch64-linux-gnu}
libs=0
for r in examples/*/run.sh; do
  for link in --gnu-link --self-link; do
    if BACKEND=native BASTIOC="$BASTIOC" BFLAGS="--arch aarch64 $link" CC="$AARCH64_CC" RUNNER="$lrun" "$r" >"$tmp/lib.out" 2>&1; then libs=$((libs+1)); else
      echo "FAIL aarch64: $r ($link)"; sed 's/^/    /' "$tmp/lib.out" | tail -20; fails=$((fails+1))
    fi
  done
done
echo "aarch64: $libs library builds pass their C harnesses under qemu-aarch64"
[ "$fails" = 0 ]
