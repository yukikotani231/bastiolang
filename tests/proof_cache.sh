#!/usr/bin/env bash
# The proof cache (BASTIO_PROOF_CACHE): a function whose verification
# conditions were proved before, with the same conditions and by the same solver, is not
# sent to the solver again.
# - a cold run proves everything and fills the cache;
# - a warm run starts the solver only to ask its version and check that it
#   exists;
# - a changed function is proved again, the others are not;
# - a function that fails is never cached: it fails the same way each time,
#   with the same report.
#
# usage: tests/proof_cache.sh
set -euo pipefail
cd "$(dirname "$0")/.."
command -v z3 >/dev/null || { echo "proof_cache: SKIPPED (no z3)"; exit 0; }
BASTIOC=$(tests/bastioc1.sh) || exit 1
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
real=$(command -v z3)

# A solver that counts how often it is started, then is z3.
cat >"$tmp/z3count" <<SH
#!/bin/sh
echo run >>"$tmp/starts"
exec "$real" "\$@"
SH
chmod +x "$tmp/z3count"
export BASTIO_SOLVER="$tmp/z3count" BASTIO_PROOF_CACHE="$tmp/cache" BASTIO_JOBS=2

starts() { if [ -f "$tmp/starts" ]; then wc -l <"$tmp/starts"; else echo 0; fi; }
entries() { if [ -d "$tmp/cache" ]; then ls "$tmp/cache" | wc -l; else echo 0; fi; }
bad=0
want() { # what, got, expectation (a test expression using $got)
  local got=$2
  if eval "[ $3 ]"; then echo "ok   $1"; else echo "FAIL $1: got $got, wanted $3"; bad=1; fi
}

src=tests/pass/bits.bastio
$BASTIOC --check $src >/dev/null
cold=$(starts); n1=$(entries)
want "a cold run starts solvers" "$cold" '"$got" -gt 1'
want "a cold run fills the cache" "$n1" '"$got" -gt 0'

: >"$tmp/starts"
$BASTIOC --check $src >/dev/null
want "a warm run starts the solver only for its version and its check" "$(starts)" '"$got" -eq 2'
want "a warm run adds nothing" "$(entries)" '"$got" -eq '"$n1"

# One function changed: only it is proved again.
sed 's/return 1 << n;/return (1 << n) + 0;/' $src >"$tmp/edit.bastio"
cmp -s $src "$tmp/edit.bastio" && { echo "FAIL the edit changed nothing"; exit 1; }
: >"$tmp/starts"
$BASTIOC --check "$tmp/edit.bastio" >/dev/null
want "a changed program is proved again" "$(starts)" '"$got" -gt 1'
want "and only that function is cached anew" "$(entries)" '"$got" -eq '"$((n1 + 1))"

# A failing function is not cached.
bad_src=tests/fail/assert.bastio
rm -rf "$tmp/cache"
for i in 1 2; do
  if $BASTIOC --check $bad_src >"$tmp/out$i" 2>&1; then echo "FAIL the failing program passed"; bad=1; fi
done
want "a failing program fails the same way twice" "$(cmp -s "$tmp/out1" "$tmp/out2" && echo same || echo differ)" '"$got" = same'
want "and says why" "$(grep -c 'error\[' "$tmp/out2")" '"$got" -ge 1'
exit $bad
