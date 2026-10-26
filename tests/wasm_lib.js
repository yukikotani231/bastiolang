// Calls the proved example libraries (examples/{lines,base64,utf8,http}) from
// JavaScript: modules written by `bastioc1 --arch wasm32`, one per library.
// usage: node tests/wasm_lib.js lines.wasm base64.wasm utf8.wasm http.wasm
'use strict';
const fs = require('fs');
const assert = require('assert');
const crypto = require('crypto');

async function load(path) {
  const imports = { bastio: { print: () => {}, putc: () => {}, host: () => 0n } };
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
  const [linesPath, base64Path, utf8Path, httpPath, childPath] = process.argv.slice(2);

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
  const H = await load(httpPath);
  const head = Buffer.from('GET /a?b=1 HTTP/1.1\r\nHost: example.org\r\nX: y\r\n\r\nbody');
  const [hp, hl] = H.put(head);
  const hres = H.e.bst_alloc(80n);
  H.e.http_parse_head(hp, hl, hres);
  assert.strictEqual(H.word(hres, 0), 0n);                             // Head::Ok
  // method_end, target, target_end, minor, fields, nfields, head_end
  assert.deepStrictEqual([1, 2, 3, 4, 5, 6, 7].map((i) => H.word(hres, i)), [3n, 4n, 10n, 1n, 21n, 2n, BigInt(head.length - 4)]);
  const [pp, pl] = H.put(head.subarray(0, 25));
  H.e.http_parse_head(pp, pl, hres);
  assert.strictEqual(H.word(hres, 0), 1n);                             // Head::Partial
  const [bp2, bl2] = H.put(Buffer.from('GET / HTTP/1.1\nHost: a\n\n'));
  H.e.http_parse_head(bp2, bl2, hres);
  assert.strictEqual(H.word(hres, 0), 2n);                             // Head::Bad: bare LF

  // The same head through the shell's typed helpers (web/bastio.js).
  const S = await require('../web/bastio.js').load(fs.readFileSync(httpPath));
  const [sp, sl] = S.put(head);
  const sres = S.alloc(10);
  S.exports.http_parse_head(sp, sl, sres);
  assert.deepStrictEqual(S.record(sres, ['tag', 'method_end', 'target', 'target_end', 'minor', 'fields', 'nfields', 'head_end']),
    { tag: 0n, method_end: 3n, target: 4n, target_end: 10n, minor: 1n, fields: 21n, nfields: 2n, head_end: BigInt(head.length - 4) });

  const S2 = await require('../web/bastio.js').load(fs.readFileSync(base64Path));
  const [ip, il] = S2.put(Buffer.from('hello'));
  const eres = S2.alloc(4);
  S2.exports.base64_encode(ip, il, eres);
  const er = S2.record(eres, ['tag', ['text', 'bytes']]);
  assert.strictEqual(er.tag, 0n);
  assert.strictEqual(Buffer.from(er.text).toString(), 'aGVsbG8=');

  // web/http.js: the head, its fields and the body framing as one object.
  const parsed = require('../web/http.js').parse(S, new Uint8Array(Buffer.from('POST /p HTTP/1.1\r\nHost: h\r\nContent-Length: 3\r\n\r\nabc')));
  assert.deepStrictEqual(parsed, { status: 'ok', method: 'POST', target: '/p', version: '1.1',
    fields: [['Host', 'h'], ['Content-Length', '3']], body: 'length', bodyLength: 3, bodyStart: 48 });
  assert.strictEqual(require('../web/http.js').parse(S, new Uint8Array(Buffer.from('GET / HTTP/1.1\r\nHo'))).status, 'partial');
  assert.strictEqual(require('../web/http.js').parse(S, new Uint8Array(Buffer.from('GET / HTTP/1.1\nHost: a\n\n'))).status, 'bad');
  assert.strictEqual(require('../web/http.js').parse(S, new Uint8Array(Buffer.from('POST / HTTP/1.1\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n'))).body, 'bad');

  const resp = Buffer.from('HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found');
  const [rp, rl] = H.put(resp);
  H.e.http_parse_response(rp, rl, hres);
  assert.strictEqual(H.word(hres, 0), 0n);                             // RespHead::Ok
  // minor, status, reason, reason_end, fields, nfields, head_end
  assert.deepStrictEqual([1, 2, 3, 4, 5, 6, 7].map((i) => H.word(hres, i)), [1n, 404n, 13n, 22n, 24n, 1n, BigInt(resp.length - 9)]);

  const chunks = Buffer.from('5\r\nhello\r\n0\r\n\r\n');
  const [cp, cl] = H.put(chunks);
  H.e.http_chunk(cp, cl, 0n, hres);
  assert.strictEqual(H.word(hres, 0), 0n);                             // Chunk::Ok
  assert.deepStrictEqual([1, 2, 3].map((i) => H.word(hres, i)), [3n, 5n, 10n]);  // data, size, next
  H.e.http_chunk(cp, cl, 10n, hres);
  assert.strictEqual(H.word(hres, 0), 1n);                             // Chunk::Last
  assert.strictEqual(H.word(hres, 1), BigInt(chunks.length));

  const framed = Buffer.from('POST / HTTP/1.1\r\nContent-Length: 11\r\n\r\n');
  const [fp, fl] = H.put(framed);
  assert.deepStrictEqual(H.e.http_body(fp, fl, 17n), [1n, 11n]);       // Body::Length(11)
  const [xp, xl] = H.put(Buffer.from('POST / HTTP/1.1\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n'));
  assert.strictEqual(H.e.http_body(xp, xl, 17n)[0], 3n);              // Body::Bad: both at once

  // Child processes: with no embedder to run programs (as in a browser) every
  // program fails to start (status 127, no output); with Node's child_process
  // the program of tests/pass/child.bastio runs and exits with 42.
  const Bastio = require('../web/bastio.js');
  const childWasm = fs.readFileSync(childPath);
  const without = await Bastio.load(childWasm, {});
  assert.notStrictEqual(without.main(), 42);
  const withProcs = await Bastio.load(childWasm, { childProcess: require('child_process') });
  assert.strictEqual(withProcs.main(), 42);

  console.log('wasm libraries: lines, base64, utf8 and http pass from JavaScript');
})().catch((e) => { console.error(e); process.exit(1); });
