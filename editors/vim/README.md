# Bastio for Vim and Neovim

A runtime directory: add it to `runtimepath` (or copy its subdirectories into `~/.vim` or `~/.config/nvim`).

| File | What it does |
| --- | --- |
| `ftdetect/bastio.vim` | `*.bastio` has the file type `bastio` |
| `ftplugin/bastio.vim` | two-space indentation, `//` comments, `gq` through `bastiofmt` when it is installed |
| `syntax/bastio.vim` | highlighting for Vim, and for Neovim without tree-sitter (for tree-sitter see [`../tree-sitter-bastio`](../tree-sitter-bastio/README.md)) |
| `indent/bastio.vim` | indentation by the nesting of brackets |
| `lua/bastio/init.lua` | Neovim: `require('bastio').setup({ ... })` starts the language server |

```lua
vim.opt.runtimepath:append('/path/to/bastiolang/editors/vim')
require('bastio').setup({
  cmd = { '/path/to/bastio-lsp' },       -- built from tools/bastio-lsp/lsp.bastio
  bastioc = '/path/to/bastioc1',         -- default: $BASTIOC, else bastioc1 on PATH
  -- target = 'user', lib = { '/path/to/std' },
  -- check_on_change = true,              -- run the compiler on every edit, not only on open and save
  format_on_save = true,
})
```

What the server does is described in [`tools/README.md`](../../tools/README.md). `test/nvim_test.lua` is run by `tests/tools.sh` in headless Neovim.
