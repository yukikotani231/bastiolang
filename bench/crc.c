#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
int main(void) {
  uint8_t *b = malloc(1048576); uint64_t x = 12345;
  for (int i = 0; i < 1048576; i++) { x = (x * 1103515245 + 12345) % 4294967296ull; b[i] = (uint8_t)(x / 65536 % 256); }
  uint64_t crc = 4294967295ull;
  for (int p = 0; p < 8; p++) for (int i = 0; i < 1048576; i++) {
    crc = (crc ^ b[i]) % 4294967296ull;
    for (int k = 0; k < 8; k++) { if (crc % 2 == 1) crc = (crc / 2 ^ 3988292384ull) % 4294967296ull; else crc = crc / 2; }
  }
  printf("%llu\n", (unsigned long long)(crc ^ 4294967295ull)); free(b); return 0;
}
