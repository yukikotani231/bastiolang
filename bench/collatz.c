#include <stdint.h>
#include <stdio.h>
#define N 1048576ULL
static uint64_t steps(uint64_t n) {
  uint64_t x = n, c = 0;
  while (x != 1 && c < 1000) {
    if (x % 2 == 0) x = x / 2;
    else if (x <= 384307168202282325ull) x = 3 * x + 1;
    else x = 1;
    c++;
  }
  return c;
}
static uint64_t gcd(uint64_t a, uint64_t b) {
  uint64_t x = a, y = b, k = 0;
  while (y != 0 && k < 64) { uint64_t t = x % y; x = y; y = t; k++; }
  return x;
}
static uint64_t pop(uint64_t n) {
  uint64_t x = n, c = 0;
  for (uint64_t k = 0; k < 64; k++) { c += x & 1; x >>= 1; }
  return c;
}
int main(void) {
  uint64_t t = 0;
  for (uint64_t i = 1; i <= N; i++) {
    t += steps(i);
    t += gcd(i, 1000003 % i + i);
    t += pop(i * 2654435761ull);
  }
  printf("%llu\n", (unsigned long long)t);
  return 0;
}
