#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
static void put(uint64_t *c, uint64_t pc, uint64_t op, uint64_t a, uint64_t b, uint64_t d) {
  c[pc * 4] = op; c[pc * 4 + 1] = a; c[pc * 4 + 2] = b; c[pc * 4 + 3] = d;
}
int main(void) {
  uint64_t *code = calloc(64, 8), *r = calloc(8, 8);
  put(code, 0, 1, 1, 1, 1000000000);
  put(code, 1, 2, 3, 0, 0);
  put(code, 2, 0, 2, 2, 3);
  put(code, 3, 5, 2, 2, 0);
  put(code, 4, 1, 0, 0, 1);
  put(code, 5, 3, 0, 1, 1);
  uint64_t pc = 0;
  for (uint64_t steps = 0; steps < 30000000; steps++) {
    uint64_t op = code[pc * 4] % 8, a = code[pc * 4 + 1] % 8, b = code[pc * 4 + 2] % 8, d = code[pc * 4 + 3];
    pc = (pc + 1) % 16;
    if (op == 0) r[a] = r[b] + r[d % 8];
    else if (op == 1) r[a] = r[b] + d;
    else if (op == 2) r[a] = r[b] * r[d % 8];
    else if (op == 3) { if (r[a] < r[b]) pc = d % 16; }
    else if (op == 4) r[a] = r[b] - r[d % 8];
    else if (op == 5) r[a] = r[b] ^ r[d % 8];
    else if (op == 6) r[a] = r[b] & r[d % 8];
  }
  printf("%llu\n", (unsigned long long)(r[2] % 1000000007 + r[0])); return 0;
}
