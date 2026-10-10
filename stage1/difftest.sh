#!/usr/bin/env bash
# Differential test: each phase of the self-hosted compiler, built by
# stage0, must print exactly what stage0 prints for every Bastio source in
# the repository (and in a BastiOS checkout next to it, if there is one):
#   lexer   stage1/lexdump.bastio  vs  bastioc0 --dump-tokens
#   parser  stage1/astdump.bastio   vs  bastioc0 --dump-ast
#   loader and monomorphizer
#           stage1/monodump.bastio  vs  bastioc0 --dump-mono
# The parser and the monomorphizer are also compared on damaged sources
# (stage1/mutate.py), so their error positions are checked too.
set -u
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-stage0/target/release/bastioc0}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for tool in lexdump astdump monodump; do
  "$BASTIOC" -o "$tmp/$tool" "stage1/$tool.bastio" 2>"$tmp/build.err" || { cat "$tmp/build.err"; exit 1; }
done
files=$(find tests stage1 ../bastios/src ../bastios/user -name '*.bastio' 2>/dev/null | sort)
status=0
compare() {  # phase, stage0 flag, stage1 tool
  local match=0 differ=0
  for f in $files; do
    "$BASTIOC" "$2" "$f" >"$tmp/want" 2>/dev/null
    "$tmp/$3" "$f" >"$tmp/got" 2>/dev/null
    if cmp -s "$tmp/want" "$tmp/got"; then
      match=$((match+1))
    else
      differ=$((differ+1))
      echo "DIFFER $f"
      diff "$tmp/want" "$tmp/got" | head -5 | cut -c1-300 | sed 's/^/    /'
    fi
  done
  echo "-- $1: $match matched, $differ differ"
  { [ "$differ" = 0 ] && [ "$match" -gt 0 ]; } || status=1
}
compare lexer --dump-tokens lexdump
compare parser --dump-ast astdump
compare mono --dump-mono monodump
python3 stage1/mutate.py "$BASTIOC" --dump-ast "$tmp/astdump" 1 ${MUTANTS:-5} $files || status=1
python3 stage1/mutate.py "$BASTIOC" --dump-mono "$tmp/monodump" 2 ${MUTANTS:-5} $files || status=1
exit $status
