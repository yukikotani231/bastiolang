/* Tests the Bastio HTTP head parser through its generated C API (http.h):
 *   1. vectors: well-formed heads, and the malformed ones a strict parser
 *      must refuse (bare LF, space before the colon, line folding, ...);
 *   2. every prefix of a well-formed head, which must be Partial until the
 *      head is complete;
 *   3. a differential fuzz against an independent parser written here in C
 *      as a byte-at-a-time state machine.
 * Built under AddressSanitizer + UBSan by run.sh. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "http.h"

static int failures;
#define CHECK(c, ...) do { if (!(c)) { failures++; fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__); fprintf(stderr, __VA_ARGS__); fputc('\n', stderr); } } while (0)

/* The reference. Returns 0 for a complete head (and its field count and
 * length), 1 for an incomplete one, 2 for a malformed one. */
enum { M0, M, T0, T, V, VCR, VLF, N0, N, OWS, VAL, CR, LF, ECR, ELF };
static int ref_run(const uint8_t *s, size_t n, int st, size_t from, uint64_t *nfields, uint64_t *len) {
    static const char ver[] = "HTTP/1.";
    size_t vi = 0;
    uint64_t count = 0;
    for (size_t i = from; i < n; i++) {
        uint8_t c = s[i];
        int tok = (c >= '0' && c <= '9') || (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c && strchr("!#$%&'*+-.^_`|~", c));
        int text = (c >= 32 && c != 127) || c == '\t';
        switch (st) {
        case M0: if (!tok) return 2; st = M; break;
        case M: if (c == ' ') st = T0; else if (!tok) return 2; break;
        case T0: if (c <= 32 || c == 127) return 2; st = T; break;
        case T: if (c == ' ') { st = V; vi = 0; } else if (c <= 32 || c == 127) return 2; break;
        case V:
            if (vi < 7) { if (c != (uint8_t)ver[vi++]) return 2; }
            else if (vi == 7) { if (c != '0' && c != '1') return 2; vi++; st = VCR; }
            break;
        case VCR: if (c != '\r') return 2; st = VLF; break;
        case VLF: if (c != '\n') return 2; st = N0; break;
        case N0: if (c == '\r') st = ELF; else if (tok) st = N; else return 2; break;
        case ELF: if (c != '\n') return 2; *nfields = count; *len = i + 1; return 0;
        case N: if (c == ':') st = OWS; else if (!tok) return 2; break;
        case OWS: if (c == ' ' || c == '\t') break; /* fall through */
        case VAL: if (c == '\r') st = LF; else if (text) st = VAL; else return 2; break;
        case LF: if (c != '\n') return 2; count++; st = N0; break;
        default: return 2;
        }
            }
    return 1;
}

static int ref_head(const uint8_t *s, size_t n, uint64_t *nfields, uint64_t *len) {
    return ref_run(s, n, M0, 0, nfields, len);
}

/* A response head: the status line checked by hand, then the same header
 * state machine. */
static int ref_resp(const uint8_t *s, size_t n, uint64_t *nfields, uint64_t *len, uint64_t *status) {
    size_t e = n;
    for (size_t i = 0; i < n; i++) {
        uint8_t c = s[i];
        if (c == '\r') {
            if (i + 1 == n) return 1;
            if (s[i + 1] != '\n') return 2;
            e = i;
            break;
        }
        if ((c < 32 && c != '\t') || c == 127) return 2;
    }
    if (e == n) return 1;
    if (e < 13 || memcmp(s, "HTTP/1.", 7) != 0 || (s[7] != '0' && s[7] != '1') || s[8] != ' ' || s[12] != ' ') return 2;
    if (s[9] < '1' || s[9] > '5' || s[10] < '0' || s[10] > '9' || s[11] < '0' || s[11] > '9') return 2;
    *status = (uint64_t)(s[9] - '0') * 100 + (uint64_t)(s[10] - '0') * 10 + (uint64_t)(s[11] - '0');
    return ref_run(s, n, N0, e + 2, nfields, len);
}

static void expect_resp(const char *s, int kind) {
    size_t n = strlen(s);
    bst_e_RespHead h = http_parse_response((const uint8_t *)s, n);
    int got = h.tag == bst_e_RespHead_Ok ? 0 : h.tag == bst_e_RespHead_Partial ? 1 : 2;
    CHECK(got == kind, "%s -> %d, wanted %d", s, got, kind);
}

/* A chunk of a chunked body, by the same state-machine style: returns 0 (a
 * data chunk: size and next set; or the last chunk, last set to 1 and next
 * the end of the message), 1 for incomplete, 2 for malformed. */
static int ref_chunk(const uint8_t *s, size_t n, size_t at, uint64_t *size, uint64_t *next, int *last) {
    size_t e = n;
    for (size_t i = at; i < n; i++) {
        uint8_t c = s[i];
        if (c == '\r') {
            if (i + 1 == n) return 1;
            if (s[i + 1] != '\n') return 2;
            e = i;
            break;
        }
        if ((c < 32 && c != '\t') || c == 127) return 2;
    }
    if (e == n) return 1;
    uint64_t v = 0;
    size_t k = at;
    for (; k < e; k++) {
        int d = s[k] >= '0' && s[k] <= '9' ? s[k] - '0' : s[k] >= 'a' && s[k] <= 'f' ? s[k] - 'a' + 10 : s[k] >= 'A' && s[k] <= 'F' ? s[k] - 'A' + 10 : -1;
        if (d < 0) break;
        v = v * 16 + (uint64_t)d;
        if (v >= (1ull << 56)) return 2;
    }
    if (k == at) return 2;
    if (k < e && s[k] != ';') return 2;
    *size = v;
    *last = 0;
    if (v == 0) {
        uint64_t cnt, len;
        int r = ref_run(s, n, N0, e + 2, &cnt, &len);
        if (r == 0) { *last = 1; *next = len; }
        return r;
    }
    size_t data = e + 2;
    if (n - data < v + 2) return 1;
    if (s[data + v] != '\r' || s[data + v + 1] != '\n') return 2;
    *next = data + v + 2;
    return 0;
}

static void expect(const char *s, int kind) {
    size_t n = strlen(s);
    bst_e_Head h = http_parse_head((const uint8_t *)s, n);
    int got = h.tag == bst_e_Head_Ok ? 0 : h.tag == bst_e_Head_Partial ? 1 : 2;
    CHECK(got == kind, "%s -> %d, wanted %d", s, got, kind);
}

static void fields_of(const char *s) {
    size_t n = strlen(s);
    bst_e_Head h = http_parse_head((const uint8_t *)s, n);
    CHECK(h.tag == bst_e_Head_Ok, "fields_of: %s", s);
    if (h.tag != bst_e_Head_Ok) return;
    uint64_t at = h.u.Ok.fields, seen = 0;
    for (;;) {
        bst_e_Next f = http_field((const uint8_t *)s, n, at);
        if (f.tag == bst_e_Next_End) { CHECK(f.u.End == h.u.Ok.head_end, "end"); break; }
        CHECK(f.tag == bst_e_Next_Ok, "field tag");
        if (f.tag != bst_e_Next_Ok) return;
        seen++;
        at = f.u.Ok.next;
    }
    CHECK(seen == h.u.Ok.nfields, "count %llu vs %llu", (unsigned long long)seen, (unsigned long long)h.u.Ok.nfields);
}

int main(void) {
    const char *good = "GET /index.html?a=1 HTTP/1.1\r\nHost: example.org\r\nAccept:  text/html \t\r\nX-Empty:\r\n\r\n";
    expect(good, 0);
    fields_of(good);
    bst_e_Head h = http_parse_head((const uint8_t *)good, strlen(good));
    CHECK(h.u.Ok.method_end == 3 && h.u.Ok.target == 4 && h.u.Ok.target_end == 19 && h.u.Ok.minor == 1, "request line");
    CHECK(h.u.Ok.nfields == 3 && h.u.Ok.head_end == strlen(good), "fields %llu", (unsigned long long)h.u.Ok.nfields);
    bst_e_Next f = http_field((const uint8_t *)good, strlen(good), h.u.Ok.fields);
    CHECK(f.tag == bst_e_Next_Ok && f.u.Ok.name_end == 34 && memcmp(good + f.u.Ok.value, "example.org", 11) == 0 && f.u.Ok.value_end - f.u.Ok.value == 11, "Host field");
    expect("POST / HTTP/1.0\r\n\r\nbody", 0);
    expect("", 1);
    expect("GET / HTTP/1.1\r\nHost: a\r\n", 1);
    expect("GET / HTTP/1.1\r\nHost: a\r\n\r", 1);
    /* malformed */
    expect("GET / HTTP/1.1\nHost: a\n\n", 2);                /* bare LF */
    expect("GET / HTTP/1.1\r\nHost : a\r\n\r\n", 2);         /* space before colon */
    expect("GET / HTTP/1.1\r\nHost: a\r\n b\r\n\r\n", 2);    /* line folding */
    expect("GET / HTTP/1.1\r\nHost: a\rb\r\n\r\n", 2);       /* bare CR */
    expect("GET / HTTP/1.1\r\nHost: a\x01\r\n\r\n", 2);      /* control character */
    expect("GET / HTTP/2.0\r\n\r\n", 2);
    expect("GET / HTTP/1.2\r\n\r\n", 2);
    expect("GET  / HTTP/1.1\r\n\r\n", 2);                    /* two spaces */
    expect("G(T / HTTP/1.1\r\n\r\n", 2);                     /* not a token */
    expect(" GET / HTTP/1.1\r\n\r\n", 2);
    expect("GET /a b HTTP/1.1\r\n\r\n", 2);
    expect("GET / HTTP/1.1\r\n: a\r\n\r\n", 2);              /* empty name */
    expect("\r\nGET / HTTP/1.1\r\n\r\n", 2);

    /* Responses. */
    const char *rgood = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nServer: x\r\n\r\nnot found";
    expect_resp(rgood, 0);
    bst_e_RespHead rh = http_parse_response((const uint8_t *)rgood, strlen(rgood));
    CHECK(rh.u.Ok.status == 404 && rh.u.Ok.minor == 1 && rh.u.Ok.reason == 13 && rh.u.Ok.reason_end == 22 && rh.u.Ok.nfields == 2 && rh.u.Ok.head_end == strlen(rgood) - 9, "response fields");
    expect_resp("HTTP/1.0 200 \r\n\r\n", 0);                  /* empty reason phrase */
    expect_resp("HTTP/1.1 200 OK\r\n", 1);
    expect_resp("HTTP/1.1 200 OK\r\nA: b\r\n\r", 1);
    expect_resp("HTTP/1.1 200\r\n\r\n", 2);                   /* no space after the code */
    expect_resp("HTTP/1.1 20 OK\r\n\r\n", 2);
    expect_resp("HTTP/1.1 2000 OK\r\n\r\n", 2);
    expect_resp("HTTP/1.1 099 OK\r\n\r\n", 2);
    expect_resp("HTTP/1.1 600 OK\r\n\r\n", 2);
    expect_resp("HTTP/2 200 OK\r\n\r\n", 2);
    expect_resp("HTTP/1.1  200 OK\r\n\r\n", 2);
    expect_resp("HTTP/1.1 200 OK\nA: b\n\n", 2);
    for (size_t k = 0; k < strlen(rgood) - 9; k++) {
        bst_e_RespHead p = http_parse_response((const uint8_t *)rgood, k);
        CHECK(p.tag == bst_e_RespHead_Partial, "response prefix %zu is %llu", k, (unsigned long long)p.tag);
    }

    /* Chunked bodies. */
    const char *cgood = "5\r\nhello\r\nA;ext=1\r\n0123456789\r\n0\r\nTrailer: x\r\n\r\n";
    {
        size_t cn = strlen(cgood), at = 0, total = 0;
        char body[64];
        for (;;) {
            bst_e_Chunk c = http_chunk((const uint8_t *)cgood, cn, at);
            if (c.tag == bst_e_Chunk_Ok) {
                memcpy(body + total, cgood + c.u.Ok.data, c.u.Ok.size);
                total += c.u.Ok.size;
                at = c.u.Ok.next;
            } else {
                CHECK(c.tag == bst_e_Chunk_Last && c.u.Last == cn, "chunked end");
                break;
            }
        }
        CHECK(total == 15 && memcmp(body, "hello0123456789", 15) == 0, "chunked body");
        for (size_t k = 0; k < cn; k++) {
            bst_e_Chunk c = http_chunk((const uint8_t *)cgood, k, 0);
            if (k < 7) CHECK(c.tag == bst_e_Chunk_Partial, "chunk prefix %zu is %llu", k, (unsigned long long)c.tag);
            (void)c;
        }
    }
    {
        const char *bad[] = { "g\r\nx\r\n", "\r\n", "5\r\nhelloX\r\n", "5\nhello\r\n", "5 \r\nhello\r\n",
                              "100000000000000\r\n", "5;a\x01\r\nhello\r\n", "0\r\nBad Header: x\r\n\r\n" };
        for (size_t i = 0; i < sizeof bad / sizeof *bad; i++) {
            bst_e_Chunk c = http_chunk((const uint8_t *)bad[i], strlen(bad[i]), 0);
            CHECK(c.tag == bst_e_Chunk_Bad, "bad chunk %zu is %llu", i, (unsigned long long)c.tag);
        }
    }

    /* Every proper prefix of a complete head is Partial (or, for a prefix
     * that already breaks a rule, malformed). */
    size_t gn = strlen(good);
    for (size_t k = 0; k < gn; k++) {
        bst_e_Head p = http_parse_head((const uint8_t *)good, k);
        CHECK(p.tag == bst_e_Head_Partial, "prefix %zu is %llu", k, (unsigned long long)p.tag);
    }

    /* Differential fuzz: mutate the good head and compare with the reference. */
    srand(12345);
    long agree = 0;
    for (int it = 0; it < 200000; it++) {
        uint8_t buf[256];
        size_t n = gn;
        memcpy(buf, good, gn);
        int muts = 1 + rand() % 3;
        for (int m = 0; m < muts; m++) {
            size_t at = (size_t)rand() % n;
            switch (rand() % 4) {
            case 0: buf[at] = (uint8_t)rand(); break;
            case 1: buf[at] = " \r\n:\t/H"[rand() % 7]; break;
            case 2: if (n > 1) { memmove(buf + at, buf + at + 1, n - at - 1); n--; } break;
            default: if (n < sizeof buf) { memmove(buf + at + 1, buf + at, n - at); buf[at] = " \r\n:a"[rand() % 5]; n++; } break;
            }
        }
        if (rand() % 4 == 0) n = (size_t)rand() % (n + 1);
        uint64_t rc = 0, rl = 0;
        int rk = ref_head(buf, n, &rc, &rl);
        bst_e_Head p = http_parse_head(buf, n);
        int k = p.tag == bst_e_Head_Ok ? 0 : p.tag == bst_e_Head_Partial ? 1 : 2;
        /* The parser reads whole lines, so it may say Partial where the
         * state machine already sees a violation; anything else must agree. */
        int ok = k == rk || (k == 1 && rk == 2);
        CHECK(ok, "fuzz %d: parser %d, reference %d", it, k, rk);
        if (k == 0 && rk == 0) CHECK(p.u.Ok.nfields == rc && p.u.Ok.head_end == rl, "fuzz %d: counts", it);
        agree += ok;
        if (failures > 20) return 1;
    }
    /* The same for responses. */
    size_t rn0 = strlen(rgood);
    for (int it = 0; it < 200000; it++) {
        uint8_t buf[256];
        size_t n = rn0;
        memcpy(buf, rgood, rn0);
        int muts = 1 + rand() % 3;
        for (int m = 0; m < muts; m++) {
            size_t at = (size_t)rand() % n;
            switch (rand() % 4) {
            case 0: buf[at] = (uint8_t)rand(); break;
            case 1: buf[at] = " \r\n:\t/H0"[rand() % 8]; break;
            case 2: if (n > 1) { memmove(buf + at, buf + at + 1, n - at - 1); n--; } break;
            default: if (n < sizeof buf) { memmove(buf + at + 1, buf + at, n - at); buf[at] = " \r\n:a5"[rand() % 6]; n++; } break;
            }
        }
        if (rand() % 4 == 0) n = (size_t)rand() % (n + 1);
        uint64_t rc = 0, rl = 0, rs = 0;
        int rk = ref_resp(buf, n, &rc, &rl, &rs);
        bst_e_RespHead p = http_parse_response(buf, n);
        int k = p.tag == bst_e_RespHead_Ok ? 0 : p.tag == bst_e_RespHead_Partial ? 1 : 2;
        int ok = k == rk || (k == 1 && rk == 2);
        CHECK(ok, "response fuzz %d: parser %d, reference %d", it, k, rk);
        if (k == 0 && rk == 0) CHECK(p.u.Ok.nfields == rc && p.u.Ok.head_end == rl && p.u.Ok.status == rs, "response fuzz %d: counts", it);
        agree += ok;
        if (failures > 20) return 1;
    }
    /* The same for chunks, from every start offset of mutated bodies. */
    size_t cn0 = strlen(cgood);
    for (int it = 0; it < 200000; it++) {
        uint8_t buf[128];
        size_t n = cn0;
        memcpy(buf, cgood, cn0);
        int muts = 1 + rand() % 3;
        for (int m = 0; m < muts; m++) {
            size_t at = (size_t)rand() % n;
            switch (rand() % 4) {
            case 0: buf[at] = (uint8_t)rand(); break;
            case 1: buf[at] = " \r\n;:0Fg"[rand() % 8]; break;
            case 2: if (n > 1) { memmove(buf + at, buf + at + 1, n - at - 1); n--; } break;
            default: if (n < sizeof buf) { memmove(buf + at + 1, buf + at, n - at); buf[at] = "\r\n;F5"[rand() % 5]; n++; } break;
            }
        }
        if (rand() % 4 == 0) n = (size_t)rand() % (n + 1);
        size_t from = (size_t)rand() % (n + 1);
        uint64_t rsz = 0, rnx = 0;
        int rlast = 0;
        int rk = ref_chunk(buf, n, from, &rsz, &rnx, &rlast);
        bst_e_Chunk p = http_chunk(buf, n, from);
        int k = (p.tag == bst_e_Chunk_Ok || p.tag == bst_e_Chunk_Last) ? 0 : p.tag == bst_e_Chunk_Partial ? 1 : 2;
        int ok = k == rk || (k == 1 && rk == 2);
        CHECK(ok, "chunk fuzz %d: parser %d, reference %d", it, k, rk);
        if (k == 0 && rk == 0) {
            if (rlast) CHECK(p.tag == bst_e_Chunk_Last && p.u.Last == rnx, "chunk fuzz %d: last", it);
            else CHECK(p.tag == bst_e_Chunk_Ok && p.u.Ok.size == rsz && p.u.Ok.next == rnx, "chunk fuzz %d: data", it);
        }
        agree += ok;
        if (failures > 20) return 1;
    }
    if (failures) { fprintf(stderr, "%d failure(s)\n", failures); return 1; }
    printf("http: vectors, prefixes and %ld fuzz cases ok\n", agree);
    return 0;
}
