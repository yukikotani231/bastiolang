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
    rsa) opts="-newkey rsa:2048 -sha256";;
    ed25519) opts="-newkey ed25519";;
  esac
  openssl req -x509 $opts -nodes -keyout "$tmp/$n.key" -out "$tmp/$n.pem" -subj "/CN=$n" -days 3650 \
    -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null || return 1
  openssl x509 -in "$tmp/$n.pem" -outform DER -out "$tmp/$n.der"
}
# A certificate for names, of key kind k, signed by the authority ca.
# ca_flag=1 makes it an intermediate that may sign.
make_cert() { # name kind ca names [ca_flag]
  local n=$1 k=$2 ca=$3 names=$4 flag=${5:-0} opts ext="$tmp/$1.ext"
  case $k in
    ec) opts="-newkey ec -pkeyopt ec_paramgen_curve:P-256";;
    rsa) opts="-newkey rsa:2048";;
    ed25519) opts="-newkey ed25519";;
  esac
  openssl req -new $opts -nodes -keyout "$tmp/$n.key" -out "$tmp/$n.csr" -subj "/CN=$n" 2>/dev/null || return 1
  if [ "$flag" = 1 ]; then
    printf 'basicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n' > "$ext"
  else
    printf 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nsubjectAltName=%s\n' "$names" > "$ext"
  fi
  local alg=-sha256; [ "$(openssl x509 -in "$tmp/$ca.pem" -noout -text | grep -c 'Public Key Algorithm: ED25519')" != 0 ] && alg=
  openssl x509 -req -in "$tmp/$n.csr" -CA "$tmp/$ca.pem" -CAkey "$tmp/$ca.key" -CAcreateserial -out "$tmp/$n.pem" \
    -days 365 $alg -extfile "$ext" 2>/dev/null || return 1
}
port=24000
# Starts a server with the certificate file cert (its chain behind it) and key, and sets $port.
serve() { # cert key [chain]
  port=$((port + 1))
  local chain=()
  [ -n "${3:-}" ] && chain=(-cert_chain "$tmp/$3.pem")
  openssl s_server -www -accept "127.0.0.1:$port" -tls1_3 -ciphersuites TLS_CHACHA20_POLY1305_SHA256 -groups X25519 \
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
make_cert rsalocal rsa rsaca "DNS:localhost"
serve rsalocal rsalocal && client "RSA certificate (rsa_pss_rsae_sha256)" 0 localhost rsa.roots
make_cert edlocal ed25519 edca "DNS:localhost"
serve edlocal edlocal && client "Ed25519 certificate" 0 localhost ed.roots

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

[ $fails = 0 ] && echo "tls: all passed" || { echo "tls: $fails failed"; exit 1; }
