#!/usr/bin/env bash
# Builds the demos' modules: the proved base64 library of examples/base64 and the HTTP parser of examples/http as
# WebAssembly. Serve this directory (python3 -m http.server) and open
# index.html. BASTIOC overrides the compiler (default: stage0's bastioc1 build).
set -eu
cd "$(dirname "$0")/.."
BASTIOC=${BASTIOC:-$(tests/bastioc1.sh)}
"$BASTIOC" --arch wasm32 -o web/base64.wasm examples/base64/lib.bastio
"$BASTIOC" --arch wasm32 -o web/http.wasm examples/http/http.bastio
