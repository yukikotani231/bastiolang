# Sourced by the test scripts: runs independent checks JOBS at a time.
#
# pfor CMD ARG...: runs `CMD ARG` (CMD is split into words; a function must
# be exported with `export -f`) for every ARG, JOBS at a time (default: the
# number of CPUs; JOBS=1 runs one at a time), each in a fresh directory of
# its own, $w. What they print is collected in $tmp/res in ARG order, so
# results read the same whatever JOBS is. Lines `=WORD` are tallies,
# counted with `n WORD`; pfor prints every other line. It fails if a job
# did not run to its end (killed, or xargs gave up), so that a lost job
# never passes as a check with nothing to report.
# Needs $tmp, a directory of the caller's that is removed at exit.
JOBS=${JOBS:-$(nproc 2>/dev/null || echo 2)}
export JOBS tmp
pfor() {
  local cmd=$1 out i=0 f
  shift
  out=$(mktemp -d "$tmp/pfor.XXXXXX")
  for f in "$@"; do printf '%s\0%s\0' "$i" "$f"; i=$((i+1)); done >"$out/args"
  P_CMD=$cmd P_OUT=$out xargs -0 -r -n2 -P "$JOBS" bash -c \
    'w=$(mktemp -d "$tmp/w.XXXXXX"); $P_CMD "$1" >"$P_OUT/$0" 2>&1; rm -rf "$w"; echo =ran >>"$P_OUT/$0"' <"$out/args"
  for ((f=0; f<i; f++)); do cat "$out/$f" 2>/dev/null; done >"$tmp/res"
  rm -rf "$out"
  grep -v '^=' "$tmp/res"
  if [ "$(n ran)" != "$i" ]; then
    echo "FAIL only $(n ran) of $i jobs ran to their end ($cmd)"
    return 1
  fi
}
n() { grep -cx "=$1" "$tmp/res"; }
