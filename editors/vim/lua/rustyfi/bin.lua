-- lua/rustyfi/bin.lua -- locate the rustyfi executable, from Lua.
--
-- The same three-step search as `autoload/rustyfi/bin.vim`, and it exists
-- because the two halves of this plugin disagreed without it: `:RustyfiFmt`
-- found a binary in the checkout's `target/release/`, while the LSP routes
-- hard-coded a bare `'rustyfi'` and simply did not attach when nothing was on
-- $PATH. In a rustyfi checkout -- which is exactly where someone tries this
-- plugin first -- formatting worked and the language server silently did not.
--
-- Keep this in step with the Vimscript. `bin_discovery_agrees_across_languages`
-- in test/run.vim asserts the two return the same answer.

local M = {}

--- Walk up from `dir` looking for a built binary.
local function from_checkout(dir)
  local seen = {}
  while dir and dir ~= '' and not seen[dir] do
    seen[dir] = true
    for _, rel in ipairs({ 'target/release/rustyfi', 'target/debug/rustyfi' }) do
      local cand = dir .. '/' .. rel
      if vim.fn.executable(cand) == 1 then
        return cand
      end
    end
    local up = vim.fn.fnamemodify(dir, ':h')
    if up == dir then
      break
    end
    dir = up
  end
  return nil
end

--- The executable to run, or nil if none was found.
---
--- `g:rustyfi_bin`, when set to anything other than the default, is honoured
--- exactly as given and never second-guessed -- someone who names a path is
--- telling us which build to use, and falling back to a different one behind
--- their back is how you debug the wrong binary for an hour.
---@param start string|nil directory to begin the walk from
---@return string|nil
function M.path(start)
  local configured = vim.g.rustyfi_bin
  if configured and configured ~= '' and configured ~= 'rustyfi' then
    return vim.fn.executable(configured) == 1 and configured or nil
  end
  if vim.fn.executable('rustyfi') == 1 then
    return 'rustyfi'
  end
  local dir = start
  if not dir or dir == '' then
    dir = vim.fn.expand('%:p:h')
  end
  if dir == '' then
    dir = vim.fn.getcwd()
  end
  return from_checkout(dir)
end

--- The `cmd` for a language-server configuration.
---
--- Falls back to a bare `rustyfi` rather than nil: a `cmd` of nil would make
--- `vim.lsp.enable` fail obscurely at attach time, while a bare name produces
--- Neovim's own "not executable" message, which names the thing that is
--- missing.
---@return string[]
function M.lsp_cmd()
  return { M.path() or 'rustyfi', 'lsp' }
end

return M
