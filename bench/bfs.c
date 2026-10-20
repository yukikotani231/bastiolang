#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define NODES 262144ULL
#define DEG 8ULL
#define EDGES 2097152ULL
int main(void) {
  uint64_t *edge = calloc(EDGES, 8), *dist = calloc(NODES, 8), *queue = calloc(NODES, 8);
  uint64_t x = 12345, total = 0;
  for (uint64_t i = 0; i < EDGES; i++) { x = (x * 1103515245 + 12345) % 4294967296ull; edge[i] = x / 16 % NODES; }
  for (uint64_t src = 0; src < 16; src++) {
    for (uint64_t j = 0; j < NODES; j++) dist[j] = 4294967295ull;
    uint64_t s = src * 16411 % NODES;
    dist[s] = 0; queue[0] = s;
    uint64_t head = 0, tail = 1;
    while (head < tail) {
      uint64_t v = queue[head] % NODES; head++;
      uint64_t d = dist[v];
      for (uint64_t k = 0; k < DEG; k++) {
        uint64_t w = edge[(v * DEG + k) % EDGES] % NODES;
        if (dist[w] == 4294967295ull && tail < NODES) { dist[w] = d + 1; queue[tail] = w; tail++; }
      }
      total += d;
    }
  }
  printf("%llu\n", (unsigned long long)total); free(edge); free(dist); free(queue); return 0;
}
