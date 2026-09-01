" autoload/rustyfi/fmt.vim -- `rustyfi fmt` over the BUFFER.
"
" The buffer is piped through `rustyfi fmt -` (stdin -> stdout) rather than
" formatting the file on disk, so an unsaved buffer formats what you can see.
"
" Exit codes, from `rustyfi fmt --help`:
"   0  clean
"   1  --check found files needing reformatting
"   2  usage
"   5  filesystem
"   6  at least one file was DECLINED: it does not lex, so there is no token
"      stream to re-emit.  Nothing at all is written to stdout -- applying it
"      would blank the buffer.
"   7  at least one file LEXED but did not PARSE, so only the older whitespace
"      formatter ran.  stdout DOES carry text here, but it is not the layout
"      the formatter promises, so by default we decline to apply it and say so.
"      Set g:rustyfi_fmt_accept_partial = 1 to take the whitespace tidy anyway.

function! s:Argv(bin, extra) abort
  let l:argv = [a:bin, 'fmt']
  call extend(l:argv, get(g:, 'rustyfi_fmt_args', []))
  call extend(l:argv, a:extra)
  call add(l:argv, '-')
  return l:argv
endfunction

function! s:FirstMessage(res) abort
  for l:line in a:res.err
    if l:line !~# '^\s*$'
      return l:line
    endif
  endfor
  return 'exit code ' . a:res.code
endfunction

" :RustyfiFmt
function! rustyfi#fmt#buffer() abort
  let l:bin = rustyfi#bin#require()
  if l:bin ==# ''
    return 0
  endif
  if !&modifiable
    call rustyfi#util#error('rustyfi fmt: buffer is not modifiable')
    return 0
  endif

  let l:stdin = join(getline(1, '$'), "\n") . "\n"
  let l:res = rustyfi#util#run(s:Argv(l:bin, []), l:stdin)

  if l:res.code == 6
    call rustyfi#util#error('rustyfi fmt declined (exit 6): ' . s:FirstMessage(l:res)
          \ . ' -- buffer left unchanged')
    return 0
  elseif l:res.code == 7
    if !get(g:, 'rustyfi_fmt_accept_partial', 0)
      call rustyfi#util#warn('rustyfi fmt (exit 7): ' . s:FirstMessage(l:res)
            \ . ' -- buffer left unchanged (g:rustyfi_fmt_accept_partial to apply anyway)')
      return 0
    endif
    call rustyfi#util#warn('rustyfi fmt (exit 7): ' . s:FirstMessage(l:res)
          \ . ' -- applied whitespace tidy only')
  elseif l:res.code != 0
    call rustyfi#util#error('rustyfi fmt failed (exit ' . l:res.code . '): '
          \ . s:FirstMessage(l:res))
    return 0
  endif

  " A last-ditch guard: never let an empty stdout blank a non-empty buffer,
  " whatever the exit code claimed.
  if empty(l:res.out) && !(line('$') == 1 && getline(1) ==# '')
    call rustyfi#util#error('rustyfi fmt produced no output -- buffer left unchanged')
    return 0
  endif

  let l:changed = rustyfi#util#replace_lines(l:res.out)
  if l:changed
    call rustyfi#util#info('rustyfi fmt: reformatted')
  endif
  return 1
endfunction

" :RustyfiFmtCheck -- report, and load the unified diff into the quickfix-less
" preview of a scratch buffer only when asked.  Exit 1 means "would change".
function! rustyfi#fmt#check() abort
  let l:bin = rustyfi#bin#require()
  if l:bin ==# ''
    return
  endif
  let l:stdin = join(getline(1, '$'), "\n") . "\n"
  let l:res = rustyfi#util#run(s:Argv(l:bin, ['--check']), l:stdin)
  if l:res.code == 0
    call rustyfi#util#info('rustyfi fmt --check: clean')
  elseif l:res.code == 1
    call rustyfi#util#warn('rustyfi fmt --check: would reformat')
    if get(g:, 'rustyfi_fmt_check_show_diff', 1) && !empty(l:res.out)
      call s:ShowDiff(l:res.out)
    endif
  elseif l:res.code == 6
    call rustyfi#util#error('rustyfi fmt --check declined (exit 6): ' . s:FirstMessage(l:res))
  else
    call rustyfi#util#error('rustyfi fmt --check failed (exit ' . l:res.code . '): '
          \ . s:FirstMessage(l:res))
  endif
endfunction

function! s:ShowDiff(lines) abort
  let l:cur = win_getid()
  botright new
  setlocal buftype=nofile bufhidden=wipe noswapfile nobuflisted
  silent file [rustyfi\ fmt\ --check]
  call setline(1, a:lines)
  setlocal nomodifiable filetype=diff
  call win_gotoid(l:cur)
endfunction

" BufWritePre hook.
function! rustyfi#fmt#on_save() abort
  if !get(b:, 'rustyfi_fmt_on_save', get(g:, 'rustyfi_fmt_on_save', 0))
    return
  endif
  call rustyfi#fmt#buffer()
endfunction

function! rustyfi#fmt#toggle_on_save() abort
  let b:rustyfi_fmt_on_save =
        \ !get(b:, 'rustyfi_fmt_on_save', get(g:, 'rustyfi_fmt_on_save', 0))
  call rustyfi#util#info('rustyfi: format-on-save '
        \ . (b:rustyfi_fmt_on_save ? 'ON' : 'OFF') . ' for this buffer')
endfunction
