# tree-sitter-bastio

A [tree-sitter](https://tree-sitter.github.io/) grammar for [Bastio](https://github.com/yukikotani231/bastiolang), with highlight (`queries/highlights.scm`) and fold (`queries/folds.scm`) queries.

It follows stage0's parser (`stage0/src/parser.rs`), and is a little more permissive where stage0's checks are not about syntax: an empty struct, an `import` after a definition, or a non-place on the left of `=` still parse, so that an editor can highlight the file while the compiler reports the problem.

## Build and test

Needs the tree-sitter CLI (`npm install -g tree-sitter-cli`) and a C compiler.

```sh
tree-sitter generate      # writes src/ (not committed)
tree-sitter test          # the cases in test/corpus/
test/parse_all.sh         # the above, then every .bastio file in the repository
test/parse_all.sh ../../../bastios   # ... and in other directories too
```

`parse_all.sh` fails if any file has an `ERROR` or `MISSING` node in its tree. The files in `stage1/lexcases/` are skipped: they are token streams for the lexer tests, not programs, and some are malformed on purpose.

## Using it in an editor

The generated parser is not committed, so run `tree-sitter generate` first, then point the editor at this directory.

**Neovim** (nvim-treesitter):

```lua
require('nvim-treesitter.parsers').get_parser_configs().bastio = {
  install_info = {
    url = '/path/to/bastiolang/editors/tree-sitter-bastio',
    files = { 'src/parser.c' },
  },
  filetype = 'bastio',
}
vim.filetype.add({ extension = { bastio = 'bastio' } })
```

Then `:TSInstall bastio`, and copy `queries/` to `~/.config/nvim/queries/bastio/`.

**Helix** (`languages.toml`):

```toml
[[language]]
name = "bastio"
scope = "source.bastio"
file-types = ["bastio"]
comment-token = "//"
indent = { tab-width = 2, unit = "  " }

[[grammar]]
name = "bastio"
source = { path = "/path/to/bastiolang/editors/tree-sitter-bastio" }
```

Then `hx --grammar build`, and copy `queries/` to `~/.config/helix/runtime/queries/bastio/`.
