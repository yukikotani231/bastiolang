#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define W 20000ULL
#define N 5000ULL
int main(void) {
  uint64_t *dp = calloc(W + 1, 8), x = 12345;
  for (uint64_t it = 0; it < N; it++) {
    x = x * 6364136223846793005ULL + 1442695040888963407ULL;
    uint64_t wt = x / 4294967296ULL % 200 + 1, val = x / 65536 % 1000 + 1;
    for (uint64_t w = W; w >= wt; w--) {
      uint64_t a = dp[w], b = dp[w - wt] + val;
      if (b > a) dp[w] = b;
    }
  }
  printf("%llu\n", (unsigned long long)dp[W]); return 0;
}
