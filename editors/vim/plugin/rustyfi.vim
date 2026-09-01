" plugin/rustyfi.vim -- commands and autocommands for the rustyfi SATySFi
" toolchain.  Works in Vim 8.2+ and Neovim 0.9+.

if exists('g:loaded_rustyfi_vim')
  finish
endif
let g:loaded_rustyfi_vim = 1

if !(has('nvim-0.9') || (!has('nvim') && v:version >= 802))
  echohl WarningMsg
  echomsg 'rustyfi.vim needs Vim 8.2+ or Neovim 0.9+; not loading'
  echohl None
  finish
endif

let s:save_cpo = &cpoptions
set cpoptions&vim

command! -bar RustyfiFmt              call rustyfi#fmt#buffer()
command! -bar RustyfiFmtCheck         call rustyfi#fmt#check()
command! -bar RustyfiFmtOnSaveToggle  call rustyfi#fmt#toggle_on_save()

command! -bar RustyfiPreview          call rustyfi#preview#open()
command! -bar RustyfiPreviewClose     call rustyfi#preview#close()
command! -bar RustyfiPreviewToggle    call rustyfi#preview#toggle()
command! -bar RustyfiPreviewRefresh   call rustyfi#preview#render(1)
command! -bar RustyfiPreviewErrors    call rustyfi#preview#errors()

augroup rustyfi_vim
  autocmd!
  autocmd BufWritePre *.saty,*.satyh,*.satyg call rustyfi#fmt#on_save()
  " vim-lsp announces its setup point; registering earlier would be ignored.
  autocmd User lsp_setup call rustyfi#lsp#register()
augroup END

let &cpoptions = s:save_cpo
unlet s:save_cpo
