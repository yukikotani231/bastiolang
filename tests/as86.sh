#!/usr/bin/env bash
# Checks the in-house x86-64 assembler (stage1/asm86.bastio, asmobj.bastio)
# against GNU as, on the assembly the native-ir backend emits for every test
# and bench program:
#
# - every distinct non-branch instruction line without a symbol is encoded by
#   both, and the bytes must be equal;
# - every whole file is assembled by both (tests/as86_obj.py), and the
#   sections' bytes, the size of .bss and the relocations must be equal.
#
# Branch relaxation follows GNU as's passes; on a very large file (the
# compiler itself, 13 MB) a few branches that fit in two bytes can come out
# longer, which is still correct code, so only the small programs are compared.
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
mkdir -p "$work/asm"
for f in tests/pass/*.bastio bench/*.bastio; do
  out="$work/asm/$(basename "$f" .bastio).s"
  "$BASTIOC" --backend native-ir --emit-asm -o "$out" "$f" >/dev/null 2>&1 || rm -f "$out"
done
cat "$work"/asm/*.s | grep -P '^\t[a-z]' \
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
tests/as86_obj.py "$work/as86" "$work"/asm/*.s
