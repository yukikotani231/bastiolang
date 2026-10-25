// The JavaScript shell of a module written by `bastioc1 --arch wasm32`: the
// imports the module expects (printing, the host routines) and a few helpers
// to pass bytes in and out. Works in a browser and in Node.js.
//
//   const b = await Bastio.load(bytes, { args: ['x'], files: new Map(), env: {},
//                                        stdout: (u8) => ..., stderr: (u8) => ... });
//   const code = b.main();            // a program's exit code
//   b.exports.f(...)                  // an `export fn`; words are BigInts
//   b.put(u8) -> [address, length]    // a `&Bytes` argument
//   b.bytesAt(address)                // a `Bytes` stored as (length, address)
(function (root) {
  'use strict';
  const enc = new TextEncoder();
  const dec = new TextDecoder();

  async function load(source, opts) {
    opts = opts || {};
    const args = opts.args || [];
    const files = opts.files || new Map();
    const env = opts.env || {};
    const fsFallback = opts.fs || null;            // Node's `fs`, to reach real files
    const out = [];
    const flush = () => {
      if (out.length) { (opts.stdout || (() => {}))(Uint8Array.from(out)); out.length = 0; }
    };
    let inst = null;
    const mem = () => new Uint8Array(inst.exports.memory.buffer);
    const view = () => new DataView(inst.exports.memory.buffer);
    const word = (a, i) => view().getBigUint64(Number(a) + 8 * i, true);
    const setWord = (a, i, v) => view().setBigUint64(Number(a) + 8 * i, BigInt.asUintN(64, v), true);
    const bytes = (p, n) => mem().slice(Number(p), Number(p) + Number(n));
    const store = (u8) => {                         // a fresh buffer holding u8: [address, length]
      const p = inst.exports.bst_alloc(BigInt(Math.max(u8.length, 1)));
      mem().set(u8, Number(p));
      return [p, BigInt(u8.length)];
    };
    // A `Bytes` cell is (length, address); the old data is freed.
    const fill = (cell, u8) => {
      const [p, n] = store(u8);
      inst.exports.bst_free(word(cell, 1));
      setWord(cell, 0, n);
      setWord(cell, 1, p);
    };
    const path = (b) => dec.decode(bytes(word(b, 0), word(b, 1)));
    const host = (call, b) => {
      switch (Number(call)) {
        case 20: {                                   // argument number word 0
          const i = Number(word(b, 0));
          const [p, n] = store(i < args.length ? enc.encode(args[i]) : new Uint8Array(0));
          setWord(b, 0, p); setWord(b, 1, n);
          return p;
        }
        case 21: {                                   // read the file named by words 0, 1 into the cell at word 2
          const name = path(b);
          let data = files.get(name);
          if (data === undefined && fsFallback) {
            try { data = fsFallback.readFileSync(name); } catch (_) { data = undefined; }
          }
          if (data === undefined) return 0n;
          fill(word(b, 2), data);
          return 1n;
        }
        case 22: {                                   // write words 2, 3 to the file named by words 0, 1
          const name = path(b);
          const data = bytes(word(b, 2), word(b, 3));
          if (fsFallback) {
            try { fsFallback.writeFileSync(name, data); } catch (_) { return 0n; }
          } else {
            files.set(name, data);
          }
          return 1n;
        }
        case 23: {                                   // environment variable named by words 0, 1 into the cell at word 2
          const v = env[path(b)];
          if (v === undefined) return 0n;
          fill(word(b, 2), enc.encode(String(v)));
          return 1n;
        }
        case 24:
          return BigInt((typeof navigator !== 'undefined' && navigator.hardwareConcurrency) || 1);
        case 25:                                     // standard error, after standard output
          flush();
          (opts.stderr || (() => {}))(bytes(word(b, 0), word(b, 1)));
          return 0n;
        case 31:
          return BigInt(args.length);
        default:
          throw new Error('bastio: unsupported host routine ' + call);
      }
    };
    const imports = {
      bastio: {
        print: (x) => { for (const c of enc.encode(BigInt.asUintN(64, x).toString() + '\n')) out.push(c); if (out.length > 1 << 16) flush(); },
        putc: (c) => { out.push(Number(BigInt.asUintN(8, c))); if (out.length > 1 << 16) flush(); },
        host,
      },
    };
    const m = await WebAssembly.instantiate(source, imports);
    inst = m.instance;
    return {
      exports: inst.exports,
      flush,
      main() {
        try {
          const r = inst.exports.main();
          return r === undefined ? 0 : Number(BigInt.asUintN(8, r));
        } finally { flush(); }
      },
      put: store,
      bytesAt(a) { return bytes(word(a, 1), word(a, 0)); },
      word,
    };
  }
  
  const api = { load };
  if (typeof module !== 'undefined' && module.exports) module.exports = api; else root.Bastio = api;
})(typeof globalThis !== 'undefined' ? globalThis : this);
