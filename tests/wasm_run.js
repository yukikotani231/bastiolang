// Runs a module that `bastioc1 --arch wasm32` wrote: its `main`, with the
// standard output and error of this process and Node's files behind the host
// routines (web/bastio.js). Exits with main's result (the low byte), or 99
// after a trap, saying what it was on the standard error.
// usage: node tests/wasm_run.js prog.wasm [args...]
'use strict';
const fs = require('fs');
const Bastio = require('../web/bastio.js');
const net = require('../web/netsync.js').create();
(async () => {
  let code = 0;
  const chunks = [];
  try {
    const b = await Bastio.load(fs.readFileSync(process.argv[2]), {
      args: [process.argv[2], ...process.argv.slice(3)],
      env: process.env,
      fs,
      childProcess: require('child_process'),
      net,
      stdout: (u8) => chunks.push(Buffer.from(u8)),
      stderr: (u8) => { fs.writeSync(1, Buffer.concat(chunks)); chunks.length = 0; fs.writeSync(2, Buffer.from(u8)); },
    });
    code = b.main();
  } catch (e) {
    process.stderr.write(String(e) + '\n');
    code = 99;
  }
  fs.writeSync(1, Buffer.concat(chunks));
  net.close();
  process.exitCode = code;
})();
