#!/usr/bin/env bash
# core/dns and std/dns against a small DNS server over TCP (python3, on
# 127.0.0.1): addresses, a CNAME before the address, name errors, an answer
# that arrives a byte at a time, an answer with the wrong id, a truncated
# answer, a name that is already an address and bad names. The programs are
# built from examples/lookup.bastio. Needs python3.
set -u
cd "$(dirname "$0")/.."
if ! command -v python3 >/dev/null; then echo "dns: python3 is missing, SKIPPED"; exit 0; fi
tmp=$(mktemp -d)
pid=""
cleanup() { [ -n "$pid" ] && kill "$pid" 2>/dev/null; rm -rf "$tmp"; }
trap cleanup EXIT
BASTIOC=${BASTIOC:-$(tests/bastioc1.sh)} || exit 1
"$BASTIOC" -o "$tmp/lookup" examples/lookup.bastio >/dev/null || { echo "dns: cannot build examples/lookup.bastio"; exit 1; }
cat > "$tmp/server.py" <<'PY'
import socket, struct, sys, threading, time
def rr(name_ptr, ty, data):
    return name_ptr + struct.pack('>HHIH', ty, 1, 60, len(data)) + data
def serve(c):
    try:
        n = struct.unpack('>H', c.recv(2))[0]
        q = b''
        while len(q) < n: q += c.recv(n - len(q))
        i = struct.unpack('>H', q[:2])[0]
        # the name starts at 12; find its end
        e = 12
        while q[e]: e += 1 + q[e]
        labels = []; p = 12
        while q[p]:
            labels.append(q[p+1:p+1+q[p]].decode()); p += 1 + q[p]
        name = '.'.join(labels)
        question = q[12:e+5]
        ptr = b'\xc0\x0c'
        rc = 0; ans = []; rid = i; tc = 0
        if name == 'example.test': ans.append(rr(ptr, 1, bytes([192,0,2,7])))
        elif name == 'www.example.test':
            ans.append(rr(ptr, 5, b'\x07example\x04test\x00'))
            ans.append(rr(b'\xc0' + bytes([12 + 1 + 3 + 1]), 1, bytes([192,0,2,8])))
        elif name == 'frag.test': ans.append(rr(ptr, 1, bytes([192,0,2,9])))
        elif name == 'badid.test': ans.append(rr(ptr, 1, bytes([192,0,2,10]))); rid = i ^ 1
        elif name == 'cut.test': ans.append(rr(ptr, 1, bytes([192,0,2,11])))
        elif name == 'none.test': pass
        else: rc = 3
        flags = 0x8180 | rc | (tc << 9)
        msg = struct.pack('>HHHHHH', rid, flags, 1, len(ans), 0, 0) + question + b''.join(ans)
        if name == 'cut.test': msg = msg[:-3]
        out = struct.pack('>H', len(msg) + (3 if name == 'cut.test' else 0)) + msg
        if name == 'frag.test':
            for b in out:
                c.sendall(bytes([b])); time.sleep(0.002)
        else:
            c.sendall(out)
    finally:
        c.close()
def mute(l):
    held = []
    while True:
        c, _ = l.accept()   # a resolver that accepts the query and never answers
        held.append(c)
m = socket.socket(); m.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
m.bind(('127.0.0.1', 0)); m.listen(16)
threading.Thread(target=mute, args=(m,), daemon=True).start()
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.1', 0)); s.listen(16)
print(s.getsockname()[1], m.getsockname()[1], flush=True)
while True:
    c, _ = s.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
PY
python3 "$tmp/server.py" > "$tmp/port" &
pid=$!
for i in $(seq 1 100); do [ -s "$tmp/port" ] && break; sleep 0.1; done
read -r port mute < "$tmp/port"
[ -n "$port" ] || { echo "dns: the server did not start"; exit 1; }
fails=0
check() { # description name port want_output want_rc
  local out rc
  out=$(timeout 20 "$tmp/lookup" "$2" ${3:+"$3"} 2>&1); rc=$?
  if [ "$rc" = "$5" ] && [ "$out" = "$4" ]; then echo "ok   $1"
  else echo "FAIL dns: $1: exit $rc, output '$out'"; fails=$((fails + 1)); fi
}
check "A record" example.test "$port" 192.0.2.7 0
check "CNAME, then the address" www.example.test "$port" 192.0.2.8 0
check "answer a byte at a time" frag.test "$port" 192.0.2.9 0
check "name error" nx.test "$port" "not found" 1
check "no records" none.test "$port" "not found" 1
check "wrong id" badid.test "$port" "not found" 1
check "answer cut short" cut.test "$port" "not found" 1
check "name in capitals (the test server knows only lower case)" EXAMPLE.TEST "$port" "not found" 1
check "empty label" a..b "$port" "not found" 1
check "name already an address" 10.20.30.40 "" 10.20.30.40 0
start=$(date +%s)
check "a resolver that never answers is given up on (5 seconds)" example.test "$mute" "not found" 1
elapsed=$(( $(date +%s) - start ))
if [ "$elapsed" -ge 4 ] && [ "$elapsed" -le 20 ]; then echo "ok   it took $elapsed seconds"
else echo "FAIL dns: the silent resolver took $elapsed seconds"; fails=$((fails + 1)); fi
check "nothing listens" example.test 1 "not found" 1
[ $fails = 0 ] && echo "dns: all passed" || { echo "dns: $fails failed"; exit 1; }
