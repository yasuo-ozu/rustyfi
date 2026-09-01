" autoload/rustyfi/lsp.vim -- Vim 8 language-server registration, via vim-lsp.
"
" Why vim-lsp and not ALE: ALE is a linting framework whose LSP support is
" expressed per-linter, so wiring `rustyfi lsp` through it gives you
" diagnostics and (with ale_completion_enabled) completion, but the rest of the
" server's surface -- documentSymbol, workspaceSymbol, hover, go-to-definition,
" documentFormatting, all of which this server advertises -- is reached only
" through ALE's own partial commands.  vim-lsp is a full client: one
" lsp#register_server() call and every capability above is available under the
" usual :Lsp* commands.  An ALE recipe is in the README for people who already
" run ALE and do not want a second client.
"
" This is deliberately thin: configuration, not a client.

function! rustyfi#lsp#register() abort
  if !exists('*lsp#register_server')
    return 0
  endif
  let l:bin = rustyfi#bin#path()
  if l:bin ==# ''
    return 0
  endif
  let l:cmd = [l:bin, 'lsp']
  call extend(l:cmd, get(g:, 'rustyfi_lsp_args', []))
  call lsp#register_server({
        \ 'name': 'rustyfi',
        \ 'cmd': {server_info -> l:cmd},
        \ 'allowlist': ['satysfi'],
        \ 'whitelist': ['satysfi'],
        \ 'root_uri': {server_info -> lsp#utils#path_to_uri(
        \     lsp#utils#find_nearest_parent_file_directory(
        \       lsp#utils#get_buffer_path(), ['Satyristes', '.git/']))},
        \ 'initialization_options': rustyfi#lsp#init_options(),
        \ })
  return 1
endfunction

" `libRoot` is read by the server from its own initializationOptions when
" neither --lib-root nor $RUSTYFI_LIB_ROOT is set (see `rustyfi lsp --help`).
function! rustyfi#lsp#init_options() abort
  let l:opts = copy(get(g:, 'rustyfi_lsp_init_options', {}))
  if !has_key(l:opts, 'libRoot') && get(g:, 'rustyfi_lib_root', '') !=# ''
    let l:opts.libRoot = g:rustyfi_lib_root
  endif
  return l:opts
endfunction
