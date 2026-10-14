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
#   proofs  stage1/verifydump.bastio vs bastioc0 --dump-verify (both ask Z3:
#           the number of proof obligations, or the first failed proof's
#           code, position and counterexample), on the programs that reach
#           the verifier
#   C       stage1/cdump.bastio     vs  bastioc0 --dump-c (the whole C file,
#           runtime prelude included), on every program the checker accepts
#   native  stage1/asmdump.bastio   vs  bastioc0 --backend native --emit-asm
#           (the whole assembly file), on every hosted program stage0
#           builds; where stage0 does not support a program yet (exit 4),
#           asmdump must not either. asmdump built by the native backend
#           must print the same assembly for itself. Without --stage0,
#           asmdump also does what stage0 does not (register arguments,
#           Child, host_eprint): the programs it builds must exit as
#           tests/pass says, and bastioc1 built by it must write the same C
#           for itself as stage0. With --user it builds tests/user without
#           the C library (they assemble and link on their own), and with
#           --kernel BastiOS's kernel when a built checkout is next to this
#           one (it must need only the platform's symbols)
#   driver  stage1/bastioc1.bastio  vs  bastioc0 itself: what `--check`
#           prints on stderr and its exit status, the C file of
#           `--emit-c`, the C header of `--header`, the list of
#           `--trusted-base`, the queries of `--dump-smt`, what
#           `--backend native` prints, and the
#           programs it builds either way, which must exit as tests/pass
#           says; with `--backend native --target user` (and `kernel`),
#           the assembly of asmdump --user (--kernel); and bastioc1 built by itself must write the same C for
#           itself as stage0 (the fixed point)
# The parser, the monomorphizer and the checker are also compared on damaged
# sources (stage1/mutate.py), so their error positions are checked too.
#
# DIFFTEST_PARTS (default: all) names the parts to run, so CI can run them
# on separate machines: dumps (lexer, parser, mono, checker, verifier,
# proofs), backends (C, native), driver (bastioc1 against stage0, and the
# fixed point), mutants-front (parser, mono and checker mutants) and
# mutants-back (verifier, proof, C and driver mutants). Each part builds
# only the tools it uses.
#
# Every comparison is independent of the others, so the files of a phase
# are compared JOBS at a time (default: the number of CPUs; JOBS=1 runs one
# at a time), and the slow single checks (the fixed point, the compiler
# built by asmdump) run in the background from the start. Results are
# printed in the same order whatever JOBS is.
set -u
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-stage0/target/release/bastioc0}
# BastiOS imports core from this checkout (`import "core/..."`).
export BASTIO_LIB="$PWD"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export BASTIOC
PARTS=${DIFFTEST_PARTS:-dumps backends driver mutants-front mutants-back}
part() { case " $PARTS " in *" $1 "*) return 0 ;; esac; return 1; }
tools=
part dumps && tools="$tools lexdump astdump monodump checkdump smtdump verifydump"
part backends && tools="$tools cdump asmdump"
part driver && tools="$tools bastioc1 asmdump"
part mutants-front && tools="$tools astdump monodump checkdump"
part mutants-back && tools="$tools smtdump verifydump cdump bastioc1"
[ -n "$tools" ] || { echo "difftest: no known part in DIFFTEST_PARTS=$PARTS"; exit 1; }
tools=$(printf '%s\n' $tools | sort -u)
pids=()
for tool in $tools; do
  "$BASTIOC" -o "$tmp/$tool" "stage1/$tool.bastio" 2>"$tmp/build.$tool.err" &
  pids+=($!)
done
built=1
for p in "${pids[@]}"; do wait "$p" || built=0; done
[ $built = 1 ] || { cat "$tmp"/build.*.err; exit 1; }
# Test programs marked `// stage1-only:` (tests/stage1_only.sh) use what
# stage0 does not have, so stage0 has no answer to compare with: they stay
# out of every corpus below. Each must still be one stage0 cannot handle,
# or the marker is stale and hides a program from the oracle: stage0 must
# reject a tests/pass program, and a tests/fail program with an error other
# than the one it expects.
keep() { for f in "$@"; do grep -q '^// stage1-only:' "$f" || echo "$f"; done; }
# BastiOS's kernel is one program (src/main.bastio imports every module), so
# the marker in src/main.bastio leaves the whole kernel out: its modules, the
# driver's run on it, and the mutants checked through it. The comparisons of
# two stage1 tools (asmdump --kernel against bastioc1) still run.
kernel_main=
if [ -f ../bastios/src/main.bastio ] && ! grep -q '^// stage1-only:' ../bastios/src/main.bastio; then
  kernel_main=../bastios/src/main.bastio
fi
files=$(keep $(find tests stage1 ${kernel_main:+../bastios/src} ../bastios/user -name '*.bastio' 2>/dev/null | sort))
pass_files=$(keep tests/pass/*.bastio)
fail_files=$(keep tests/fail/*.bastio)
user_files=$(keep tests/user/*.bastio)
status=0
if part dumps; then
s1only=0
for f in $(grep -l '^// stage1-only:' tests/pass/*.bastio tests/fail/*.bastio tests/user/*.bastio ../bastios/user/*.bastio 2>/dev/null); do
  s1only=$((s1only+1))
  want=$(sed -n 's|^// expect-error: *||p' "$f" | head -1)
  target=$(sed -n 's|^// target: *||p' "$f" | head -1)
  arch=$(sed -n 's|^// arch: *||p' "$f" | head -1)
  case $f in tests/user/* | ../bastios/user/*) target=user ;; esac
  if "$BASTIOC" --check --target "${target:-hosted}" --arch "${arch:-x86_64}" "$f" 2>"$tmp/s1.err" \
     || { [ -n "$want" ] && grep -q "error\[$want\]" "$tmp/s1.err" && ! grep -q "unknown function" "$tmp/s1.err"; }; then
    echo "STALE $f is marked stage1-only, but stage0 handles it: remove the marker"; status=1
  fi
done
if [ -f ../bastios/src/main.bastio ] && [ -z "$kernel_main" ]; then
  s1only=$((s1only+1))
  if "$BASTIOC" --check --target kernel ../bastios/src/main.bastio >/dev/null 2>&1; then
    echo "STALE ../bastios/src/main.bastio is marked stage1-only, but stage0 handles BastiOS's kernel: remove the marker"; status=1
  fi
fi
[ "$s1only" = 0 ] || echo "-- stage1-only: $s1only program(s) left out of the comparison"
fi

. tests/pfor.sh
# want and got in $w: the same, or a difference labelled $1.
verdict() {
  if cmp -s "$w/want" "$w/got"; then
    echo =match
  else
    echo =differ
    echo "$1"
    diff "$w/want" "$w/got" | head -5 | cut -c1-300 | sed 's/^/    /'
  fi
}
target_of() {
  case "$1" in
    ../bastios/src/*) echo kernel ;;
    ../bastios/user/*) echo user ;;
    *) echo hosted ;;
  esac
}
# The driver. Everything is compared except what bastioc1 cannot know:
# the reason a file cannot be read and the solver's own words when it
# fails (E-SOLVER) come from outside; and bastioc1 names itself.
mask() {  # stage0's stderr
  sed -e 's/^\(.*: error\[E-SOLVER\]\): .*$/\1/' \
      -e 's/^\(.*: error\[E-IMPORT\]: cannot read `[^`]*`\): .*$/\1/' \
      -e 's/^\(.*: error\[E-PARSE\]: include_bytes: cannot read .*\): [^:]*$/\1/' -e 's/^bastioc0: /bastioc1: /' "$1"
}
export -f verdict target_of mask

# The slow checks that are one program each start now, in the background;
# their results are printed where they belong below. Each one leaves a
# NAME.ok file when it passes: by the time the script asks, tens of
# thousands of processes have run, and when one of them gets the PID of
# a finished check, bash forgets that check's exit status ("wait: pid ...
# is not a child of this shell").
bg_ok() {  # pid, name
  wait "$1" 2>/dev/null
  [ -f "$tmp/$2.ok" ]
}
if part backends; then
# asmdump built by the native backend must print the same assembly for itself.
(
  "$BASTIOC" --backend native -o "$tmp/asmdump_n" stage1/asmdump.bastio 2>/dev/null \
    && "$tmp/asmdump_n" --stage0 stage1/asmdump.bastio >"$tmp/bg_n.got" 2>/dev/null \
    && "$tmp/asmdump" --stage0 stage1/asmdump.bastio >"$tmp/bg_n.want" 2>/dev/null \
    && cmp -s "$tmp/bg_n.want" "$tmp/bg_n.got" \
    && touch "$tmp/native.ok"
) & bg_native=$!
# asmdump built by itself must print the same assembly for itself.
(
  "$tmp/asmdump" stage1/asmdump.bastio >"$tmp/self.s" 2>/dev/null \
    && as --64 -o "$tmp/self.o" "$tmp/self.s" && cc -o "$tmp/asmdump_s" "$tmp/self.o" \
    && "$tmp/asmdump_s" stage1/asmdump.bastio >"$tmp/bg_s.got" 2>/dev/null \
    && cmp -s "$tmp/self.s" "$tmp/bg_s.got" \
    && touch "$tmp/self.ok"
) & bg_self=$!
# bastioc1 built by asmdump (it runs Z3 as a Child) must write the same C
# for itself as stage0.
(
  "$tmp/asmdump" stage1/bastioc1.bastio >"$tmp/b1.s" 2>/dev/null \
    && as --64 -o "$tmp/b1.o" "$tmp/b1.s" && cc -o "$tmp/bastioc1_n" "$tmp/b1.o" \
    && "$tmp/bastioc1_n" --emit-c -o "$tmp/b1n.c" stage1/bastioc1.bastio 2>/dev/null \
    && "$BASTIOC" --emit-c -o "$tmp/b1c.c" stage1/bastioc1.bastio 2>/dev/null \
    && cmp -s "$tmp/b1c.c" "$tmp/b1n.c" \
    && touch "$tmp/b1.ok"
) & bg_b1=$!
fi
if part driver; then
# The fixed point: bastioc1 (built by stage0) builds itself, and that
# build writes the same C for bastioc1 as stage0 does.
(
  "$tmp/bastioc1" -o "$tmp/gen2" stage1/bastioc1.bastio 2>"$tmp/gen2.err" \
    && "$tmp/gen2" --emit-c -o "$tmp/gen2.c" stage1/bastioc1.bastio 2>>"$tmp/gen2.err" \
    && "$BASTIOC" --emit-c -o "$tmp/gen1.c" stage1/bastioc1.bastio 2>/dev/null \
    && cmp -s "$tmp/gen1.c" "$tmp/gen2.c" \
    && touch "$tmp/fixed.ok"
) & bg_fixed=$!

fi
dump() {  # stage0 flag, stage1 tool, file
  "$BASTIOC" "$1" "$3" >"$w/want" 2>/dev/null
  "$tmp/$2" "$3" >"$w/got" 2>/dev/null
  verdict "DIFFER $3"
}
export -f dump
compare() {  # phase, stage0 flag, stage1 tool
  pfor "dump $2 $3" $files || status=1
  local match differ
  match=$(n match) differ=$(n differ)
  echo "-- $1: $match matched, $differ differ"
  { [ "$differ" = 0 ] && [ "$match" -gt 0 ]; } || status=1
}
if part dumps; then
compare lexer --dump-tokens lexdump
compare parser --dump-ast astdump
compare mono --dump-mono monodump
fi

check_one() {
  local t; t=$(target_of "$1")
  "$BASTIOC" --dump-check --target "$t" "$1" >"$w/want" 2>/dev/null
  "$tmp/checkdump" "$t" "$1" >"$w/got" 2>/dev/null
  verdict "DIFFER $1 ($t)"
}
smt_one() {
  local t; t=$(target_of "$1")
  if ! "$BASTIOC" --check --dump-smt --target "$t" "$1" >"$w/want" 2>"$w/err"; then
    echo =skipped
    return
  fi
  sed -n 's/.*: \([0-9]*\) proof obligation.*/\1 proof obligation(s)/p' "$w/err" >>"$w/want"
  "$tmp/smtdump" "$t" "$1" >"$w/got" 2>/dev/null
  verdict "DIFFER $1 ($t)"
}
proof_one() {
  local t; t=$(target_of "$1")
  "$BASTIOC" --dump-verify --target "$t" "$1" >"$w/want" 2>/dev/null
  if ! grep -q -e 'proof obligation' -e '^error [^ ]* E-VERIFY' -e '^error [^ ]* E-SOLVER' -e '^error [^ ]* E-EXPORT' "$w/want"; then
    echo =skipped
    return
  fi
  grep -q '^error' "$w/want" && echo =failed
  "$tmp/verifydump" "$t" "$1" >"$w/got" 2>/dev/null
  verdict "DIFFER $1 ($t)"
}
c_one() {
  local t; t=$(target_of "$1")
  "$BASTIOC" --dump-c --target "$t" "$1" >"$w/want" 2>/dev/null
  grep -q '^error' "$w/want" && echo =error
  "$tmp/cdump" "$t" "$1" >"$w/got" 2>/dev/null
  verdict "DIFFER $1 ($t)"
}
asm_one() {
  [ "$(target_of "$1")" = hosted ] || return
  local e ge
  "$BASTIOC" --backend native --emit-asm -o "$w/want" "$1" 2>/dev/null; e=$?
  [ "$e" = 0 ] || [ "$e" = 4 ] || return
  "$tmp/asmdump" --stage0 "$1" >"$w/got" 2>/dev/null; ge=$?
  if [ "$e" = 4 ] && [ "$ge" = 4 ]; then
    echo =unsupported; echo =match
  elif [ "$e" = 0 ] && [ "$ge" = 0 ] && cmp -s "$w/want" "$w/got"; then
    echo =match
  else
    echo =differ
    echo "DIFFER $1 (native: stage0 exit $e, asmdump exit $ge)"
    diff "$w/want" "$w/got" | head -5 | cut -c1-300 | sed 's/^/    /'
  fi
}
export -f check_one smt_one proof_one c_one asm_one

if part dumps; then
pfor check_one $files || status=1
match=$(n match) differ=$(n differ)
echo "-- checker: $match matched, $differ differ"
{ [ "$differ" = 0 ] && [ "$match" -gt 0 ]; } || status=1

pfor smt_one $files || status=1
match=$(n match) differ=$(n differ)
echo "-- verifier: $match matched, $differ differ ($(n skipped) not verified by stage0)"
{ [ "$differ" = 0 ] && [ "$match" -gt 0 ]; } || status=1

pfor proof_one $files || status=1
match=$(n match) differ=$(n differ)
echo "-- proofs: $match matched ($(n failed) of them failed proofs), $differ differ ($(n skipped) not reaching the verifier)"
{ [ "$differ" = 0 ] && [ "$match" -gt 0 ]; } || status=1

fi
if part backends; then
pfor c_one $files || status=1
match=$(n match) differ=$(n differ) errors=$(n error)
echo "-- C: $match matched ($((match-errors)) of them C files), $differ differ"
{ [ "$differ" = 0 ] && [ "$match" -gt "$errors" ]; } || status=1

pfor asm_one $files || status=1
match=$(n match) differ=$(n differ) unsup=$(n unsupported)
if bg_ok $bg_native native; then
  match=$((match+1))
else
  differ=$((differ+1)); echo "DIFFER asmdump built by the native backend"
fi
echo "-- native: $match matched ($unsup of them unsupported, plus asmdump built natively), $differ differ"
{ [ "$differ" = 0 ] && [ "$match" -gt "$unsup" ]; } || status=1

# What only stage1's backend does (asmdump without --stage0) has no
# oracle: the programs it builds must exit as tests/pass says, and asmdump
# built that way must print the same assembly for itself.
asm_run() {
  local want e got
  want=$(sed -n 's|^// expect-exit: *||p' "$1" | head -1)
  "$tmp/asmdump" "$1" >"$w/prog.s" 2>/dev/null; e=$?
  [ "$e" = 4 ] && return
  if [ "$e" = 0 ] && as --64 -o "$w/prog.o" "$w/prog.s" && cc -o "$w/prog" "$w/prog.o"; then
    "$w/prog" </dev/null >/dev/null; got=$?
  else
    got="not built"
  fi
  if [ "$got" = "$want" ]; then
    echo =built
  else
    echo =differ; echo "DIFFER $1 (built by asmdump: exit $got, want $want)"
  fi
}
export -f asm_run
pfor asm_run $pass_files || status=1
built=$(n built) differ=$(n differ)
if bg_ok $bg_self self; then
  built=$((built+1))
else
  differ=$((differ+1)); echo "DIFFER asmdump built by itself"
fi
if bg_ok $bg_b1 b1; then
  built=$((built+1))
else
  differ=$((differ+1)); echo "DIFFER bastioc1 built by asmdump"
fi
# User programs (BastiOS) cannot run here: they must assemble and link on
# their own, with no C library (bastios's make test runs them).
users=0
for f in $user_files; do
  if "$tmp/asmdump" --user "$f" >"$tmp/user.s" 2>/dev/null \
     && as --64 -o "$tmp/user.o" "$tmp/user.s" \
     && ld -static -nostdlib --no-undefined -e bst_umain -o "$tmp/user" "$tmp/user.o"; then
    users=$((users+1))
  else
    differ=$((differ+1)); echo "DIFFER $f (user program not built by asmdump --user)"
  fi
done
# BastiOS's kernel, when a built checkout is next to this one (its blobs are
# the user programs it builds): it must assemble, needing nothing but the
# platform's own symbols (boot/ in BastiOS).
kernel=""
if [ -f ../bastios/build/user/sh.bin ]; then
  if "$tmp/asmdump" --lib "$PWD" --kernel ../bastios/src/main.bastio >"$tmp/kernel.s" 2>/dev/null \
     && as --64 -o "$tmp/kernel.o" "$tmp/kernel.s" \
     && ! nm -u "$tmp/kernel.o" | awk '{print $2}' \
          | grep -v -x -e _end -e bst_kernel_pml4 -e bst_cpu_enter -e bst_cpu_wait -e bst_platform_ctx -e bst_platform_ctx_mask \
                    -e bst_platform_cpus -e bst_platform_smp_start -e bst_platform_smp_join -e bst_platform_acpi -e bst_platform_info -e bst_platform_deferred_cr3 -e bst_platform_mmu_state -e bst_ram_limit | grep -q .; then
    kernel=", and BastiOS's kernel"
  else
    differ=$((differ+1)); echo "DIFFER ../bastios/src/main.bastio (kernel not built by asmdump --kernel)"
  fi
fi
echo "-- native (stage1 only): $built programs built and run (plus asmdump built by itself, and bastioc1), $users user programs built$kernel, $differ differ"
{ [ "$differ" = 0 ] && [ "$built" -gt 1 ] && [ "$users" -gt 0 ]; } || status=1

fi
drv_check() {
  local t; t=$(target_of "$1")
  "$BASTIOC" --check --target "$t" "$1" 2>"$w/want.err" >/dev/null; echo "exit $?" >>"$w/want.err"
  "$tmp/bastioc1" --check --target "$t" "$1" 2>"$w/got" >/dev/null; echo "exit $?" >>"$w/got"
  grep -q 'error\[' "$w/want.err" && echo =error
  mask "$w/want.err" >"$w/want"
  verdict "DIFFER $1 ($t, --check)"
}
drv_emitc() {
  local t
  case "$1" in tests/user/*) t=user ;; *) t=$(target_of "$1") ;; esac
  "$BASTIOC" --emit-c --target "$t" -o "$w/want.c" "$1" 2>/dev/null || return
  echo =cfile
  if "$tmp/bastioc1" --emit-c --target "$t" -o "$w/got.c" "$1" 2>/dev/null && cmp -s "$w/want.c" "$w/got.c"; then
    echo =match
  else
    echo =differ; echo "DIFFER $1 ($t, --emit-c)"
  fi
}
# The C header of a library with an `export fn`.
drv_header() {
  mkdir -p "$w/hw" "$w/hg"
  "$BASTIOC" --emit-c --header "$w/hw/lib.h" -o "$w/want.c" "$1" 2>/dev/null || return
  echo =cfile
  if "$tmp/bastioc1" --emit-c --header "$w/hg/lib.h" -o "$w/got.c" "$1" 2>/dev/null && cmp -s "$w/hw/lib.h" "$w/hg/lib.h"; then
    echo =match
  else
    echo =differ; echo "DIFFER $1 (--header)"
    diff "$w/hw/lib.h" "$w/hg/lib.h" | head -5 | cut -c1-300 | sed 's/^/    /'
  fi
}
drv_run() {
  local want got
  want=$(sed -n 's|^// expect-exit: *||p' "$1" | head -1)
  if "$tmp/bastioc1" -o "$w/prog" "$1" 2>/dev/null; then
    "$w/prog" </dev/null >/dev/null; got=$?
  else
    got=rejected
  fi
  if [ "$got" = "$want" ]; then
    echo =built; echo =match
  else
    echo =differ; echo "DIFFER $1 (built by bastioc1: exit $got, want $want)"
  fi
}
# `--backend native`: the same stderr and exit status as bastioc0 (an
# unsupported program exits 4 with the same reason), except that stage1's
# backend may build what stage0's does not support, or stop at something
# later; what it builds must exit as tests/pass says.
drv_native() {
  local e0 e ok want got
  "$BASTIOC" --backend native -o "$w/nwant" "$1" 2>"$w/want.err" >/dev/null; e0=$?; echo "exit $e0" >>"$w/want.err"
  "$tmp/bastioc1" --backend native -o "$w/ngot" "$1" 2>"$w/got.err" >/dev/null; e=$?; echo "exit $e" >>"$w/got.err"
  mask "$w/want.err" >"$w/want"
  ok=1
  # Both unsupported: stage1's backend supports more, so it may stop at a
  # later construct than stage0's, with its own reason.
  [ "$e0" = 4 ] && { [ "$e" = 0 ] || { [ "$e" = 4 ] && grep -q '^bastioc1: the native backend does not support ' "$w/got.err"; }; } \
    || cmp -s "$w/want" "$w/got.err" || ok=0
  if [ $ok = 1 ] && [ "$e" = 0 ]; then
    want=$(sed -n 's|^// expect-exit: *||p' "$1" | head -1)
    "$w/ngot" </dev/null >/dev/null; got=$?
    if [ "$got" = "$want" ]; then echo =nbuilt; else ok=0; fi
  fi
  if [ $ok = 1 ]; then
    echo =match
  else
    echo =differ
    echo "DIFFER $1 (--backend native)"
    diff "$w/want" "$w/got.err" | head -5 | cut -c1-300 | sed 's/^/    /'
  fi
}
export -f drv_check drv_emitc drv_header drv_run drv_native

if part driver; then
pfor drv_check $files || status=1
match=$(n match) differ=$(n differ) errors=$(n error)
cfiles=0
cfs=()
for f in $pass_files $user_files $kernel_main $(keep ../bastios/user/*.bastio 2>/dev/null); do
  [ -f "$f" ] && cfs+=("$f")
done
pfor drv_emitc "${cfs[@]}" || status=1
match=$((match + $(n match))) differ=$((differ + $(n differ))) cfiles=$((cfiles + $(n cfile)))
pfor drv_header $(grep -l '^export fn' $pass_files examples/*/*.bastio) || status=1
match=$((match + $(n match))) differ=$((differ + $(n differ))) cfiles=$((cfiles + $(n cfile)))
# `--lib`: a program outside the repository, without and with it.
mkdir -p "$tmp/away"
printf 'import "core/option.bastio";\nfn main() -> u64 { return opt_or[u64](Option[u64]::Some(7), 0); }\n' >"$tmp/away/prog.bastio"
BASTIO_LIB= "$BASTIOC" --check "$tmp/away/prog.bastio" 2>"$tmp/want.err" >/dev/null; echo "exit $?" >>"$tmp/want.err"
BASTIO_LIB= "$tmp/bastioc1" --check "$tmp/away/prog.bastio" 2>"$tmp/got.err" >/dev/null; echo "exit $?" >>"$tmp/got.err"
mask "$tmp/want.err" "$tmp/got.err" >"$tmp/want"
if cmp -s "$tmp/want" "$tmp/got.err"; then
  match=$((match+1))
else
  differ=$((differ+1))
  echo "DIFFER --lib (missing)"
  diff "$tmp/want" "$tmp/got.err" | head -5 | cut -c1-300 | sed 's/^/    /'
fi
BASTIO_LIB= "$BASTIOC" --emit-c --lib "$PWD" -o "$tmp/want.c" "$tmp/away/prog.bastio" 2>/dev/null
if BASTIO_LIB= "$tmp/bastioc1" --emit-c --lib "$PWD" -o "$tmp/got.c" "$tmp/away/prog.bastio" 2>/dev/null && cmp -s "$tmp/want.c" "$tmp/got.c"; then
  match=$((match+1))
else
  differ=$((differ+1))
  echo "DIFFER --lib (--emit-c)"
fi
# The same through BASTIO_LIB (a missing directory first, to skip).
BASTIO_LIB="/nonexistent:$PWD" "$BASTIOC" --emit-c -o "$tmp/want.c" "$tmp/away/prog.bastio" 2>/dev/null
if BASTIO_LIB="/nonexistent:$PWD" "$tmp/bastioc1" --emit-c -o "$tmp/got.c" "$tmp/away/prog.bastio" 2>/dev/null && cmp -s "$tmp/want.c" "$tmp/got.c"; then
  match=$((match+1))
else
  differ=$((differ+1))
  echo "DIFFER BASTIO_LIB (--emit-c)"
fi
for f in tests/pass/fill_sum.bastio stage1/lexdump.bastio; do
  if "$tmp/bastioc1" --check --dump-smt "$f" 2>/dev/null | cmp -s - <("$BASTIOC" --check --dump-smt "$f" 2>/dev/null); then
    match=$((match+1))
  else
    differ=$((differ+1))
    echo "DIFFER --dump-smt $f"
  fi
done
# (stage1's list also names its own builtins, marked "stage1 only".)
if "$tmp/bastioc1" --trusted-base | grep -v 'stage1 only' | cmp -s - <("$BASTIOC" --trusted-base); then
  match=$((match+1))
else
  differ=$((differ+1))
  echo "DIFFER --trusted-base"
fi
pfor drv_run $pass_files || status=1
match=$((match + $(n match))) differ=$((differ + $(n differ))) built=$(n built)
pfor drv_native $pass_files || status=1
match=$((match + $(n match))) differ=$((differ + $(n differ))) nbuilt=$(n nbuilt)
# `--backend native --target user` (stage0 has no such backend): the
# assembly asmdump --user writes, which links with no C library.
nuser=0
for f in $user_files; do
  if "$tmp/bastioc1" --backend native --target user -o "$tmp/nuser.s" "$f" 2>/dev/null \
     && "$tmp/asmdump" --user "$f" >"$tmp/nuser_want.s" 2>/dev/null \
     && cmp -s "$tmp/nuser.s" "$tmp/nuser_want.s" \
     && as --64 -o "$tmp/nuser.o" "$tmp/nuser.s" \
     && ld -static -nostdlib --no-undefined -e bst_umain -o "$tmp/nuser" "$tmp/nuser.o"; then
    nuser=$((nuser+1)); match=$((match+1))
  else
    differ=$((differ+1)); echo "DIFFER $f (--backend native --target user)"
  fi
done
# And BastiOS's kernel (`--target kernel`), when a built checkout is next to
# this one and asmdump builds it (see above).
if [ -f ../bastios/build/user/sh.bin ] \
   && "$tmp/asmdump" --kernel ../bastios/src/main.bastio >"$tmp/nkernel_want.s" 2>/dev/null; then
  if "$tmp/bastioc1" --backend native --target kernel -o "$tmp/nkernel.s" ../bastios/src/main.bastio 2>/dev/null \
     && cmp -s "$tmp/nkernel.s" "$tmp/nkernel_want.s"; then
    nuser=$((nuser+1)); match=$((match+1))
  else
    differ=$((differ+1)); echo "DIFFER ../bastios/src/main.bastio (--backend native --target kernel)"
  fi
fi
echo "-- driver: $match matched ($errors rejected, $cfiles C files, $built programs built and run, $nbuilt built natively and run, $nuser BastiOS programs built natively), $differ differ"
{ [ "$differ" = 0 ] && [ "$built" -gt 0 ] && [ "$nbuilt" -gt 0 ] && [ "$nuser" -gt 0 ]; } || status=1
if bg_ok $bg_fixed fixed; then
  echo "-- fixed point: bastioc1 built by itself writes the same C for itself as stage0 ($(wc -l <"$tmp/gen1.c") lines)"
else
  echo "-- fixed point: FAILED"; sed 's/^/    /' "$tmp/gen2.err" | head -10; status=1
fi
fi
# The mutants (stage1/mutate.py runs JOBS of them at a time).
if part mutants-front; then
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
if [ -n "$kernel_main" ]; then
  python3 stage1/mutate.py --via ../bastios/src/main.bastio "$BASTIOC" "--dump-check --target kernel" "$tmp/checkdump kernel" 5 ${MUTANTS:-5} \
    $(ls ../bastios/src/*.bastio | grep -v /main.bastio) || status=1
fi
fi
if part mutants-back; then
# The verifier, on the mutants that still verify (most do not).
python3 stage1/mutate.py --smt "$BASTIOC" "--check --dump-smt" "$tmp/smtdump" 6 $((${MUTANTS:-5} * 4)) $pass_files || status=1
# Proofs, on the mutants that reach the verifier: most of those fail a proof.
python3 stage1/mutate.py --verify "$BASTIOC" "--dump-verify" "$tmp/verifydump" 7 $((${MUTANTS:-5} * 2)) \
  $pass_files $fail_files stage1/verifycases/*.bastio || status=1
# C, on the mutants that still type-check (the rest print the same error).
python3 stage1/mutate.py "$BASTIOC" "--dump-c" "$tmp/cdump" 8 ${MUTANTS:-5} $pass_files || status=1
if [ -n "$kernel_main" ]; then
  python3 stage1/mutate.py --via ../bastios/src/main.bastio "$BASTIOC" "--dump-c --target kernel" "$tmp/cdump kernel" 9 ${MUTANTS:-5} \
    $(ls ../bastios/src/*.bastio | grep -v /main.bastio) || status=1
fi
# The driver's error output, on damaged copies of every test program.
python3 stage1/mutate.py --driver "$BASTIOC" "--check" "$tmp/bastioc1" 10 ${MUTANTS:-5} \
  $pass_files $fail_files || status=1
fi
exit $status
