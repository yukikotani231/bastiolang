#!/usr/bin/env bash
# One-command setup for working on Bastio: installs the tools the build and the
# tests need, builds the bootstrap compiler (stage0) and then bastioc1, and
# runs the hello-world example to show that the toolchain works.
#
# usage: tools/setup-dev.sh [--no-install]
#   --no-install   do not touch the system; only check for the tools and build
#
# Supported: Debian and Ubuntu (apt). On another system install z3, a C
# compiler, make, git, Python 3 and rustup yourself (README.md lists the tested
# versions) and run this with --no-install.
set -euo pipefail
cd "$(dirname "$0")/.."

install=1
[ "${1:-}" = "--no-install" ] && install=0

as_root() { if [ "$(id -u)" = 0 ]; then "$@"; else sudo "$@"; fi; }

if [ "$install" = 1 ]; then
  if ! command -v apt-get >/dev/null; then
    echo "setup-dev: apt-get not found; install the tools by hand and use --no-install" >&2
    exit 1
  fi
  echo "== installing packages (apt)"
  as_root apt-get update -q
  as_root apt-get install -y -q z3 build-essential python3 git curl ca-certificates
fi

if ! command -v cargo >/dev/null && [ -x "$HOME/.cargo/bin/cargo" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v cargo >/dev/null; then
  if [ "$install" = 0 ]; then
    echo "setup-dev: cargo not found; install rustup from https://rustup.rs" >&2
    exit 1
  fi
  echo "== installing rustup"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
  export PATH="$HOME/.cargo/bin:$PATH"
fi

missing=0
for tool in z3 cc python3 git; do
  command -v "$tool" >/dev/null || { echo "setup-dev: missing tool: $tool" >&2; missing=1; }
done
[ "$missing" = 0 ] || exit 1

echo "== building stage0 (rustup installs the pinned Rust release on first use)"
(cd stage0 && cargo build --release --locked)

echo "== building bastioc1 with stage0 (about half a minute)"
tests/bastioc1.sh

echo "== hello world"
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
stage0/target/release/bastioc1 -o "$out/hello" examples/hello.bastio
"$out/hello"

cat <<'MSG'

Done. Next:
  tests/run.sh      must-pass and must-fail programs
  tests/ci.sh       every check CI runs (run it before each push)
  CONTRIBUTING.md   the workflow
  docs/architecture.md   a map of the compiler for newcomers
MSG
