#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define N 65536ULL
#define BOX 1048576ULL
typedef struct { uint64_t x, y, vx, vy; } P;
static void step(P *ps) {
  for (uint64_t i = 0; i < N; i++) {
    P p = ps[i];
    uint64_t x = p.x + p.vx, vx = p.vx;
    if (x >= BOX) { x = (BOX - 1) - x % BOX; vx = 0 - vx; }
    uint64_t y = p.y + p.vy, vy = p.vy;
    if (y >= BOX) { y = (BOX - 1) - y % BOX; vy = 0 - vy; }
    ps[i] = (P){x, y, vx, vy};
  }
}
int main(void) {
  P *ps = malloc(N * sizeof(P));
  uint64_t s = 12345;
  for (uint64_t i = 0; i < N; i++) {
    s = (s * 1103515245 + 12345) % 4294967296ull;
    uint64_t a = s / 65536 % 4096;
    s = (s * 1103515245 + 12345) % 4294967296ull;
    uint64_t b = s / 65536 % 4096;
    ps[i] = (P){a * 256, b * 256, a % 61 + 1, b % 53 + 1};
  }
  for (int r = 0; r < 200; r++) step(ps);
  uint64_t t = 0;
  for (uint64_t i = 0; i < N; i++) t += ps[i].x + ps[i].y * 3;
  printf("%llu\n", (unsigned long long)t); free(ps); return 0;
}
