-- lsp/rustyfi.lua -- Neovim 0.11+ native LSP configuration.
--
-- Neovim 0.11 reads `lsp/<name>.lua` from the runtimepath, so with this plugin
-- installed the whole setup is:
--
--     vim.lsp.enable('rustyfi')
--
-- On 0.9/0.10, which have no `vim.lsp.config`, use
-- `require('rustyfi.lsp').setup()` instead -- it registers the same table with
-- nvim-lspconfig, or starts the server directly if lspconfig is absent.

return {
  -- Discovered, not hard-coded: `require('rustyfi.bin')` runs the same
  -- search the Vimscript half does (g:rustyfi_bin -> $PATH -> a
  -- `target/{release,debug}/rustyfi` above the buffer). Without it this
  -- route needed rustyfi on $PATH, so inside a checkout `:RustyfiFmt`
  -- worked and the server quietly never attached.
  cmd = require('rustyfi.bin').lsp_cmd(),
  filetypes = { 'satysfi' },
  root_markers = { 'Satyristes', 'rustyfi-deps.yaml', '.git' },
  -- `libRoot` is read from initializationOptions when neither --lib-root nor
  -- $RUSTYFI_LIB_ROOT is set; see `rustyfi lsp --help`.
  init_options = {},
  settings = {},
}
