/* Reads standard input and prints how many lines it has and how long the
 * longest one is, using the two proved functions in lines.bastio. */
#include <stdio.h>
#include <stdlib.h>
#include "lines.h"

int main(void) {
    size_t cap = 1 << 16, len = 0, got;
    uint8_t *buf = malloc(cap);
    if (!buf) return 1;
    while ((got = fread(buf + len, 1, cap - len, stdin)) > 0) {
        len += got;
        if (len == cap) {
            uint8_t *bigger = realloc(buf, cap *= 2);
            if (!bigger) { free(buf); return 1; }
            buf = bigger;
        }
    }

    uint64_t lines = count_byte(buf, len, '\n');
    uint64_t longest = 0, start = 0;
    while (start < len) {
        bst_e_Line l = line_len(buf, len, start);
        if (l.tag != bst_e_Line_Len) break;   /* cannot happen: start <= len */
        if (l.u.Len > longest) longest = l.u.Len;
        start += l.u.Len + 1;                 /* the header promises start + Len <= len */
    }
    printf("lines: %llu, longest: %llu bytes\n",
           (unsigned long long)lines, (unsigned long long)longest);
    free(buf);
    return 0;
}
