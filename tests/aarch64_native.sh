#!/usr/bin/env bash
# Differential test of the native AArch64 backend (`--arch aarch64 --backend
# native`, stage1/irarm64.bastio): every tests/pass program and every bench
# program is built for AArch64 with no C compiler on the proved code, and
# must exit with the status of its `// expect-exit:` header (the C backend's
# status for a bench program) and print what the C backend prints (stdin
# empty). The executables run under qemu-aarch64 unless this machine is an
# AArch64 one. A program the backend does not cover yet is reported as
# skipped; anything else that differs fails.
#
# Needs an AArch64 assembler and linker driver (aarch64-linux-gnu-as and
# aarch64-linux-gnu-gcc, or CC) and qemu-aarch64 (Debian/Ubuntu:
# gcc-aarch64-linux-gnu qemu-user).
set -u
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
BASTIOC=${BASTIOC:-$(tests/bastioc1.sh)} || exit 1
. tests/pfor.sh
. tests/stage1_only.sh
if [ "$(uname -m)" = aarch64 ]; then RUN=; else
  RUN=qemu-aarch64
  export QEMU_LD_PREFIX=${QEMU_LD_PREFIX:-/usr/aarch64-linux-gnu}
fi
export BASTIOC RUN
# One program; JOBS of them at a time.
both() {
  local f=$1 want xs cs
  if ! "$BASTIOC" -o "$w/c.exe" "$f" 2>/dev/null; then
    echo "FAIL $f: the C backend rejected it"; echo =fail; return
  fi
  "$w/c.exe" </dev/null >"$w/c.out" 2>/dev/null; cs=$?
  "$BASTIOC" --arch aarch64 --backend native -o "$w/x.exe" "$f" 2>"$w/err"; rc=$?
  if [ $rc = 4 ]; then echo "skip $f: $(grep -o 'does not cover.*' "$w/err" | head -1)"; echo =skip; return; fi
  if [ $rc != 0 ]; then echo "FAIL $f: AArch64 native build failed"; sed 's/^/    /' "$w/err" | head -20; echo =fail; return; fi
  $RUN "$w/x.exe" </dev/null >"$w/x.out" 2>/dev/null; xs=$?
  if [ $cs != $xs ]; then echo "FAIL $f: exit $xs, C backend $cs"; echo =fail; return; fi
  if ! cmp -s "$w/c.out" "$w/x.out"; then
    echo "FAIL $f: output differs from the C backend"; diff "$w/c.out" "$w/x.out" | head -10 | sed 's/^/    /'; echo =fail; return
  fi
  echo =pass
}
export -f both
lost=0
pfor both $(s1files tests/pass/*.bastio bench/*.bastio) || lost=1
# A copy to or from a page lent to a device is ordered both ways (`dmb osh`,
# as the C prelude's bytes_to_dma and dma_to_bytes are): two copies, four
# barriers, besides the two full ones of dma_lend and dma_take.
"$BASTIOC" --arch aarch64 --backend native --emit-asm -o "$tmp/dma.s" tests/ir_a64/dma_copy.bastio 2>/dev/null
if [ "$(grep -c 'dmb osh' "$tmp/dma.s")" = 4 ] && [ "$(grep -c 'dmb ish' "$tmp/dma.s")" = 2 ]; then
  echo "aarch64 native: DMA copies carry dmb osh before and after"
else
  echo "FAIL tests/ir_a64/dma_copy.bastio: the DMA copies are not bracketed by dmb osh"; lost=1
fi
# The compiler itself, built for AArch64 with no C compiler on its code, must
# compile itself to the same C as the build that runs here. It needs z3 on
# the machine (qemu-aarch64 runs it as a host program).
if [ "${SELFHOST:-1}" = 1 ]; then
  if ! "$BASTIOC" --arch aarch64 --backend native -o "$tmp/bastioc1.a64" stage1/bastioc1.bastio 2>"$tmp/self.err"; then
    echo "FAIL stage1/bastioc1.bastio: AArch64 native build failed"; sed 's/^/    /' "$tmp/self.err" | head -20; lost=1
  elif ! "$BASTIOC" --emit-c -o "$tmp/self.here.c" stage1/bastioc1.bastio 2>/dev/null \
       || ! $RUN "$tmp/bastioc1.a64" --emit-c -o "$tmp/self.a64.c" stage1/bastioc1.bastio 2>"$tmp/self.err"; then
    echo "FAIL the AArch64 bastioc1 did not compile itself"; sed 's/^/    /' "$tmp/self.err" | head -20; lost=1
  elif ! cmp -s "$tmp/self.here.c" "$tmp/self.a64.c"; then
    echo "FAIL the AArch64 bastioc1 wrote different C for itself"; lost=1
  else
    echo "aarch64 native: bastioc1 built for AArch64 writes the same C for itself ($(wc -l <"$tmp/self.a64.c") lines)"
  fi
fi
pass=$(n pass) skip=$(n skip) fail=$(($(n fail) + lost))
echo "aarch64 native: $pass same as the C backend, $skip skipped (not covered yet), $fail failed"
[ $fail = 0 ]
