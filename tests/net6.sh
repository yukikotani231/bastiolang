#!/usr/bin/env bash
# host_net IPv6 connect (operation 9) on every Linux backend: a LD_PRELOAD shim
# replaces socket and connect, records the domain and the sockaddr that reach
# them and lets the connection "succeed" on an IPv4 socket, so the bytes of the
# sockaddr_in6 are checked even on a host without IPv6. Then, if the host has
# IPv6 on its loopback, the program really connects to a python3 listener on
# [::1]. Needs cc; the AArch64 runs need aarch64-linux-gnu-gcc and
# qemu-aarch64 (skipped without them).
set -u
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
BASTIOC=${BASTIOC:-$(tests/bastioc1.sh)} || exit 1
fails=0
fail() { echo "FAIL net6: $1"; fails=$((fails + 1)); }
cat > "$tmp/shim.c" <<'C'
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <errno.h>
#include <stdlib.h>
int socket(int d, int t, int p) {
    int (*real)(int, int, int) = (int (*)(int, int, int))dlsym(RTLD_NEXT, "socket");
    fprintf(stderr, "socket %d\n", d);
    return real(d == AF_INET6 ? AF_INET : d, t, p);
}
int connect(int fd, const struct sockaddr *a, socklen_t l) {
    const unsigned char *b = (const unsigned char *)a;
    fprintf(stderr, "connect %u ", (unsigned)l);
    for (socklen_t i = 0; i < l; i++) fprintf(stderr, "%02x", b[i]);
    fprintf(stderr, "\n");
    if (getenv("SHIM_FAIL")) { errno = ENETUNREACH; return -1; }   // a connect that fails at once, as with no route
    return 0;
}
C
want_connect="connect 28 0a00abcd0000000020010db8000100020003000400050006 00000000"
want_connect=$(echo "$want_connect" | tr -d ' ' | sed 's/^connect28/connect 28 /')
check_probe() { # description binary [runner]
    local out
    out=$(env LD_PRELOAD="$tmp/shim.so" ${3:-} "$2" probe 2>&1 >/dev/null | sort | uniq -c | sed 's/^ *//')
    local want="2 $want_connect
2 socket 10"
    if [ "$out" = "$want" ]; then echo "ok   $1"; else fail "$1: got '$out'"; fi
}
cc -shared -fPIC -o "$tmp/shim.so" "$tmp/shim.c" -ldl || { echo "net6: cannot build the shim, SKIPPED"; exit 0; }
for backend in "" "--backend c" "--backend native" "--backend native-ir"; do
    name=${backend:-default}
    "$BASTIOC" $backend -o "$tmp/p" tests/pass/host_net6.bastio >/dev/null 2>"$tmp/err" || { fail "$name: cannot build ($(head -c 200 "$tmp/err"))"; continue; }
    check_probe "x86-64 $name: sockaddr_in6 reaches connect" "$tmp/p"
    # A connect that fails at once (no route) fails the call, with a time limit too.
    if SHIM_FAIL=1 LD_PRELOAD="$tmp/shim.so" "$tmp/p" up 5 >/dev/null 2>&1; then fail "x86-64 $name: a connect that fails at once was taken for a success"; else echo "ok   x86-64 $name: a connect that fails at once fails"; fi
done
if command -v aarch64-linux-gnu-gcc >/dev/null && command -v qemu-aarch64 >/dev/null; then
    aarch64-linux-gnu-gcc -shared -fPIC -o "$tmp/shim64.so" "$tmp/shim.c" -ldl || fail "cannot build the AArch64 shim"
    export QEMU_LD_PREFIX=${QEMU_LD_PREFIX:-/usr/aarch64-linux-gnu}
    for backend in "--backend c" "--backend native"; do
        CC=aarch64-linux-gnu-gcc "$BASTIOC" --arch aarch64 $backend -o "$tmp/p64" tests/pass/host_net6.bastio >/dev/null 2>"$tmp/err" || { fail "aarch64 $backend: cannot build ($(head -c 200 "$tmp/err"))"; continue; }
        out=$(QEMU_SET_ENV="LD_PRELOAD=$tmp/shim64.so" qemu-aarch64 "$tmp/p64" probe 2>&1 >/dev/null | sort | uniq -c | sed 's/^ *//')
        want="2 $want_connect
2 socket 10"
        if [ "$out" = "$want" ]; then echo "ok   aarch64 $backend: sockaddr_in6 reaches connect"; else fail "aarch64 $backend: got '$out'"; fi
        if SHIM_FAIL=1 QEMU_SET_ENV="LD_PRELOAD=$tmp/shim64.so,SHIM_FAIL=1" qemu-aarch64 "$tmp/p64" up 5 >/dev/null 2>&1; then fail "aarch64 $backend: a connect that fails at once was taken for a success"; else echo "ok   aarch64 $backend: a connect that fails at once fails"; fi
    done
else
    echo "net6: aarch64-linux-gnu-gcc or qemu-aarch64 is missing, the AArch64 runs are SKIPPED"
fi
# A real connection, where the host has IPv6 on its loopback.
if command -v python3 >/dev/null && python3 -c "import socket; s = socket.socket(socket.AF_INET6); s.bind(('::1', 0))" 2>/dev/null; then
    cat > "$tmp/srv.py" <<'PY'
import socket, sys
s = socket.socket(socket.AF_INET6); s.bind(('::1', 0)); s.listen(1)
print(s.getsockname()[1], flush=True)
c, _ = s.accept()
sys.stdout.write(c.recv(16).decode() + "\n"); sys.stdout.flush()
PY
    python3 "$tmp/srv.py" > "$tmp/srv.out" &
    spid=$!
    for i in $(seq 1 50); do [ -s "$tmp/srv.out" ] && break; sleep 0.1; done
    port=$(head -1 "$tmp/srv.out")
    "$BASTIOC" -o "$tmp/p" tests/pass/host_net6.bastio >/dev/null 2>&1
    if timeout 20 "$tmp/p" lo "$port" && sleep 0.5 && grep -qx hi "$tmp/srv.out"; then echo "ok   connect to [::1]"; else fail "connect to [::1] failed"; fi
    kill $spid 2>/dev/null
else
    echo "net6: no IPv6 on the loopback of this host, the real connection is SKIPPED"
fi
[ $fails = 0 ] && echo "net6: all passed" || { echo "net6: $fails failed"; exit 1; }
