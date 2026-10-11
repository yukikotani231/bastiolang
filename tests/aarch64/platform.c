/* The AArch64 boot test's platform, the part in C (see boot.S). */
#include <stdint.h>
#include <stddef.h>

uint64_t bst_cpu_wait(void) {
    __asm__ volatile ("msr daifclr, #2; wfi; msr daifset, #2" : : : "memory");
    return 32;
}
uint64_t bst_platform_cpus(void) { return 1; }
void bst_platform_smp_start(void (*fn)(void *), void *arg) { (void)fn; (void)arg; }
void bst_platform_smp_join(void) { }
uint64_t bst_platform_acpi(uint64_t sig, uint64_t off) { (void)sig; (void)off; return 256; }
uint64_t bst_platform_info(uint64_t what, uint64_t i) { (void)what; (void)i; return 0; }

/* GCC may call these even in freestanding code. */
void *memset(void *d, int c, size_t n) { unsigned char *p = d; while (n--) *p++ = (unsigned char)c; return d; }
void *memcpy(void *d, const void *s, size_t n) { unsigned char *p = d; const unsigned char *q = s; while (n--) *p++ = *q++; return d; }
void *memmove(void *d, const void *s, size_t n) {
    unsigned char *p = d; const unsigned char *q = s;
    if (p < q) while (n--) *p++ = *q++;
    else while (n--) p[n] = q[n];
    return d;
}
