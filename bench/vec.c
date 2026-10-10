#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
typedef struct { uint64_t len, cap; uint64_t *data; } vec;
static void push(vec *v, uint64_t x) { if (v->len == v->cap) { v->cap = v->cap ? v->cap * 2 : 4; v->data = realloc(v->data, v->cap * 8); } v->data[v->len++] = x; }
int main(void) { uint64_t t = 0;
  for (int r = 0; r < 20; r++) { vec v = {0}; for (uint64_t i = 0; i < 10000000; i++) push(&v, i % 1000);
    uint64_t s = 0; for (uint64_t i = 0; i < v.len; i++) s += v.data[i] % 1000; t = (t + s) % 1000000007; free(v.data); }
  printf("%llu\n", (unsigned long long)t); return 0; }
