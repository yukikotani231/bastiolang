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
#           prints on stderr and its exit status (a failed proof's
#           sentence masked, as bastioc1 does not print it yet), the
#           C file of `--emit-c`, the C header of `--header`, the list of
#           `--trusted-base`, and the programs it builds, which must
#           exit as tests/pass says; and bastioc1 built by itself must
#           write the same C for itself as stage0 (the fixed point)
# The parser, the monomorphizer and the checker are also compared on damaged
# sources (stage1/mutate.py), so their error positions are checked too.
set -u
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-stage0/target/release/bastioc0}
# BastiOS imports core from this checkout (`import "core/..."`).
export BASTIO_LIB="$PWD"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for tool in lexdump astdump monodump checkdump smtdump verifydump cdump asmdump bastioc1; do
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
match=0 differ=0 skipped=0 failed=0
for f in $files; do
  t=$(target_of "$f")
  "$BASTIOC" --dump-verify --target "$t" "$f" >"$tmp/want" 2>/dev/null
  if ! grep -q -e 'proof obligation' -e '^error [^ ]* E-VERIFY' -e '^error [^ ]* E-SOLVER' -e '^error [^ ]* E-EXPORT' "$tmp/want"; then
    skipped=$((skipped+1))
    continue
  fi
  grep -q '^error' "$tmp/want" && failed=$((failed+1))
  "$tmp/verifydump" "$t" "$f" >"$tmp/got" 2>/dev/null
  if cmp -s "$tmp/want" "$tmp/got"; then
    match=$((match+1))
  else
    differ=$((differ+1))
    echo "DIFFER $f ($t)"
    diff "$tmp/want" "$tmp/got" | head -5 | cut -c1-300 | sed 's/^/    /'
  fi
done
echo "-- proofs: $match matched ($failed of them failed proofs), $differ differ ($skipped not reaching the verifier)"
{ [ "$differ" = 0 ] && [ "$match" -gt 0 ]; } || status=1
match=0 differ=0 errors=0
for f in $files; do
  t=$(target_of "$f")
  "$BASTIOC" --dump-c --target "$t" "$f" >"$tmp/want" 2>/dev/null
  grep -q '^error' "$tmp/want" && errors=$((errors+1))
  "$tmp/cdump" "$t" "$f" >"$tmp/got" 2>/dev/null
  if cmp -s "$tmp/want" "$tmp/got"; then
    match=$((match+1))
  else
    differ=$((differ+1))
    echo "DIFFER $f ($t)"
    diff "$tmp/want" "$tmp/got" | head -5 | cut -c1-300 | sed 's/^/    /'
  fi
done
echo "-- C: $match matched ($((match-errors)) of them C files), $differ differ"
{ [ "$differ" = 0 ] && [ "$match" -gt "$errors" ]; } || status=1
match=0 differ=0 unsup=0
for f in $files; do
  [ "$(target_of "$f")" = hosted ] || continue
  "$BASTIOC" --backend native --emit-asm -o "$tmp/want" "$f" 2>/dev/null; e=$?
  [ "$e" = 0 ] || [ "$e" = 4 ] || continue
  "$tmp/asmdump" --stage0 "$f" >"$tmp/got" 2>/dev/null; ge=$?
  if [ "$e" = 4 ] && [ "$ge" = 4 ]; then
    unsup=$((unsup+1)); match=$((match+1))
  elif [ "$e" = 0 ] && [ "$ge" = 0 ] && cmp -s "$tmp/want" "$tmp/got"; then
    match=$((match+1))
  else
    differ=$((differ+1))
    echo "DIFFER $f (native: stage0 exit $e, asmdump exit $ge)"
    diff "$tmp/want" "$tmp/got" | head -5 | cut -c1-300 | sed 's/^/    /'
  fi
done
if "$BASTIOC" --backend native -o "$tmp/asmdump_n" stage1/asmdump.bastio 2>/dev/null \
   && "$tmp/asmdump_n" --stage0 stage1/asmdump.bastio >"$tmp/got" 2>/dev/null \
   && "$tmp/asmdump" --stage0 stage1/asmdump.bastio >"$tmp/want" 2>/dev/null \
   && cmp -s "$tmp/want" "$tmp/got"; then
  match=$((match+1))
else
  differ=$((differ+1)); echo "DIFFER asmdump built by the native backend"
fi
echo "-- native: $match matched ($unsup of them unsupported, plus asmdump built natively), $differ differ"
{ [ "$differ" = 0 ] && [ "$match" -gt "$unsup" ]; } || status=1
# What only stage1's backend does (asmdump without --stage0) has no
# oracle: the programs it builds must exit as tests/pass says, and asmdump
# built that way must print the same assembly for itself.
built=0 differ=0
for f in tests/pass/*.bastio; do
  want=$(sed -n 's|^// expect-exit: *||p' "$f" | head -1)
  "$tmp/asmdump" "$f" >"$tmp/prog.s" 2>/dev/null; e=$?
  [ "$e" = 4 ] && continue
  if [ "$e" = 0 ] && as --64 -o "$tmp/prog.o" "$tmp/prog.s" && cc -o "$tmp/prog" "$tmp/prog.o"; then
    "$tmp/prog" </dev/null >/dev/null; got=$?
  else
    got="not built"
  fi
  if [ "$got" = "$want" ]; then
    built=$((built+1))
  else
    differ=$((differ+1)); echo "DIFFER $f (built by asmdump: exit $got, want $want)"
  fi
done
if "$tmp/asmdump" stage1/asmdump.bastio >"$tmp/self.s" 2>/dev/null \
   && as --64 -o "$tmp/self.o" "$tmp/self.s" && cc -o "$tmp/asmdump_s" "$tmp/self.o" \
   && "$tmp/asmdump_s" stage1/asmdump.bastio >"$tmp/got" 2>/dev/null \
   && cmp -s "$tmp/self.s" "$tmp/got"; then
  built=$((built+1))
else
  differ=$((differ+1)); echo "DIFFER asmdump built by itself"
fi
# bastioc1 built this way (it runs Z3 as a Child) must write the same C
# for itself as stage0.
if "$tmp/asmdump" stage1/bastioc1.bastio >"$tmp/b1.s" 2>/dev/null \
   && as --64 -o "$tmp/b1.o" "$tmp/b1.s" && cc -o "$tmp/bastioc1_n" "$tmp/b1.o" \
   && "$tmp/bastioc1_n" --emit-c -o "$tmp/b1n.c" stage1/bastioc1.bastio 2>/dev/null \
   && "$BASTIOC" --emit-c -o "$tmp/b1c.c" stage1/bastioc1.bastio 2>/dev/null \
   && cmp -s "$tmp/b1c.c" "$tmp/b1n.c"; then
  built=$((built+1))
else
  differ=$((differ+1)); echo "DIFFER bastioc1 built by asmdump"
fi
# User programs (BastiOS) cannot run here: they must assemble and link on
# their own, with no C library (bastios's make test runs them).
users=0
for f in tests/user/*.bastio; do
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
  if "$tmp/asmdump" --kernel ../bastios/src/main.bastio >"$tmp/kernel.s" 2>/dev/null \
     && as --64 -o "$tmp/kernel.o" "$tmp/kernel.s" \
     && ! nm -u "$tmp/kernel.o" | awk '{print $2}' \
          | grep -v -x -e _end -e bst_kernel_pml4 -e bst_cpu_enter -e bst_cpu_wait -e bst_platform_ctx \
                    -e bst_platform_cpus -e bst_platform_smp_start -e bst_platform_smp_join | grep -q .; then
    kernel=", and BastiOS's kernel"
  else
    differ=$((differ+1)); echo "DIFFER ../bastios/src/main.bastio (kernel not built by asmdump --kernel)"
  fi
fi
echo "-- native (stage1 only): $built programs built and run (plus asmdump built by itself, and bastioc1), $users user programs built$kernel, $differ differ"
{ [ "$differ" = 0 ] && [ "$built" -gt 1 ] && [ "$users" -gt 0 ]; } || status=1
# The driver. The sentence after `error[CODE]:` and the hints (`  = ..`)
# of the errors before the verifier are being ported phase by phase: where
# bastioc1 prints none, bastioc0's are left out of the comparison (a failed
# proof's counterexample is always compared). The reason a file cannot be
# read comes from the OS and is left out, and bastioc1 names itself.
mask() {  # stage0's stderr, bastioc1's stderr
  local keep_msg=1 keep_notes=1
  grep -q 'error\[[A-Z-]*\]: ' "$2" || keep_msg=0
  grep -q 'error\[E-\(VERIFY\|SOLVER\)' "$1" || grep -q '^  = ' "$2" || keep_notes=0
  if [ $keep_notes = 1 ]; then cat "$1"; else grep -v '^  = ' "$1"; fi \
    | if [ $keep_msg = 1 ]; then cat; else sed -e 's/^\(.*: error\[[A-Z-]*\]\): .*$/\1/'; fi \
    | sed -e 's/^\(.*: error\[E-IMPORT\]: cannot read `[^`]*`\): .*$/\1/' \
          -e 's/^\(.*: error\[E-PARSE\]: include_bytes: cannot read .*\): [^:]*$/\1/' -e 's/^bastioc0: /bastioc1: /'
}
match=0 differ=0 errors=0
for f in $files; do
  t=$(target_of "$f")
  "$BASTIOC" --check --target "$t" "$f" 2>"$tmp/want.err" >/dev/null; echo "exit $?" >>"$tmp/want.err"
  "$tmp/bastioc1" --check --target "$t" "$f" 2>"$tmp/got.err" >/dev/null; echo "exit $?" >>"$tmp/got.err"
  grep -q 'error\[' "$tmp/want.err" && errors=$((errors+1))
  mask "$tmp/want.err" "$tmp/got.err" >"$tmp/want"
  cp "$tmp/got.err" "$tmp/got"
  if cmp -s "$tmp/want" "$tmp/got"; then
    match=$((match+1))
  else
    differ=$((differ+1))
    echo "DIFFER $f ($t, --check)"
    diff "$tmp/want" "$tmp/got" | head -5 | cut -c1-300 | sed 's/^/    /'
  fi
done
cfiles=0
for f in tests/pass/*.bastio tests/user/*.bastio ../bastios/src/main.bastio ../bastios/user/*.bastio; do
  [ -f "$f" ] || continue
  case "$f" in tests/user/*) t=user ;; *) t=$(target_of "$f") ;; esac
  "$BASTIOC" --emit-c --target "$t" -o "$tmp/want.c" "$f" 2>/dev/null || continue
  cfiles=$((cfiles+1))
  if "$tmp/bastioc1" --emit-c --target "$t" -o "$tmp/got.c" "$f" 2>/dev/null && cmp -s "$tmp/want.c" "$tmp/got.c"; then
    match=$((match+1))
  else
    differ=$((differ+1))
    echo "DIFFER $f ($t, --emit-c)"
  fi
done
# The C header of every library with an `export fn`, and the trusted base.
mkdir -p "$tmp/hw" "$tmp/hg"
for f in $(grep -l '^export fn' tests/pass/*.bastio examples/*/*.bastio); do
  "$BASTIOC" --emit-c --header "$tmp/hw/lib.h" -o "$tmp/want.c" "$f" 2>/dev/null || continue
  cfiles=$((cfiles+1))
  if "$tmp/bastioc1" --emit-c --header "$tmp/hg/lib.h" -o "$tmp/got.c" "$f" 2>/dev/null && cmp -s "$tmp/hw/lib.h" "$tmp/hg/lib.h"; then
    match=$((match+1))
  else
    differ=$((differ+1))
    echo "DIFFER $f (--header)"
    diff "$tmp/hw/lib.h" "$tmp/hg/lib.h" | head -5 | cut -c1-300 | sed 's/^/    /'
  fi
done
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
if "$tmp/bastioc1" --trusted-base | cmp -s - <("$BASTIOC" --trusted-base); then
  match=$((match+1))
else
  differ=$((differ+1))
  echo "DIFFER --trusted-base"
fi
built=0
for f in tests/pass/*.bastio; do
  want=$(sed -n 's|^// expect-exit: *||p' "$f" | head -1)
  if "$tmp/bastioc1" -o "$tmp/prog" "$f" 2>/dev/null; then
    "$tmp/prog" </dev/null >/dev/null; got=$?
  else
    got=rejected
  fi
  if [ "$got" = "$want" ]; then
    built=$((built+1)); match=$((match+1))
  else
    differ=$((differ+1))
    echo "DIFFER $f (built by bastioc1: exit $got, want $want)"
  fi
done
echo "-- driver: $match matched ($errors rejected, $cfiles C files, $built programs built and run), $differ differ"
{ [ "$differ" = 0 ] && [ "$built" -gt 0 ]; } || status=1
# The fixed point: bastioc1 (built by stage0) builds itself, and that
# build writes the same C for bastioc1 as stage0 does.
if "$tmp/bastioc1" -o "$tmp/gen2" stage1/bastioc1.bastio 2>"$tmp/gen2.err" \
   && "$tmp/gen2" --emit-c -o "$tmp/gen2.c" stage1/bastioc1.bastio 2>>"$tmp/gen2.err" \
   && "$BASTIOC" --emit-c -o "$tmp/gen1.c" stage1/bastioc1.bastio 2>/dev/null \
   && cmp -s "$tmp/gen1.c" "$tmp/gen2.c"; then
  echo "-- fixed point: bastioc1 built by itself writes the same C for itself as stage0 ($(wc -l <"$tmp/gen1.c") lines)"
else
  echo "-- fixed point: FAILED"; sed 's/^/    /' "$tmp/gen2.err" | head -10; status=1
fi
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
# Proofs, on the mutants that reach the verifier: most of those fail a proof.
python3 stage1/mutate.py --verify "$BASTIOC" "--dump-verify" "$tmp/verifydump" 7 $((${MUTANTS:-5} * 2)) \
  tests/pass/*.bastio tests/fail/*.bastio stage1/verifycases/*.bastio || status=1
# C, on the mutants that still type-check (the rest print the same error).
python3 stage1/mutate.py "$BASTIOC" "--dump-c" "$tmp/cdump" 8 ${MUTANTS:-5} tests/pass/*.bastio || status=1
if [ -f ../bastios/src/main.bastio ]; then
  python3 stage1/mutate.py --via ../bastios/src/main.bastio "$BASTIOC" "--dump-c --target kernel" "$tmp/cdump kernel" 9 ${MUTANTS:-5} \
    $(ls ../bastios/src/*.bastio | grep -v /main.bastio) || status=1
fi
# The driver's error output, on damaged copies of every test program.
python3 stage1/mutate.py --driver "$BASTIOC" "--check" "$tmp/bastioc1" 10 ${MUTANTS:-5} \
  tests/pass/*.bastio tests/fail/*.bastio || status=1
exit $status
