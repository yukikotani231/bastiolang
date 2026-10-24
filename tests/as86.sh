#!/usr/bin/env bash
# Checks the in-house x86-64 assembler (stage1/asm86.bastio) against GNU as:
# every distinct instruction line the native backend emits for the test and
# bench programs is encoded by both, and the bytes must be equal. Branches,
# directives and lines that name a symbol are not covered yet (they need
# labels and relocations); lines with a `(%rip)` operand are skipped here.
#
# usage: tests/as86.sh
set -euo pipefail
cd "$(dirname "$0")/.."
command -v as >/dev/null && command -v objdump >/dev/null || { echo "as86.sh: needs GNU as and objdump" >&2; exit 0; }
BASTIOC=$(tests/bastioc1.sh)
b0=${BASTIOC0:-stage0/target/release/bastioc0}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

"$b0" -o "$work/as86" stage1/as86.bastio >/dev/null
for f in tests/pass/*.bastio bench/*.bastio; do
  "$BASTIOC" --backend native-ir --emit-asm -o "$work/a.s" "$f" >/dev/null 2>&1 && cat "$work/a.s" || true
done | grep -P '^\t[a-z]' \
  | sed -e 's/[[:space:]]*#.*$//' -e 's/^\t//' \
  | grep -v -E '^(call|jmp|j[a-z]+)[[:space:]]' | grep -v -F '(%rip)' \
  | sort -u > "$work/forms.txt"

n=$(wc -l < "$work/forms.txt")
as --64 -o "$work/f.o" "$work/forms.txt"
objdump -d --insn-width=16 "$work/f.o" \
  | awk -F'\t' 'NF >= 2 && $1 ~ /:$/ {print $2}' | sed 's/ *$//' > "$work/ref.txt"
"$work/as86" "$work/forms.txt" | head -n "$n" > "$work/mine.txt"
if ! diff <(paste -d'|' "$work/forms.txt" "$work/ref.txt") <(paste -d'|' "$work/forms.txt" "$work/mine.txt") > "$work/d"; then
  head -30 "$work/d"
  echo "as86.sh: FAILED: the assembler differs from GNU as" >&2
  exit 1
fi
echo "as86.sh: $n distinct instruction lines encode identically to GNU as"
