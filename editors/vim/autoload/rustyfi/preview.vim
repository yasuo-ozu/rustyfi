" autoload/rustyfi/preview.vim -- live unicode-math preview in a scratch split.
"
" Pipeline:  rustyfi <file> --format markdown --unicode-math
" `--unicode-math` writes equations as their characters in reading order
" (x^2 as a superscript two, sums with real sub/superscripts, a/b fractions),
" which is the one output form that is plain text -- so the preview is a
" buffer, not a browser.
"
" Two facts about the CLI shape this file:
"
"  1. There is NO stdout mode.  `-o -` creates a file literally named `-` in
"     the working directory; it does not write to standard output.  So the
"     render goes to a temp file which we then read back.
"
"  2. The compiler does not read a document from stdin, and `@import:` is
"     resolved relative to the INPUT FILE's directory (verified: the same
"     source compiles from its own directory and fails with
"     "cannot resolve `@import: helper`" from anywhere else).  `@require:`
"     discovers its lib root by walking up from that directory too.  So the
"     buffer is written to a dotfile *beside the real file*, not into
"     $TMPDIR, and is removed when the preview closes.

let s:state = {
      \ 'srcbuf': -1,
      \ 'prevbuf': -1,
      \ 'job': 0,
      \ 'seq': 0,
      \ 'timer': 0,
      \ 'tmpsrc': '',
      \ 'tmpout': '',
      \ 'status': '',
      \ 'errors': [],
      \ 'rendered': 0,
      \ 'bin': '',
      \ }

function! rustyfi#preview#is_open() abort
  return s:state.prevbuf > 0 && bufexists(s:state.prevbuf)
endfunction

function! s:PrevWin() abort
  if !rustyfi#preview#is_open()
    return 0
  endif
  let l:wins = win_findbuf(s:state.prevbuf)
  return empty(l:wins) ? 0 : l:wins[0]
endfunction

" --------------------------------------------------------------------------
" Opening / closing
" --------------------------------------------------------------------------

function! rustyfi#preview#open() abort
  " Resolve the executable once, here: the walk-up search keys on the CURRENT
  " buffer's directory, and a timer callback can fire while the preview
  " scratch buffer is current.
  let l:bin = rustyfi#bin#require()
  if l:bin ==# ''
    return
  endif
  let s:state.bin = l:bin
  if rustyfi#preview#is_open()
    " There is ONE preview, and it was pinned to whichever buffer opened it.
    " Asking for a preview from a DIFFERENT document used to re-render the old
    " one and report `ok`, so the pane showed a file the user was not editing
    " and typing in the new one changed nothing -- with no message anywhere.
    " Re-point it instead.  (Called from inside the pane itself, `q`/`R`, the
    " current buffer IS the preview: leave the target alone.)
    let l:here = bufnr('%')
    if l:here != s:state.srcbuf && l:here != s:state.prevbuf
      call s:CancelJob()
      call s:CleanTemps()
      let s:state.srcbuf = l:here
      let s:state.rendered = 0
      let s:state.errors = []
      call s:InstallAutocmds()
    endif
    call rustyfi#preview#render(1)
    return
  endif

  let s:state.srcbuf = bufnr('%')
  let l:cur = win_getid()

  let l:cmd = get(g:, 'rustyfi_preview_split', 'vertical')
  if l:cmd ==# 'vertical'
    execute 'vertical rightbelow ' . get(g:, 'rustyfi_preview_width', 62) . 'new'
  elseif l:cmd ==# 'horizontal'
    execute 'rightbelow ' . get(g:, 'rustyfi_preview_height', 15) . 'new'
  else
    tabnew
  endif

  let s:state.prevbuf = bufnr('%')
  setlocal buftype=nofile bufhidden=hide noswapfile nobuflisted
  setlocal nomodifiable nonumber norelativenumber signcolumn=no
  setlocal winfixwidth
  silent! execute 'file' fnameescape('[rustyfi preview]')
  let &l:filetype = get(g:, 'rustyfi_preview_filetype', 'markdown')
  " After the filetype, because the treesitter layer attaches to the buffer
  " and the matches are window-local -- both want the window as it will be.
  call rustyfi#rich#apply()
  let &l:statusline = '%!rustyfi#preview#statusline()'
  nnoremap <buffer><silent> q :RustyfiPreviewClose<CR>
  nnoremap <buffer><silent> R :RustyfiPreviewRefresh<CR>

  call win_gotoid(l:cur)
  call s:InstallAutocmds()
  call rustyfi#preview#render(1)
endfunction

function! rustyfi#preview#close() abort
  " BufWipeout on the preview buffer calls this, and so does closing it
  " explicitly; the bwipeout below would then re-enter.  Clearing the
  " autocommand group first stops new events, and this flag stops the one
  " already in flight.
  if get(s:state, 'closing', 0)
    return
  endif
  let s:state.closing = 1
  call s:CancelJob()
  if s:state.timer
    call timer_stop(s:state.timer)
    let s:state.timer = 0
  endif
  silent! autocmd! rustyfi_preview
  if rustyfi#preview#is_open()
    let l:b = s:state.prevbuf
    silent! execute 'bwipeout!' l:b
  endif
  call s:CleanTemps()
  let s:state.prevbuf = -1
  let s:state.srcbuf = -1
  let s:state.rendered = 0
  let s:state.status = ''
  let s:state.errors = []
  let s:state.bin = ''
  let s:state.closing = 0
endfunction

function! rustyfi#preview#toggle() abort
  if rustyfi#preview#is_open()
    call rustyfi#preview#close()
  else
    call rustyfi#preview#open()
  endif
endfunction

function! s:CleanTemps() abort
  if s:state.tmpsrc !=# ''
    call delete(s:state.tmpsrc)
    let s:state.tmpsrc = ''
  endif
  if s:state.tmpout !=# ''
    call delete(s:state.tmpout)
    call delete(s:state.tmpout . '.aux')
    let s:state.tmpout = ''
  endif
endfunction

function! s:InstallAutocmds() abort
  augroup rustyfi_preview
    autocmd!
    execute 'autocmd TextChanged,TextChangedI <buffer=' . s:state.srcbuf . '> call rustyfi#preview#schedule()'
    execute 'autocmd InsertLeave,BufWritePost <buffer=' . s:state.srcbuf . '> call rustyfi#preview#schedule()'
    execute 'autocmd BufUnload <buffer=' . s:state.srcbuf . '> call rustyfi#preview#close()'
    execute 'autocmd BufWipeout <buffer=' . s:state.prevbuf . '> call rustyfi#preview#close()'
    autocmd VimLeavePre * call rustyfi#preview#close()
  augroup END
endfunction

" --------------------------------------------------------------------------
" Debounce
" --------------------------------------------------------------------------

" A render of a real corpus document costs 0.2 s (latexcmds) to 2.7-3.9 s
" (xpath, heavy vector graphics), median around 0.7-1.3 s, measured with
" `--format markdown --unicode-math` on this machine.  A debounce shorter than
" the render is pointless -- it only queues work -- and one longer than a
" typing pause stops feeling live.  500 ms sits just past a normal
" inter-keystroke gap, so a burst of typing produces exactly one render; the
" in-flight job is killed when the next one starts, so a slow document
" self-throttles instead of backing up.
function! s:Delay() abort
  return get(g:, 'rustyfi_preview_debounce', 500)
endfunction

function! rustyfi#preview#schedule() abort
  if !rustyfi#preview#is_open()
    return
  endif
  if s:state.timer
    call timer_stop(s:state.timer)
  endif
  let s:state.timer = timer_start(s:Delay(), function('s:OnTimer'))
endfunction

function! s:OnTimer(timer) abort
  let s:state.timer = 0
  call rustyfi#preview#render(0)
endfunction

" --------------------------------------------------------------------------
" Rendering
" --------------------------------------------------------------------------

function! s:CancelJob() abort
  if s:state.job isnot 0
    " Bump the sequence first: a callback that still arrives from the killed
    " job carries a stale seq and is dropped, so a cancelled render can never
    " overwrite the pane a newer one is filling.
    let s:state.seq += 1
    call rustyfi#job#stop(s:state.job)
    let s:state.job = 0
  endif
endfunction

" Write the buffer next to the real file so relative @import:/@require:
" resolution is unchanged.  Returns [srcpath, cwd] or ['', ''] on failure.
function! s:WriteScratch() abort
  let l:lines = getbufline(s:state.srcbuf, 1, '$')
  let l:name = bufname(s:state.srcbuf)
  if l:name ==# '' || !filereadable(l:name)
    " Never-saved buffer: the working directory is the best guess at where its
    " neighbours are.
    let l:dir = getcwd()
    let l:base = 'unnamed'
  else
    let l:full = fnamemodify(l:name, ':p')
    let l:dir = fnamemodify(l:full, ':h')
    let l:base = fnamemodify(l:full, ':t:r')
  endif
  let l:path = l:dir . '/.' . l:base . '.rustyfi-preview.saty'
  if s:state.tmpsrc !=# '' && s:state.tmpsrc !=# l:path
    call delete(s:state.tmpsrc)
  endif
  " `writefile()` does not merely return -1 on failure, it raises E482 -- and
  " this function is `abort`, so the raise unwound it and the caller's
  " `let [l:src, l:dir] = ...` then died on E714 (List required), twice per
  " render, forever, in any directory the user cannot write to (a read-only
  " checkout, a document opened from a package tree).  Catch it, and report
  " through the status line like every other failure.
  try
    if writefile(l:lines, l:path) != 0
      return ['', l:dir]
    endif
  catch
    return ['', l:dir]
  endtry
  let s:state.tmpsrc = l:path
  return [l:path, l:dir]
endfunction

function! rustyfi#preview#render(force) abort
  if !rustyfi#preview#is_open()
    return
  endif
  let l:bin = s:state.bin
  if l:bin ==# ''
    return
  endif

  call s:CancelJob()

  let [l:src, l:dir] = s:WriteScratch()
  if l:src ==# ''
    call s:SetStatus('cannot write the preview scratch file into ' . l:dir,
          \ ['rustyfi preview: could not write a scratch copy of the buffer into '
          \  . l:dir . ' -- the preview compiles a file beside the real document so'
          \  . ' that @import: and @require: resolve the way they do for the real'
          \  . ' one, and that directory is not writable.'])
    return
  endif

  if s:state.tmpout ==# ''
    let s:state.tmpout = tempname() . '.md'
  endif
  call delete(s:state.tmpout)

  let l:argv = [l:bin, fnamemodify(l:src, ':t')]
  call extend(l:argv, ['--format', 'markdown', '--unicode-math'])
  " Keep the fixpoint's auxiliary file out of the user's tree: without this the
  " compiler writes `<scratch>.satysfi-aux` beside the real document.
  call extend(l:argv, ['--aux-file', s:state.tmpout . '.aux'])
  call extend(l:argv, ['-o', s:state.tmpout])
  if get(g:, 'rustyfi_lib_root', '') !=# ''
    call extend(l:argv, ['--lib-root', g:rustyfi_lib_root])
  endif
  call extend(l:argv, get(g:, 'rustyfi_preview_args', []))

  let s:state.seq += 1
  let l:seq = s:state.seq
  call s:SetStatus(s:state.rendered ? 'rendering...' : 'first render...', s:state.errors)
  let s:state.job = rustyfi#job#start(l:argv, {
        \ 'cwd': l:dir,
        \ 'on_exit': function('s:OnExit', [l:seq]),
        \ })
  if s:state.job is 0
    call s:SetStatus('could not start ' . l:bin, ['rustyfi preview: job start failed'])
  endif
endfunction

function! s:OnExit(seq, code, out, err) abort
  " Superseded by a newer render: drop this result.
  "
  " The comment is on its OWN line on purpose.  `:return` takes an
  " expression, so a trailing `" ...` is parsed as the start of a STRING and
  " not as a comment -- it raised `E114: Missing quote` out of every job
  " callback a supersede reached, which on Neovim is every keystroke burst
  " that outruns a compile (Vim 8 never saw it: its job#stop drops the
  " callbacks before killing, Neovim's jobstop still delivers on_exit).
  if a:seq != s:state.seq
    return
  endif
  let s:state.job = 0
  call delete(s:state.tmpout . '.aux')

  if a:code == 0 && filereadable(s:state.tmpout)
    let l:lines = readfile(s:state.tmpout)
    if !empty(filter(copy(l:lines), 'v:val !~# "^\\s*$"'))
      call s:Fill(l:lines)
      let s:state.rendered = 1
      call s:SetStatus('ok', [])
      return
    endif
  endif

  " Failure: KEEP the last good render.  A document mid-edit is broken most of
  " the time; clearing the pane on every transient parse error makes the
  " feature useless.
  let l:msg = s:FirstError(a:out, a:err, a:code)
  call s:SetStatus((s:state.rendered ? 'stale: ' : 'error: ') . l:msg,
        \ map(filter(a:err + a:out, 'v:val !~# "^\\s*$"'), 's:Unscratch(v:val)'))
  if !s:state.rendered
    call s:Fill(['rustyfi preview: nothing rendered yet.', ''] + s:state.errors)
  endif
endfunction

function! s:FirstError(out, err, code) abort
  for l:line in a:err + a:out
    if l:line !~# '^\s*$'
      return s:Unscratch(substitute(l:line, '^\s*', '', ''))
    endif
  endfor
  return 'exit code ' . a:code
endfunction

" The compiler names the scratch file it was handed; the reader thinks in
" terms of their own document, so put that name back.
function! s:Unscratch(msg) abort
  if s:state.tmpsrc ==# ''
    return a:msg
  endif
  let l:real = bufname(s:state.srcbuf)
  let l:real = l:real ==# '' ? '[buffer]' : fnamemodify(l:real, ':t')
  " The REPLACEMENT half of substitute() has its own metacharacters: `&` is
  " the whole match and `~` is the previous replacement.  Unescaped, a file
  " called `Q&A.saty` spliced the scratch path back INTO the message this
  " function exists to take it out of.
  let l:sub = escape(l:real, '\&~')
  let l:msg = substitute(a:msg, '\V' . escape(s:state.tmpsrc, '\'), l:sub, 'g')
  return substitute(l:msg, '\V' . escape(fnamemodify(s:state.tmpsrc, ':t'), '\'), l:sub, 'g')
endfunction

function! s:Fill(lines) abort
  let l:win = s:PrevWin()
  if l:win == 0
    return
  endif
  let l:cur = win_getid()
  noautocmd call win_gotoid(l:win)
  let l:view = winsaveview()
  setlocal modifiable
  silent! call deletebufline(s:state.prevbuf, 1, '$')
  call setline(1, a:lines)
  setlocal nomodifiable
  call winrestview(l:view)
  noautocmd call win_gotoid(l:cur)
  redraw
endfunction

function! s:SetStatus(status, errors) abort
  let s:state.status = a:status
  let s:state.errors = a:errors
  let l:win = s:PrevWin()
  if l:win != 0
    call setwinvar(l:win, '&statusline', '%!rustyfi#preview#statusline()')
  endif
  redrawstatus!
endfunction

function! rustyfi#preview#statusline() abort
  let l:s = s:state.status
  if l:s ==# 'ok'
    return '[rustyfi preview]%=ok '
  endif
  return '[rustyfi preview] ' . l:s . '%='
endfunction

" :RustyfiPreviewErrors -- the full compiler output for the last failure.
function! rustyfi#preview#errors() abort
  if empty(s:state.errors)
    call rustyfi#util#info('rustyfi preview: no errors')
    return
  endif
  botright new
  setlocal buftype=nofile bufhidden=wipe noswapfile nobuflisted
  silent! file [rustyfi\ preview\ errors]
  call setline(1, s:state.errors)
  setlocal nomodifiable
endfunction

" Test / introspection hook.
function! rustyfi#preview#state() abort
  return copy(s:state)
endfunction
