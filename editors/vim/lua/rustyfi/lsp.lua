-- lua/rustyfi/lsp.lua -- one setup entry point that works on Neovim 0.9
-- through 0.11+, with or without nvim-lspconfig.
--
-- Precedence:
--   * 0.11+ with the native runtime loader -> vim.lsp.config + vim.lsp.enable
--   * nvim-lspconfig present               -> register a server definition
--   * neither                              -> a FileType autocommand that
--                                             vim.lsp.start()s the server
--
-- Whether this belongs upstream in nvim-lspconfig: yes, and it would be a NEW
-- entry.  lspconfig already ships `satysfi_ls`, which is the OCaml
-- `satysfi-language-server`, a different binary with a different command line;
-- `rustyfi lsp` is a second server for the same filetype, so the upstream
-- shape is a sibling entry named `rustyfi` (or `rustyfi_ls`) rather than a
-- patch to the existing one.  Until then this file is the whole config.

local M = {}

local function base_config(user)
  local cfg = {
    cmd = { user.cmd_bin or require('rustyfi.bin').path() or 'rustyfi', 'lsp' },
    filetypes = { 'satysfi' },
    root_markers = { 'Satyristes', 'rustyfi-deps.yaml', '.git' },
    init_options = user.init_options or {},
  }
  for k, v in pairs(user) do
    if k ~= 'cmd_bin' then
      cfg[k] = v
    end
  end
  if user.lib_root then
    cfg.init_options = vim.tbl_extend('force', cfg.init_options, { libRoot = user.lib_root })
    cfg.cmd = vim.list_extend(vim.deepcopy(cfg.cmd), { '--lib-root', user.lib_root })
  end
  return cfg
end

--- @param opts table|nil
---   opts.cmd_bin      path to the rustyfi executable (default 'rustyfi')
---   opts.lib_root     passed as --lib-root and as initializationOptions.libRoot
---   opts.init_options extra initializationOptions
---   any other key is merged into the client config (on_attach, capabilities, ...)
function M.setup(opts)
  opts = opts or {}
  local cfg = base_config(opts)

  -- Neovim 0.11+: the native config registry.
  if vim.lsp.config and vim.lsp.enable then
    vim.lsp.config('rustyfi', cfg)
    vim.lsp.enable('rustyfi')
    return
  end

  -- nvim-lspconfig (0.9/0.10 route).
  local ok, lspconfig = pcall(require, 'lspconfig')
  if ok then
    local configs = require('lspconfig.configs')
    if not configs.rustyfi then
      local util = require('lspconfig.util')
      configs.rustyfi = {
        default_config = {
          cmd = cfg.cmd,
          filetypes = cfg.filetypes,
          root_dir = util.root_pattern('Satyristes', 'rustyfi-deps.yaml', '.git'),
          single_file_support = true,
          init_options = cfg.init_options,
        },
        docs = { description = 'rustyfi: SATySFi language server (Rust port)' },
      }
    end
    lspconfig.rustyfi.setup(cfg)
    return
  end

  -- No lspconfig: start it ourselves.  vim.lsp.start reuses an existing client
  -- whose name and root_dir match, so one server per project is what happens.
  local root_markers = cfg.root_markers
  vim.api.nvim_create_autocmd('FileType', {
    group = vim.api.nvim_create_augroup('RustyfiLsp', { clear = true }),
    pattern = 'satysfi',
    callback = function(args)
      local found = vim.fs.find(root_markers, {
        upward = true,
        path = vim.fs.dirname(vim.api.nvim_buf_get_name(args.buf)),
      })[1]
      vim.lsp.start(vim.tbl_extend('force', cfg, {
        name = 'rustyfi',
        root_dir = found and vim.fs.dirname(found) or vim.fn.getcwd(),
      }), { bufnr = args.buf })
    end,
  })
end

return M
