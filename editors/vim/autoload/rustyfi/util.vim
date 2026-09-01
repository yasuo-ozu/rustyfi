" autoload/rustyfi/util.vim -- messages, and the buffer edit that keeps the
" cursor where the user left it.

function! rustyfi#util#error(msg) abort
  echohl ErrorMsg | echomsg a:msg | echohl None
endfunction

function! rustyfi#util#warn(msg) abort
  echohl WarningMsg | echomsg a:msg | echohl None
endfunction

function! rustyfi#util#info(msg) abort
  if get(g:, 'rustyfi_quiet', 0)
    return
  endif
  echomsg a:msg
endfunction

" Replace the current buffer's lines with `new`, touching as few lines as
" possible, and leave the cursor on the same *text* it was on.
"
" Naively doing `%delete | call setline(1, new)` and then winrestview() puts
" the cursor back on the same LINE NUMBER, which is the wrong line whenever
" the formatter added or removed lines above it.  Instead we trim the common
" prefix and the common suffix, rewrite only the middle, and shift the cursor
" by the line delta when it sat in the (unchanged) suffix.  In the common case
" -- reformatting a region the cursor is not in -- the cursor does not move at
" all, and neither do the surrounding lines' marks or folds.
"
" Returns 1 if anything changed.
function! rustyfi#util#replace_lines(new) abort
  let l:old = getline(1, '$')
  if l:old ==# a:new
    return 0
  endif

  let l:no = len(l:old)
  let l:nn = len(a:new)

  " Longest common prefix.
  let l:p = 0
  while l:p < l:no && l:p < l:nn && l:old[l:p] ==# a:new[l:p]
    let l:p += 1
  endwhile
  " Longest common suffix, not overlapping the prefix.
  let l:s = 0
  while l:s < (l:no - l:p) && l:s < (l:nn - l:p)
        \ && l:old[l:no - 1 - l:s] ==# a:new[l:nn - 1 - l:s]
    let l:s += 1
  endwhile

  let l:view = winsaveview()
  let l:delta = l:nn - l:no

  " Rewrite lines [p+1, no-s] (1-based, inclusive) with new[p : nn-s-1].
  let l:first = l:p + 1
  let l:last  = l:no - l:s
  let l:mid   = l:nn - l:s - 1 >= l:p ? a:new[l:p : l:nn - l:s - 1] : []

  " setline over the overlap, then add or remove the difference.
  let l:keep = l:last - l:first + 1
  let l:i = 0
  while l:i < l:keep && l:i < len(l:mid)
    call setline(l:first + l:i, l:mid[l:i])
    let l:i += 1
  endwhile
  if len(l:mid) > l:keep
    call append(l:first + l:keep - 1, l:mid[l:keep :])
  elseif len(l:mid) < l:keep
    let l:from = l:first + len(l:mid)
    silent execute l:from . ',' . l:last . 'delete _'
  endif

  " If the cursor sat in the untouched suffix, its line number moved by delta.
  if l:view.lnum > l:last
    let l:view.lnum += l:delta
    let l:view.topline += l:delta
  endif
  let l:view.lnum = max([1, min([l:view.lnum, line('$')])])
  let l:view.topline = max([1, l:view.topline])
  call winrestview(l:view)
  return 1
endfunction

" Run `argv` (a list) with `stdin` (a string), capturing stdout and stderr
" separately.  Vim's and Neovim's system() both capture stdout only, so stderr
" is redirected to a temp file.  Returns {'code': n, 'out': [...], 'err': [...]}.
function! rustyfi#util#run(argv, stdin) abort
  let l:errfile = tempname()
  let l:cmd = join(map(copy(a:argv), 'shellescape(v:val)'), ' ')
        \ . ' 2> ' . shellescape(l:errfile)
  let l:out = system(l:cmd, a:stdin)
  let l:code = v:shell_error
  let l:err = filereadable(l:errfile) ? readfile(l:errfile) : []
  call delete(l:errfile)
  " system() returns a single string; split on NL and drop the trailing empty
  " element a final newline produces.
  let l:lines = split(l:out, "\n", 1)
  if len(l:lines) > 0 && l:lines[-1] ==# ''
    call remove(l:lines, -1)
  endif
  return {'code': l:code, 'out': l:lines, 'err': l:err}
endfunction
