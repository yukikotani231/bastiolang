// The proved HTTP/1.1 request parser of examples/http, driven from JavaScript:
// `Bastio.http(b, bytes)` takes a module built from examples/http/http.bastio
// (loaded with Bastio.load) and returns the parsed head. Every offset comes
// from the module; this file only turns them into strings.
(function (root) {
  'use strict';
  const dec = new TextDecoder();

  function parse(b, bytes) {
    const [p, n] = b.put(bytes);
    const cell = b.alloc(10);
    try {
      b.exports.http_parse_head(p, n, cell);
      const tags = ['ok', 'partial', 'bad'];
      const tag = Number(b.word(cell, 0));
      if (tag !== 0) return { status: tags[tag], at: tag === 2 ? Number(b.word(cell, 1)) : null };
      const r = b.record(cell, ['tag', 'method_end', 'target', 'target_end', 'minor', 'fields', 'nfields', 'head_end']);
      const text = (a, e) => dec.decode(bytes.subarray(Number(a), Number(e)));
      const fields = [];
      const f = b.alloc(6);
      try {
        for (let at = r.fields; ;) {
          b.exports.http_field(p, n, at, f);
          const t = b.word(f, 0);
          if (t !== 0n) break;                           // Next::End: the empty line
          const x = b.record(f, ['tag', 'name_end', 'value', 'value_end', 'next']);
          fields.push([text(at, x.name_end), text(x.value, x.value_end)]);
          at = x.next;
        }
      } finally { b.exports.bst_free(f); }
      // http_body takes the head's fields and says how the body is delimited.
      const body = b.exports.http_body(p, n, r.fields);
      const kinds = ['none', 'length', 'chunked', 'bad'];
      return {
        status: 'ok',
        method: text(0n, r.method_end),
        target: text(r.target, r.target_end),
        version: '1.' + r.minor,
        fields,
        body: kinds[Number(body[0])],
        bodyLength: body[0] === 1n ? Number(body[1]) : null,
        bodyStart: Number(r.head_end),
      };
    } finally { b.exports.bst_free(p); b.exports.bst_free(cell); }
  }

  const api = { parse };
  if (typeof module !== 'undefined' && module.exports) module.exports = api; else root.BastioHttp = api;
})(typeof globalThis !== 'undefined' ? globalThis : this);
