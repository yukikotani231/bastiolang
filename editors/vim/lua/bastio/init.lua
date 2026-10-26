-- Neovim: starts bastio-lsp for Bastio files.
--
--   vim.opt.runtimepath:append('/path/to/bastiolang/editors/vim')
--   require('bastio').setup({ bastioc = '/path/to/bastioc1', format_on_save = true })
--
-- Options:
--   cmd             the language server's command (default { 'bastio-lsp' })
--   bastioc         the compiler it runs for diagnostics (default $BASTIOC, else `bastioc1`)
--   target          `hosted`, `kernel` or `user`, for programs that are not hosted ones
--   lib             library directories, for imports and for the compiler
--   check_on_change run the compiler as the text changes, not only when a file is
--                   opened or saved (slow for programs with many proofs)
--   format_on_save  format the buffer through the server before writing it
local M = {}

local defaults = {
  cmd = { 'bastio-lsp' },
  bastioc = nil,
  target = nil,
  lib = {},
  check_on_change = false,
  format_on_save = false,
}

function M.setup(opts)
  opts = vim.tbl_deep_extend('force', defaults, opts or {})
  local group = vim.api.nvim_create_augroup('bastio', { clear = true })

  vim.api.nvim_create_autocmd('FileType', {
    group = group,
    pattern = 'bastio',
    callback = function(args)
      local name = vim.api.nvim_buf_get_name(args.buf)
      local git = vim.fs.find('.git', { upward = true, path = vim.fs.dirname(name) })[1]
      local init = { bastioc = opts.bastioc, target = opts.target, checkOnChange = opts.check_on_change or nil }
      if opts.lib and #opts.lib > 0 then
        init.lib = opts.lib
      end
      vim.lsp.start({
        name = 'bastio-lsp',
        cmd = opts.cmd,
        root_dir = git and vim.fs.dirname(git) or vim.fs.dirname(name),
        init_options = init,
      })
    end,
  })

  if opts.format_on_save then
    vim.api.nvim_create_autocmd('BufWritePre', {
      group = group,
      pattern = '*.bastio',
      callback = function()
        vim.lsp.buf.format({ async = false, name = 'bastio-lsp' })
      end,
    })
  end
end

return M
