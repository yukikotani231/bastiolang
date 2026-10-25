#!/usr/bin/env bash
# The example libraries (`export fn`, no `main`) built as wasm modules and
# called from JavaScript under Node.js (tests/wasm_lib.js). Needs node.
set -eu
cd "$(dirname "$0")/.."
if ! command -v node >/dev/null; then echo "wasm_lib: node is missing, SKIPPED"; exit 0; fi
BASTIOC=${BASTIOC:-$(tests/bastioc1.sh)}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
"$BASTIOC" --arch wasm32 -o "$tmp/lines.wasm" examples/lines/lines.bastio >/dev/null
"$BASTIOC" --arch wasm32 -o "$tmp/base64.wasm" examples/base64/lib.bastio >/dev/null
"$BASTIOC" --arch wasm32 -o "$tmp/utf8.wasm" examples/utf8/utf8.bastio >/dev/null
"$BASTIOC" --arch wasm32 -o "$tmp/http.wasm" examples/http/http.bastio >/dev/null
node tests/wasm_lib.js "$tmp/lines.wasm" "$tmp/base64.wasm" "$tmp/utf8.wasm" "$tmp/http.wasm"
