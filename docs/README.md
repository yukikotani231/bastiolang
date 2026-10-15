# Bastio documentation

Bastio is a systems language whose memory safety is proved at compile time. The [README](../README.md) says what it is, how to build the compilers and what state it is in. This page is the map of everything else. The same pages are on the [documentation website](https://yukikotani231.github.io/bastiolang/).

## Learn the language

| Page | What it covers |
| --- | --- |
| [Tutorial](tutorial.md) | A tour for newcomers: ownership, indices and arithmetic proved by Z3, specifications, calling Bastio from C. Every example is compiled by the tests |
| [Tutorial part 2: bytes, bits and sized integers](tutorial-bits.md) | `u8` to `i64`, `cast` and `wrap`, bit operators, parsing a binary format |
| [Tutorial part 3: capabilities](tutorial-capabilities.md) | The console, pages, page tables, ports, device registers and DMA, and what is proved about each |
| [The toolchain](toolchain.md) | The two compilers, options, targets, backends and what each trusts |
| [Common proof failures](proof-failures.md) | The errors people meet first, with the counterexample the compiler prints and how to fix each |
| [Examples](../examples/README.md) | Small programs and proved libraries (base64, UTF-8, DER, QOI) to read and build |

## Reference

| Page | What it covers |
| --- | --- |
| [Language reference](language.md) | Every rule of the language and its "Soundness scope": what is and is not guaranteed |
| [Sized integers](integers.md) | `u8` to `u64` and `i8` to `i64`: the design and how each step was built |
| [Concurrency](concurrency.md) | The concurrency design and what exists so far |
| [Core and std libraries](core.md) | What each library function does and what is proved; the per-module pages are in [`lib/`](lib/core-bytes.md) and are generated from the sources |

## How the compilers work

| Page | What it covers |
| --- | --- |
| [Bootstrapping](bootstrap.md) | How the Rust compiler `bastioc0` builds `bastioc1`, how to check the fixed point, and what the stage0 freeze allows |
| [Stage 1](../stage1/README.md) | The structure of `bastioc1`, the compiler written in Bastio |
| [The IR](ir.md) | The intermediate representation behind the native backend |
| [The trusted base](trusted-base.md) | Everything the proofs rest on and are not themselves proved, and how it is counted |

## The project

| Page | What it covers |
| --- | --- |
| [ROADMAP.md](../ROADMAP.md) | The direction, the invariants every change keeps, and the plan |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | The workflow and the checks CI runs |
| [Releasing](releasing.md) | How a version is cut |
| [CHANGELOG.md](../CHANGELOG.md) | What changed, by version |
| [SECURITY.md](../SECURITY.md) | How to report a soundness or security bug |
| [Editor support](../editors/README.md) | The tree-sitter grammar, the VS Code extension and the Vim/Neovim files |
| [Developer tools](../tools/README.md) | The formatter `bastiofmt` and the language server `bastio-lsp`, with Neovim setup |

## The kernel written in Bastio

BastiOS lives in its own repository, [yukikotani231/bastios](https://github.com/yukikotani231/bastios), with its own [documentation index](https://github.com/yukikotani231/bastios/blob/main/docs/README.md).
