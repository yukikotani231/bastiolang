#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
int main(void) {
  uint64_t *s = calloc(4096, 8), *t = calloc(4096, 8), x = 12345;
  for (int i = 0; i < 4096; i++) {
    x = (x * 1103515245 + 12345) % 4294967296; s[i] = x / 65536 % 4;
    x = (x * 1103515245 + 12345) % 4294967296; t[i] = x / 65536 % 4;
  }
  uint64_t *prev = calloc(4097, 8), *cur = calloc(4097, 8);
  for (int i = 0; i < 4096; i++) {
    cur[0] = 0; uint64_t a = s[i];
    for (int j = 0; j < 4096; j++) {
      uint64_t v;
      if (a == t[j]) v = prev[j] + 1;
      else { uint64_t up = prev[j + 1], left = cur[j]; v = up > left ? up : left; }
      cur[j + 1] = v;
    }
    uint64_t *tmp = prev; prev = cur; cur = tmp;
  }
  printf("%llu\n", (unsigned long long)prev[4096]); return 0;
}
