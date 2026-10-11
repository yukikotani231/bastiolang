#!/usr/bin/env bash
# Runs, on this machine, every check the CI workflow runs, in the same order,
# and stops at the first failure. Run it before every push: a failure here
# costs seconds, the same failure in CI costs a run and a notification.
#
# Steps whose tools are missing (z3, the tree-sitter CLI, mdbook, npm, an
# AArch64 cross compiler and QEMU) are reported as SKIPPED, not
# passed; CI still runs them.
#
# usage: tests/ci.sh [--bench]    (--bench also runs bench/run.sh)
set -euo pipefail
cd "$(dirname "$0")/.."

skipped=()
step() { printf '\n== %s\n' "$1"; }

if git rev-parse -q --verify origin/main >/dev/null; then
  step "stage0 changes have stage1 mirrors"; tests/stage0_mirror.sh origin/main
else
  skipped+=("stage0 mirror check (no origin/main to compare with)")
fi

step "every builtin has native code"; tests/native_builtins.sh
step "the prelude refuses hostile boot information"; tests/prelude_boot.sh

step "build stage0 (warnings are errors)"
(cd stage0 && RUSTFLAGS="-D warnings" cargo build --release --locked)

if command -v z3 >/dev/null; then
  step "build bastioc1 with stage0"; tests/bastioc1.sh
  step "tests/run.sh";       tests/run.sh
  step "tests/native.sh";    tests/native.sh
  step "stage1/difftest.sh"; stage1/difftest.sh
  step "tests/fuzz.py (short run)"; tests/fuzz.py --seed 1 --count 1000
  if command -v "${AARCH64_CC:-aarch64-linux-gnu-gcc}" >/dev/null \
     && command -v qemu-system-aarch64 >/dev/null \
     && { [ "$(uname -m)" = aarch64 ] || command -v qemu-aarch64 >/dev/null; }; then
    step "tests/aarch64.sh"; tests/aarch64.sh
  else
    skipped+=("tests/aarch64.sh (needs aarch64-linux-gnu-gcc, qemu-aarch64 and qemu-system-aarch64: apt install gcc-aarch64-linux-gnu qemu-user qemu-system-arm)")
  fi
  if [ "${1:-}" = --bench ]; then
    step "bench/run.sh"; bench/run.sh
  fi
else
  skipped+=("tests/run.sh, tests/native.sh, stage1/difftest.sh, tests/fuzz.py (no z3)")
fi

ts=${TREE_SITTER:-tree-sitter}
if command -v "$ts" >/dev/null; then
  step "tree-sitter grammar"; editors/tree-sitter-bastio/test/parse_all.sh
else
  skipped+=("tree-sitter grammar (npm install -g tree-sitter-cli@0.25.10)")
fi

if command -v "${MDBOOK:-mdbook}" >/dev/null; then
  step "documentation site"; site/build.sh
else
  skipped+=("documentation site (no mdbook; see site/README.md)")
fi

if command -v npm >/dev/null; then
  step "VS Code grammar"; (cd editors/vscode && npm ci --silent && npm test)
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
