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
pass=$(n pass) skip=$(n skip) fail=$(($(n fail) + lost))
echo "aarch64 native: $pass same as the C backend, $skip skipped (not covered yet), $fail failed"
[ $fail = 0 ]
