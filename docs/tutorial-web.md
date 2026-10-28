# A web service

This page builds the pieces of [`examples/todoapi.bastio`](../examples/todoapi.bastio), a to-do list service over HTTP and JSON, one step at a time. It assumes the first part of the [tutorial](tutorial.md) (contracts, `Bytes`, linear values). The library calls are in the reference for [`core/json.bastio`](lib/core-json.md) and [`std/http.bastio`](lib/std-http.md), which are hosted-target modules: sockets come from `host_net`, so a service runs on Linux, macOS and Windows, not in a kernel.

As in the other tutorial parts, every `bastio` block is compiled (and run) by `tests/run.sh`.

## 1. Bytes from the network are untrusted

A client may send anything. Bastio does not make a parser correct for you, but it does make it safe: a read outside a buffer, or a buffer that is never freed, is a compile-time error, so a hostile request can make the service answer 400 and nothing worse. The JSON scanner in `core/json.bastio` is the first example. It allocates nothing, and on any input it either finds a whole value, says the text ends too early, or says where the text is wrong:

```bastio
// expect-exit: 0
import "core/json.bastio";

fn main() -> u64 {
  let doc = bfrom("{\"title\": \"milk\", \"n\": 3}");
  let mut bad = 0;
  match json_value(doc, 0) {
    Json::Ok(e) => { if e != blen(doc) { bad = bad + 1; } },
    Json::Partial => { bad = bad + 1; },
    Json::Bad(x) => { bad = bad + 1; },
  }
  match json_member(doc, 0, "n") {
    Option::Some(at) => {
      match json_u64(doc, at) {
        Option::Some(n) => { if n != 3 { bad = bad + 1; } },
        Option::None => { bad = bad + 1; },
      }
    },
    Option::None => { bad = bad + 1; },
  }
  let cut = bfrom("{\"title\": ");
  match json_value(cut, 0) {
    Json::Ok(e) => { bad = bad + 1; },
    Json::Partial => { },
    Json::Bad(x) => { bad = bad + 1; },
  }
  bfree(doc);
  bfree(cut);
  return bad;
}
```

`json_member` and `json_u64` return `Option`, so a missing key and a wrong type are cases you must write, not exceptions you may forget. The `match` arms for `Json` are exhaustive for the same reason.

## 2. Writing JSON that is well formed

Answers are built with `JsonOut`, which tracks commas, keys and nesting. If the calls do not describe one whole document, `json_out_finish` returns `None` instead of malformed text, and a service turns that into a 500:

```bastio
// expect-exit: 0
import "core/json.bastio";

fn main() -> u64 {
  let mut o = json_out_new();
  json_out_begin_obj(&mut o);
  let k = bfrom("done");
  json_out_key(&mut o, k);
  json_out_bool(&mut o, true);
  json_out_end(&mut o);
  bfree(k);
  match json_out_finish(o) {
    Option::Some(b) => {
      let mut bad = 0;
      if !bytes_eq_str(b, "{\"done\":true}") { bad = 1; }
      bfree(b);
      return bad;
    },
    Option::None => { return 1; },
  }
}
```

A string you write with `json_out_str` is escaped, and one that is not valid UTF-8 has its bad bytes replaced, so a client's title can never break out of the document.

## 3. State with an invariant

The to-do list keeps four vectors that must stay the same length. Say so once, as a `spec fn`, and make every function that changes the list require and keep it:

```bastio
// expect-exit: 0
struct Todos { starts: Vec[u64], state: Vec[u8] }

spec fn todos_ok(t: &Todos) -> bool {
  vlen(t.state) == vlen(t.starts)
}

fn todos_new() -> Todos
  ensures todos_ok(result)
{
  return Todos { starts: vec_new[u64](), state: vec_new[u8]() };
}

fn todos_add(t: &mut Todos, at: u64) -> u64
  requires todos_ok(t)
  ensures todos_ok(t) && result >= 1
{
  vec_push(&mut t.starts, at);
  vec_push(&mut t.state, 0);
  return vec_len(t.starts);
}

fn main() -> u64 {
  let mut t = todos_new();
  let id = todos_add(&mut t, 0);
  vec_free(t.starts);
  vec_free(t.state);
  return id - 1;
}
```

Forget the second `vec_push` and the compiler refuses `todos_add`, because the vectors no longer have the same length when the function returns. That is the kind of mistake that, in a service, becomes an out-of-range read on the next request.

## 4. Serving requests

`http_accept` takes a connection and parses the request head with the strict parser from [`examples/http`](examples-http.md): a head the parser rejects is answered with 400 and never reaches your code. `http_req_body` reads the body by `Content-Length` or chunked framing, within a bound you give, and refuses ambiguous framing. `http_answer` writes the reply and decides whether the connection stays open. This program runs a client and a server in one process over the loopback interface:

```bastio
// expect-exit: 0
import "std/http.bastio";

fn main(h: &mut Host) -> u64 {
  match net_listen(h, net_ip4(127, 0, 0, 1), 0) {
    Option::None => { return 1; },
    Option::Some(s) => {
      let p = net_port(h, s);
      let mut bad = 0;
      match net_connect(h, net_ip4(127, 0, 0, 1), p) {
        Option::None => { bad = 1; },
        Option::Some(c) => {
          let body = bfrom("{\"title\":\"milk\"}");
          let req = http_post_request("localhost", "/todos", "application/json", body);
          bfree(body);
          if !net_send_all(h, c, req) { bad = 1; }
          bfree(req);
          match http_accept(h, s, 4096) {
            Option::None => { bad = 1; },
            Option::Some(mut r) => {
              match http_req_body(h, &mut r, 4096) {
                Option::Some(b) => {
                  if !bytes_eq_str(b, "{\"title\":\"milk\"}") { bad = 1; }
                  bfree(b);
                  let empty = bfrom("{}");
                  let k = http_answer(h, r, 201, "Created", "application/json", empty);
                  bfree(empty);
                  match k {
                    Option::Some(c2) => { net_close(h, c2); },
                    Option::None => { },
                  }
                },
                Option::None => { http_close(h, r); bad = 1; },
              }
            },
          }
          match http_read_all(h, c, 4096) {
            Option::Some(resp) => { bfree(resp); },
            Option::None => { bad = 1; },
          }
          net_close(h, c);
        },
      }
      net_close(h, s);
      return bad;
    },
  }
}
```

The complete service, with routing, error answers, keep-alive and a self-check, is `examples/todoapi.bastio`. Run it with no arguments to see it check itself, or `todoapi serve 8080` and talk to it with `curl`.

## What is proved, and what is not

The compiler proves that no buffer is read or written out of bounds, that none leaks, and that the invariants you wrote hold after every request. It does not prove that the routes do what you meant (the self-check in `todoapi` tests that), that the JSON scanner accepts exactly the documents of RFC 8259, or that the C library and the operating system's sockets are correct. TLS and WebAssembly sockets are not built yet; see the [roadmap](../ROADMAP.md).
