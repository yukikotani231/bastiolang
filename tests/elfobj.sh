#!/usr/bin/env bash
# The relocatable objects that bastioc1 writes itself (stage1/elf.bastio,
# `elf_rel`) hold the same global symbols and the same relocation types as
# the objects GNU as writes for the same assembly. The example libraries
# link with their C harnesses and pass them in tests/native.sh.
# Needs readelf; BASTIOC overrides the compiler (default: stage0's build).
set -eu
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-$(tests/bastioc1.sh | tail -1)}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
syms() { readelf -sW "$1" | awk '$5=="GLOBAL" {print $5, $4=="NOTYPE"?"-":"t", ($7=="UND")?"UND":"def", $8}' | sort; }
rels() { readelf -rW "$1" | awk '$3 ~ /^R_X86_64/ {sub(/_REX/,"",$3); sub(/GOTPCRELX/,"GOTPCREL",$3); print $3}' | sort | uniq -c; }
n=0
for f in examples/*/*.bastio; do
  grep -q '^export fn' "$f" || continue
  "$BASTIOC" --backend native --self-link -o "$tmp/own.o" "$f" >/dev/null 2>&1 || { echo "FAIL $f: no object"; exit 1; }
  "$BASTIOC" --backend native --gnu-link -o "$tmp/gnu.o" "$f" >/dev/null 2>&1 || { echo "FAIL $f: no GNU object"; exit 1; }
  # GNU as adds _GLOBAL_OFFSET_TABLE_ when a GOT reference is present.
  if ! diff <(syms "$tmp/own.o") <(syms "$tmp/gnu.o" | grep -v _GLOBAL_OFFSET_TABLE_) >/dev/null; then
    echo "FAIL $f: global symbols differ"; diff <(syms "$tmp/own.o") <(syms "$tmp/gnu.o"); exit 1
  fi
  if ! diff <(rels "$tmp/own.o") <(rels "$tmp/gnu.o") >/dev/null; then
    echo "FAIL $f: relocations differ"; diff <(rels "$tmp/own.o") <(rels "$tmp/gnu.o"); exit 1
  fi
  n=$((n+1))
done
echo "elfobj: $n library objects match GNU as's global symbols and relocation types"
