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
