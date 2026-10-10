# Calling Bastio from C

An `export fn` is a proved Bastio function with a plain C signature, and
`bastioc0 --header` writes the C header for it. The examples in this section
are libraries for untrusted input, compiled to C and called from a C test
driver that runs them under AddressSanitizer and UBSan. Each one's `run.sh`
builds and tests it, and `tests/run.sh` runs all of them in CI.

- [base64](examples-base64.md)
- [UTF-8](examples-utf8.md)
- [DER](examples-der.md)

The [language reference](language.md) defines `export fn`, `Bytes` and
`&mut Bytes`.
