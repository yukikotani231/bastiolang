#!/usr/bin/env bash
# The kernel prelude's boot-information readers (stage0/src/codegen.rs:
# bst_pool_init, bst_boot_fb) take untrusted input from the boot loader and
# make promises the verifier relies on. This compiles them on their own
# against hostile inputs:
# - a framebuffer address that would wrap `addr + pitch * h`, or one at
#   4 GiB, must not be reported (the verifier takes each field as < 2^32);
# - a memory-map entry whose size field is 0xFFFFFFFC must not make the
#   pool's walk loop forever (in 32 bits, size + 4 is 0);
# - with two runs of RAM, a page id in the second run must name a page in
#   that run, whether taken zeroed or dirty (bst_page_at,
#   bst_pool_take_dirty).
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
pool = pool.replace("(uint64_t)_end", "bst_endv").replace("&_image_start", "bst_img").replace("(uint64_t)_image_start", "bst_img")
# Page ids to addresses: the page reader and the dirty take.
pages = func("static inline uint64_t bst_page_at(uint64_t i) {")
i = s.index("static inline bst_frame bst_pool_take_dirty(")
pages += s[i:s.index("\n", i) + 1]
open(sys.argv[1], "w").write(r'''
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
typedef int bst_cpu;
typedef struct { uint64_t base, size; } bst_pool;
typedef uint64_t bst_frame;
static uint64_t bst_mbi, bst_pool_base;
static uint64_t bst_endv = 0x200000ull, bst_img;   /* the image's end, and its start (0: not named) */
static uint64_t *bst_arena2, bst_arena2_words;
''' + fb + pool + pages + r'''
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

    /* Two runs: 8 MiB at 512 MiB and 4 MiB at 768 MiB. The bookkeeping is
     * carved from the front of the first (6 pages of counters and 384 of
     * arena), so the first run holds ids 0 to 1657 and the second the rest. */
    uint8_t *ram2 = mmap((void *)0x30000000, 4ull << 20, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    if (ram2 != (uint8_t *)0x30000000) { printf("SKIP two runs (no low mapping)\n"); return bad != 0; }
    *(uint32_t *)(m + 24) = 20; *(uint64_t *)(m + 28) = 0x30000000; *(uint64_t *)(m + 36) = 4ull << 20; *(uint32_t *)(m + 44) = 1;
    p = bst_pool_init((uint64_t)info);
    want("two runs: the pool holds both", p.size, 2048 + 1024 - 6 - 384);
    want("two runs: the first id of the second run", bst_page_at(1658), 0x30000000);
    want("two runs: a dirty page in the second run", bst_pool_take_dirty(&p, 1661), 0x30000000 + 3 * 4096);
    want("two runs: a dirty page in the first run", bst_pool_take_dirty(&p, 1), 0x20000000 + 390 * 4096 + 4096);

    /* RAM below the image: the run at 0x20000000 holds the image at 0x20200000
     * (to 0x20400000). Its low 2 MiB join the pool, less the page the
     * information is in and the one the command line is in. */
    bst_endv = 0x20400000; bst_img = 0x20200000;
    *(uint32_t *)(m + 24) = 0;                          /* one run: 0x20000000, 8 MiB */
    info[11] = 24;
    uint32_t *in2 = (uint32_t *)(0x20000800);
    memcpy(in2, info, sizeof info);
    in2[0] = 64 | 4; in2[4] = 0x20003000; strcpy((char *)0x20003000, "console=ttyS0");
    p = bst_pool_init(0x20000800);
    want("below the image: both parts, less the loader's pages and the carve", p.size, 1340);
    want("below the image: the first page is above the information", bst_page_at(0), 0x20001000);
    want("below the image: the run after the command line", bst_page_at(2), 0x200C6000);
    bst_img = 0;
    p = bst_pool_init(0x20000800);
    want("an image start that is not named leaves the pool as it was", p.size, 1024 - 2 - 128);
    return bad != 0;
}
''')
PY
cc -O1 -w -o "$tmp/t" "$tmp/t.c"
"$tmp/t"
