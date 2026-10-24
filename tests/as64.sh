#!/usr/bin/env bash
# Checks the in-house AArch64 assembler (stage1/asma64.bastio) against GNU as
# on the assembly the native AArch64 backend emits for every test and bench
# program, plus the hand-written edge cases of tests/as64_extra.txt: every
# distinct instruction line is encoded by both (tests/as64_forms.py), and the
# word and the relocation (type, symbol, addend) must be equal. Labels and
# symbols are replaced by one symbol so that each distinct form is checked once.
#
# usage: tests/as64.sh
set -euo pipefail
cd "$(dirname "$0")/.."
command -v aarch64-linux-gnu-as >/dev/null && command -v aarch64-linux-gnu-objcopy >/dev/null \
  && command -v aarch64-linux-gnu-readelf >/dev/null || { echo "as64.sh: needs the aarch64-linux-gnu binutils" >&2; exit 0; }
BASTIOC=$(tests/bastioc1.sh)
b0=${BASTIOC0:-stage0/target/release/bastioc0}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

"$b0" -o "$work/as64" stage1/as64.bastio >/dev/null
mkdir -p "$work/asm"
for f in tests/pass/*.bastio bench/*.bastio; do
  out="$work/asm/$(basename "$f" .bastio).s"
  "$BASTIOC" --arch aarch64 --backend native --emit-asm -o "$out" "$f" >/dev/null 2>&1 || rm -f "$out"
done
cat "$work"/asm/*.s | grep -P '^\t[a-z]' \
  | sed -e 's/[[:space:]]*\/\/.*$//' -e 's/^\t//' \
  | sed -E -e 's/\.L[A-Za-z0-9_]+/Lsym/g' -e 's/(^|[ ,])[0-9]+[fb]$/\1Lsym/' \
  | sort -u > "$work/forms.txt"
tests/as64_forms.py "$work/as64" "$work/forms.txt"
tests/as64_forms.py "$work/as64" tests/as64_extra.txt
