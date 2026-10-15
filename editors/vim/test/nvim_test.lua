-- Runs in headless Neovim (tests/tools.sh; skipped without nvim):
--   BASTIO_LSP=... BASTIOC=... nvim --headless -u NONE -i NONE \
--     --cmd 'set rtp^=editors/vim' --cmd 'filetype plugin indent on' --cmd 'syntax on' \
--     -c 'luafile editors/vim/test/nvim_test.lua'
-- It opens the programs in tools/bastio-lsp/test and checks that the language
-- server attaches, answers hover and go-to-definition, publishes a diagnostic
-- and formats the buffer, and that the syntax file and file type are found.
local dir = vim.env.BASTIO_TEST_DIR
local failed = false
local function check(cond, what)
  if not cond then
    failed = true
    io.stderr:write('FAIL ' .. what .. '\n')
  end
end

require('bastio').setup({ cmd = { vim.env.BASTIO_LSP }, bastioc = vim.env.BASTIOC })

vim.cmd('edit ' .. dir .. '/main.bastio')
check(vim.bo.filetype == 'bastio', 'the file type is bastio: ' .. vim.bo.filetype)
check(vim.fn.synIDattr(vim.fn.synID(3, 1, 1), 'name') == 'bastioKeyword', 'syntax: fn is a keyword')
check(vim.bo.shiftwidth == 2 and vim.bo.expandtab, 'indentation settings')

local attached = vim.wait(20000, function()
  return #vim.lsp.get_active_clients({ bufnr = 0, name = 'bastio-lsp' }) > 0
end, 50)
check(attached, 'bastio-lsp attaches')

local function request(method)
  local params = vim.lsp.util.make_position_params()
  local res = vim.lsp.buf_request_sync(0, method, params, 20000)
  for _, r in pairs(res or {}) do
    return r.result
  end
end

vim.fn.cursor(1, 1)
vim.fn.search('double')
local hover = request('textDocument/hover')
check(hover and hover.contents.value:find('fn double(x: u64) -> u64', 1, true), 'hover on a function')
local def = request('textDocument/definition')
check(def and def.uri:find('lib.bastio$') and def.range.start.line == 2, 'go to definition in another file')

-- The diagnostic of the compiler appears after opening a file with an error.
vim.cmd('edit ' .. dir .. '/bad.bastio')
local got = vim.wait(30000, function()
  return #vim.diagnostic.get(0) > 0
end, 50)
check(got, 'a diagnostic is published for bad.bastio')
if got then
  local d = vim.diagnostic.get(0)[1]
  check(d.lnum == 1 and d.code == 'E-TYPE' and d.message:find('must be u64', 1, true), 'its place and text')
end

-- Formatting through the server.
vim.cmd('enew')
vim.bo.filetype = 'bastio'
vim.cmd('file ' .. vim.fn.tempname() .. '.bastio')
vim.api.nvim_buf_set_lines(0, 0, -1, false, { 'fn main()->u64{', 'return   1 ;', '}' })
vim.wait(20000, function()
  return #vim.lsp.get_active_clients({ bufnr = 0, name = 'bastio-lsp' }) > 0
end, 50)
vim.lsp.buf.format({ async = false })
check(table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), '\n') == 'fn main() -> u64 {\n  return 1;\n}',
  'format through the language server')

vim.cmd(failed and 'cquit' or 'qall!')
