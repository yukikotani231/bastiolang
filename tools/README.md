# Developer tools

Tools for people who write Bastio. They are written in Bastio, so they are proved memory safe like any other program, and they are built with `bastioc1`:

```sh
tests/bastioc1.sh                                          # builds bastioc1, prints its path
bastioc1 -o bastiofmt tools/bastiofmt/bastiofmt.bastio
```

## bastiofmt, the formatter

```sh
bastiofmt file.bastio            # print the formatted file
bastiofmt --write a.bastio b.bastio
bastiofmt --check src/*.bastio   # list the files that are not formatted; exit status 1 if any
bastiofmt -                      # standard input to standard output
```

It lays a file out from its tokens, without a syntax tree:

- two spaces of indentation per level of `{`, `(` and `[`; a line that continues a statement is indented two more, and the lines of a `requires`, `ensures` or `invariant` clause four;
- one space on each side of a binary operator, none after a prefix `-`, `!` or `&`, none before `,` `;` `)` `]` `.`, none inside `Vec[u8]` or `f(x)`;
- at most one blank line in a row, none after `{` or before `}`; no trailing white space; one newline at the end of the file.

It never joins or splits lines, so it does not wrap long lines: where you break a line is yours to decide. The text of comments, strings and numbers is kept, and so is the number of spaces between code and a comment after it, so a column of aligned comments stays aligned.

Formatting cannot change a program. `bastiofmt` lexes its own output and compares the tokens with the input's; if they differ it prints nothing, leaves the file alone and exits with status 3. A file that does not lex exits with status 2 (run `bastioc1 --check` for the error).

`tests/tools.sh` formats every Bastio source in the repository twice and checks that the second pass changes nothing, and that the sources in `tools/` are themselves formatted.

### In Neovim and Vim

`bastiofmt -` reads standard input, so it works as a filter. The files in [`editors/vim/`](../editors/vim/README.md) set it as `formatprg` (`gq`), and with the language server below, `vim.lsp.buf.format()` does the same through the server. With [conform.nvim](https://github.com/stevearc/conform.nvim): `formatters = { bastiofmt = { command = "bastiofmt", args = { "-" } } }` and `formatters_by_ft = { bastio = { "bastiofmt" } }`.

A file that does not lex makes `bastiofmt -` fail with no output and a message on standard error, and the editor keeps the buffer.

## bastio-lsp, the language server

```sh
bastioc1 -o bastio-lsp tools/bastio-lsp/lsp.bastio
```

A [Language Server Protocol](https://microsoft.github.io/language-server-protocol/) server over standard input and output, written in Bastio, so it is proved memory safe like the compiler. What it does:

| Feature | How |
| --- | --- |
| Diagnostics | The lexical error of the buffer as you type. When a file is opened or saved (with `checkOnChange`, on every edit too), the first error of `bastioc1 --check` on the text of the buffer: syntax, types, linearity and failed proofs, with the counterexample, at the position and width the compiler gives. A library file (no `main`) is checked through a program that only imports it |
| Go to definition | Functions, `spec fn`s, structs, enums, constants, variants (`E::V`), fields (`x.f`), and local variables (`let`, parameters, `match` bindings, `forall` variables), in the file and in every file it imports (found as the compiler finds them, with `--lib` directories and `BASTIO_LIB`) |
| Hover | The signature (the whole definition for a type) and the `///` doc comment above it; for a local variable, its `let` or parameter |
| Document symbols | Top-level definitions, with variants and fields as children |
| Formatting | The whole document, with `bastiofmt`'s rules |

Definitions are found from tokens, not from a syntax tree, so they work while a file is half typed; the cost is that scopes are approximate (a local is the nearest declaration before the use in the same function) and a name that two files define is resolved to the one in the current file first. Types are not inferred, so hover on `p.a` finds a field by name, not by the type of `p`. Diagnostics from the compiler show only its first error, because the compiler stops there.

Options go in `initializationOptions`: `bastioc` (the compiler: default `$BASTIOC`, else `bastioc1` on `PATH`), `target` (`hosted`, `kernel` or `user`), `lib` (a list of directories), `checkOnChange` (run the compiler on every edit; off by default, as a program with many proofs takes a while to check). The compiler checks the text of the buffer, not the file on disk: the text is written to a file beside the document (`.bastio-lsp-NAME`, so that its imports resolve the same way) for the run and removed after, and the saved file is checked only if that directory cannot be written.

### Neovim

Add `editors/vim` to the runtime path; it detects `*.bastio`, sets two-space indentation, adds a syntax file and starts the server:

```lua
vim.opt.runtimepath:append('/path/to/bastiolang/editors/vim')
require('bastio').setup({
  cmd = { '/path/to/bastio-lsp' },
  bastioc = '/path/to/bastioc1',
  format_on_save = true,
})
```

Without the plugin, any client works, for example in Neovim 0.8 or later:

```lua
vim.api.nvim_create_autocmd('FileType', {
  pattern = 'bastio',
  callback = function(args)
    vim.lsp.start({
      name = 'bastio-lsp',
      cmd = { '/path/to/bastio-lsp' },
      root_dir = vim.fs.dirname(vim.fs.find('.git', { upward = true, path = vim.api.nvim_buf_get_name(args.buf) })[1]),
      init_options = { bastioc = '/path/to/bastioc1' },
    })
  end,
})
```

Then `vim.lsp.buf.hover()`, `vim.lsp.buf.definition()` (`<C-]>` works through `tagfunc`), `vim.lsp.buf.document_symbol()` and `vim.lsp.buf.format()` are the usual Neovim calls (Neovim 0.10 maps `K` to hover by default), and the diagnostics appear as for any server.

### Tests

`tests/tools.sh` runs `tools/bastio-lsp/test/lsp_test.py`, which speaks the protocol to the server (hover, definitions in the file and in an imported one, symbols, formatting, diagnostics with their ranges, malformed input), and, when `nvim` is installed, `editors/vim/test/nvim_test.lua`, which does the same from headless Neovim through the plugin.
