#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
int main(void) {
  uint8_t *b = malloc(4194304); uint64_t x = 12345;
  for (int i = 0; i < 4194304; i++) { x = (x * 1103515245 + 12345) % 4294967296ull; b[i] = (uint8_t)(x / 65536 % 256); }
  uint64_t h = 14695981039346656037ull;
  for (int p = 0; p < 8; p++) for (int i = 0; i < 4194304; i++) h = (h ^ b[i]) * 1099511628211ull;
  printf("%llu\n", (unsigned long long)h); free(b); return 0;
}
