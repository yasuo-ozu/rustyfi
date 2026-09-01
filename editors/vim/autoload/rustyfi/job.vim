" autoload/rustyfi/job.vim -- one async-process surface over Neovim's
" jobstart() and Vim 8's job_start().
"
" The two APIs differ in three ways that matter here:
"   * callback names and arities (on_exit(id, code, event) vs exit_cb(job, code));
"   * output delivery -- Neovim hands you a list of lines with a possibly-empty
"     last element meaning "no trailing newline", Vim 8 hands you a string per
"     callback with out_mode/err_mode deciding the chunking;
"   * stopping -- jobstop() vs job_stop(), and Vim's job object stays queryable
"     after exit while Neovim's id does not.
"
" rustyfi#job#start(argv, opts) -> handle (opaque; 0 on failure)
"   opts.cwd      : directory to run in (both back ends support it)
"   opts.on_exit  : funcref(code, stdout_lines, stderr_lines)
"
" Output is buffered and delivered once, at exit, which is all the preview
" needs and is the only shape both back ends give cheaply.

let s:vim_state = {}

function! s:NvimExit(opts, state, id, code, event) abort
  if has_key(a:opts, 'on_exit')
    call a:opts.on_exit(a:code, a:state.out, a:state.err)
  endif
endfunction

function! rustyfi#job#start(argv, opts) abort
  if has('nvim')
    let l:state = {'out': [], 'err': []}
    let l:jobopts = {
          \ 'stdout_buffered': v:true,
          \ 'stderr_buffered': v:true,
          \ 'on_stdout': {id, data, ev -> extend(l:state.out, data)},
          \ 'on_stderr': {id, data, ev -> extend(l:state.err, data)},
          \ 'on_exit': function('s:NvimExit', [a:opts, l:state]),
          \ }
    if has_key(a:opts, 'cwd')
      let l:jobopts.cwd = a:opts.cwd
    endif
    try
      let l:id = jobstart(a:argv, l:jobopts)
    catch
      return 0
    endtry
    return l:id > 0 ? l:id : 0
  endif

  " Vim 8
  let l:state = {'out': [], 'err': [], 'opts': a:opts, 'code': -1, 'done': 0}
  let l:jobopts = {
        \ 'out_mode': 'nl',
        \ 'err_mode': 'nl',
        \ 'in_io': 'null',
        \ 'out_cb': function('s:VimOut', [l:state]),
        \ 'err_cb': function('s:VimErr', [l:state]),
        \ 'exit_cb': function('s:VimExit', [l:state]),
        \ 'close_cb': function('s:VimClose', [l:state]),
        \ }
  if has_key(a:opts, 'cwd')
    let l:jobopts.cwd = a:opts.cwd
  endif
  try
    let l:job = job_start(a:argv, l:jobopts)
  catch
    return 0
  endtry
  if job_status(l:job) ==# 'fail'
    return 0
  endif
  let l:state.job = l:job
  let s:vim_state[s:JobKey(l:job)] = l:state
  return l:job
endfunction

function! s:JobKey(job) abort
  return substitute(string(a:job), '\D', '', 'g')
endfunction

function! s:VimOut(state, ch, msg) abort
  call add(a:state.out, a:msg)
endfunction

function! s:VimErr(state, ch, msg) abort
  call add(a:state.err, a:msg)
endfunction

" exit_cb can fire before the channel has been drained; close_cb fires when it
" has.  Only when BOTH have happened is the output complete, so each arms the
" other and whichever is second delivers.
function! s:VimExit(state, job, code) abort
  let a:state.code = a:code
  let a:state.done += 1
  call s:VimMaybeDeliver(a:state)
endfunction

function! s:VimClose(state, ch) abort
  let a:state.done += 1
  call s:VimMaybeDeliver(a:state)
endfunction

function! s:VimMaybeDeliver(state) abort
  if a:state.done < 2
    return
  endif
  if has_key(s:vim_state, s:JobKey(a:state.job))
    call remove(s:vim_state, s:JobKey(a:state.job))
  endif
  if has_key(a:state.opts, 'on_exit')
    call a:state.opts.on_exit(a:state.code, a:state.out, a:state.err)
  endif
endfunction

function! rustyfi#job#running(handle) abort
  if empty(a:handle) || a:handle is 0
    return 0
  endif
  if has('nvim')
    try
      return jobwait([a:handle], 0)[0] == -1
    catch
      return 0
    endtry
  endif
  return job_status(a:handle) ==# 'run'
endfunction

" Stop a job.
"
" On VIM 8 the callbacks are dropped first, so on_exit is guaranteed NOT to be
" delivered.  On NEOVIM there is no way to unregister a running job's
" callbacks: jobstop() sends the signal and on_exit still arrives, carrying the
" signal's exit code.  So a caller must NOT rely on silence here -- the preview
" carries its own sequence number and drops a stale delivery (s:OnExit).  This
" asymmetry is why the stale-guard bug in that function only ever fired on
" Neovim, and why the guard has to exist at all.
function! rustyfi#job#stop(handle) abort
  if empty(a:handle) || a:handle is 0
    return
  endif
  if has('nvim')
    try
      call jobstop(a:handle)
    catch
    endtry
    return
  endif
  let l:key = s:JobKey(a:handle)
  if has_key(s:vim_state, l:key)
    " Drop the callbacks before killing, so a delivery in flight is discarded.
    let s:vim_state[l:key].opts = {}
    call remove(s:vim_state, l:key)
  endif
  try
    call job_stop(a:handle, 'kill')
  catch
  endtry
endfunction
