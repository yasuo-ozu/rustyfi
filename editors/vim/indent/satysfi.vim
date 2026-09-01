" indent/satysfi.vim -- a bracket-depth indenter.
"
" This is a heuristic, not a model of the language: it indents one shiftwidth
" per bracket left open on the previous line and dedents a line that opens
" with a closer.  `:RustyfiFmt` is the authority on layout; this only aims to
" put the cursor somewhere sane while you type.  Set g:rustyfi_no_indent = 1
" to skip it entirely.

if exists('b:did_indent') || get(g:, 'rustyfi_no_indent', 0)
  finish
endif
let b:did_indent = 1

setlocal indentexpr=RustyfiSatysfiIndent(v:lnum)
setlocal indentkeys=0{,0},0),0],0>,!^F,o,O
setlocal nolisp nosmartindent autoindent

let b:undo_indent = 'setlocal indentexpr< indentkeys< lisp< smartindent< autoindent<'

if exists('*RustyfiSatysfiIndent')
  finish
endif

" Strip comments and string literals so their brackets do not count.
function! s:Strip(line) abort
  let l:s = substitute(a:line, '`\{1,3}.\{-}`\{1,3}', '', 'g')
  let l:s = substitute(l:s, '%.*$', '', '')
  " `\{` and `\}` are escaped literals in text mode, not brackets.
  let l:s = substitute(l:s, '\\.', '', 'g')
  return l:s
endfunction

function! s:Delta(line) abort
  let l:s = s:Strip(a:line)
  let l:open  = len(substitute(l:s, '[^({\[]', '', 'g'))
  let l:close = len(substitute(l:s, '[^)}\]]', '', 'g'))
  " `'<` opens block text, a bare `>` closes it.  `->` and `<-` are arrows,
  " and `<=`/`>=` comparisons, so only these exact forms count.
  let l:open  += len(split(l:s, "'<", 1)) - 1
  let l:close += len(substitute(l:s, '[^>]', '', 'g'))
        \ - (len(split(l:s, '->', 1)) - 1)
        \ - (len(split(l:s, '>=', 1)) - 1)
  return l:open - l:close
endfunction

function! RustyfiSatysfiIndent(lnum) abort
  let l:prev = prevnonblank(a:lnum - 1)
  if l:prev == 0
    return 0
  endif
  let l:ind = indent(l:prev) + s:Delta(getline(l:prev)) * shiftwidth()
  " A line that starts with a closer lines up with its opener's line instead.
  if getline(a:lnum) =~# '^\s*[)}\]>]'
    let l:ind -= shiftwidth()
  endif
  return max([0, l:ind])
endfunction
