" autoload/rustyfi/bin.vim -- locate the rustyfi executable.

let s:cache = {}

" Walk up from `dir` looking for a built binary, so the plugin works inside a
" rustyfi checkout with nothing installed on $PATH.
function! s:FromCheckout(dir) abort
  let l:dir = a:dir
  while 1
    for l:rel in ['target/release/rustyfi', 'target/debug/rustyfi']
      let l:cand = l:dir . '/' . l:rel
      if executable(l:cand)
        return l:cand
      endif
    endfor
    let l:up = fnamemodify(l:dir, ':h')
    if l:up ==# l:dir
      return ''
    endif
    let l:dir = l:up
  endwhile
endfunction

" Returns the path to use, or '' if none could be found.
function! rustyfi#bin#path() abort
  let l:configured = get(g:, 'rustyfi_bin', 'rustyfi')
  if l:configured !=# 'rustyfi'
    " An explicit setting is honoured as given; never second-guessed.
    return executable(l:configured) ? l:configured : ''
  endif
  if executable('rustyfi')
    return 'rustyfi'
  endif
  let l:dir = expand('%:p:h')
  if l:dir ==# ''
    let l:dir = getcwd()
  endif
  if has_key(s:cache, l:dir)
    return s:cache[l:dir]
  endif
  let l:found = s:FromCheckout(l:dir)
  " Only a HIT is cached.  Caching the miss meant that the first thing anyone
  " does in a fresh checkout -- open a document, discover there is no binary,
  " run `cargo build --release` in another terminal -- left the plugin saying
  " `executable not found` for the rest of the session, with no way back short
  " of restarting or calling rustyfi#bin#clear_cache() by hand.  A miss is also
  " the cheap case: the walk-up stops at the filesystem root having done a
  " couple of stat()s per level, once per :Rustyfi* command rather than per
  " keystroke.  (The Lua half caches nothing at all, which is now the same
  " answer rather than a second one.)
  if l:found !=# ''
    let s:cache[l:dir] = l:found
  endif
  return l:found
endfunction

" Resolve, or report the failure once and return ''.
function! rustyfi#bin#require() abort
  let l:bin = rustyfi#bin#path()
  if l:bin ==# ''
    call rustyfi#util#error(
          \ 'rustyfi: executable not found. Put it on $PATH or set g:rustyfi_bin.')
  endif
  return l:bin
endfunction

function! rustyfi#bin#clear_cache() abort
  let s:cache = {}
endfunction
