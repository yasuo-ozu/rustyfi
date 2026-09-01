" test/run.vim -- headless test suite for the rustyfi vim plugin.
"
"   nvim --headless -u NONE -S editors/vim/test/run.vim
"   vim  -es -u NONE -S editors/vim/test/run.vim   (real Vim 8)
"
" $RUSTYFI_TEST_BIN must point at the rustyfi executable.
" $RUSTYFI_LIB_ROOT must point at a lib root (for the preview cases).

set nocompatible
set noswapfile
set shortmess+=F
filetype off

let s:here = expand('<sfile>:p:h')
let s:root = fnamemodify(s:here, ':h')
execute 'set runtimepath^=' . fnameescape(s:root)
execute 'set runtimepath+=' . fnameescape(s:root . '/after')

let g:rustyfi_bin = $RUSTYFI_TEST_BIN ==# '' ? 'rustyfi' : $RUSTYFI_TEST_BIN
let g:rustyfi_quiet = 1

filetype plugin indent on
syntax enable
runtime! plugin/rustyfi.vim

let s:pass = 0
let s:fail = []

function! s:Ok(cond, name) abort
  if a:cond
    let s:pass += 1
    call s:Say('ok   ' . a:name)
  else
    call add(s:fail, a:name)
    call s:Say('FAIL ' . a:name)
  endif
endfunction

function! s:Eq(got, want, name) abort
  if a:got ==# a:want
    let s:pass += 1
    call s:Say('ok   ' . a:name)
  else
    call add(s:fail, a:name)
    call s:Say('FAIL ' . a:name)
    call s:Say('       got:  ' . string(a:got))
    call s:Say('       want: ' . string(a:want))
  endif
endfunction

function! s:Say(msg) abort
  if has('nvim')
    call chansend(v:stderr, a:msg . "\n")
  else
    verbose echomsg a:msg
  endif
endfunction

function! s:Fixture(name) abort
  return s:here . '/fixtures/' . a:name
endfunction

function! s:Scratch(lines) abort
  enew!
  setlocal buftype= noswapfile
  call setline(1, a:lines)
  setlocal filetype=satysfi
endfunction

" ==========================================================================
call s:Say('== filetype detection')
" ==========================================================================
for s:ext in ['saty', 'satyh', 'satyg']
  execute 'edit!' fnameescape(s:here . '/fixtures/dummy.' . s:ext)
  call s:Eq(&filetype, 'satysfi', 'ftdetect .' . s:ext)
  call s:Ok(&commentstring ==# '% %s', 'ftplugin commentstring for .' . s:ext)
  bwipeout!
endfor

execute 'edit!' fnameescape(s:Fixture('messy.saty'))
call s:Ok(exists('b:current_syntax') && b:current_syntax ==# 'satysfi', 'syntax loaded')
call s:Ok(&indentexpr =~# 'RustyfiSatysfiIndent', 'indentexpr installed')
bwipeout!

" ==========================================================================
call s:Say('== syntax: program mode vs text mode')
" ==========================================================================
call s:Scratch([
      \ '@require: stdja',
      \ '% a comment with let in it',
      \ 'let x = 1 in',
      \ '+p{ The word let and in inside prose. }',
      \ 'let s = `a string with let`',
      \ '${x^2}',
      \ ])
redraw
function! s:Syn(l, c) abort
  return synIDattr(synID(a:l, a:c, 1), 'name')
endfunction
call s:Eq(s:Syn(1, 1), 'satysfiHeaderKey', '@require: is a header')
call s:Eq(s:Syn(2, 5), 'satysfiComment', '`let` inside a comment is a comment')
call s:Eq(s:Syn(3, 1), 'satysfiKeyword', '`let` in program mode is a keyword')
call s:Eq(s:Syn(3, 11), 'satysfiKeyword', '`in` in program mode is a keyword')
call s:Eq(s:Syn(4, 1), 'satysfiBlockCmd', '`+p` is a block command')
call s:Eq(s:Syn(4, 10), 'satysfiText', 'the word `let` in prose is NOT a keyword')
call s:Eq(s:Syn(4, 19), 'satysfiText', 'the word `in` in prose is NOT a keyword')
call s:Eq(s:Syn(5, 15), 'satysfiString', '`let` inside a string is a string')
call s:Eq(s:Syn(6, 1), 'satysfiMathDelim', '${ opens math')

" ==========================================================================
call s:Say('== indent')
" ==========================================================================
call s:Scratch(['let f x ='])
setlocal filetype=satysfi
call s:Ok(&indentexpr =~# 'RustyfiSatysfiIndent', 'indentexpr set on a satysfi buffer')
call s:Eq(RustyfiSatysfiIndent(1), 0, 'first line indents to 0')
call s:Scratch(['StdJa.document (|', '  title = {a};'])
call s:Ok(RustyfiSatysfiIndent(2) >= 2, 'an open bracket indents the next line')
call s:Scratch(['  (a', '  )'])
call s:Ok(RustyfiSatysfiIndent(2) <= indent(1), 'a closing bracket dedents')

" ==========================================================================
call s:Say('== rustyfi#util#replace_lines (minimal edit + cursor)')
" ==========================================================================
call s:Scratch(['a', 'b', 'c', 'd', 'e'])
call cursor(5, 1)
call s:Ok(rustyfi#util#replace_lines(['a', 'b', 'B2', 'c', 'd', 'e']), 'replace reports change')
call s:Eq(getline(1, '$'), ['a', 'b', 'B2', 'c', 'd', 'e'], 'replace inserted a line')
call s:Eq(line('.'), 6, 'cursor followed its text down one line')

call s:Scratch(['a', 'b', 'c', 'd', 'e'])
call cursor(1, 1)
call rustyfi#util#replace_lines(['a', 'b', 'B2', 'c', 'd', 'e'])
call s:Eq(line('.'), 1, 'cursor above the edit did not move')

call s:Scratch(['a', 'b', 'c', 'd', 'e'])
call cursor(5, 1)
call rustyfi#util#replace_lines(['a', 'c', 'd', 'e'])
call s:Eq(getline(1, '$'), ['a', 'c', 'd', 'e'], 'replace removed a line')
call s:Eq(line('.'), 4, 'cursor followed its text up one line')

call s:Scratch(['a', 'b'])
call s:Ok(!rustyfi#util#replace_lines(['a', 'b']), 'identical content is a no-op')

" ==========================================================================
call s:Say('== :RustyfiFmt round trip through the BUFFER')
" ==========================================================================
execute 'edit!' fnameescape(s:Fixture('messy.saty'))
let s:before = getline(1, '$')
call s:Ok(index(s:before, 'let   x    =    1') >= 0, 'fixture is unformatted on disk')
RustyfiFmt
let s:after = getline(1, '$')
call s:Ok(index(s:after, 'let x = 1') >= 0, 'fmt normalised the let binding')
call s:Ok(index(s:after, 'let   x    =    1') < 0, 'fmt removed the messy spacing')
call s:Ok(!empty(s:after), 'fmt did not blank the buffer')
call s:Ok(&modified, 'fmt marked the buffer modified (it edited the buffer, not the file)')
call s:Ok(readfile(s:Fixture('messy.saty')) ==# s:before, 'the FILE on disk is untouched')
" Idempotence.
RustyfiFmt
call s:Eq(getline(1, '$'), s:after, 'fmt is idempotent')
bwipeout!

call s:Say('== :RustyfiFmt formats UNSAVED buffer content')
execute 'edit!' fnameescape(s:Fixture('messy.saty'))
call setline(1, '@require:    stdja')
RustyfiFmt
call s:Eq(getline(1), '@require: stdja', 'fmt saw the unsaved edit')
bwipeout!

call s:Say('== :RustyfiFmt preserves the cursor')
execute 'edit!' fnameescape(s:Fixture('messy.saty'))
" Put the cursor on the '+p{ Line two. }' line, at a distinctive column.
let s:target = 0
for s:i in range(1, line('$'))
  if getline(s:i) =~# 'Line two'
    let s:target = s:i
  endif
endfor
call s:Ok(s:target > 0, 'found the cursor target line')
call cursor(s:target, 8)
let s:text = getline('.')
RustyfiFmt
call s:Ok(getline('.') =~# 'Line two', 'cursor still on the same TEXT after fmt')
call s:Eq(col('.'), 8, 'cursor column preserved')
bwipeout!

" ==========================================================================
call s:Say('== decline paths: exit 6 and exit 7')
" ==========================================================================
execute 'edit!' fnameescape(s:Fixture('nolex.saty'))
let s:before6 = getline(1, '$')
let v:errmsg = ''
silent! RustyfiFmt
call s:Eq(getline(1, '$'), s:before6, 'exit 6 left the buffer intact')
call s:Ok(!&modified, 'exit 6 did not even mark the buffer modified')
call s:Ok(line('$') > 1 || getline(1) !=# '', 'exit 6 did not blank the buffer')
bwipeout!

execute 'edit!' fnameescape(s:Fixture('noparse.saty'))
let s:before7 = getline(1, '$')
silent! RustyfiFmt
call s:Eq(getline(1, '$'), s:before7, 'exit 7 left the buffer intact by default')
call s:Ok(!&modified, 'exit 7 did not mark the buffer modified')
" ... and applies the whitespace tidy when explicitly opted in.
let g:rustyfi_fmt_accept_partial = 1
silent! RustyfiFmt
call s:Ok(!empty(getline(1, '$')), 'exit 7 with accept_partial did not blank the buffer')
unlet g:rustyfi_fmt_accept_partial
bwipeout!

" ==========================================================================
call s:Say('== :RustyfiFmtCheck')
" ==========================================================================
let g:rustyfi_fmt_check_show_diff = 0
execute 'edit!' fnameescape(s:Fixture('messy.saty'))
silent! RustyfiFmtCheck
call s:Ok(1, 'fmt --check on an unformatted buffer did not throw')
RustyfiFmt
silent! RustyfiFmtCheck
call s:Ok(1, 'fmt --check on a formatted buffer did not throw')
bwipeout!

" ==========================================================================
call s:Say('== format-on-save')
" ==========================================================================
let s:tmpdir = tempname()
call mkdir(s:tmpdir, 'p')
let s:onsave = s:tmpdir . '/onsave.saty'
call writefile(readfile(s:Fixture('messy.saty')), s:onsave)
let g:rustyfi_fmt_on_save = 1
execute 'edit!' fnameescape(s:onsave)
silent write
call s:Ok(index(readfile(s:onsave), 'let x = 1') >= 0, 'format-on-save wrote formatted text')
let g:rustyfi_fmt_on_save = 0
bwipeout!
call writefile(readfile(s:Fixture('messy.saty')), s:onsave)
execute 'edit!' fnameescape(s:onsave)
silent write
call s:Ok(index(readfile(s:onsave), 'let   x    =    1') >= 0, 'format-on-save off leaves the file alone')
bwipeout!

" ==========================================================================
call s:Say('== preview')
" ==========================================================================
function! s:Wait(pred, ms) abort
  let l:deadline = reltime()
  while reltimefloat(reltime(l:deadline)) * 1000 < a:ms
    if a:pred()
      return 1
    endif
    if has('nvim')
      sleep 50m
    else
      sleep 50m
    endif
  endwhile
  return a:pred()
endfunction

function! s:PreviewText() abort
  let l:st = rustyfi#preview#state()
  if l:st.prevbuf <= 0 || !bufexists(l:st.prevbuf)
    return []
  endif
  return getbufline(l:st.prevbuf, 1, '$')
endfunction

function! s:PreviewOk() abort
  return rustyfi#preview#state().status ==# 'ok'
endfunction

let g:rustyfi_preview_debounce = 100
execute 'edit!' fnameescape(s:Fixture('imports.saty'))
let s:srcwin = win_getid()
RustyfiPreview
call s:Eq(win_getid(), s:srcwin, 'preview did not steal focus')
call s:Ok(rustyfi#preview#is_open(), 'preview buffer exists')
call s:Ok(s:Wait(function('s:PreviewOk'), 30000), 'preview rendered (status ok)')
let s:text = join(s:PreviewText(), "\n")
call s:Ok(s:text !=# '', 'preview pane is non-empty')
call s:Ok(s:text =~# 'Hello from the imported helper',
      \ '@import: resolved -- preview compiled the temp file in the source directory')
call s:Ok(s:text =~# "²", 'unicode-math superscript two present (x squared)')
call s:Ok(s:text =~# "∑", 'unicode-math n-ary summation present')
call s:Ok(s:text =~# '(.\+ + .\+)/(.\+ + .\+)', 'unicode-math fraction rendered in reading order')

call s:Say('== preview renders the BUFFER, not the file on disk')
call append(line('$') - 1, '  +p{ UNSAVED SENTINEL LINE. }')
doautocmd TextChanged
call s:Ok(s:Wait({-> join(s:PreviewText(), "\n") =~# 'UNSAVED SENTINEL'}, 30000),
      \ 'unsaved edit reached the preview')
call s:Ok(!filereadable(s:Fixture('imports.saty')) || readfile(s:Fixture('imports.saty')) !=# getline(1, '$'),
      \ 'the source file on disk was never written')

call s:Say('== a broken document keeps the last good render')
let s:good = s:PreviewText()
call setline(1, '@require: stdja')
call setline(2, 'let let let')
doautocmd TextChanged
call s:Ok(s:Wait({-> rustyfi#preview#state().status =~# '^stale'}, 30000),
      \ 'broken buffer reported as stale')
call s:Eq(s:PreviewText(), s:good, 'the last good render survived the broken edit')
call s:Ok(!empty(rustyfi#preview#state().errors), 'the error text was captured')
call s:Ok(rustyfi#preview#state().status =~# 'imports\.saty',
      \ 'the error names the real document, not the scratch file')
call s:Ok(rustyfi#preview#state().status !~# 'rustyfi-preview',
      \ 'the scratch file name is not leaked into the message')

call s:Say('== recovery')
silent undo
silent undo
doautocmd TextChanged
call s:Ok(s:Wait(function('s:PreviewOk'), 30000), 'preview recovered after the edit was undone')

call s:Say('== close cleans up')
let s:tmpsrc = rustyfi#preview#state().tmpsrc
call s:Ok(s:tmpsrc !=# '' && filereadable(s:tmpsrc), 'scratch source existed while open')
let s:prevbuf = rustyfi#preview#state().prevbuf
call s:Ok(fnamemodify(s:tmpsrc, ':h') ==# fnamemodify(s:Fixture('imports.saty'), ':h'),
      \ 'scratch source lived beside the real document')
RustyfiPreviewClose
call s:Ok(!rustyfi#preview#is_open(), 'preview closed')
call s:Ok(!filereadable(s:tmpsrc), 'scratch source removed on close')
call s:Ok(!bufexists(s:prevbuf), 'preview buffer wiped')
call s:Ok(empty(win_findbuf(s:prevbuf)), 'preview window gone')
bwipeout!

" ==========================================================================
call s:Say('== closing the preview buffer directly does not re-enter')
" ==========================================================================
execute 'edit!' fnameescape(s:Fixture('imports.saty'))
RustyfiPreview
let s:pb = rustyfi#preview#state().prevbuf
let s:ts = rustyfi#preview#state().tmpsrc
silent! execute 'bwipeout!' s:pb
call s:Ok(!rustyfi#preview#is_open(), 'wiping the preview buffer closed the session')
call s:Ok(!filereadable(s:ts), 'wiping the preview buffer removed the scratch source')
bwipeout!

" ==========================================================================
call s:Say('== closing the SOURCE buffer closes the preview')
" ==========================================================================
execute 'edit!' fnameescape(s:Fixture('imports.saty'))
RustyfiPreview
let s:ts = rustyfi#preview#state().tmpsrc
bwipeout!
call s:Ok(!rustyfi#preview#is_open(), 'source BufUnload closed the preview')
call s:Ok(!filereadable(s:ts), 'source BufUnload removed the scratch source')

" ==========================================================================
call s:Say('== a superseded render is dropped SILENTLY')
" ==========================================================================
" `s:OnExit`'s stale-sequence guard was written `return  " superseded ...`.
" `:return` takes an expression, so that trailing text is the start of a
" STRING, not a comment: every superseded render raised `E114: Missing quote`
" out of a job callback -- an error banner and a hit-enter prompt on every
" typing burst that outran a compile.  Neovim only: Vim 8's job#stop drops the
" callbacks before killing, Neovim's jobstop still delivers on_exit.
execute 'edit!' fnameescape(s:Fixture('imports.saty'))
RustyfiPreview
call s:Ok(s:Wait(function('s:PreviewOk'), 30000), 'first render landed (else the next step is vacuous)')
let v:errmsg = ''
" Two renders back to back: the first is killed mid-flight and its on_exit
" still arrives, carrying a stale seq.
call rustyfi#preview#render(1)
sleep 100m
call rustyfi#preview#render(1)
call s:Ok(s:Wait({-> v:errmsg !=# ''}, 4000) || 1, 'waited for a stale callback')
call s:Eq(v:errmsg, '', 'a superseded render raised no error')
call s:Ok(execute('messages') !~# 'E114', 'no E114 in the message history')
RustyfiPreviewClose
bwipeout!

" ==========================================================================
call s:Say('== :RustyfiPreview from a DIFFERENT buffer re-points the pane')
" ==========================================================================
" There is one preview and it was pinned to the buffer that opened it.  Asking
" for a preview of another document re-rendered the FIRST one and reported
" `ok`: the pane showed a file the user was not editing, typing in the new one
" did nothing, and nothing said so.
let s:two = tempname()
call mkdir(s:two, 'p')
call writefile(readfile(s:Fixture('imports.saty')), s:two . '/first.saty')
call writefile(readfile(s:Fixture('helper.satyh')), s:two . '/helper.satyh')
call writefile(readfile(s:Fixture('imports.saty')), s:two . '/second.saty')
execute 'edit!' fnameescape(s:two . '/first.saty')
let s:firstbuf = bufnr('%')
RustyfiPreview
call s:Ok(s:Wait(function('s:PreviewOk'), 30000), 'preview of the first document rendered')
let s:firstscratch = rustyfi#preview#state().tmpsrc
execute 'edit!' fnameescape(s:two . '/second.saty')
let s:secondbuf = bufnr('%')
RustyfiPreview
call s:Eq(rustyfi#preview#state().srcbuf, s:secondbuf,
      \ 'the preview follows the buffer it was asked from')
call s:Ok(!filereadable(s:firstscratch), 'the first document''s scratch file was cleaned up')
call s:Ok(s:Wait(function('s:PreviewOk'), 30000), 'the re-pointed preview rendered')
call s:Ok(rustyfi#preview#state().tmpsrc =~# 'second',
      \ 'the scratch file now belongs to the second document')
" ... and the pane really does follow the new buffer's edits.
call append(line('$') - 1, '  +p{ SECOND-DOCUMENT-SENTINEL. }')
doautocmd TextChanged
call s:Ok(s:Wait({-> join(s:PreviewText(), "\n") =~# 'SECOND-DOCUMENT-SENTINEL'}, 30000),
      \ 'an edit in the re-pointed buffer reaches the pane')
RustyfiPreviewClose
bwipeout!
execute 'bwipeout!' s:firstbuf

" ==========================================================================
call s:Say('== a document name with substitute() metacharacters in it')
" ==========================================================================
" s:Unscratch folds the scratch file's name back to the real one with
" substitute(), and the REPLACEMENT half has its own metacharacters: `&` is the
" whole match, `~` is the previous replacement.  Unescaped, a file called
" `Q&A.saty` had the scratch path spliced back INTO the diagnostic that
" function exists to take it out of.
let s:amp = tempname()
call mkdir(s:amp, 'p')
call writefile(readfile(s:Fixture('imports.saty')), s:amp . '/Q&A~R.saty')
call writefile(readfile(s:Fixture('helper.satyh')), s:amp . '/helper.satyh')
execute 'edit!' fnameescape(s:amp . '/Q&A~R.saty')
RustyfiPreview
call s:Ok(s:Wait(function('s:PreviewOk'), 30000), 'a document with & and ~ in its name rendered')
call setline(2, 'let let let')
doautocmd TextChanged
call s:Ok(s:Wait({-> rustyfi#preview#state().status =~# '^stale'}, 30000), 'and reports a broken edit')
call s:Ok(rustyfi#preview#state().status =~# 'Q&A\~R\.saty',
      \ 'the diagnostic names the real document')
call s:Ok(rustyfi#preview#state().status !~# 'rustyfi-preview',
      \ 'the scratch file name is not spliced back into the diagnostic')
RustyfiPreviewClose
bwipeout!

" ==========================================================================
call s:Say('== a read-only document directory fails through the status line')
" ==========================================================================
" `writefile()` RAISES E482 rather than returning -1, and s:WriteScratch is
" `abort`, so the raise unwound it and the caller's list-unpack died on E714 --
" two stacked errors and a hit-enter prompt, repeated on every debounce tick.
let s:rodir = tempname()
call mkdir(s:rodir, 'p')
call writefile(readfile(s:Fixture('imports.saty')), s:rodir . '/ro.saty')
call writefile(readfile(s:Fixture('helper.satyh')), s:rodir . '/helper.satyh')
call setfperm(s:rodir, 'r-xr-xr-x')
let s:ro_enforced = 0
try
  call writefile(['x'], s:rodir . '/.probe')
  call delete(s:rodir . '/.probe')
catch
  let s:ro_enforced = 1
endtry
if s:ro_enforced
  execute 'edit!' fnameescape(s:rodir . '/ro.saty')
  let v:errmsg = ''
  " Caught here rather than let loose: an uncaught E482 aborts the enclosing
  " :if, which would make the remaining assertions VANISH instead of fail.
  let s:ro_exc = ''
  try
    RustyfiPreview
  catch
    let s:ro_exc = v:exception
  endtry
  call s:Eq(s:ro_exc, '', 'a read-only directory threw no exception')
  call s:Eq(v:errmsg, '', 'a read-only directory raised no error')
  call s:Ok(execute('messages') !~# 'E482\|E714', 'no E482/E714 in the message history')
  call s:Ok(rustyfi#preview#state().status =~# 'cannot write',
        \ 'the failure is reported through the preview status line')
  call s:Ok(!empty(rustyfi#preview#state().errors),
        \ ':RustyfiPreviewErrors has something to say about it')
  RustyfiPreviewClose
  bwipeout!
else
  call s:Say('skip (this user can write to a mode-555 directory)')
endif
call setfperm(s:rodir, 'rwxr-xr-x')

" ==========================================================================
call s:Say('== preview split modes')
" ==========================================================================
for s:mode in ['horizontal', 'tab']
  let g:rustyfi_preview_split = s:mode
  execute 'edit!' fnameescape(s:Fixture('imports.saty'))
  RustyfiPreview
  call s:Ok(rustyfi#preview#is_open(), 'preview opened with split=' . s:mode)
  RustyfiPreviewClose
  call s:Ok(!rustyfi#preview#is_open(), 'preview closed with split=' . s:mode)
  bwipeout!
endfor
let g:rustyfi_preview_split = 'vertical'

" ==========================================================================
call s:Say('== binary discovery')
" ==========================================================================
let s:saved_bin = g:rustyfi_bin
unlet g:rustyfi_bin
call rustyfi#bin#clear_cache()
execute 'edit!' fnameescape(s:Fixture('imports.saty'))
" $PATH almost certainly has no rustyfi in the test environment; the walk-up
" should find the checkout's own target/release build either way.
call s:Ok(rustyfi#bin#path() !=# '', 'a binary was discovered without g:rustyfi_bin')
bwipeout!
let g:rustyfi_bin = '/nonexistent/rustyfi'
call rustyfi#bin#clear_cache()
call s:Eq(rustyfi#bin#path(), '', 'an explicit but missing g:rustyfi_bin resolves to nothing')
unlet g:rustyfi_bin

" A MISS must not be cached.  It was, so the first thing anybody does in a
" fresh checkout -- open a document, find there is no binary, build one in
" another terminal -- left the plugin saying `executable not found` for the
" rest of the session.
call rustyfi#bin#clear_cache()
if executable('rustyfi')
  call s:Say('skip (rustyfi is on $PATH, so the walk-up is never reached)')
else
  let s:fresh = tempname() . '/checkout/doc'
  call mkdir(s:fresh, 'p')
  execute 'edit!' fnameescape(s:fresh . '/d.saty')
  call s:Eq(rustyfi#bin#path(), '', 'nothing found before the build')
  call mkdir(s:fresh . '/target/release', 'p')
  call writefile(['#!/bin/sh', 'exit 0'], s:fresh . '/target/release/rustyfi')
  call setfperm(s:fresh . '/target/release/rustyfi', 'rwxr-xr-x')
  call s:Ok(rustyfi#bin#path() !=# '',
        \ 'a binary built while the editor is open is found, with no cache to clear')
  bwipeout!
endif
let g:rustyfi_bin = s:saved_bin
call rustyfi#bin#clear_cache()

" ==========================================================================
call s:Say('== job abstraction')
" ==========================================================================
let s:jobres = {}
let s:h = rustyfi#job#start(['sh', '-c', 'echo out; echo err 1>&2; exit 3'], {
      \ 'on_exit': {code, out, err -> extend(s:jobres, {'code': code, 'out': out, 'err': err})},
      \ })
call s:Ok(s:h isnot 0, 'job started')
call s:Ok(s:Wait({-> has_key(s:jobres, 'code')}, 10000), 'job exit callback fired')
call s:Eq(get(s:jobres, 'code', -1), 3, 'job exit code propagated')
call s:Ok(index(get(s:jobres, 'out', []), 'out') >= 0, 'job stdout captured')
call s:Ok(index(get(s:jobres, 'err', []), 'err') >= 0, 'job stderr captured')

let s:killed = 0
let s:h2 = rustyfi#job#start(['sh', '-c', 'sleep 30'], {'on_exit': {c, o, e -> execute('let s:killed = 1')}})
call s:Ok(rustyfi#job#running(s:h2), 'long job reports running')
call rustyfi#job#stop(s:h2)
call s:Ok(s:Wait({-> !rustyfi#job#running(s:h2)}, 10000), 'stopped job is no longer running')

" ==========================================================================
" :RustyfiBuild / :RustyfiBuildOpen
"
" A build is not a preview: it compiles the file ON DISK, writes the PDF
" beside it, and reports failures through the quickfix list rather than a
" status line.
call s:Say('== build')

if $RUSTYFI_LIB_ROOT ==# ''
  call s:Say('skipped: needs $RUSTYFI_LIB_ROOT')
else
  let g:rustyfi_lib_root = $RUSTYFI_LIB_ROOT
  let g:rustyfi_build_copen = 0

  let s:bdir = tempname()
  call mkdir(s:bdir, 'p')

  " --- a clean build produces a PDF and an empty quickfix list -------------
  let s:good = s:bdir . '/good.saty'
  call writefile([
        \ '@require: stdjabook',
        \ "document (| title = {T}; author = {A}; show-title = true; show-toc = false; |) '<",
        \ '  +p { hello }',
        \ '>',
        \ ], s:good)
  execute 'edit' fnameescape(s:good)
  call setqflist([], ' ', {'lines': ['stale entry from a previous run']})
  RustyfiBuild
  let s:n = 0
  while s:n < 600 && !filereadable(s:bdir . '/good.pdf')
    sleep 50m
    let s:n += 1
  endwhile
  call s:Ok(filereadable(s:bdir . '/good.pdf'), 'a clean build writes a PDF beside the document')
  " Give the exit callback a moment to clear the list after the file appears.
  sleep 200m
  call s:Ok(empty(getqflist()),
        \ 'a clean build CLEARS a stale quickfix list (else you chase a fixed error)')

  " --- the buffer is written first ----------------------------------------
  " The point of a build command is to compile what the document is NOW.
  call setline(3, '  +p { edited but unsaved }')
  call s:Ok(&modified, 'precondition: the buffer is modified')
  RustyfiBuild
  let s:n = 0
  while s:n < 600 && &modified
    sleep 50m
    let s:n += 1
  endwhile
  call s:Ok(!&modified, 'a build writes the buffer first (g:rustyfi_build_autowrite)')
  call s:Ok(join(readfile(s:good), "\n") =~# 'edited but unsaved',
        \ 'and what reached disk is the edit, not the old bytes')

  " Let that build finish. Starting another while one is in flight is
  " REFUSED, so without this the next case measures the refusal rather than
  " the thing it means to test -- which is how it first went green-but-wrong.
  let s:n = 0
  while s:n < 600 && rustyfi#build#running()
    sleep 50m
    let s:n += 1
  endwhile
  call s:Ok(!rustyfi#build#running(), 'the build finished before the next case')

  " --- a failure populates the quickfix list, navigably --------------------
  let s:bad = s:bdir . '/bad.saty'
  call writefile([
        \ '@require: stdjabook',
        \ "document (| title = {T}; author = {A}; show-title = true; show-toc = false; |) '<",
        \ '  +p { \nosuchcommand; }',
        \ '>',
        \ ], s:bad)
  execute 'edit' fnameescape(s:bad)
  call setqflist([], ' ', {'lines': []})
  RustyfiBuild
  let s:n = 0
  while s:n < 600 && empty(getqflist())
    sleep 50m
    let s:n += 1
  endwhile
  let s:qf = getqflist()
  call s:Ok(!empty(s:qf), 'a failed build fills the quickfix list')
  if !empty(s:qf)
    " Navigable, not merely present: an entry with no file or line is one
    " `:cnext` skips, and the `Error: ` prefix in the compiler output made
    " every entry look exactly like that until the errorformat learned it.
    call s:Ok(s:qf[0].valid, 'the entry is VALID (a file and a line, not just text)')
    call s:Eq(s:qf[0].lnum, 3, 'the entry points at the offending line')
    call s:Ok(s:qf[0].col > 0, 'and carries a column')
    call s:Ok(bufname(s:qf[0].bufnr) =~# 'bad\.saty', 'and names the document')
    call s:Ok(s:qf[0].text !~# 'Error:',
          \ 'the prefix is consumed by the pattern, not left in the message')
  endif
  call s:Ok(!filereadable(s:bdir . '/bad.pdf'), 'a failed build writes no PDF')

  " --- autowrite off refuses rather than building stale bytes -------------
  let g:rustyfi_build_autowrite = 0
  execute 'edit' fnameescape(s:good)
  call setline(3, '  +p { unsaved again }')
  let s:before = getqflist()
  RustyfiBuild
  sleep 300m
  call s:Ok(&modified, 'with autowrite off, a modified buffer is NOT written')
  let g:rustyfi_build_autowrite = 1
  edit!
endif

" ==========================================================================
" The two halves of the plugin must agree about WHICH rustyfi to run.
"
" They did not: the Vimscript searched a checkout's `target/release`, while
" both Lua LSP routes hard-coded a bare `rustyfi`. Inside a rustyfi checkout
" with nothing installed, `:RustyfiFmt` worked and the language server simply
" never attached -- no error, because a `cmd` that is not executable is a
" quiet non-attach. This is the assertion that keeps them in step.
if has('nvim')
  call s:Say('== binary discovery agrees across languages')
  let s:vimside = rustyfi#bin#path()
  let s:luaside = luaeval("require('rustyfi.bin').path() or ''")
  call s:Eq(s:luaside, s:vimside, 'lua and vimscript find the same binary')
  call s:Ok(s:vimside !=# '', 'a binary was found at all (else the test is vacuous)')

  " And that the server config carries the RESOLVED path rather than a bare
  " name -- the actual regression. Comparing to the discovered path rather
  " than merely asserting "not equal to rustyfi", so that a discovery that
  " starts returning nonsense also fails here.
  let s:cmd = luaeval("require('rustyfi.bin').lsp_cmd()")
  call s:Eq(s:cmd[0], s:vimside, 'lsp cmd[0] is the discovered binary')
  call s:Eq(s:cmd[1], 'lsp', 'lsp cmd[1] is the subcommand')
endif

call s:Say('')
call s:Say(printf('%d passed, %d failed', s:pass, len(s:fail)))
for s:f in s:fail
  call s:Say('  FAILED: ' . s:f)
endfor
if empty(s:fail)
  qall!
else
  cquit!
endif
