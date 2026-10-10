#!/usr/bin/env bash
# Differential test: each phase of the self-hosted compiler, built by
# stage0, must print exactly what stage0 prints for every Bastio source in
# the repository (and in a BastiOS checkout next to it, if there is one):
#   lexer   stage1/lexdump.bastio  vs  bastioc0 --dump-tokens
#   parser  stage1/astdump.bastio   vs  bastioc0 --dump-ast
#   loader and monomorphizer
#           stage1/monodump.bastio  vs  bastioc0 --dump-mono
#   checker stage1/checkdump.bastio vs  bastioc0 --dump-check (BastiOS
#           kernel sources for --target kernel, its user programs for user)
#   verifier stage1/smtdump.bastio  vs  bastioc0 --check --dump-smt (every
#           SMT command, and the number of proof obligations), on the
#           programs stage0 verifies
# The parser, the monomorphizer and the checker are also compared on damaged
# sources (stage1/mutate.py), so their error positions are checked too.
set -u
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-stage0/target/release/bastioc0}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for tool in lexdump astdump monodump checkdump smtdump; do
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
target_of() {
  case "$1" in
    ../bastios/src/*) echo kernel ;;
    ../bastios/user/*) echo user ;;
    *) echo hosted ;;
  esac
}
match=0 differ=0
for f in $files; do
  t=$(target_of "$f")
  "$BASTIOC" --dump-check --target "$t" "$f" >"$tmp/want" 2>/dev/null
  "$tmp/checkdump" "$t" "$f" >"$tmp/got" 2>/dev/null
  if cmp -s "$tmp/want" "$tmp/got"; then
    match=$((match+1))
  else
    differ=$((differ+1))
    echo "DIFFER $f ($t)"
    diff "$tmp/want" "$tmp/got" | head -5 | cut -c1-300 | sed 's/^/    /'
  fi
done
echo "-- checker: $match matched, $differ differ"
{ [ "$differ" = 0 ] && [ "$match" -gt 0 ]; } || status=1
match=0 differ=0 skipped=0
for f in $files; do
  t=$(target_of "$f")
  if ! "$BASTIOC" --check --dump-smt --target "$t" "$f" >"$tmp/want" 2>"$tmp/err"; then
    skipped=$((skipped+1))
    continue
  fi
  sed -n 's/.*: \([0-9]*\) proof obligation.*/\1 proof obligation(s)/p' "$tmp/err" >>"$tmp/want"
  "$tmp/smtdump" "$t" "$f" >"$tmp/got" 2>/dev/null
  if cmp -s "$tmp/want" "$tmp/got"; then
    match=$((match+1))
  else
    differ=$((differ+1))
    echo "DIFFER $f ($t)"
    diff "$tmp/want" "$tmp/got" | head -5 | cut -c1-300 | sed 's/^/    /'
  fi
done
echo "-- verifier: $match matched, $differ differ ($skipped not verified by stage0)"
{ [ "$differ" = 0 ] && [ "$match" -gt 0 ]; } || status=1
python3 stage1/mutate.py "$BASTIOC" --dump-ast "$tmp/astdump" 1 ${MUTANTS:-5} $files || status=1
python3 stage1/mutate.py "$BASTIOC" --dump-mono "$tmp/monodump" 2 ${MUTANTS:-5} $files || status=1
for t in hosted kernel user; do
  tfiles=$(for f in $files; do [ "$(target_of "$f")" = "$t" ] && echo "$f"; done)
  [ -n "$tfiles" ] || continue
  python3 stage1/mutate.py "$BASTIOC" "--dump-check --target $t" "$tmp/checkdump $t" 3 ${MUTANTS:-5} $tfiles || status=1
done
# Library files have no `main`, so on their own they only reach "no main";
# check their mutants through a program that imports them.
python3 stage1/mutate.py --via stage1/checkdump.bastio "$BASTIOC" "--dump-check" "$tmp/checkdump" 4 ${MUTANTS:-5} \
  $(ls stage1/*.bastio | grep -v -e dump.bastio) || status=1
if [ -f ../bastios/src/main.bastio ]; then
  python3 stage1/mutate.py --via ../bastios/src/main.bastio "$BASTIOC" "--dump-check --target kernel" "$tmp/checkdump kernel" 5 ${MUTANTS:-5} \
    $(ls ../bastios/src/*.bastio | grep -v /main.bastio) || status=1
fi
# The verifier, on the mutants that still verify (most do not).
python3 stage1/mutate.py --smt "$BASTIOC" "--check --dump-smt" "$tmp/smtdump" 6 $((${MUTANTS:-5} * 4)) tests/pass/*.bastio || status=1
exit $status
