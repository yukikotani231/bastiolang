#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
int main(void) {
  uint64_t t = 0;
  for (int r = 0; r < 5; r++) {
    uint64_t n = 100000000; uint64_t *b = calloc(n, 8);
    for (uint64_t i = 2; i * i < n; i++) if (b[i] == 0) for (uint64_t j = i * i; j < n; j += i) b[j] = 1;
    uint64_t c = 0; for (uint64_t i = 2; i < n; i++) if (b[i] == 0) c++;
    t += c; free(b);
  }
  printf("%llu\n", (unsigned long long)t); return 0;
}
