// Synchronous TCP for a WebAssembly module in Node.js. A module calls
// `host_net` and waits for the answer, but Node's sockets are asynchronous,
// so they run in a worker thread and the module's thread sleeps on a shared
// buffer until the worker has the answer (Atomics.wait).
//
//   const net = require('./netsync.js').create();
//   const b = await Bastio.load(bytes, { net, ... });
//   ... b.main() ...
//   net.close();
//
// `call(op, a, b, data)` takes the operations of `bst_host_net` in the C
// prelude (0 listen, 1 accept, 2 connect, 3 read at most b bytes, 4 write
// `data`, 5 close, 6 local port) and returns `{ result, data }`; a failure is
// 2^64 - 1. A listen or connect address is in host order.
'use strict';
const { Worker, isMainThread, workerData } = require('worker_threads');

const FAIL = BigInt.asUintN(64, -1n);
const CAP = 1 << 16;                               // the most data one call moves
// ctrl: [0] state (0 idle, 1 request, 2 answer), [1] request data length, [2] answer data length
// args: BigInt64 [0] op, [1] a, [2] b, [3] result

if (isMainThread) {
  module.exports.create = function create() {
    const ctrl = new Int32Array(new SharedArrayBuffer(16));
    const args = new BigInt64Array(new SharedArrayBuffer(32));
    const data = new Uint8Array(new SharedArrayBuffer(CAP));
    const worker = new Worker(__filename, { workerData: { ctrl, args, data } });
    worker.unref();
    const one = (op, a, b, payload) => {
      args[0] = BigInt(op); args[1] = BigInt.asIntN(64, a); args[2] = BigInt.asIntN(64, b);
      ctrl[1] = payload ? payload.length : 0;
      if (payload) data.set(payload);
      Atomics.store(ctrl, 0, 1);
      Atomics.notify(ctrl, 0);
      while (Atomics.load(ctrl, 0) !== 2) Atomics.wait(ctrl, 0, 1);
      const result = BigInt.asUintN(64, args[3]);
      const out = new Uint8Array(ctrl[2]);
      out.set(data.subarray(0, ctrl[2]));
      Atomics.store(ctrl, 0, 0);
      Atomics.notify(ctrl, 0);
      return { result, data: out };
    };
    return {
      call(op, a, b, payload) {
        if (op !== 4 || !payload || payload.length <= CAP) return one(op, a, b, payload);
        let sent = 0n;                              // a long write goes in pieces
        for (let at = 0; at < payload.length; at += CAP) {
          const r = one(op, a, b, payload.subarray(at, at + CAP));
          if (r.result === FAIL) return { result: FAIL, data: new Uint8Array(0) };
          sent += r.result;
        }
        return { result: sent, data: new Uint8Array(0) };
      },
      close() { worker.terminate(); },
    };
  };
} else {
  const net = require('net');
  const { ctrl, args, data } = workerData;
  // A pending Atomics.waitAsync does not keep the thread alive: once every
  // socket is closed the event loop would end and a later request would wait
  // for ever. The timer keeps it running until the module ends the worker.
  setInterval(() => {}, 1 << 30);
  let next = 3n;
  const socks = new Map();                          // id -> { s, buf: [Buffer], ended, wait: fn }
  const servers = new Map();                        // id -> { srv, queue: [id], wait: fn }
  const wake = (o) => { const w = o.wait; o.wait = null; if (w) w(); };
  const track = (s) => {
    const o = { s, buf: [], ended: false, wait: null };
    s.on('data', (c) => { o.buf.push(c); wake(o); });
    s.on('end', () => { o.ended = true; wake(o); });
    s.on('close', () => { o.ended = true; wake(o); });
    s.on('error', () => { o.ended = true; wake(o); });
    const id = next++;
    socks.set(id, o);
    return id;
  };
  const ip = (a) => `${(a >> 24n) & 255n}.${(a >> 16n) & 255n}.${(a >> 8n) & 255n}.${a & 255n}`;
  const reply = (result, out) => {
    const n = out ? Math.min(out.length, CAP) : 0;
    if (n) data.set(out.subarray(0, n));
    ctrl[2] = n;
    args[3] = BigInt.asIntN(64, result);
    Atomics.store(ctrl, 0, 2);
    Atomics.notify(ctrl, 0);
    loop();
  };
  const handle = (op64, a, b, payload) => {
    const op = op64;
    if (op === 0n) {
      const srv = net.createServer();
      const o = { srv, queue: [], wait: null };
      srv.on('connection', (s) => { o.queue.push(track(s)); wake(o); });
      srv.on('error', () => reply(FAIL));
      srv.listen({ host: ip(BigInt.asUintN(32, a)), port: Number(b) }, () => {
        const id = next++;
        servers.set(id, o);
        reply(id);
      });
    } else if (op === 1n) {
      const o = servers.get(a);
      if (!o) return reply(FAIL);
      const go = () => { if (o.queue.length) reply(o.queue.shift()); else o.wait = go; };
      go();
    } else if (op === 2n) {
      const limit = Number(BigInt.asUintN(48, b) >> 16n);   // the bits of the port word above bit 15: a time limit in milliseconds
      const s = net.connect({ host: ip(BigInt.asUintN(32, a)), port: Number(b & 0xffffn) });
      let done = false;
      const timer = limit > 0 ? setTimeout(() => { if (!done) { done = true; s.destroy(); reply(FAIL); } }, Math.min(limit, 3600000)) : null;
      s.once('connect', () => { if (done) return; done = true; if (timer) clearTimeout(timer); const id = track(s); socks.get(id).limit = limit > 0 ? Math.min(limit, 3600000) : 0; reply(id); });
      s.once('error', () => { if (done) return; done = true; if (timer) clearTimeout(timer); reply(FAIL); });
    } else if (op === 3n) {
      const o = socks.get(a);
      if (!o) return reply(FAIL);
      const want = Number(b < BigInt(CAP) ? b : BigInt(CAP));
      const go = () => {
        if (o.buf.length) {
          const all = Buffer.concat(o.buf);
          const n = Math.min(all.length, want);
          o.buf = n < all.length ? [all.subarray(n)] : [];
          reply(BigInt(n), all.subarray(0, n));
        } else if (o.ended || want === 0) reply(0n, new Uint8Array(0));
        else {
          let t = null;
          const resume = () => { if (t) clearTimeout(t); go(); };
          o.wait = resume;
          if (o.limit > 0) {                        // a read that has waited for the socket's time limit fails
            t = setTimeout(() => { if (o.wait === resume) { o.wait = null; reply(FAIL); } }, o.limit);
          }
        }
      };
      go();
    } else if (op === 4n) {
      const o = socks.get(a);
      if (!o || o.s.destroyed) return reply(FAIL);
      const copy = Buffer.from(payload);
      o.s.write(copy, (e) => reply(e ? FAIL : BigInt(copy.length)));
    } else if (op === 5n) {
      const o = socks.get(a);
      if (o) { socks.delete(a); o.s.destroy(); return reply(0n); }
      const sv = servers.get(a);
      if (sv) { servers.delete(a); sv.srv.close(); return reply(0n); }
      reply(FAIL);
    } else if (op === 6n) {
      const sv = servers.get(a);
      const addr = sv ? sv.srv.address() : (socks.get(a) ? { port: socks.get(a).s.localPort } : null);
      reply(addr && addr.port ? BigInt(addr.port) : FAIL);
    } else {
      reply(FAIL);
    }
  };
  // Waits until the module's thread has taken the last answer and asked
  // for the next thing.
  const loop = () => {
    const st = Atomics.load(ctrl, 0);
    if (st === 1) {
      const payload = ctrl[1] ? data.slice(0, ctrl[1]) : null;
      handle(args[0], BigInt.asUintN(64, args[1]), BigInt.asUintN(64, args[2]), payload);
      return;
    }
    const w = Atomics.waitAsync(ctrl, 0, st);
    if (w.async) w.value.then(loop); else setImmediate(loop);
  };
  loop();
}
