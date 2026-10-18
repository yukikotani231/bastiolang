#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
static void step(const uint64_t *src, uint64_t *dst) {
  for (uint64_t y = 0; y < 256; y++) {
    uint64_t up = (y + 255) % 256, dn = (y + 1) % 256;
    for (uint64_t x = 0; x < 256; x++) {
      uint64_t lf = (x + 255) % 256, rt = (x + 1) % 256;
      uint64_t n = src[up*256+lf]%2 + src[up*256+x]%2 + src[up*256+rt]%2
                 + src[y*256+lf]%2 + src[y*256+rt]%2
                 + src[dn*256+lf]%2 + src[dn*256+x]%2 + src[dn*256+rt]%2;
      uint64_t me = src[y*256+x]%2, v = 0;
      if (n == 3 || (n == 2 && me == 1)) v = 1;
      dst[y*256+x] = v;
    }
  }
}
int main(void) {
  uint64_t *a = calloc(65536, 8), *b = calloc(65536, 8), s = 12345;
  for (int i = 0; i < 65536; i++) { s = (s * 1103515245 + 12345) % 4294967296ull; a[i] = s / 65536 % 2; }
  for (int g = 0; g < 200; g++) { step(a, b); uint64_t *t = a; a = b; b = t; }
  uint64_t live = 0;
  for (int i = 0; i < 65536; i++) live += a[i] % 2;
  printf("%llu\n", (unsigned long long)live); free(a); free(b); return 0;
}
