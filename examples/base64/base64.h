/* Base64 (RFC 4648), decoded and encoded by Bastio code whose memory safety
 * is proved at compile time. See README.md. */
#ifndef BASTIO_BASE64_H
#define BASTIO_BASE64_H

#include <stddef.h>
#include <stdint.h>

/* Decodes `len` bytes at `src` (strict: only what base64_encode produces is
 * accepted). On success returns 0 and stores a new buffer in *out and its
 * length in *out_len; release it with base64_free. On failure returns -1 and
 * stores in *err_at the offset of the first offending byte, or `len` when the
 * length is wrong. `src` is only read, and only within src[0..len). */
int base64_decode(const uint8_t *src, size_t len, uint8_t **out, size_t *out_len, size_t *err_at);

/* Encodes `len` bytes at `src`, with `=` padding. Returns 0 and a new buffer
 * as above, or -1 if `len` exceeds 2^59 bytes. */
int base64_encode(const uint8_t *src, size_t len, uint8_t **out, size_t *out_len);

void base64_free(uint8_t *p);

#endif
