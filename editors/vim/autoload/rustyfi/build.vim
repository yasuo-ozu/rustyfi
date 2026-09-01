" autoload/rustyfi/build.vim -- compile the document to a PDF, for real.
"
" Distinct from the preview in three ways, and each one is a deliberate
" difference rather than an accident of implementation:
"
"   * it builds the FILE ON DISK, not a scratch copy, so the output lands
"     where the author expects it and `@import:` resolves exactly as it will
"     for anyone else;
"   * the output is a PDF beside the document -- the compiler's own default,
"     so this command has no opinion the CLI does not already have;
"   * a failure populates the QUICKFIX list. The preview shows one error in
"     a status line because it is a glance; a build is a thing you fix, and
"     `:cnext` is how that is done in this editor.

let s:job = 0

function! s:Report(msg, hl) abort
  execute 'echohl' a:hl
  echomsg '[rustyfi] ' . a:msg
  echohl None
endfunction

" Write the buffer first, unless the user has turned that off.
"
" A build command that silently compiles yesterday's bytes is worse than one
" that touches the file: the whole point is to see what the document is NOW.
" `g:rustyfi_build_autowrite = 0` restores the cautious behaviour, and then a
" modified buffer refuses rather than lying about what it built.
function! s:EnsureWritten() abort
  if !&modified
    return 1
  endif
  if empty(expand('%:p'))
    call s:Report('buffer has no file name; :w it somewhere first', 'ErrorMsg')
    return 0
  endif
  if !get(g:, 'rustyfi_build_autowrite', 1)
    call s:Report('buffer has unsaved changes (:w first, or set g:rustyfi_build_autowrite)', 'ErrorMsg')
    return 0
  endif
  try
    silent write
  catch
    call s:Report('could not write the buffer: ' . v:exception, 'ErrorMsg')
    return 0
  endtry
  return 1
endfunction

" Where the compiler will put the PDF: alongside the document, same stem.
" Mirrors `rustyfi`'s own default rather than passing `-o`, so the two cannot
" drift apart.
function! rustyfi#build#output_path() abort
  return expand('%:p:r') . '.pdf'
endfunction

function! s:OpenExternally(path) abort
  if !filereadable(a:path)
    call s:Report('no PDF at ' . a:path, 'ErrorMsg')
    return
  endif
  let l:opener = get(g:, 'rustyfi_build_opener', '')
  if empty(l:opener)
    if has('mac') || has('macunix')
      let l:opener = 'open'
    elseif has('win32') || has('win64')
      let l:opener = 'start'
    else
      let l:opener = 'xdg-open'
    endif
  endif
  if !executable(l:opener) && l:opener !=# 'start'
    call s:Report(l:opener . ' is not executable; set g:rustyfi_build_opener', 'ErrorMsg')
    return
  endif
  " Detached and output-discarded: a viewer that lives longer than the editor
  " must not hold the job table open, and its chatter is not ours to print.
  call rustyfi#job#start([l:opener, a:path], {})
endfunction

" Turn the compiler's diagnostics into quickfix entries.
"
" The shape, measured rather than assumed:
"
"     Error: /path/doc.saty: line 3, characters 7-21: unbound inline command …
"
" One line, and prefixed. Two things this got wrong first time round and a
" test now pins:
"
"   * the `Error: ` prefix. A pattern anchored at `%f` matched nothing, so
"     the whole build reported no entries at all while plainly having failed.
"   * a trailing `%-G%.%#`. `%-G` DROPS what it matches, so a catch-all in
"     that position throws away every line the earlier patterns missed --
"     which is precisely the diagnostic you needed. The catch-all here is
"     `%m`, which keeps the text as an entry with no file or line (`valid`
"     0): `:copen` shows it, `:cnext` skips it. Silence is the failure mode
"     worth avoiding; an unnavigable entry is not.
function! s:Quickfix(lines, title) abort
  let l:save = &errorformat
  try
    let &errorformat = 'Error: %f: line %l\, characters %c-%*[0-9]: %m,'
          \ . 'Error: %f: line %l\, character %c: %m,'
          \ . '%f: line %l\, characters %c-%*[0-9]: %m,'
          \ . '%f: line %l\, character %c: %m,'
          \ . '%m'
    call setqflist([], ' ', {'lines': a:lines, 'title': a:title})
  finally
    let &errorformat = l:save
  endtry
endfunction

 " Is a build in flight?
"
" Public because the refusal in `run()` is observable behaviour -- a second
" `:RustyfiBuild` while one is running does nothing -- and anything that
" wants to sequence around that (a test, a user's mapping, a wrapper that
" builds then does something else) needs to be able to ask.
function! rustyfi#build#running() abort
  return s:job isnot 0 && rustyfi#job#running(s:job)
endfunction

function! rustyfi#build#run(open) abort
  if rustyfi#build#running()
    call s:Report('a build is already running', 'WarningMsg')
    return
  endif
  let l:bin = rustyfi#bin#require()
  if l:bin ==# ''
    return
  endif
  if !s:EnsureWritten()
    return
  endif

  let l:src = expand('%:p')
  let l:out = rustyfi#build#output_path()
  let l:argv = [l:bin, l:src]
  let l:root = get(g:, 'rustyfi_lib_root', '')
  if !empty(l:root)
    let l:argv += ['--lib-root', l:root]
  endif
  let l:argv += get(g:, 'rustyfi_build_args', [])

  call s:Report('building ' . fnamemodify(l:src, ':t') . '…', 'None')
  let s:job = rustyfi#job#start(l:argv, {
        \ 'cwd': expand('%:p:h'),
        \ 'on_exit': function('s:Done', [l:out, a:open, reltime()]),
        \ })
  if s:job is 0
    call s:Report('could not start ' . l:bin, 'ErrorMsg')
  endif
endfunction

function! s:Done(out, open, started, code, stdout, stderr) abort
  let s:job = 0
  let l:ms = float2nr(reltimefloat(reltime(a:started)) * 1000)
  if a:code == 0
    " A clean build clears a stale list: leaving the previous failure's
    " entries in the quickfix window after a success is how you end up
    " chasing an error you already fixed.
    call setqflist([], ' ', {'lines': [], 'title': 'rustyfi build'})
    call s:Report(printf('built %s (%dms)', fnamemodify(a:out, ':t'), l:ms), 'None')
    if a:open
      call s:OpenExternally(a:out)
    endif
    return
  endif

  let l:lines = filter(a:stderr + a:stdout, '!empty(v:val)')
  call s:Quickfix(l:lines, 'rustyfi build')
  call s:Report(printf('build failed (exit %d) — :copen for details', a:code), 'ErrorMsg')
  if get(g:, 'rustyfi_build_copen', 1) && !empty(getqflist())
    copen
  endif
endfunction
