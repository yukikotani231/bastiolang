#include <stdint.h>
#include <stdio.h>
#define FULL 8191ULL
static uint64_t solve(uint64_t cols, uint64_t d1, uint64_t d2) {
  if (cols == FULL) return 1;
  uint64_t avail = FULL - ((cols | d1 | d2) & FULL), n = 0, guard = 0;
  while (avail != 0 && guard < 13) {
    uint64_t bit = avail & (0 - avail);
    if (bit <= avail) {
      avail &= avail - 1;
      uint64_t c = solve((cols | bit) & FULL, ((d1 | bit) << 1) & FULL, ((d2 | bit) >> 1) & FULL);
      if (n + c <= 100000000ULL) n += c;
    } else {
      avail = 0;
    }
    guard++;
  }
  return n;
}
int main(void) { printf("%llu\n", (unsigned long long)solve(0, 0, 0)); return 0; }
