// Calls the proved example libraries (examples/{lines,base64,utf8}) from
// JavaScript: modules written by `bastioc1 --arch wasm32`, one per library.
// usage: node tests/wasm_lib.js lines.wasm base64.wasm utf8.wasm
'use strict';
const fs = require('fs');
const assert = require('assert');
const crypto = require('crypto');

async function load(path) {
  const imports = { bastio: { print: () => {}, putc: () => {} } };
  const { instance } = await WebAssembly.instantiate(fs.readFileSync(path), imports);
  const e = instance.exports;
  const u8 = () => new Uint8Array(e.memory.buffer);
  const dv = () => new DataView(e.memory.buffer);
  return {
    e,
    // A copy of `bytes` in the module's memory: [address, length] as the words of a `&Bytes`.
    put(bytes) {
      const p = e.bst_alloc(BigInt(Math.max(bytes.length, 1)));
      u8().set(bytes, Number(p));
      return [p, BigInt(bytes.length)];
    },
    word(addr, i) { return dv().getBigUint64(Number(addr) + 8 * i, true); },
    // The bytes of a `Bytes` stored as (length, address) at `addr`.
    bytesAt(addr) {
      const len = Number(this.word(addr, 0)), at = Number(this.word(addr, 1));
      return Buffer.from(u8().slice(at, at + len));
    },
  };
}

(async () => {
  const [linesPath, base64Path, utf8Path] = process.argv.slice(2);

  const L = await load(linesPath);
  const text = Buffer.from('one\ntwo\nthree\n\nx');
  const [lp, ll] = L.put(text);
  assert.strictEqual(L.e.count_byte(lp, ll, 10n), 4n);
  assert.deepStrictEqual(L.e.line_len(lp, ll, 0n), [0n, 3n]);          // Line::Len(3)
  assert.deepStrictEqual(L.e.line_len(lp, ll, 4n), [0n, 3n]);
  assert.deepStrictEqual(L.e.line_len(lp, ll, 15n), [0n, 1n]);         // the last line has no '\n'
  assert.strictEqual(L.e.line_len(lp, ll, 99n)[0], 1n);                // Line::OutOfRange

  const B = await load(base64Path);
  const inputs = [Buffer.alloc(0), Buffer.from('a'), Buffer.from('ab'), Buffer.from('abc'),
                  Buffer.from('hello world'), crypto.randomBytes(1000), crypto.randomBytes(4097)];
  for (const raw of inputs) {
    const [sp, sl] = B.put(raw);
    const res = B.e.bst_alloc(32n);
    B.e.base64_encode(sp, sl, res);
    assert.strictEqual(B.word(res, 0), 0n);                            // Encoded::Ok
    const enc = B.bytesAt(res + 8n);
    assert.strictEqual(enc.toString(), raw.toString('base64'));
    const [ep, el] = B.put(enc);
    const dres = B.e.bst_alloc(32n);
    B.e.base64_decode(ep, el, dres);
    assert.strictEqual(B.word(dres, 0), 0n);                           // Decoded::Ok
    assert.ok(B.bytesAt(dres + 8n).equals(raw));
    B.e.bst_free(B.word(res, 2));                                      // the encoder's buffer
    B.e.bst_free(B.word(dres, 2));
  }
  const [bp, bl] = B.put(Buffer.from('abc'));                          // not a multiple of 4
  const bres = B.e.bst_alloc(32n);
  B.e.base64_decode(bp, bl, bres);
  assert.strictEqual(B.word(bres, 0), 1n);                             // Decoded::Err

  const U = await load(utf8Path);
  const cases = [Buffer.from(''), Buffer.from('plain ascii'), Buffer.from('héllo wörld ✓ 𝄞'),
                 Buffer.from([0xff]), Buffer.from([0xc0, 0x80]), Buffer.from([0xed, 0xa0, 0x80]),
                 Buffer.from([0xf4, 0x90, 0x80, 0x80]), Buffer.from('oké').subarray(0, 3)];
  for (const raw of cases) {
    const [p, n] = U.put(raw);
    const [tag, val] = U.e.utf8_validate(p, n);
    let want;
    try {
      const s = new TextDecoder('utf-8', { fatal: true }).decode(raw);
      want = [0n, BigInt([...s].length)];
    } catch (_) { want = null; }
    if (want) assert.deepStrictEqual([tag, val], want, `valid ${raw.toString('hex')}`);
    else assert.strictEqual(tag, 1n, `invalid ${raw.toString('hex')}`);   // Utf8::Invalid
  }
  console.log('wasm libraries: lines, base64 and utf8 pass from JavaScript');
})().catch((e) => { console.error(e); process.exit(1); });
