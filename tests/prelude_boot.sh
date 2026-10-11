#!/usr/bin/env bash
# The kernel prelude's boot-information readers (stage0/src/codegen.rs:
# bst_pool_init, bst_boot_fb) take untrusted input from the boot loader and
# make promises the verifier relies on. This compiles them on their own
# against hostile inputs:
# - a framebuffer address that would wrap `addr + pitch * h`, or one at
#   4 GiB, must not be reported (the verifier takes each field as < 2^32);
# - a memory-map entry whose size field is 0xFFFFFFFC must not make the
#   pool's walk loop forever (in 32 bits, size + 4 is 0).
# stage1's prelude (stage1/cprelude.bastio) is byte-identical (difftest),
# so this covers both C backends.
#
# usage: tests/prelude_boot.sh
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

python3 - "$tmp/t.c" <<'PY'
import sys
s = open("stage0/src/codegen.rs").read()
def func(head):
    i = s.index(head)
    return s[i:s.index("\n}\n", i) + 3]
fb = func("static uint64_t bst_boot_fb(bst_cpu c, uint64_t f) {")
pool = func("static bst_pool bst_pool_init(uint64_t mbi) {")
# The pool's own definitions: the runs, their limits and the bookkeeping.
pool = s[s.index("extern uint64_t bst_ram_limit"):s.index("static bst_pool bst_pool_init(uint64_t mbi) {")] + pool
pool = pool.replace("(uint64_t)_end", "0x200000ull")
open(sys.argv[1], "w").write(r'''
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
typedef int bst_cpu;
typedef struct { uint64_t base, size; } bst_pool;
static uint64_t bst_mbi, bst_pool_base;
static uint64_t *bst_arena2, bst_arena2_words;
''' + fb + pool + r'''
static int bad;
static void want(const char *what, uint64_t got, uint64_t expect) {
    if (got != expect) { printf("FAIL %s: got %#llx, want %#llx\n", what, (unsigned long long)got, (unsigned long long)expect); bad++; }
    else printf("ok   %s\n", what);
}
int main(void) {
    static uint8_t mb[120];
    *(uint32_t *)mb = 1u << 12; mb[109] = 1; mb[108] = 32;
    *(uint32_t *)(mb + 96) = 4096; *(uint32_t *)(mb + 100) = 1024;
    bst_mbi = (uint64_t)mb;
    *(uint64_t *)(mb + 88) = 0xFFFFFFFFFFFFF000ull; *(uint32_t *)(mb + 104) = 2;
    want("a framebuffer that wraps past 2^64 is refused", bst_boot_fb(0, 0), 0);
    *(uint64_t *)(mb + 88) = 1ull << 32; *(uint32_t *)(mb + 104) = 0;
    want("a framebuffer at 4 GiB is refused", bst_boot_fb(0, 0), 0);
    *(uint64_t *)(mb + 88) = 0xFFFFF000; *(uint32_t *)(mb + 104) = 2;
    want("a framebuffer past 4 GiB is refused", bst_boot_fb(0, 0), 0);
    *(uint32_t *)(mb + 104) = 1;
    want("a framebuffer ending at 4 GiB is reported", bst_boot_fb(0, 0), 0xFFFFF000);
    *(uint64_t *)(mb + 88) = 0xFD000000; *(uint32_t *)(mb + 104) = 768;
    want("a usual framebuffer is reported", bst_boot_fb(0, 0), 0xFD000000);

    /* The map must lie below 1 GiB, where the prelude reads it, and so
     * must the RAM it lists: the pool's bookkeeping is written into it. */
    uint8_t *m = mmap((void *)0x10000000, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    uint8_t *ram = mmap((void *)0x20000000, 8ull << 20, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    if (m != (uint8_t *)0x10000000 || ram != (uint8_t *)0x20000000) { printf("SKIP the memory map (no low mapping)\n"); return bad != 0; }
    *(uint32_t *)m = 20; *(uint64_t *)(m + 4) = 0x20000000; *(uint64_t *)(m + 12) = 8ull << 20; *(uint32_t *)(m + 20) = 1;
    *(uint32_t *)(m + 24) = 0xFFFFFFFC;                  /* then a size that is 0 in 32 bits once 4 is added */
    uint32_t info[30] = {0};
    info[0] = 64; info[11] = 48; info[12] = (uint32_t)(uint64_t)m;
    alarm(5);                                           /* the old walk never returned */
    bst_pool p = bst_pool_init((uint64_t)info);
    /* 2048 pages, less 4 for the counters and 256 for the arena's extension */
    want("a map entry with a size of 0xFFFFFFFC ends the walk", p.size, 2048 - 4 - 256);
    return bad != 0;
}
''')
PY
cc -O1 -w -o "$tmp/t" "$tmp/t.c"
"$tmp/t"
