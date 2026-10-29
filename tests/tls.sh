#!/usr/bin/env bash
# The TLS 1.3 client of core/tls13client against a real server: OpenSSL's
# `s_server` (TLS_CHACHA20_POLY1305_SHA256 and X25519 only) with certificates
# made here, run by examples/httpsget.bastio. The client must finish the
# handshake and read the answer for ECDSA, RSA and Ed25519 certificates, with
# and without an intermediate, and must refuse a name that is not in the
# certificate, a root it does not trust, a certificate that is not valid yet
# or any more, a server that is not allowed to sign, and a certificate whose
# chain is broken. Needs openssl 1.1.1 or later (the Ed25519 and ChaCha20
# parts) and uses ports on 127.0.0.1 only.
set -u
cd "$(dirname "$0")/.."
if ! command -v openssl >/dev/null; then echo "tls: openssl is missing, SKIPPED"; exit 0; fi
tmp=$(mktemp -d)
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done; rm -rf "$tmp"; }
trap cleanup EXIT
BASTIOC=${BASTIOC:-$(tests/bastioc1.sh)} || exit 1
"$BASTIOC" -o "$tmp/httpsget" examples/httpsget.bastio >/dev/null || { echo "tls: cannot build examples/httpsget.bastio"; exit 1; }
head -c 32 /dev/urandom > "$tmp/seed"
now=$(date +%s)
fails=0
fail() { echo "FAIL tls: $1"; fails=$((fails + 1)); }

# A certificate authority of the given key kind (ec, rsa, ed25519).
make_ca() { # name kind
  local n=$1 k=$2 opts
  case $k in
    ec) opts="-newkey ec -pkeyopt ec_paramgen_curve:P-256 -sha256";;
    ec384) opts="-newkey ec -pkeyopt ec_paramgen_curve:P-384 -sha384";;
    rsa) opts="-newkey rsa:2048 -sha256";;
    rsa384) opts="-newkey rsa:2048 -sha384";;
    rsa512) opts="-newkey rsa:2048 -sha512";;
    ed25519) opts="-newkey ed25519";;
  esac
  openssl req -x509 $opts -nodes -keyout "$tmp/$n.key" -out "$tmp/$n.pem" -subj "/CN=$n" -days 3650 \
    -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null || return 1
  openssl x509 -in "$tmp/$n.pem" -outform DER -out "$tmp/$n.der"
}
# A certificate for names, of key kind k, signed by the authority ca.
# ca_flag=1 makes it an intermediate that may sign.
make_cert() { # name kind ca names [ca_flag [hash]]
  local n=$1 k=$2 ca=$3 names=$4 flag=${5:-0} hash=${6:--sha256} opts ext="$tmp/$1.ext"
  case $k in
    ec) opts="-newkey ec -pkeyopt ec_paramgen_curve:P-256";;
    ec384) opts="-newkey ec -pkeyopt ec_paramgen_curve:P-384";;
    rsa) opts="-newkey rsa:2048";;
    ed25519) opts="-newkey ed25519";;
  esac
  openssl req -new $opts -nodes -keyout "$tmp/$n.key" -out "$tmp/$n.csr" -subj "/CN=$n" 2>/dev/null || return 1
  if [ "$flag" = 1 ]; then
    printf 'basicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n' > "$ext"
  else
    printf 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nsubjectAltName=%s\n' "$names" > "$ext"
  fi
  local alg=$hash; [ "$(openssl x509 -in "$tmp/$ca.pem" -noout -text | grep -c 'Public Key Algorithm: ED25519')" != 0 ] && alg=
  openssl x509 -req -in "$tmp/$n.csr" -CA "$tmp/$ca.pem" -CAkey "$tmp/$ca.key" -CAcreateserial -out "$tmp/$n.pem" \
    -days 365 $alg -extfile "$ext" 2>/dev/null || return 1
}
port=24000
suites=TLS_CHACHA20_POLY1305_SHA256
# Starts a server with the certificate file cert (its chain behind it) and key, and sets $port.
serve() { # cert key [chain]
  port=$((port + 1))
  local chain=()
  [ -n "${3:-}" ] && chain=(-cert_chain "$tmp/$3.pem")
  openssl s_server -www -accept "127.0.0.1:$port" -tls1_3 -ciphersuites "$suites" -groups X25519 \
    -cert "$tmp/$1.pem" -key "$tmp/$1.key" "${chain[@]}" >"$tmp/server.log" 2>&1 &
  pids+=($!)
  local i
  for i in $(seq 1 100); do
    (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null && return 0
    sleep 0.1
  done
  echo "tls: the server did not start"; cat "$tmp/server.log"; return 1
}
# Runs the client against the server on $port and checks its exit code.
client() { # description want(0|nonzero) name roots now
  local d=$1 want=$2 name=$3 roots=$4 t=${5:-$(date +%s)}
  timeout 60 "$tmp/httpsget" "$port" "$name" "$tmp/$roots" "$tmp/seed" "$t" > "$tmp/out" 2>&1
  local rc=$?
  if [ "$want" = 0 ] && [ $rc != 0 ]; then fail "$d: exit $rc"; sed 's/^/    /' "$tmp/out" | head -5
  elif [ "$want" != 0 ] && [ $rc = 0 ]; then fail "$d: accepted"; sed 's/^/    /' "$tmp/out" | head -5
  else echo "ok   $d"; fi
}

# The same with the system's clock and random bytes (std/https's https_get).
system_client() { # description want(0|nonzero) name roots
  local d=$1 want=$2
  timeout 60 "$tmp/httpsget" "$port" "$3" "$tmp/$4" > "$tmp/out" 2>&1
  local rc=$?
  if [ "$want" = 0 ] && [ $rc != 0 ]; then fail "$d: exit $rc"; sed 's/^/    /' "$tmp/out" | head -5
  elif [ "$want" != 0 ] && [ $rc = 0 ]; then fail "$d: accepted"; sed 's/^/    /' "$tmp/out" | head -5
  else echo "ok   $d"; fi
}

make_ca ecca ec; make_ca rsaca rsa; make_ca edca ed25519; make_ca other ec
cat "$tmp/ecca.der" > "$tmp/ec.roots"; cat "$tmp/rsaca.der" > "$tmp/rsa.roots"; cat "$tmp/edca.der" > "$tmp/ed.roots"
cat "$tmp/other.der" > "$tmp/other.roots"; cat "$tmp/ecca.der" "$tmp/rsaca.der" "$tmp/edca.der" > "$tmp/all.roots"

make_cert eclocal ec ecca "DNS:localhost,DNS:*.test.example"
serve eclocal eclocal && {
  client "ECDSA certificate" 0 localhost ec.roots
  client "ECDSA certificate, name from a wildcard" 0 www.test.example ec.roots
  client "ECDSA certificate, name in capitals" 0 LOCALHOST ec.roots
  client "ECDSA certificate, several roots" 0 localhost all.roots
  client "wrong name" 1 example.org ec.roots
  client "name below a wildcard of two labels" 1 a.b.test.example ec.roots
  client "untrusted root" 1 localhost other.roots
  client "no roots at all" 1 localhost /dev/null
  client "not valid yet" 1 localhost ec.roots 1000000000
  client "expired" 1 localhost ec.roots 4102444800
  system_client "std/https: https_get" 0 localhost ec.roots
  system_client "std/https: wrong name" 1 example.org ec.roots
  system_client "std/https: untrusted root" 1 localhost other.roots
}
suites=TLS_AES_128_GCM_SHA256
serve eclocal eclocal && client "AES-128-GCM only server" 0 localhost ec.roots
suites=TLS_AES_256_GCM_SHA384
serve eclocal eclocal && client "AES-256-GCM only server (SHA-384 key schedule)" 0 localhost ec.roots
suites=TLS_AES_128_GCM_SHA256:TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256
serve eclocal eclocal && client "server that offers all three suites" 0 localhost ec.roots
suites=TLS_CHACHA20_POLY1305_SHA256
make_cert rsalocal rsa rsaca "DNS:localhost"
serve rsalocal rsalocal && client "RSA certificate (rsa_pss_rsae_sha256)" 0 localhost rsa.roots
make_cert edlocal ed25519 edca "DNS:localhost"
serve edlocal edlocal && client "Ed25519 certificate" 0 localhost ed.roots

# P-384 and SHA-384 / SHA-512 signatures: the certificate chains that Let's Encrypt and most public CAs use.
make_ca ec384ca ec384; cat "$tmp/ec384ca.der" > "$tmp/ec384.roots"
make_cert ec384local ec384 ec384ca "DNS:localhost" 0 -sha384
serve ec384local ec384local && client "P-384 certificate under a P-384 root (ecdsa_secp384r1_sha384)" 0 localhost ec384.roots
suites=TLS_AES_256_GCM_SHA384
serve ec384local ec384local && client "P-384 certificate, AES-256-GCM-SHA384 suite" 0 localhost ec384.roots
suites=TLS_CHACHA20_POLY1305_SHA256
make_cert ec256under384 ec ec384ca "DNS:localhost" 0 -sha384
serve ec256under384 ec256under384 && client "P-256 certificate signed by a P-384 root with SHA-384" 0 localhost ec384.roots
make_cert ec384under384 ec384 ec384ca "DNS:localhost" 0 -sha512
serve ec384under384 ec384under384 && client "P-384 certificate signed with ecdsa-with-SHA512" 0 localhost ec384.roots
make_ca rsa384ca rsa384; cat "$tmp/rsa384ca.der" > "$tmp/rsa384.roots"
make_cert rsa384local rsa rsa384ca "DNS:localhost" 0 -sha384
serve rsa384local rsa384local && client "RSA certificate signed with sha384WithRSAEncryption" 0 localhost rsa384.roots
make_ca rsa512ca rsa512; cat "$tmp/rsa512ca.der" > "$tmp/rsa512.roots"
make_cert rsa512local ec rsa512ca "DNS:localhost" 0 -sha512
serve rsa512local rsa512local && client "ECDSA certificate under a root signed with sha512WithRSAEncryption" 0 localhost rsa512.roots
serve ec384local ec384local && client "P-384 certificate, other root" 1 localhost ec.roots

# A chain of three: root, intermediate (RSA), leaf (ECDSA); the server sends the leaf and the intermediate.
make_cert mid rsa ecca "" 1
make_cert chainleaf ec mid "DNS:localhost"
serve chainleaf chainleaf mid && {
  client "chain through an intermediate" 0 localhost ec.roots
  client "chain, root of another chain" 1 localhost rsa.roots
}
# The server sends only the leaf: the intermediate is missing.
serve chainleaf chainleaf && client "intermediate missing" 1 localhost ec.roots
# A leaf signed by a certificate that is not a CA.
make_cert notca ec ecca "DNS:localhost"
make_cert badchild ec notca "DNS:localhost"
serve badchild badchild notca && client "signer is not a CA" 1 localhost ec.roots

# https_fetch (URLs, host lookup, redirects) against a small HTTPS server in python3 with the ECDSA certificate.
if command -v python3 >/dev/null; then
  port=$((port + 1))
  cat > "$tmp/fetchd.py" <<'PY'
import http.server, ssl, sys
port, cert, key = int(sys.argv[1]), sys.argv[2], sys.argv[3]
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        p, hdr, code, body = self.path, [], 200, b'hello'
        raw = {
            '/chunked': b'5;x=y\r\nhello\r\n7\r\n, world\r\n0\r\nT: v\r\n\r\n',
            '/chunkedcut': b'5\r\nhello\r\n',
            '/chunkedbad': b'zz\r\nhello\r\n0\r\n\r\n',
            '/chunkedone': b'1\r\nA\r\n0\r\n\r\n',
        }
        if p in raw:
            self.wfile.write(b'HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n' + raw[p])
            return
        if p == '/both':
            self.wfile.write(b'HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n5\r\nhello\r\n0\r\n\r\n')
            return
        def to(loc, c=302):
            return c, [('Location', loc)], b'moved'
        if p == '/r1': code, hdr, body = to('/ok')
        elif p == '/r2': code, hdr, body = to('https://localhost:%d/ok' % port, 301)
        elif p.startswith('/chain/'):
            n = int(p[7:])
            code, hdr, body = to('/ok' if n == 0 else '/chain/%d' % (n - 1), (303, 307, 308)[n % 3])
        elif p == '/loop': code, hdr, body = to('/loop')
        elif p == '/http': code, hdr, body = to('http://localhost/ok')
        elif p == '/evil': code, hdr, body = to('//evil.example/ok')
        elif p.startswith('/echo'): body = p.encode()
        elif p != '/ok': code, body = 404, b'nope'
        self.send_response(code)
        for k, v in hdr: self.send_header(k, v)
        if p != '/nolen': self.send_header('Content-Length', str(100 if p == '/short' else len(body)))
        self.end_headers()
        self.wfile.write(body)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.minimum_version = ssl.TLSVersion.TLSv1_3
ctx.load_cert_chain(cert, key)
srv = http.server.HTTPServer(('127.0.0.1', port), H)
srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
srv.serve_forever()
PY
  python3 "$tmp/fetchd.py" "$port" "$tmp/eclocal.pem" "$tmp/eclocal.key" >"$tmp/fetchd.log" 2>&1 &
  pids+=($!)
  for i in $(seq 1 100); do (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null && break; sleep 0.1; done
  fetch() { # description url roots want_rc want_text
    local out rc
    out=$(timeout 60 "$tmp/httpsget" "$2" "$tmp/$3" 2>&1); rc=$?
    if [ "$rc" = "$4" ] && printf '%s' "$out" | grep -qF -- "$5"; then echo "ok   $1"
    else fail "$1: exit $rc, output '$(printf '%s' "$out" | head -c 200)'"; fi
  }
  base="https://localhost:$port"
  fetch "fetch: plain" "$base/ok" ec.roots 0 hello
  fetch "fetch: relative redirect" "$base/r1" ec.roots 0 hello
  fetch "fetch: absolute redirect" "$base/r2" ec.roots 0 hello
  fetch "fetch: three redirects in a row" "$base/chain/3" ec.roots 0 hello
  fetch "fetch: too many redirects" "$base/chain/9" ec.roots 1 "status 30"
  fetch "fetch: redirect loop" "$base/loop" ec.roots 1 "status 302"
  fetch "fetch: redirect to plain http is not followed" "$base/http" ec.roots 1 "status 302"
  fetch "fetch: redirect to another host by scheme-relative URL is not followed" "$base/evil" ec.roots 1 "status 302"
  fetch "fetch: 404 is reported" "$base/missing" ec.roots 1 "status 404"
  fetch "fetch: query string" "$base/echo?a=b&c=d#frag" ec.roots 0 "/echo?a=b&c=d"
  fetch "fetch: cut off with no Content-Length (no close_notify) is refused" "$base/nolen" ec.roots 1 "no answer"
  fetch "fetch: fewer bytes than Content-Length is refused" "$base/short" ec.roots 1 "no answer"
  fetch "fetch: chunked body, with an extension and a trailer" "$base/chunked" ec.roots 0 "hello, world"
  fetch "fetch: chunked body of one chunk" "$base/chunkedone" ec.roots 0 "status 200"
  fetch "fetch: chunked body without its last chunk is refused" "$base/chunkedcut" ec.roots 1 "no answer"
  fetch "fetch: malformed chunk size is refused" "$base/chunkedbad" ec.roots 1 "no answer"
  fetch "fetch: Transfer-Encoding with Content-Length is refused" "$base/both" ec.roots 1 "no answer"
  fetch "fetch: untrusted root" "$base/ok" other.roots 1 "no answer"
  fetch "fetch: address where the certificate has a name" "https://127.0.0.1:$port/ok" ec.roots 1 "no answer"
  fetch "fetch: not an https URL" "http://localhost:$port/ok" ec.roots 1 "no answer"
fi

[ $fails = 0 ] && echo "tls: all passed" || { echo "tls: $fails failed"; exit 1; }
