#!/usr/bin/env bash
# Differential test: the self-hosted lexer (stage1/lexdump.bastio, built
# by stage0) must print exactly the tokens stage0 prints with
# `--dump-tokens`, for every Bastio source in the repository (and in a
# BastiOS checkout next to it, if there is one).
set -u
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-stage0/target/release/bastioc0}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
"$BASTIOC" -o "$tmp/lexdump" stage1/lexdump.bastio 2>"$tmp/build.err" || { cat "$tmp/build.err"; exit 1; }
match=0
differ=0
for f in $(find tests stage1 ../bastios/src ../bastios/user -name '*.bastio' 2>/dev/null | sort); do
  "$BASTIOC" --dump-tokens "$f" >"$tmp/want" 2>/dev/null
  "$tmp/lexdump" "$f" >"$tmp/got" 2>/dev/null
  if cmp -s "$tmp/want" "$tmp/got"; then
    match=$((match+1))
  else
    differ=$((differ+1))
    echo "DIFFER $f"
    diff "$tmp/want" "$tmp/got" | head -5 | sed 's/^/    /'
  fi
done
echo "-- lexer: $match matched, $differ differ"
[ "$differ" = 0 ] && [ "$match" -gt 0 ]
