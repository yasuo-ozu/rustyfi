" ftplugin/satysfi.vim
if exists('b:did_ftplugin')
  finish
endif
let b:did_ftplugin = 1

let s:undo = []

setlocal commentstring=%\ %s
setlocal comments=:%
setlocal formatoptions-=t formatoptions+=croql
" `rustyfi fmt`'s own defaults are two spaces per step and a 100-column budget.
setlocal expandtab shiftwidth=2 softtabstop=2 tabstop=2
setlocal textwidth=0
" `gf` over an @import: target, and over an @require:'d package name.
setlocal suffixesadd=.satyh,.satyg,.saty
setlocal include=^\\s*@\\(require\\\|import\\):
setlocal iskeyword+=-

let b:undo_ftplugin = 'setlocal commentstring< comments< formatoptions<'
      \ . ' expandtab< shiftwidth< softtabstop< tabstop< textwidth<'
      \ . ' suffixesadd< include< iskeyword<'
