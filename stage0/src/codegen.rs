// C backend (stage0 only; the self-hosted compiler will emit x86-64 directly).
//
// Zero overhead: everything the verifier proved is erased here. There are no
// bounds checks, no overflow checks and no length bookkeeping beyond the
// `len` field the program itself can read. Specifications, invariants and
// asserts produce no code at all, and so does every capability's ghost
// state: a Frame or PageTable is just a physical address at run time, and
// Serial, Mmu and Cpu are empty tokens.
//
// The preludes below are the trusted runtime of each target (listed in
// docs/language.md). They are the only C a Bastio program runs on.

use crate::ast::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Target {
    /// A normal program on a hosted C library. The console is stdin/stdout;
    /// physical pages are simulated with ordinary memory.
    Hosted,
    /// Freestanding x86-64 kernel code: no C library. The console is COM1,
    /// memory comes from a fixed arena, and the entry point is `bst_kmain`.
    /// The platform (boot code) provides the trap entry points `bst_cpu_enter`
    /// and `bst_cpu_wait` and the kernel's top-level table `bst_kernel_pml4`.
    Kernel,
    /// A user program under a Bastio kernel: the console and process control
    /// are system calls (`int 0x80`), and the entry point is `bst_umain`.
    User,
}

struct Gen<'a> {
    /// `&mut` parameters of the current function: their C value is a pointer.
    mut_params: HashSet<String>,
    blobs: &'a mut Vec<Vec<u8>>,
    /// Counter for the temporaries that hold a `match` scrutinee.
    tmps: usize,
    /// The type of every variable declared so far in this function (a name
    /// reused in a later scope is overwritten, as code is emitted in order).
    types: HashMap<String, Type>,
    sigs: &'a HashMap<String, crate::check::Sig>,
    /// Boxed types used anywhere: each gets a pair of helpers.
    boxed: &'a mut Vec<Type>,
    /// Element types of vectors operated on: each gets a set of helpers.
    vecs: &'a mut Vec<Type>,
}

pub fn emit(p: &Program, target: Target) -> String {
    let sigs = crate::check::signatures(p, target).expect("checked");
    let mut blobs = Vec::new();
    let mut boxed = Vec::new();
    let mut vecs = Vec::new();
    let mut bodies = String::new();
    for f in &p.fns {
        let mut g = Gen {
            mut_params: f.params.iter().filter(|p| p.ty.is_mut_ref()).map(|p| p.name.clone()).collect(),
            blobs: &mut blobs,
            tmps: 0,
            types: f.params.iter().map(|p| (p.name.clone(), p.ty.base().clone())).collect(),
            sigs: &sigs,
            boxed: &mut boxed,
            vecs: &mut vecs,
        };
        bodies.push_str(&format!("{} {{\n", proto(f)));
        for s in &f.body {
            g.stmt(s, 1, &mut bodies);
        }
        bodies.push_str("}\n\n");
    }
    let mut out = String::new();
    for (_, part) in prelude_parts(target) {
        out.push_str(part);
    }
    out.push_str(&struct_decls(p));
    for t in &vecs {
        let (c, n) = (cty(t), box_helper(t));
        out.push_str(&format!("static void bst_vec_push_{n}(bst_vec *v, {c} x) {{ if (v->len == v->cap) bst_vec_grow(v, sizeof({c})); (({c} *)v->data)[v->len++] = x; }}\n"));
        out.push_str(&format!("static {c} bst_vec_pop_{n}(bst_vec *v) {{ return (({c} *)v->data)[--v->len]; }}\n"));
        out.push_str(&format!("static {c} bst_vec_get_{n}(bst_vec v, uint64_t i) {{ return (({c} *)v.data)[i]; }}\n"));
        out.push_str(&format!("static void bst_vec_set_{n}(bst_vec *v, uint64_t i, {c} x) {{ (({c} *)v->data)[i] = x; }}\n"));
        out.push_str(&format!("static {c} bst_vec_swap_{n}(bst_vec *v, uint64_t i, {c} x) {{ {c} o = (({c} *)v->data)[i]; (({c} *)v->data)[i] = x; return o; }}\n"));
        out.push_str(&format!("static void bst_vec_free_{n}(bst_vec v) {{ bst_vec_release(v, sizeof({c})); }}\n"));
    }
    for t in &boxed {
        let (c, n) = (cty(t), box_helper(t));
        out.push_str(&format!("static {c} bst_unbox_{n}({c} *p) {{ {c} v = *p; bst_box_free(p, sizeof({c})); return v; }}\n"));
    }
    for (i, b) in blobs.iter().enumerate() {
        out.push_str(&format!("static const unsigned char bst_blob{}[{}] = {{", i, b.len().max(1)));
        for (j, x) in b.iter().enumerate() {
            if j % 24 == 0 {
                out.push('\n');
            }
            out.push_str(&format!("{},", x));
        }
        out.push_str("\n};\n");
    }
    for f in &p.fns {
        out.push_str(&format!("{};\n", proto(f)));
    }
    out.push('\n');
    out.push_str(&bodies);
    out.push_str(&entry(p, target));
    out
}

/// The entry point: builds the capabilities `main` asks for and calls it.
fn entry(p: &Program, target: Target) -> String {
    let main = p.fns.iter().find(|f| f.name == "main").expect("checked");
    let legacy = main.params.len() == 1 && main.params[0].ty == Type::Serial;
    let mut setup = String::new();
    let mut args = Vec::new();
    let mut teardown = String::new();
    if legacy {
        args.push("0".to_string());
    } else {
        for (i, prm) in main.params.iter().enumerate() {
            let t = prm.ty.base();
            let init = match (t, target) {
                (Type::Pool, Target::Kernel) => "bst_pool_init(mbi)",
                (Type::Pool, _) => "bst_pool_init()",
                _ => "0",
            };
            setup.push_str(&format!("    {} c{} = {};\n", cty(t), i, init));
            args.push(format!("&c{}", i));
            if *t == Type::Pool && target == Target::Hosted {
                teardown.push_str(&format!("    bst_pool_fini(c{});\n", i));
            }
        }
    }
    let call = format!("f_main({})", args.join(", "));
    let returns = main.ret == Type::U64;
    match target {
        Target::Hosted => format!(
            "int main(int argc, char **argv) {{\n    bst_argc = argc;\n    bst_argv = argv;\n{}    {}{};\n{}    return {};\n}}\n",
            setup,
            if returns { "uint64_t r = " } else { "(void)" },
            call,
            teardown,
            if returns { "(int)r" } else { "0" }
        ),
        Target::Kernel => format!("void bst_kmain(uint64_t mbi) {{\n    (void)mbi;\n    bst_serial_init();\n{}    (void){};\n}}\n", setup, call),
        Target::User => format!("uint64_t bst_umain(void) {{\n{}    uint64_t r = {};\n    bst_flush();\n    return r;\n}}\n", setup, call),
    }
}

const PRELUDE_COMMON: &str = r#"/* generated by bastioc0 -- do not edit */
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
typedef struct { uint64_t len; uint64_t *data; } bst_buf;
typedef struct { uint64_t len; const unsigned char *data; } bst_str;
typedef struct { uint64_t len; unsigned char *data; } bst_bytes;
typedef int bst_serial, bst_mmu, bst_cpu, bst_host;   /* capability tokens: no data */
typedef uint64_t bst_frame, bst_pt;         /* a page's physical address */
typedef struct { uint64_t base, size; } bst_pool;
static inline bst_buf bst_set(bst_buf b, uint64_t i, uint64_t v) { b.data[i] = v; return b; }
"#;

const PRELUDE_HOSTED: &str = r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static inline void bst_serial_write(bst_serial s, uint64_t b) { (void)s; putchar((int)b); if (b == '\n') fflush(stdout); }
static inline uint64_t bst_serial_read(bst_serial s) { (void)s; fflush(stdout); int c = getchar(); return c == EOF ? 4 : (uint64_t)c; }

static bst_buf bst_alloc(uint64_t n) {
    bst_buf b;
    b.len = n;
    b.data = (uint64_t *)calloc(n ? n : 1, sizeof(uint64_t));
    if (!b.data) { fputs("Bastio: out of memory\n", stderr); abort(); }
    return b;
}
static inline void bst_free(bst_buf b) { free(b.data); }
/* Grow a block of `old` words to `n` words, keeping its contents; the new
 * words are not zeroed (only Vec uses it, and never reads past its length). */
static uint64_t *bst_regrow(uint64_t *p, uint64_t old, uint64_t n) {
    (void)old;
    uint64_t *q = (uint64_t *)realloc(p, (n ? n : 1) * sizeof(uint64_t));
    if (!q) { fputs("Bastio: out of memory\n", stderr); abort(); }
    return q;
}
static inline void bst_print(uint64_t v) { printf("%llu\n", (unsigned long long)v); }

static bst_bytes bst_balloc(uint64_t n) {
    bst_bytes b;
    b.len = n;
    b.data = (unsigned char *)calloc(n ? n : 1, 1);
    if (!b.data) { fputs("Bastio: out of memory\n", stderr); abort(); }
    return b;
}
static inline void bst_bfree(bst_bytes b) { free(b.data); }

/* The Host capability: command-line arguments and whole-file I/O. */
static int bst_argc;
static char **bst_argv;
static uint64_t bst_host_argc(bst_host h) { (void)h; return (uint64_t)bst_argc; }
static bst_bytes bst_host_arg(bst_host h, uint64_t i) {
    (void)h;
    if (i >= (uint64_t)bst_argc) return bst_balloc(0);
    size_t n = strlen(bst_argv[i]);
    bst_bytes b = bst_balloc(n);
    memcpy(b.data, bst_argv[i], n);
    return b;
}
static char *bst_cpath(bst_bytes p) {   /* a NUL-terminated copy; a NUL inside ends the path */
    char *c = (char *)malloc(p.len + 1);
    if (!c) { fputs("Bastio: out of memory\n", stderr); abort(); }
    memcpy(c, p.data, p.len);
    c[p.len] = 0;
    return c;
}
static bool bst_host_read(bst_host h, bst_bytes path, bst_bytes *out) {
    (void)h;
    char *c = bst_cpath(path);
    FILE *f = fopen(c, "rb");
    free(c);
    if (!f) return false;
    size_t cap = 4096, n = 0;
    unsigned char *d = (unsigned char *)malloc(cap);
    if (!d) { fputs("Bastio: out of memory\n", stderr); abort(); }
    size_t got;
    while ((got = fread(d + n, 1, cap - n, f)) > 0) {
        n += got;
        if (n == cap) {
            cap *= 2;
            d = (unsigned char *)realloc(d, cap);
            if (!d) { fputs("Bastio: out of memory\n", stderr); abort(); }
        }
    }
    bool ok = !ferror(f);
    fclose(f);
    if (!ok) { free(d); return false; }
    free(out->data);
    out->len = n;
    out->data = d;
    return true;
}
static bool bst_host_write(bst_host h, bst_bytes path, bst_bytes data) {
    (void)h;
    char *c = bst_cpath(path);
    FILE *f = fopen(c, "wb");
    free(c);
    if (!f) return false;
    bool ok = fwrite(data.data, 1, data.len, f) == data.len;
    return fclose(f) == 0 && ok;
}

/* Simulated physical memory: a pool of page-aligned pages from the heap. */
#define BST_HOSTED_PAGES 256
static uint64_t bst_pool_base;
static bst_pool bst_pool_init(void) {
    bst_pool p;
    p.size = BST_HOSTED_PAGES;
    p.base = (uint64_t)aligned_alloc(4096, BST_HOSTED_PAGES * 4096);
    if (!p.base) { fputs("Bastio: out of memory\n", stderr); abort(); }
    bst_pool_base = p.base;
    return p;
}
static void bst_pool_fini(bst_pool p) { free((void *)p.base); }
"#;

/* Pages and page tables: shared by the hosted simulation and the kernel.
 * An entry is a physical address plus flag bits; slot 0 of a kernel
 * top-level table points at the kernel's own map (not owned). */
const PRELUDE_PAGES: &str = r#"
#define BST_PTE_P 1ull
#define BST_PTE_W 2ull
#define BST_PTE_U 4ull
#define BST_PTE_ADDR 0x000FFFFFFFFFF000ull
static inline void bst_zero_page(uint64_t a) {
#if defined(__x86_64__)
    void *p = (void *)a;
    uint64_t n = 512;
    __asm__ volatile ("rep stosq" : "+D"(p), "+c"(n) : "a"(0ull) : "memory");
#else
    uint64_t *w = (uint64_t *)a;
    for (int k = 0; k < 512; k++) w[k] = 0;
#endif
}
static inline uint64_t bst_pool_size(bst_pool p) { return p.size; }
static inline bst_frame bst_pool_take(bst_pool *p, uint64_t i) {
    uint64_t a = p->base + i * 4096;
    bst_zero_page(a);      /* never leak a page's old contents */
    return a;
}
static inline void bst_pool_give(bst_pool *p, bst_frame f) { (void)p; (void)f; }
static inline uint64_t bst_frame_id(bst_frame f) { return (f - bst_pool_base) / 4096; }
static inline uint64_t bst_frame_read(bst_frame f, uint64_t w) { return ((uint64_t *)f)[w]; }
static inline void bst_frame_write(bst_frame *f, uint64_t w, uint64_t v) { ((uint64_t *)*f)[w] = v; }
static inline uint64_t bst_frame_read8(bst_frame f, uint64_t o) { return ((unsigned char *)f)[o]; }
static inline void bst_frame_write8(bst_frame *f, uint64_t o, uint64_t v) { ((unsigned char *)*f)[o] = (unsigned char)v; }
static inline bst_pt bst_pt_new(bst_frame f, uint64_t level) {
    uint64_t *e = (uint64_t *)f;
    bst_zero_page(f);      /* f may be a page that held data, not a fresh one */
    BST_PT_NEW_HOOK
    return f;
}
static inline bst_frame bst_pt_free(bst_pt t) { return t; }
static inline bool bst_pt_has(bst_pt t, uint64_t i) { return (((uint64_t *)t)[i] & BST_PTE_P) != 0; }
static inline void bst_pt_put(bst_pt *t, uint64_t i, bst_pt c) { ((uint64_t *)*t)[i] = c | BST_PTE_P | BST_PTE_W | BST_PTE_U; }
static inline void bst_pt_put_frame(bst_pt *t, uint64_t i, bst_frame f, bool w) {
    ((uint64_t *)*t)[i] = f | BST_PTE_P | BST_PTE_U | (w ? BST_PTE_W : 0);
}
static inline uint64_t bst_pt_take(bst_pt *t, uint64_t i) {
    uint64_t *e = &((uint64_t *)*t)[i];
    uint64_t a = *e & BST_PTE_ADDR;
    *e = 0;
    return a;
}
#define bst_pt_take_frame bst_pt_take
"#;

// The freestanding runtime. Part of the trusted base: the only code that
// touches I/O ports and control registers.
const PRELUDE_KERNEL: &str = r#"
static inline void bst_outb(uint16_t port, uint8_t v) { __asm__ volatile ("outb %0, %1" : : "a"(v), "Nd"(port)); }
static inline uint8_t bst_inb(uint16_t port) { uint8_t v; __asm__ volatile ("inb %1, %0" : "=a"(v) : "Nd"(port)); return v; }

#define BST_COM1 0x3F8
static void bst_serial_init(void) {
    bst_outb(BST_COM1 + 1, 0x00);   /* interrupts off while configuring */
    bst_outb(BST_COM1 + 3, 0x80);   /* DLAB on */
    bst_outb(BST_COM1 + 0, 0x01);   /* 115200 baud */
    bst_outb(BST_COM1 + 1, 0x00);
    bst_outb(BST_COM1 + 3, 0x03);   /* 8N1 */
    bst_outb(BST_COM1 + 2, 0xC7);   /* FIFO on */
    bst_outb(BST_COM1 + 4, 0x0B);   /* DTR, RTS, OUT2 (routes the IRQ line) */
    bst_outb(BST_COM1 + 1, 0x01);   /* interrupt on received data */
}
static inline void bst_serial_write(bst_serial s, uint64_t b) {
    (void)s;
    while ((bst_inb(BST_COM1 + 5) & 0x20) == 0) { }
    bst_outb(BST_COM1, (uint8_t)b);
}
static inline uint64_t bst_serial_read(bst_serial s) {
    (void)s;
    while ((bst_inb(BST_COM1 + 5) & 0x01) == 0) { __asm__ volatile ("pause"); }
    return bst_inb(BST_COM1);
}
static inline uint64_t bst_serial_poll(bst_serial s) {
    (void)s;
    return (bst_inb(BST_COM1 + 5) & 0x01) ? bst_inb(BST_COM1) : 256;
}

static void bst_halt(void) { for (;;) __asm__ volatile ("cli; hlt"); }
static void bst_puts(const char *m) { for (; *m; m++) bst_serial_write(0, (unsigned char)*m); }
static void bst_puthex(uint64_t v) {
    bst_puts("0x");
    for (int i = 60; i >= 0; i -= 4) bst_serial_write(0, (unsigned char)"0123456789abcdef"[(v >> i) & 15]);
}

/* Memory for Buf: a fixed arena with an exact-size free list. */
#define BST_ARENA_WORDS (1u << 20)   /* 8 MiB */
static uint64_t bst_arena[BST_ARENA_WORDS];
static uint64_t bst_arena_top = 0;
typedef struct bst_free { uint64_t words; struct bst_free *next; } bst_free_t;
static bst_free_t *bst_free_list = 0;
static bst_buf bst_alloc(uint64_t n) {
    uint64_t words = (n ? n : 1) + 1;            /* one header word for the size */
    if (words < 2) words = 2;
    bst_free_t **pp = &bst_free_list;
    for (bst_free_t *f = bst_free_list; f; pp = &f->next, f = f->next) {
        if (f->words == words) {
            *pp = f->next;
            uint64_t *blk = (uint64_t *)f;
            for (uint64_t i = 1; i < words; i++) blk[i] = 0;
            bst_buf b = { n, blk + 1 };
            return b;
        }
    }
    if (bst_arena_top + words > BST_ARENA_WORDS) { bst_puts("bastio: out of memory\n"); bst_halt(); }
    uint64_t *blk = &bst_arena[bst_arena_top];
    bst_arena_top += words;
    blk[0] = words;
    for (uint64_t i = 1; i < words; i++) blk[i] = 0;
    bst_buf b = { n, blk + 1 };
    return b;
}
static void bst_free(bst_buf b) {
    uint64_t *blk = b.data - 1;
    bst_free_t *f = (bst_free_t *)blk;
    f->words = blk[0];
    f->next = bst_free_list;
    bst_free_list = f;
}
static void bst_print(uint64_t v) {
    char d[20]; int n = 0;
    do { d[n++] = (char)('0' + v % 10); v /= 10; } while (v);
    while (n) bst_serial_write(0, (unsigned char)d[--n]);
    bst_serial_write(0, '\n');
}

/* Physical pages: everything from the end of the kernel image to the end of
 * RAM (from the Multiboot memory info), capped at 2^16 pages and at the
 * 1 GiB the boot code identity-maps. */
extern char _end[];
extern uint64_t bst_kernel_pml4[512];
static uint64_t bst_pool_base;
static bst_pool bst_pool_init(uint64_t mbi) {
    uint32_t *info = (uint32_t *)mbi;
    uint64_t ram_end = 64ull << 20;
    if (info && (info[0] & 1)) ram_end = (1ull << 20) + ((uint64_t)info[2] << 10);
    if (ram_end > (1ull << 30)) ram_end = 1ull << 30;
    uint64_t base = ((uint64_t)_end + 4095) & ~4095ull;
    bst_pool p;
    p.base = base;
    p.size = ram_end > base ? (ram_end - base) / 4096 : 0;
    if (p.size > 65536) p.size = 65536;
    bst_pool_base = base;
    return p;
}
#define BST_PT_NEW_HOOK if (level == 4) e[0] = ((uint64_t)bst_kernel_pml4[0] & BST_PTE_ADDR) | BST_PTE_P | BST_PTE_W;
"#;

// Bytes on the freestanding targets: carved from the same arena as Buf.
const PRELUDE_BYTES_ARENA: &str = r#"
static bst_bytes bst_balloc(uint64_t n) {
    bst_buf w = bst_alloc((n + 7) / 8);
    bst_bytes b = { n, (unsigned char *)w.data };
    return b;
}
static void bst_bfree(bst_bytes b) {
    bst_buf w = { (b.len + 7) / 8, (uint64_t *)b.data };
    bst_free(w);
}
/* Grow a block of `old` words to `n` words, keeping its contents. */
static uint64_t *bst_regrow(uint64_t *p, uint64_t old, uint64_t n) {
    uint64_t *q = bst_alloc(n).data;
    if (p) {
        for (uint64_t i = 0; i < old; i++) q[i] = p[i];
        bst_buf b = { old, p };
        bst_free(b);
    }
    return q;
}
"#;

// Child processes (hosted only): fork and exec with a pipe each way.
const PRELUDE_CHILD: &str = r#"
#include <unistd.h>
#include <signal.h>
#include <sys/wait.h>
typedef struct { int pid; int in; FILE *out; } bst_child;
/* A child that cannot be started (no such program, no pipes or processes
 * left) behaves like one that exited with 127 without output, which is
 * what a failed exec in the child looks like anyway. */
static bst_child bst_host_spawn(bst_host h, bst_vec argv) {
    (void)h;
    bst_child c = { -1, -1, NULL };
    int to[2], from[2];
    if (argv.len == 0 || pipe(to) != 0) return c;
    if (pipe(from) != 0) { close(to[0]); close(to[1]); return c; }
    char **av = (char **)malloc((argv.len + 1) * sizeof(char *));
    if (!av) { fputs("Bastio: out of memory\n", stderr); abort(); }
    for (uint64_t i = 0; i < argv.len; i++) av[i] = bst_cpath(((bst_bytes *)argv.data)[i]);
    av[argv.len] = NULL;
    fflush(NULL);
    int pid = fork();
    if (pid == 0) {
        dup2(to[0], 0);
        dup2(from[1], 1);
        close(to[0]); close(to[1]); close(from[0]); close(from[1]);
        execvp(av[0], av);
        _exit(127);
    }
    for (uint64_t i = 0; i < argv.len; i++) free(av[i]);
    free(av);
    close(to[0]);
    close(from[1]);
    if (pid < 0) { close(to[1]); close(from[0]); return c; }
    signal(SIGPIPE, SIG_IGN);   /* writing to a child that has exited fails instead */
    c.pid = pid;
    c.in = to[1];
    c.out = fdopen(from[0], "rb");
    if (!c.out) { fputs("Bastio: out of memory\n", stderr); abort(); }
    return c;
}
static bool bst_child_write(bst_child *c, bst_bytes d) {
    if (c->in < 0) return false;
    uint64_t n = 0;
    while (n < d.len) {
        ssize_t w = write(c->in, d.data + n, d.len - n);
        if (w <= 0) return false;
        n += (uint64_t)w;
    }
    return true;
}
static void bst_child_close(bst_child *c) {
    if (c->in >= 0) { close(c->in); c->in = -1; }
}
static bool bst_child_read_line(bst_child *c, bst_bytes *out) {
    if (!c->out) return false;
    size_t cap = 64, n = 0;
    unsigned char *d = (unsigned char *)malloc(cap);
    if (!d) { fputs("Bastio: out of memory\n", stderr); abort(); }
    int ch;
    while ((ch = getc(c->out)) != EOF && ch != '\n') {
        if (n == cap) {
            cap *= 2;
            d = (unsigned char *)realloc(d, cap);
            if (!d) { fputs("Bastio: out of memory\n", stderr); abort(); }
        }
        d[n++] = (unsigned char)ch;
    }
    if (ch == EOF && n == 0) { free(d); return false; }
    free(out->data);
    out->len = n;
    out->data = d;
    return true;
}
static uint64_t bst_child_wait(bst_child c) {
    bst_child_close(&c);
    if (c.out) fclose(c.out);
    if (c.pid < 0) return 127;
    int st;
    while (waitpid(c.pid, &st, 0) < 0) {}
    if (WIFEXITED(st)) return (uint64_t)WEXITSTATUS(st);
    if (WIFSIGNALED(st)) return 128 + (uint64_t)WTERMSIG(st);
    return 255;
}
"#;

// Bytes operations shared by every target.
const PRELUDE_BYTES: &str = r#"
/* Box[T]: one allocation from the same allocator as Buf. */
static void *bst_box_alloc(uint64_t n) { return bst_alloc((n + 7) / 8).data; }
static void bst_box_free(void *p, uint64_t n) { bst_buf w = { (n + 7) / 8, (uint64_t *)p }; bst_free(w); }

/* Vec[T]: elements in one allocation that doubles when full. A size past
 * 2^60 bytes asks the allocator for more than it can give, so it aborts
 * (out of memory), as `alloc` does. */
typedef struct { uint64_t len, cap; void *data; } bst_vec;
static void bst_vec_release(bst_vec v, uint64_t size) {
    if (v.data) { bst_buf b = { (v.cap * size + 7) / 8, (uint64_t *)v.data }; bst_free(b); }
}
static void bst_vec_grow(bst_vec *v, uint64_t size) {
    uint64_t ncap = v->cap ? v->cap * 2 : 4;
    uint64_t bytes = ncap > (UINT64_C(1) << 60) / size ? ~UINT64_C(0) - 8 : ncap * size;
    v->data = bst_regrow((uint64_t *)v->data, (v->cap * size + 7) / 8, (bytes + 7) / 8);
    v->cap = ncap;
}
static inline void bst_bset(bst_bytes *b, uint64_t i, uint64_t v) { b->data[i] = (unsigned char)v; }
static bst_bytes bst_bfrom(bst_str s) {
    bst_bytes b = bst_balloc(s.len);
    for (uint64_t i = 0; i < s.len; i++) b.data[i] = s.data[i];
    return b;
}
"#;

// Kernel-only: CR3 and user mode. The platform supplies the trap machinery.
const PRELUDE_KERNEL_CPU: &str = r#"
static inline void bst_load_cr3(uint64_t a) { __asm__ volatile ("mov %0, %%cr3" : : "r"(a) : "memory"); }
static inline void bst_mmu_switch(bst_mmu *m, bst_pt root) { (void)m; bst_load_cr3(root); }
static inline void bst_mmu_kernel(bst_mmu *m) { (void)m; bst_load_cr3((uint64_t)bst_kernel_pml4); }
static inline bst_frame bst_pt_free_root(bst_mmu m, bst_pt t) { (void)m; return t; }

/* Saved user registers: rax rbx rcx rdx rsi rdi rbp r8-r15 (0-14), rip (15),
 * rsp (16), rflags (17), error code (18), fault address (19). */
extern uint64_t bst_cpu_ctx[20];
extern uint64_t bst_cpu_enter(void);   /* enter user mode; returns the trap vector */
extern uint64_t bst_cpu_wait(void);    /* sti; hlt; cli; returns the interrupt vector */
static inline uint64_t bst_cpu_get(bst_cpu c, uint64_t r) { (void)c; return bst_cpu_ctx[r]; }
static inline void bst_cpu_set(bst_cpu *c, uint64_t r, uint64_t v) { (void)c; bst_cpu_ctx[r] = v; }
static inline uint64_t bst_cpu_run(bst_cpu *c, bst_mmu m) {
    (void)c; (void)m;
    uint64_t cr3;
    __asm__ volatile ("mov %%cr3, %0" : "=r"(cr3));
    bst_load_cr3(cr3);                  /* flush the TLB: pages may have moved since the last run */
    return bst_cpu_enter();
}
static inline uint64_t bst_cpu_idle(bst_cpu *c) { (void)c; return bst_cpu_wait(); }

/* Called by the platform on a fault in kernel mode: that is a bug in the
 * trusted base or the compiler, never in verified code. */
void bst_kernel_fault(uint64_t vec, uint64_t rip, uint64_t cr2, uint64_t err) {
    bst_puts("\nbastio: kernel fault, vector "); bst_puthex(vec);
    bst_puts(" rip "); bst_puthex(rip); bst_puts(" cr2 "); bst_puthex(cr2); bst_puts(" err "); bst_puthex(err);
    bst_puts("\n");
    bst_halt();
}
"#;

// User programs: everything goes through system calls (`int 0x80`, number in
// rax, arguments in rdi, rsi, rdx, result in rax). The numbers are the
// BastiOS ABI.
const PRELUDE_USER: &str = r#"
enum { BST_SYS_EXIT, BST_SYS_WRITE, BST_SYS_READ, BST_SYS_SPAWN, BST_SYS_WAIT, BST_SYS_YIELD,
       BST_SYS_GETPID, BST_SYS_INFO, BST_SYS_PS, BST_SYS_KILL, BST_SYS_POWEROFF };
static inline uint64_t bst_sys(uint64_t n, uint64_t a, uint64_t b, uint64_t c) {
    uint64_t r;
    __asm__ volatile ("int $0x80" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c) : "memory");
    return r;
}
static unsigned char bst_out[256];
static uint64_t bst_out_n;
static void bst_flush(void) {
    if (bst_out_n) bst_sys(BST_SYS_WRITE, (uint64_t)bst_out, bst_out_n, 0);
    bst_out_n = 0;
}
static inline void bst_serial_write(bst_serial s, uint64_t b) {
    (void)s;
    bst_out[bst_out_n++] = (unsigned char)b;
    if (b == '\n' || bst_out_n == sizeof bst_out) bst_flush();
}
static inline uint64_t bst_serial_read(bst_serial s) { (void)s; bst_flush(); return bst_sys(BST_SYS_READ, 0, 0, 0); }

#define BST_ARENA_WORDS (1u << 13)   /* 64 KiB */
static uint64_t bst_arena[BST_ARENA_WORDS];
static uint64_t bst_arena_top = 0;
typedef struct bst_free { uint64_t words; struct bst_free *next; } bst_free_t;
static bst_free_t *bst_free_list = 0;
static bst_buf bst_alloc(uint64_t n) {
    uint64_t words = (n ? n : 1) + 1;
    if (words < 2) words = 2;
    bst_free_t **pp = &bst_free_list;
    for (bst_free_t *f = bst_free_list; f; pp = &f->next, f = f->next) {
        if (f->words == words) {
            *pp = f->next;
            uint64_t *blk = (uint64_t *)f;
            for (uint64_t i = 1; i < words; i++) blk[i] = 0;
            bst_buf b = { n, blk + 1 };
            return b;
        }
    }
    if (bst_arena_top + words > BST_ARENA_WORDS) {
        static const char m[] = "bastio: out of memory\n";
        bst_flush();
        bst_sys(BST_SYS_WRITE, (uint64_t)m, sizeof m - 1, 0);
        bst_sys(BST_SYS_EXIT, 255, 0, 0);
    }
    uint64_t *blk = &bst_arena[bst_arena_top];
    bst_arena_top += words;
    blk[0] = words;
    for (uint64_t i = 1; i < words; i++) blk[i] = 0;
    bst_buf b = { n, blk + 1 };
    return b;
}
static void bst_free(bst_buf b) {
    uint64_t *blk = b.data - 1;
    bst_free_t *f = (bst_free_t *)blk;
    f->words = blk[0];
    f->next = bst_free_list;
    bst_free_list = f;
}
static void bst_print(uint64_t v) {
    char d[20]; int n = 0;
    do { d[n++] = (char)('0' + v % 10); v /= 10; } while (v);
    while (n) bst_serial_write(0, (unsigned char)d[--n]);
    bst_serial_write(0, '\n');
}
static uint64_t bst_sys_spawn(bst_buf b, uint64_t from, uint64_t to) {
    unsigned char name[64];
    uint64_t n = 0;
    for (uint64_t i = from; i < to && n < sizeof name; i++) name[n++] = (unsigned char)b.data[i];
    return bst_sys(BST_SYS_SPAWN, (uint64_t)name, n, 0);
}
static inline uint64_t bst_sys_wait(uint64_t pid) { return bst_sys(BST_SYS_WAIT, pid, 0, 0); }
static inline uint64_t bst_sys_kill(uint64_t pid) { return bst_sys(BST_SYS_KILL, pid, 0, 0); }
static inline void bst_sys_yield(void) { bst_sys(BST_SYS_YIELD, 0, 0, 0); }
static inline uint64_t bst_sys_getpid(void) { return bst_sys(BST_SYS_GETPID, 0, 0, 0); }
static inline uint64_t bst_sys_info(uint64_t k) { return bst_sys(BST_SYS_INFO, k, 0, 0); }
static inline void bst_sys_ps(void) { bst_flush(); bst_sys(BST_SYS_PS, 0, 0, 0); }
static inline void bst_sys_poweroff(void) { bst_flush(); bst_sys(BST_SYS_POWEROFF, 0, 0, 0); }
/* Calls with numbers the language does not know: the kernel defines them.
 * sys_send lends the kernel bytes 0..n of b to read; sys_recv lends all of b
 * to overwrite. Console output is flushed first so it stays in order. */
static inline uint64_t bst_sys_call(uint64_t n, uint64_t a, uint64_t b) { bst_flush(); return bst_sys(n, a, b, 0); }
static inline uint64_t bst_sys_send(uint64_t n, uint64_t a, bst_bytes b, uint64_t len) { bst_flush(); return bst_sys(n, a, (uint64_t)b.data, len); }
static inline uint64_t bst_sys_recv(uint64_t n, uint64_t a, bst_bytes *b) { bst_flush(); return bst_sys(n, a, (uint64_t)b->data, b->len); }
"#;

fn cty(t: &Type) -> String {
    match t {
        Type::U64 => "uint64_t".into(),
        Type::Bool => "bool".into(),
        Type::Unit => "void".into(),
        Type::Buf => "bst_buf".into(),
        Type::Bytes => "bst_bytes".into(),
        Type::Host => "bst_host".into(),
        Type::Child => "bst_child".into(),
        Type::Str => "bst_str".into(),
        Type::Serial => "bst_serial".into(),
        Type::Frame => "bst_frame".into(),
        Type::PageTable => "bst_pt".into(),
        Type::Pool => "bst_pool".into(),
        Type::Mmu => "bst_mmu".into(),
        Type::Cpu => "bst_cpu".into(),
        Type::Struct(n) => format!("bst_s_{}", cid(n)),
        Type::Enum(n) => format!("bst_e_{}", cid(n)),
        Type::Generic(..) => unreachable!("removed by monomorphization"),
        Type::Box(t) => format!("{} *", cty(t)),
        Type::Vec(_) => "bst_vec".into(),
        Type::Ref(t) => cty(t),
        Type::MutRef(_) => unreachable!("written by proto"),
    }
}

/// C types for the program's structs and enums. All are declared first, so a
/// box (a pointer) can name any of them; then each is defined after the types
/// it holds by value. An enum is a tag plus a union of its payloads.
fn struct_decls(p: &Program) -> String {
    fn emit_one(n: &str, p: &Program, done: &mut Vec<String>, out: &mut String) {
        if done.iter().any(|d| d == n) {
            return;
        }
        done.push(n.to_string());
        let held: Vec<Type> = match p.structs.iter().find(|x| x.name == n) {
            Some(d) => d.fields.iter().map(|f| f.1.clone()).collect(),
            None => p.enums.iter().find(|x| x.name == n).unwrap().variants.iter().filter_map(|v| v.1.clone()).collect(),
        };
        for t in &held {
            if let Type::Struct(m) | Type::Enum(m) = t {
                emit_one(m, p, done, out);
            }
        }
        if let Some(d) = p.structs.iter().find(|x| x.name == n) {
            out.push_str(&format!("struct bst_s_{} {{", cid(n)));
            for (f, t) in &d.fields {
                out.push_str(&format!(" {} {};", cty(t), f));
            }
            out.push_str(" };\n");
        } else {
            let d = p.enums.iter().find(|x| x.name == n).unwrap();
            out.push_str(&format!("struct bst_e_{} {{ uint64_t tag;", cid(n)));
            if held.iter().any(|t| *t != Type::Unit) {
                out.push_str(" union {");
                for (v, t) in &d.variants {
                    if let Some(t) = t.as_ref().filter(|t| **t != Type::Unit) {
                        out.push_str(&format!(" {} {};", cty(t), v));
                    }
                }
                out.push_str(" } u;");
            }
            out.push_str(" };\n");
        }
    }
    let mut out = String::new();
    for d in &p.structs {
        out.push_str(&format!("typedef struct bst_s_{0} bst_s_{0};\n", cid(&d.name)));
    }
    for d in &p.enums {
        out.push_str(&format!("typedef struct bst_e_{0} bst_e_{0};\n", cid(&d.name)));
    }
    let mut done = Vec::new();
    for n in p.structs.iter().map(|d| &d.name).chain(p.enums.iter().map(|d| &d.name)) {
        emit_one(n, p, &mut done, &mut out);
    }
    out
}

/// A C identifier for a struct, enum or function name. Generic instances
/// (`Pair[u64, Buf]`) are spelled with `_L`, `_C` and `_R`; plain names are
/// unchanged.
fn cid(n: &str) -> String {
    let mut s = String::new();
    for c in n.chars() {
        match c {
            '[' => s.push_str("_L"),
            ']' => s.push_str("_R"),
            ',' => s.push_str("_C"),
            ' ' => {}
            c => s.push(c),
        }
    }
    s
}

/// The C identifier part for a boxed type's helpers.
fn box_helper(t: &Type) -> String {
    t.name().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect()
}

fn proto(f: &FnDef) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| match &p.ty {
            Type::MutRef(t) => format!("{} *v_{}", cty(t), p.name),
            t => format!("{} v_{}", cty(t), p.name),
        })
        .collect();
    let ps = if params.is_empty() { "void".to_string() } else { params.join(", ") };
    format!("static {} f_{}({})", cty(&f.ret), cid(&f.name), ps)
}

fn ind(n: usize) -> String {
    "    ".repeat(n)
}

impl<'a> Gen<'a> {
    fn var(&self, n: &str) -> String {
        if self.mut_params.contains(n) { format!("(*v_{})", n) } else { format!("v_{}", n) }
    }

    /// The C lvalue for a dotted place; a box on the way is dereferenced.
    fn place(&self, path: &str) -> String {
        let mut parts = path.split('.');
        let root = parts.next().unwrap();
        let mut s = self.var(root);
        let mut t = self.types[root].clone();
        for f in parts {
            while let Type::Box(i) = t {
                s = format!("(*{})", s);
                t = *i;
            }
            s = format!("{}.{}", s, f);
            t = struct_def(t.name().as_str()).unwrap().field(f).unwrap().clone();
        }
        s
    }

    /// The element type of the vector a `vec_*` call's first argument names.
    fn vec_elem(&self, a: &Expr) -> Type {
        let t = match &a.kind {
            ExprKind::MutBorrow(n) => self.place_type(n),
            _ => self.type_of(a),
        };
        let Type::Vec(el) = t else { unreachable!("checked: a Vec") };
        *el
    }

    fn place_type(&self, path: &str) -> Type {
        let mut parts = path.split('.');
        let mut t = self.types[parts.next().unwrap()].clone();
        for f in parts {
            while let Type::Box(i) = t {
                t = *i;
            }
            t = struct_def(t.name().as_str()).unwrap().field(f).unwrap().clone();
        }
        t
    }

    /// The type of a code expression (the checker has already accepted it).
    fn type_of(&self, e: &Expr) -> Type {
        match &e.kind {
            ExprKind::TypeApp(targs, _) => Type::Vec(Box::new(targs[0].clone())), // vec_new[T]()
            ExprKind::Int(_) => Type::U64,
            ExprKind::Bool(_) | ExprKind::Not(_) | ExprKind::Is(..) => Type::Bool,
            ExprKind::Str(_) | ExprKind::Blob(_) => Type::Str,
            ExprKind::Var(n) => self.types[n].clone(),
            ExprKind::Field(b, f) => {
                let mut t = self.type_of(b);
                while let Type::Box(i) = t {
                    t = *i;
                }
                struct_def(t.name().as_str()).unwrap().field(f).unwrap().clone()
            }
            ExprKind::StructLit(n, _) => Type::Struct(n.clone()),
            ExprKind::Variant(n, ..) => Type::Enum(n.clone()),
            ExprKind::Bin(op, ..) => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => Type::U64,
                _ => Type::Bool,
            },
            ExprKind::Call(n, args) if n == "box" => Type::Box(Box::new(self.type_of(&args[0]))),
            ExprKind::Call(n, args) if n == "unbox" => match self.type_of(&args[0]) {
                Type::Box(i) => *i,
                _ => unreachable!("checked"),
            },
            ExprKind::Call(n, args) if crate::check::VEC_BUILTINS.contains(&n.as_str()) => match n.as_str() {
                "vec_len" => Type::U64,
                "vec_push" | "vec_set" | "vec_free" => Type::Unit,
                _ => self.vec_elem(&args[0]),
            },
            ExprKind::Call(n, _) => self.sigs[n].ret.clone(),
            ExprKind::MutBorrow(_) | ExprKind::Forall(..) => unreachable!("not a value"),
        }
    }

    fn stmt(&mut self, s: &Stmt, d: usize, out: &mut String) {
        match &s.kind {
            StmtKind::Let { name, ty, init, .. } => {
                let e = self.expr(init);
                self.types.insert(name.clone(), ty.clone().expect("typed by checker"));
                out.push_str(&format!("{}{} v_{} = {};\n", ind(d), cty(ty.as_ref().expect("typed by checker")), name, e));
            }
            StmtKind::Assign { name, value } => {
                let e = self.expr(value);
                out.push_str(&format!("{}{} = {};\n", ind(d), self.place(name), e));
            }
            StmtKind::If { cond, then_b, else_b } => {
                let c = self.expr(cond);
                out.push_str(&format!("{}if ({}) {{\n", ind(d), c));
                for s in then_b {
                    self.stmt(s, d + 1, out);
                }
                if else_b.is_empty() {
                    out.push_str(&format!("{}}}\n", ind(d)));
                } else {
                    out.push_str(&format!("{}}} else {{\n", ind(d)));
                    for s in else_b {
                        self.stmt(s, d + 1, out);
                    }
                    out.push_str(&format!("{}}}\n", ind(d)));
                }
            }
            StmtKind::While { cond, body, .. } => {
                let c = self.expr(cond);
                out.push_str(&format!("{}while ({}) {{\n", ind(d), c));
                for s in body {
                    self.stmt(s, d + 1, out);
                }
                out.push_str(&format!("{}}}\n", ind(d)));
            }
            StmtKind::Match { scrut, arms } => {
                // The enum comes from the scrutinee's type (a pattern may
                // leave out a generic enum's type arguments).
                let e = self.expr(scrut);
                let en = self.type_of(scrut).name();
                let d_en = enum_def(&en).unwrap();
                let m = format!("m_{}", self.tmps);
                self.tmps += 1;
                out.push_str(&format!("{}{{\n{}bst_e_{} {} = {};\n", ind(d), ind(d + 1), cid(&en), m, e));
                // The match is exhaustive (checked), so the last arm needs no test.
                for (i, a) in arms.iter().enumerate() {
                    let kw = if i == 0 { "if" } else { "} else if" };
                    let last = i + 1 == arms.len();
                    match &a.pat {
                        Pat::Variant(_, v, b, _) => {
                            let k = d_en.index(v).unwrap();
                            match (i, last) {
                                (0, true) => out.push_str(&format!("{}{{\n", ind(d + 1))),
                                (_, true) => out.push_str(&format!("{}}} else {{\n", ind(d + 1))),
                                _ => out.push_str(&format!("{}{} ({}.tag == {}) {{\n", ind(d + 1), kw, m, k)),
                            }
                            if let Some(b) = b.as_ref().filter(|b| *b != "_") {
                                let pt = d_en.variants[k].1.as_ref().unwrap();
                                self.types.insert(b.clone(), pt.clone());
                                out.push_str(&format!("{}{} v_{} = {}.u.{};\n", ind(d + 2), cty(pt), b, m, v));
                            }
                        }
                        Pat::Wild if i == 0 => out.push_str(&format!("{}{{\n", ind(d + 1))),
                        Pat::Wild => out.push_str(&format!("{}}} else {{\n", ind(d + 1))),
                    }
                    for s in &a.body {
                        self.stmt(s, d + 2, out);
                    }
                }
                if !arms.is_empty() {
                    out.push_str(&format!("{}}}\n", ind(d + 1)));
                }
                out.push_str(&format!("{}}}\n", ind(d)));
            }
            StmtKind::Return(Some(e)) => {
                let e = self.expr(e);
                out.push_str(&format!("{}return {};\n", ind(d), e));
            }
            StmtKind::Return(None) => out.push_str(&format!("{}return;\n", ind(d))),
            StmtKind::Assert(_) => {} // ghost: proved at compile time, erased
            StmtKind::Unreachable => out.push_str(&format!("{}__builtin_unreachable();\n", ind(d))),
            StmtKind::Expr(e) => {
                let e = self.expr(e);
                out.push_str(&format!("{}{};\n", ind(d), e));
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::TypeApp(..) => "((bst_vec){ 0, 0, 0 })".to_string(), // vec_new[T]()
            ExprKind::Int(v) => format!("UINT64_C({})", v),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(bytes) => {
                let mut lit = String::new();
                for b in bytes {
                    lit.push_str(&format!("\\x{:02x}", b));
                }
                format!("((bst_str){{ {}, (const unsigned char *)\"{}\" }})", bytes.len(), lit)
            }
            ExprKind::Blob(bytes) => {
                self.blobs.push(bytes.clone());
                format!("((bst_str){{ {}, bst_blob{} }})", bytes.len(), self.blobs.len() - 1)
            }
            ExprKind::Var(n) => self.var(n),
            ExprKind::MutBorrow(n) => {
                if self.mut_params.contains(n) { format!("v_{}", n) } else { format!("&{}", self.place(n)) }
            }
            ExprKind::Field(..) => self.place(&place(e).expect("checked: a place")),
            ExprKind::StructLit(n, fs) => {
                let xs: Vec<&Expr> = fs.iter().map(|(_, x)| x).collect();
                let (pre, vals) = self.operands(&xs, false);
                let inits: Vec<String> = fs.iter().zip(vals).map(|((f, _), v)| format!(".{} = {}", f, v)).collect();
                seq(pre, format!("((bst_s_{}){{ {} }})", cid(n), inits.join(", ")))
            }
            ExprKind::Variant(n, v, payload) => match payload {
                Some(x) => format!("((bst_e_{}){{ .tag = {}, .u.{} = {} }})", cid(n), enum_def(n).unwrap().index(v).unwrap(), v, self.expr(x)),
                None => format!("((bst_e_{}){{ .tag = {} }})", cid(n), enum_def(n).unwrap().index(v).unwrap()),
            },
            ExprKind::Is(x, _, v) => {
                let n = self.type_of(x).name();
                format!("({}.tag == {})", self.expr(x), enum_def(&n).unwrap().index(v).unwrap())
            }
            ExprKind::Not(x) => format!("(!{})", self.expr(x)),
            // && and || are sequenced in C already (and must stay short-circuit).
            ExprKind::Bin(op, a, b) if matches!(op, BinOp::And | BinOp::Or) => format!("({} {} {})", self.expr(a), op.text(), self.expr(b)),
            ExprKind::Bin(op, a, b) => {
                let (pre, v) = self.operands(&[a, b], false);
                seq(pre, format!("({} {} {})", v[0], op.text(), v[1]))
            }
            ExprKind::Forall(..) => unreachable!("specification only"),
            ExprKind::Call(name, args) if crate::check::VEC_BUILTINS.contains(&name.as_str()) => {
                let el = self.vec_elem(&args[0]);
                if !self.vecs.contains(&el) {
                    self.vecs.push(el.clone());
                }
                let (pre, a) = self.operands(&args.iter().collect::<Vec<_>>(), true);
                seq(pre, if name == "vec_len" {
                    format!("{}.len", a[0])
                } else {
                    format!("bst_{}_{}({})", name, box_helper(&el), a.join(", "))
                })
            }
            ExprKind::Call(name, args) if name == "box" || name == "unbox" => {
                let t = self.type_of(&args[0]);
                let inner = if name == "box" { t } else { let Type::Box(i) = t else { unreachable!("checked") }; *i };
                if !self.boxed.contains(&inner) {
                    self.boxed.push(inner.clone());
                }
                if name == "unbox" {
                    return format!("bst_unbox_{}({})", box_helper(&inner), self.expr(&args[0]));
                }
                // Allocate the box before evaluating its contents, so a
                // recursive structure is laid out parent first (preorder),
                // the order a traversal reads it in. Allocation has no
                // observable effect, so moving it earlier changes nothing else.
                let c = cty(&inner);
                let p = format!("bp_{}", self.tmps);
                self.tmps += 1;
                format!("({{ {c} *{p} = ({c} *)bst_box_alloc(sizeof({c})); *{p} = {}; {p}; }})", self.expr(&args[0]))
            }
            ExprKind::Call(name, args) => {
                let (pre, a) = self.operands(&args.iter().collect::<Vec<_>>(), true);
                seq(pre, match name.as_str() {
                    "len" => format!("{}.len", a[0]),
                    "get" => format!("{}.data[{}]", a[0], a[1]),
                    "slen" => format!("{}.len", a[0]),
                    "sget" => format!("((uint64_t){}.data[{}])", a[0], a[1]),
                    "blen" => format!("{}.len", a[0]),
                    "bget" => format!("((uint64_t){}.data[{}])", a[0], a[1]),
                    n if crate::check::is_builtin(n) => format!("bst_{}({})", name, a.join(", ")),
                    _ => format!("f_{}({})", cid(name), a.join(", ")),
                })
            }
        }
    }

    /// Emits the operands of a call, binary operator or struct literal so
    /// they run in the order the verifier assumes. C leaves that order
    /// unspecified, so when any operand has a side effect, each one is
    /// evaluated into a temporary first (the returned prefix). For a call
    /// (`call`), the verifier reads scalars and computed values left to
    /// right, then the linear places and `&mut` borrows (plain reads with no
    /// effect), so those stay inline and are read after every temporary.
    /// Without side effects the order cannot matter and nothing changes.
    fn operands(&mut self, xs: &[&Expr], call: bool) -> (String, Vec<String>) {
        let late = |cg: &Self, x: &Expr| {
            call && match &x.kind {
                ExprKind::MutBorrow(_) => true,
                ExprKind::Var(_) | ExprKind::Field(..) => !matches!(cg.type_of(x).base(), Type::U64 | Type::Bool | Type::Str),
                _ => false,
            }
        };
        if !xs.iter().any(|x| has_effect(x)) {
            return (String::new(), xs.iter().map(|x| self.expr(x)).collect());
        }
        let mut pre = String::new();
        let mut vals = vec![String::new(); xs.len()];
        for (i, x) in xs.iter().enumerate() {
            if !late(self, x) {
                let t = format!("o_{}", self.tmps);
                self.tmps += 1;
                pre.push_str(&format!("__auto_type {} = {}; ", t, self.expr(x)));
                vals[i] = t;
            }
        }
        for (i, x) in xs.iter().enumerate() {
            if late(self, x) {
                vals[i] = self.expr(x);
            }
        }
        (pre, vals)
    }
}

/// `e` after the statements in `pre`, as one C expression (a GNU statement
/// expression when there are any).
fn seq(pre: String, e: String) -> String {
    if pre.is_empty() { e } else { format!("({{ {}{}; }})", pre, e) }
}

/// Whether evaluating `e` may have a side effect: any call other than the
/// read-only accessors.
fn has_effect(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Call(n, args) => {
            !matches!(n.as_str(), "len" | "get" | "slen" | "sget" | "blen" | "bget" | "vec_len") || args.iter().any(has_effect)
        }
        ExprKind::StructLit(_, fs) => fs.iter().any(|(_, x)| has_effect(x)),
        ExprKind::Variant(_, _, p) => p.as_deref().map_or(false, has_effect),
        ExprKind::Is(x, ..) | ExprKind::Not(x) => has_effect(x),
        ExprKind::Bin(_, a, b) => has_effect(a) || has_effect(b),
        _ => false,
    }
}

/// The full prelude for a target (the page and CPU parts are spliced in here
/// so each piece above stays readable on its own).
/// Each part is named, so `--trusted-base` can list them.
pub fn prelude_parts(target: Target) -> Vec<(&'static str, &'static str)> {
    match target {
        Target::Hosted => vec![
            // POSIX for the child-process functions; set before any header.
            ("posix", "#define _POSIX_C_SOURCE 200809L\n"),
            ("common", PRELUDE_COMMON),
            ("hosted", PRELUDE_HOSTED),
            ("bytes", PRELUDE_BYTES),
            ("pt_hook_hosted", "#define BST_PT_NEW_HOOK (void)level;\n"),
            ("pages", PRELUDE_PAGES),
            ("child", PRELUDE_CHILD),
        ],
        Target::Kernel => vec![
            ("common", PRELUDE_COMMON),
            ("kernel", PRELUDE_KERNEL),
            ("bytes_arena", PRELUDE_BYTES_ARENA),
            ("bytes", PRELUDE_BYTES),
            ("pages", PRELUDE_PAGES),
            ("kernel_cpu", PRELUDE_KERNEL_CPU),
        ],
        Target::User => vec![("common", PRELUDE_COMMON), ("user", PRELUDE_USER), ("bytes_arena", PRELUDE_BYTES_ARENA), ("bytes", PRELUDE_BYTES)],
    }
}

/// The hand-written trusted base, listed and counted (ROADMAP invariant 3):
/// every prelude part with its count of non-blank lines, the parts each
/// target links, and every builtin with where it is available. The output
/// is checked into `tests/trusted_base.txt`, so any growth shows up in a
/// diff for review.
pub fn trusted_base() -> String {
    let mut out = String::from("# The trusted base of bastioc0 (ROADMAP invariant 3). Regenerate with\n# `bastioc0 --trusted-base > tests/trusted_base.txt` and review the diff.\n");
    let lines = |s: &str| s.lines().filter(|l| !l.trim().is_empty()).count();
    let mut seen: Vec<&str> = Vec::new();
    for (tn, t) in [("hosted", Target::Hosted), ("kernel", Target::Kernel), ("user", Target::User)] {
        let parts = prelude_parts(t);
        let total: usize = parts.iter().map(|p| lines(p.1)).sum();
        out.push_str(&format!("target {}: {} prelude lines ({})\n", tn, total, parts.iter().map(|p| p.0).collect::<Vec<_>>().join(" ")));
        for (n, s) in parts {
            if !seen.contains(&n) {
                seen.push(n);
                out.push_str(&format!("  prelude {}: {} lines\n", n, lines(s)));
            }
        }
    }
    for (name, avail) in crate::check::builtin_list() {
        out.push_str(&format!("builtin {} ({})\n", name, avail));
    }
    out
}
