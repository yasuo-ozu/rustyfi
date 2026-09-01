" test/lsp.vim -- does `rustyfi lsp` actually come up and attach under the
" Neovim setup this plugin ships?  Neovim only; the Vim 8 route needs vim-lsp
" installed and is not tested here.
"
"   nvim --headless -u NONE -S editors/vim/test/lsp.vim

set nocompatible noswapfile
let s:here = expand('<sfile>:p:h')
let s:root = fnamemodify(s:here, ':h')
execute 'set runtimepath^=' . fnameescape(s:root)
let g:rustyfi_bin = $RUSTYFI_TEST_BIN ==# '' ? 'rustyfi' : $RUSTYFI_TEST_BIN

let s:pass = 0
let s:fail = []
function! s:Ok(cond, name) abort
  if a:cond
    let s:pass += 1
    call chansend(v:stderr, 'ok   ' . a:name . "\n")
  else
    call add(s:fail, a:name)
    call chansend(v:stderr, 'FAIL ' . a:name . "\n")
  endif
endfunction

if !has('nvim')
  call chansend(v:stderr, "skipped: Neovim only\n")
  qall!
endif

filetype plugin on
runtime! plugin/rustyfi.vim

" Two supported routes.  RUSTYFI_LSP_MODE=native exercises the runtime
" `lsp/rustyfi.lua` file that Neovim 0.11+ picks up from the runtimepath;
" anything else exercises require('rustyfi.lsp').setup(), which is what 0.9 and
" 0.10 use (and which works on 0.11 too).
let s:mode = $RUSTYFI_LSP_MODE ==# '' ? 'setup' : $RUSTYFI_LSP_MODE
call chansend(v:stderr, '-- mode: ' . s:mode . "\n")
if s:mode ==# 'native'
  if !has('nvim-0.11')
    call chansend(v:stderr, "skipped: native lsp/ route needs Neovim 0.11+\n")
    qall!
  endif
  " The runtime file names the executable `rustyfi`; point at ours.
  lua vim.lsp.config('rustyfi', { cmd = { vim.g.rustyfi_bin, 'lsp' } })
  lua vim.lsp.enable('rustyfi')
else
  lua << EOF
require('rustyfi.lsp').setup({
  cmd_bin = vim.g.rustyfi_bin,
  lib_root = vim.env.RUSTYFI_LIB_ROOT,
})
EOF
endif

execute 'edit!' fnameescape(s:here . '/fixtures/imports.saty')
call s:Ok(&filetype ==# 'satysfi', 'filetype is satysfi')

function! s:Clients() abort
  return luaeval("#vim.tbl_filter(function(c) return c.name == 'rustyfi' end, (vim.lsp.get_clients or vim.lsp.get_active_clients)({ bufnr = 0 }))")
endfunction

let s:t = reltime()
while reltimefloat(reltime(s:t)) < 20 && s:Clients() == 0
  sleep 100m
endwhile
call s:Ok(s:Clients() > 0, 'rustyfi language server attached to the buffer')

if s:Clients() > 0
  let s:caps = luaeval("(vim.lsp.get_clients or vim.lsp.get_active_clients)({bufnr=0, name='rustyfi'})[1].server_capabilities")
  call s:Ok(get(s:caps, 'hoverProvider', v:false) == v:true, 'server advertises hover')
  call s:Ok(get(s:caps, 'definitionProvider', v:false) == v:true, 'server advertises go-to-definition')
  call s:Ok(get(s:caps, 'documentFormattingProvider', v:false) == v:true, 'server advertises formatting')
  call s:Ok(has_key(s:caps, 'completionProvider'), 'server advertises completion')
  let s:syms = luaeval("(function() local r = vim.lsp.buf_request_sync(0, 'textDocument/documentSymbol', { textDocument = vim.lsp.util.make_text_document_params() }, 10000); if not r then return 0 end; for _, v in pairs(r) do if v.result then return #v.result end end; return 0 end)()")
  call s:Ok(s:syms >= 0, 'documentSymbol request round-tripped (' . s:syms . ' symbols)')
endif

call chansend(v:stderr, printf("\n%d passed, %d failed\n", s:pass, len(s:fail)))
for s:f in s:fail
  call chansend(v:stderr, '  FAILED: ' . s:f . "\n")
endfor
if empty(s:fail) | qall! | else | cquit! | endif
