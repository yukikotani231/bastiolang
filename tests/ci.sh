#!/usr/bin/env bash
# Runs, on this machine, every check the CI workflow runs, in the same order,
# and stops at the first failure. Run it before every push: a failure here
# costs seconds, the same failure in CI costs a run and a notification.
#
# Steps whose tools are missing (z3, the tree-sitter CLI, mdbook, npm) are reported
# as SKIPPED, not passed; CI still runs them.
#
# usage: tests/ci.sh [--bench]    (--bench also runs bench/run.sh)
set -euo pipefail
cd "$(dirname "$0")/.."

skipped=()
step() { printf '\n== %s\n' "$1"; }

# Interrupted or killed (any signal but KILL), this script stops every
# process it started before it exits, the Z3 solvers included. Without
# this, `kill` on ci.sh ended only ci.sh: the test scripts went on, and the
# solvers the compilers start on script files ran their queries to the end
# even after their compiler was gone, slowing whatever ran next. The whole
# tree is listed before anything is stopped, since a process stopped first
# would hand its children to init, out of reach. The steps run in the
# background and ci.sh waits for them, because bash runs a trap only once a
# foreground command has finished.
descendants() {
  ps -e -o pid= -o ppid= | awk -v root="$1" '
    { up[$1] = $2 }
    END { for (p in up) { q = p; while (q in up && q != root && q > 1) q = up[q]; if (q == root && p != root) print p } }'
}
stop_all() {
  trap - INT TERM HUP
  local pids
  pids=$(descendants $$)
  if [ -n "$pids" ]; then kill -TERM $pids 2>/dev/null || true; fi
  echo "ci.sh: interrupted; stopped the checks it started" >&2
  exit 130
}
trap stop_all INT TERM HUP
run() { "$@" & wait $!; }

if git rev-parse -q --verify origin/main >/dev/null; then
  step "stage0 changes have stage1 mirrors"; run tests/stage0_mirror.sh origin/main
else
  skipped+=("stage0 mirror check (no origin/main to compare with)")
fi

step "every builtin has native code"; run tests/native_builtins.sh
step "the prelude refuses hostile boot information"; run tests/prelude_boot.sh

step "build stage0 (warnings are errors)"
run bash -c 'cd stage0 && RUSTFLAGS="-D warnings" cargo build --release --locked'

if command -v z3 >/dev/null; then
  step "build bastioc1 with stage0"; run tests/bastioc1.sh
  step "tests/run.sh";       run tests/run.sh
  step "tests/native.sh";    run tests/native.sh
  step "stage1/difftest.sh"; run stage1/difftest.sh
  step "tests/fuzz.py (short run)"; run tests/fuzz.py --seed 1 --count 1000
  if [ "${1:-}" = --bench ]; then
    step "bench/run.sh"; run bench/run.sh
  fi
else
  skipped+=("tests/run.sh, tests/native.sh, stage1/difftest.sh, tests/fuzz.py (no z3)")
fi

ts=${TREE_SITTER:-tree-sitter}
if command -v "$ts" >/dev/null; then
  step "tree-sitter grammar"; run editors/tree-sitter-bastio/test/parse_all.sh
else
  skipped+=("tree-sitter grammar (npm install -g tree-sitter-cli@0.25.10)")
fi

if command -v "${MDBOOK:-mdbook}" >/dev/null; then
  step "documentation site"; run site/build.sh
else
  skipped+=("documentation site (no mdbook; see site/README.md)")
fi

if command -v npm >/dev/null; then
  step "VS Code grammar"; run bash -c 'cd editors/vscode && npm ci --silent && npm test'
else
  skipped+=("VS Code grammar (no npm)")
fi

echo
if [ ${#skipped[@]} -gt 0 ]; then
  printf 'SKIPPED: %s\n' "${skipped[@]}"
  echo "ci.sh: the steps that ran passed; the skipped ones were not checked"
else
  echo "ci.sh: every CI check passed"
fi
