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

### In Neovim

`bastiofmt -` reads standard input, so it works as a filter. With the tree-sitter grammar from [`editors/`](../editors/README.md) installed:

```lua
vim.filetype.add({ extension = { bastio = "bastio" } })
vim.api.nvim_create_autocmd("FileType", {
  pattern = "bastio",
  callback = function()
    vim.bo.formatprg = "bastiofmt -"      -- gq formats a selection or the file
    vim.bo.shiftwidth = 2
    vim.bo.expandtab = true
  end,
})
-- format on save; the buffer is replaced only if bastiofmt succeeds
vim.api.nvim_create_autocmd("BufWritePre", {
  pattern = "*.bastio",
  callback = function(args)
    local text = table.concat(vim.api.nvim_buf_get_lines(args.buf, 0, -1, false), "\n") .. "\n"
    local r = vim.system({ "bastiofmt", "-" }, { stdin = text }):wait()
    if r.code == 0 and r.stdout ~= text then
      local view = vim.fn.winsaveview()
      vim.api.nvim_buf_set_lines(args.buf, 0, -1, false, vim.split((r.stdout:gsub("\n$", "")), "\n"))
      vim.fn.winrestview(view)
    end
  end,
})
```

With [conform.nvim](https://github.com/stevearc/conform.nvim): `formatters = { bastiofmt = { command = "bastiofmt", args = { "-" } } }` and `formatters_by_ft = { bastio = { "bastiofmt" } }`.

A file that does not lex makes `bastiofmt -` fail with no output and a message on standard error, and the editor keeps the buffer.
