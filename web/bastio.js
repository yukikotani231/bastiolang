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
//   b.alloc(words) -> address         // a zeroed result slot for a struct returned through a pointer
//   b.record(address, ['tag', 'end']) // the words at address as { tag: 0n, end: 12n }; a name
//                                     // may be a [name, 'bytes'] pair to read a `Bytes` cell
//                                     // (it takes two words)
//
// Child processes (`host_spawn`) need an embedder that can run programs:
// pass `childProcess: require('child_process')` in Node.js, or your own
// `spawn(argv, input) -> { status, stdout }`. Without either, as in a browser,
// every program fails to start (exit status 127, no output). The program runs
// when its output is first read or it is waited for, with everything written
// to it until then as its input; a program that must see its input before it
// answers cannot be driven from the module.
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
    const SIGNALS = { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGABRT: 6, SIGKILL: 9, SIGSEGV: 11, SIGPIPE: 13, SIGALRM: 14, SIGTERM: 15 };
    const spawn = opts.spawn || (opts.childProcess && ((argv, input) => {
      const r = opts.childProcess.spawnSync(argv[0], argv.slice(1), { input, stdio: ['pipe', 'pipe', 'inherit'], maxBuffer: 1 << 30 });
      if (r.error) return { status: 127, stdout: new Uint8Array(0) };
      return { status: r.status !== null ? r.status : 128 + (SIGNALS[r.signal] || 0), stdout: r.stdout };
    })) || null;
    const kids = new Map();                          // a Child's words are (pid, input, output); the pid keys this table
    let nextKid = 1n;
    const NONE = BigInt.asUintN(64, -1n);
    const run = (kid) => {
      if (kid.ran) return;
      kid.ran = true;
      const input = new Uint8Array(kid.input.reduce((n, c) => n + c.length, 0));
      let at = 0;
      for (const c of kid.input) { input.set(c, at); at += c.length; }
      const r = spawn(kid.argv, input);
      kid.status = r.status;
      kid.out = r.stdout;
    };
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
        case 26: {                                   // start argv: data, capacity, length in words 0 to 2
          const data = word(b, 0), len = Number(word(b, 2));
          const argv = [];
          for (let i = 0; i < len; i++) {
            const cell = data + BigInt(16 * i);
            argv.push(dec.decode(bytes(word(cell, 1), word(cell, 0))));
          }
          if (!spawn || argv.length === 0) {
            setWord(b, 0, 0n); setWord(b, 1, NONE); setWord(b, 2, NONE);
            return 0n;
          }
          const id = nextKid++;
          kids.set(id, { argv, input: [], ran: false, out: new Uint8Array(0), pos: 0, status: 127 });
          setWord(b, 0, id); setWord(b, 1, id); setWord(b, 2, id);   // output, input, pid
          return 0n;
        }
        case 27: {                                   // write words 0, 1 to the Child at word 2
          const ch = word(b, 2);
          const kid = kids.get(word(ch, 0));
          if (!kid || kid.ran || word(ch, 1) === NONE) return 0n;
          kid.input.push(bytes(word(b, 0), word(b, 1)));
          return 1n;
        }
        case 28:                                     // close the input of the Child at word 0
          setWord(word(b, 0), 1, NONE);
          return 0n;
        case 29: {                                   // the next line of the Child at word 0 into the cell at word 1
          const ch = word(b, 0);
          const kid = kids.get(word(ch, 0));
          if (!kid || word(ch, 2) === 0n) return 0n;
          run(kid);
          if (kid.pos >= kid.out.length) return 0n;
          let end = kid.out.indexOf(10, kid.pos);
          if (end < 0) end = kid.out.length;
          fill(word(b, 1), kid.out.subarray(kid.pos, end));
          kid.pos = end + 1;
          return 1n;
        }
        case 30: {                                   // wait for the Child (output, input, pid in words 0 to 2)
          const kid = kids.get(word(b, 2));
          if (!kid) return 127n;
          run(kid);
          kids.delete(word(b, 2));
          return BigInt(kid.status);
        }
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
      alloc(n) {
        const p = inst.exports.bst_alloc(BigInt(8 * n));
        mem().fill(0, Number(p), Number(p) + 8 * n);
        return p;
      },
      record(a, fields) {
        const r = {};
        let i = 0;
        for (const f of fields) {
          if (Array.isArray(f)) { r[f[0]] = bytes(word(a, i + 1), word(a, i)); i += 2; }
          else r[f] = word(a, i++);
        }
        return r;
      },
    };
  }
  
  const api = { load };
  if (typeof module !== 'undefined' && module.exports) module.exports = api; else root.Bastio = api;
})(typeof globalThis !== 'undefined' ? globalThis : this);
