#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
typedef struct Tree Tree; struct Node;
struct Tree { int tag; union { uint64_t leaf; struct Node *br; }; };
struct Node { Tree l, r; };
static Tree build(uint64_t d) { Tree t; if (d == 0) { t.tag = 0; t.leaf = 1; return t; }
  struct Node *n = malloc(sizeof *n); n->l = build(d - 1); n->r = build(d - 1); t.tag = 1; t.br = n; return t; }
static uint64_t count(const Tree *t) { return t->tag == 0 ? t->leaf % 2 : count(&t->br->l) + count(&t->br->r); }
static void drop(Tree t) { if (t.tag) { struct Node n = *t.br; free(t.br); drop(n.l); drop(n.r); } }
int main(void) { uint64_t s = 0; for (int r = 0; r < 20; r++) { Tree t = build(20); s += count(&t); drop(t); }
  printf("%llu\n", (unsigned long long)s); return 0; }
