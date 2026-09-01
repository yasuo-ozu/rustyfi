" ftdetect/satysfi.vim -- SATySFi / rustyfi filetype detection.
"
" `setf` (as opposed to `set filetype=`) does nothing when a filetype has
" already been set for the buffer, so loading this alongside another plugin
" that claims .saty -- qnighy/satysfi.vim, or a future runtime default -- is
" not a collision: whichever ftdetect script runs first wins and the other is
" a no-op.  The name `satysfi` is the established one: it is what
" qnighy/satysfi.vim sets and what nvim-lspconfig's `satysfi_ls` entry lists
" in its `filetypes`.
autocmd BufRead,BufNewFile *.saty,*.satyh,*.satyg setf satysfi
