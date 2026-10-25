// Runs a module that `bastioc1 --arch wasm32` wrote: its `main`, with `print`
// and `putc` writing to the standard output. Exits with main's result (the
// low byte), or 99 after a trap, saying what it was on the standard error.
// usage: node tests/wasm_run.js prog.wasm
'use strict';
const fs = require('fs');
const chunks = [];
const put = (s) => chunks.push(Buffer.from(s));
const imports = {
  bastio: {
    print: (x) => put(BigInt.asUintN(64, x).toString() + '\n'),
    putc: (c) => put(Uint8Array.of(Number(BigInt.asUintN(8, c)))),
  },
};
(async () => {
  let code = 0;
  try {
    const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), imports);
    const r = instance.exports.main();
    code = Number(BigInt.asUintN(8, r === undefined ? 0n : r));
  } catch (e) {
    process.stderr.write(String(e) + '\n');
    code = 99;
  }
  fs.writeSync(1, Buffer.concat(chunks));
  process.exitCode = code;
})();
