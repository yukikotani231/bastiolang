#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define N 8192ULL
typedef struct { uint64_t key, val; } R;
int main(void) {
  R *rs = malloc(N * sizeof(R));
  uint64_t s = 12345;
  for (uint64_t i = 0; i < N; i++) {
    s = (s * 1103515245 + 12345) % 4294967296ull;
    rs[i].key = s / 16 % 1048576; rs[i].val = i;
  }
  for (uint64_t i = 1; i < N; i++) {
    R cur = rs[i];
    uint64_t j = i;
    while (j > 0 && rs[j - 1].key > cur.key) { rs[j] = rs[j - 1]; j--; }
    rs[j] = cur;
  }
  uint64_t t = 0;
  for (uint64_t i = 0; i < N; i++) t += (rs[i].key * i) ^ rs[i].val;
  printf("%llu\n", (unsigned long long)t);
  free(rs);
  return 0;
}
