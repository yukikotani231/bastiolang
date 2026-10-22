#!/usr/bin/env bash
# The developer tools in tools/, written in Bastio and built with bastioc1.
#
# bastiofmt (tools/bastiofmt):
# - formats test/ugly.in into test/ugly.expected;
# - formats every Bastio source in the repository twice: the second pass must
#   change nothing (idempotence), and the output must lex to the tokens of the
#   input (bastiofmt refuses to print anything else, exit status 3);
# - the sources of the tools themselves are formatted (--check).
#
# bastio-lsp (tools/bastio-lsp), the language server:
# - test/lsp_test.py drives it over standard input and output: hover,
#   go to definition (in the file and in an imported one), references,
#   highlights, rename, completion, signature help, document symbols, workspace symbols,
#   formatting, the compiler's diagnostics with their ranges, lexical errors as
#   the text changes, malformed input;
# - editors/vim/test/nvim_test.lua does the same from headless Neovim, through
#   the plugin in editors/vim (skipped when there is no nvim).
#
# usage: tests/tools.sh
set -euo pipefail
cd "$(dirname "$0")/.."
command -v z3 >/dev/null || { echo "tools: SKIPPED (no z3)"; exit 0; }
BASTIOC=$(tests/bastioc1.sh) || exit 1
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail=0
bad() { echo "FAIL $*"; fail=1; }

"$BASTIOC" -o "$tmp/bastiofmt" tools/bastiofmt/bastiofmt.bastio 2>"$tmp/err" \
  || { cat "$tmp/err"; exit 1; }
fmt="$tmp/bastiofmt"

"$fmt" tools/bastiofmt/test/ugly.in >"$tmp/ugly.out" \
  || bad "bastiofmt ugly.in: exit $?"
diff -u tools/bastiofmt/test/ugly.expected "$tmp/ugly.out" || bad "ugly.in is not formatted as expected"

n=0
while IFS= read -r f; do
  n=$((n + 1))
  rc=0; "$fmt" "$f" >"$tmp/once.bastio" 2>"$tmp/err" || rc=$?
  if [ "$rc" = 2 ]; then continue; fi   # does not lex: the lexer tests' damaged inputs
  if [ "$rc" != 0 ]; then bad "$f: bastiofmt exit $rc: $(cat "$tmp/err")"; continue; fi
  "$fmt" "$tmp/once.bastio" >"$tmp/twice.bastio" 2>"$tmp/err" \
    || { bad "$f: the formatted text does not format: $(cat "$tmp/err")"; continue; }
  cmp -s "$tmp/once.bastio" "$tmp/twice.bastio" || bad "$f: formatting twice changes the text"
done < <(git ls-files '*.bastio' | grep -v '^tools/' ; find tools -name '*.bastio' | sort)
echo "bastiofmt: $n sources formatted twice"

# The tools' own sources are formatted.
"$fmt" --check $(find tools -name '*.bastio' | sort) || bad "tools/ has sources that bastiofmt would change"

"$BASTIOC" -o "$tmp/bastio-lsp" tools/bastio-lsp/lsp.bastio 2>"$tmp/err" \
  || { cat "$tmp/err"; exit 1; }
python3 tools/bastio-lsp/test/lsp_test.py "$tmp/bastio-lsp" "$BASTIOC" || bad "tools/bastio-lsp/test/lsp_test.py"
echo "bastio-lsp: protocol tests done"

if command -v nvim >/dev/null; then
  BASTIO_TEST_DIR="$PWD/tools/bastio-lsp/test" BASTIO_LSP="$tmp/bastio-lsp" BASTIOC="$BASTIOC" \
    timeout 300 nvim --headless -u NONE -i NONE --cmd "set rtp^=$PWD/editors/vim" \
    --cmd 'filetype plugin indent on' --cmd 'syntax on' \
    -c 'luafile editors/vim/test/nvim_test.lua' || bad "editors/vim/test/nvim_test.lua"
  echo "bastio-lsp: Neovim test done"
else
  echo "bastio-lsp: Neovim test SKIPPED (no nvim)"
fi

if [ "$fail" = 0 ]; then echo "tools: ok"; else exit 1; fi
