#!/usr/bin/env bash
# Performance benchmarks (ROADMAP invariant 5). Prints one line per
# measurement, so two runs can be diffed or kept as a CI artifact:
#
#   run NAME bastio=SECONDS c=SECONDS ratio=X native=SECONDS native_ratio=Y
#                                         generated code vs its C twin; `native`
#                                         is the same program built with
#                                         `--backend native` (no C compiler)
#   compile FILE obligations=N check=SECONDS build=SECONDS
#                                         proof time; proof + C build (one run)
#
# Each bench/NAME.bastio has a twin bench/NAME.c with the same algorithm,
# built with the same compiler and -O2. Both must print the same output.
# Run times are the best of REPS runs (default 3), wall clock.
# Usage: bench/run.sh [NAME...]   (default: every bench/*.bastio)
set -u
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-stage0/target/release/bastioc0}
CC=${CC:-cc}
REPS=${REPS:-3}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
status=0

now() { date +%s.%N; }
# best NAME CMD...: the fastest of REPS runs of CMD, in seconds.
best() {
  local b="" s e t
  for _ in $(seq "$REPS"); do
    s=$(now); "$@" >/dev/null || return 1; e=$(now)
    t=$(awk -v s="$s" -v e="$e" 'BEGIN { printf "%.3f", e - s }')
    if [ -z "$b" ] || awk -v t="$t" -v b="$b" 'BEGIN { exit !(t < b) }'; then b=$t; fi
  done
  echo "$b"
}

if [ $# -gt 0 ]; then names=("$@"); else
  names=(); for f in bench/*.bastio; do names+=("$(basename "$f" .bastio)"); done
fi

for n in "${names[@]}"; do
  if ! "$BASTIOC" -o "$tmp/$n" "bench/$n.bastio" 2>"$tmp/$n.err"; then
    echo "run $n FAILED: bastioc0 rejected it"; sed 's/^/    /' "$tmp/$n.err"; status=1; continue
  fi
  "$CC" -O2 -std=c11 -w -o "$tmp/$n.c.exe" "bench/$n.c" || { echo "run $n FAILED: C twin did not build"; status=1; continue; }
  if [ "$("$tmp/$n")" != "$("$tmp/$n.c.exe")" ]; then
    echo "run $n FAILED: output differs from the C twin"; status=1; continue
  fi
  tb=$(best "$tmp/$n") && tc=$(best "$tmp/$n.c.exe") || { echo "run $n FAILED: nonzero exit"; status=1; continue; }
  # Exit status 4: the native backend does not support the program yet.
  "$BASTIOC" --backend native -o "$tmp/$n.x" "bench/$n.bastio" 2>/dev/null; rc=$?
  if [ $rc = 4 ]; then
    native=" native=unsupported"
  elif [ $rc = 0 ] && [ "$("$tmp/$n.x")" = "$("$tmp/$n.c.exe")" ] && tx=$(best "$tmp/$n.x"); then
    native=" native=$tx native_ratio=$(awk -v b="$tx" -v c="$tc" 'BEGIN { printf "%.2f", (c > 0 ? b / c : 0) }')"
  else
    echo "run $n FAILED: the native build failed or its output differs"; status=1; native=""
  fi
  echo "run $n bastio=$tb c=$tc ratio=$(awk -v b="$tb" -v c="$tc" 'BEGIN { printf "%.2f", (c > 0 ? b / c : 0) }')$native"
done

# Compile time: the benchmarks themselves (when no names were given, also the
# self-hosted compiler's entry points, which pull in the rest of stage1).
files=(); for n in "${names[@]}"; do files+=("bench/$n.bastio"); done
if [ $# -eq 0 ]; then for f in stage1/*dump.bastio; do files+=("$f"); done; fi
for f in "${files[@]}"; do
  s=$(now); "$BASTIOC" --check "$f" 2>"$tmp/check.err"; rc=$?; e=$(now)
  [ $rc -eq 0 ] || { echo "compile $f FAILED"; sed 's/^/    /' "$tmp/check.err"; status=1; continue; }
  tch=$(awk -v s="$s" -v e="$e" 'BEGIN { printf "%.3f", e - s }')
  ob=$(grep -o '[0-9]* proof obligation' "$tmp/check.err" | head -1 | cut -d' ' -f1)
  s=$(now); "$BASTIOC" -o "$tmp/out" "$f" 2>/dev/null || { echo "compile $f FAILED: build"; status=1; continue; }; e=$(now)
  tbu=$(awk -v s="$s" -v e="$e" 'BEGIN { printf "%.3f", e - s }')
  echo "compile $f obligations=${ob:-0} check=$tch build=$tbu"
done
exit $status
