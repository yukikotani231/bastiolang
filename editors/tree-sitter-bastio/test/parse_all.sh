#!/usr/bin/env bash
# Generates the parser, runs the corpus tests, then parses every .bastio file
# in this repository (and in any extra directories given as arguments, such as
# a BastiOS checkout) and fails if any of them has a syntax error in the tree.
#
# Needs the tree-sitter CLI (`npm install -g tree-sitter-cli`, or set
# TREE_SITTER) and a C compiler.
#
# usage: editors/tree-sitter-bastio/test/parse_all.sh [extra-dir ...]
set -euo pipefail

here=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$here/../.." && pwd)
ts=${TREE_SITTER:-tree-sitter}

cd "$here"
"$ts" generate
"$ts" test

list=$(mktemp)
trap 'rm -f "$list"' EXIT
# stage1/lexcases holds token streams for the lexer tests, not programs; some
# are deliberately malformed. So are tests/fail/parse_*, which test the
# compiler's syntax errors, and tests/diag/parse_*, which test how they are
# shown.
find "$repo" "$@" -name '*.bastio' -not -path '*/stage1/lexcases/*' -not -path '*/editors/*' \
  -not -path '*/tests/fail/parse_*' -not -path '*/tests/diag/parse_*' | sort > "$list"
n=$(wc -l < "$list")
if "$ts" parse --quiet --paths "$list"; then
  echo "parse_all: all $n files parse without errors"
else
  echo "parse_all: the files listed above have syntax errors in the tree" >&2
  exit 1
fi
