" Rich rendering for the preview window.
"
" The preview is Markdown, and a Markdown buffer shown as plain text wastes
" most of what the format carries: headings look like body text with a `#` in
" front, every inline `code` span is fenced in visible backticks, and a wall
" of CJK prose runs off the right edge because the window does not wrap.
"
" Three layers, applied in this order, each degrading on its own:
"
"   1. WINDOW options -- wrapping, conceal. Work everywhere, Vim included.
"   2. TREESITTER highlighting, when Neovim has the `markdown` parser. Its
"      bundled queries already conceal backtick pairs, emphasis markers and
"      list bullets once 'conceallevel' is 2, which is most of "rich" for
"      free -- and correctly, because it is parsing rather than pattern
"      matching.
"   3. A few MATCHES on top for what neither layer styles: the heading rule,
"      the blockquote bar. `matchadd()` is window-local and exists in both
"      editors, so this layer is the same code for both.
"
" Turn the whole thing off with `g:rustyfi_preview_rich = 0` -- which is also
" what somebody running render-markdown.nvim or markview.nvim wants, since
" those attach to the buffer's `markdown` filetype by themselves and layer 3
" would be drawing over them.

function! s:HasTreesitterMarkdown() abort
  if !has('nvim')
    return 0
  endif
  " `vim.treesitter.start()` throws when the parser is absent, and a preview
  " that raises on open is worse than a plain one -- so ask first.
  return luaeval("(function() local ok = pcall(vim.treesitter.language.inspect, 'markdown') return ok and 1 or 0 end)()")
endfunction

" Style only what the highlight layers leave alone, and only in THIS window.
"
" `matchadd()` is window-local, so these vanish with the window and cannot
" leak into a real Markdown file the user opens later -- which an
" `after/syntax/markdown.vim` would have done.
function! s:AddMatches() abort
  " A heading, whatever its level: the `#` run and the space after it are
  " concealed by layer 2 on Neovim, so what is left to do is make the text
  " itself read as a heading.
  call matchadd('rustyfiPreviewHeading', '^#\{1,6}\s.*$', 10)
  " The bar down the left of a blockquote. Matching the marker rather than
  " the line so the quoted text keeps its own highlighting.
  call matchadd('rustyfiPreviewQuote', '^\s*>\+', 11)
  " A fence line. Concealing it entirely would silently merge a code block
  " into the prose around it; dimming it keeps the boundary visible and out
  " of the way.
  call matchadd('rustyfiPreviewFence', '^\s*```.*$', 11)
  " The stale marker the renderer writes into the first line when a compile
  " failed and the last good render is still on screen.
  call matchadd('rustyfiPreviewStale', '^\s*stale:.*$', 12)
endfunction

" Hide the markers that carry no information once the text around them is
" styled. Buffer-local by construction: `syntax` commands always are.
function! s:ConcealMarkers() abort
  " The `#` run and the space after it. The heading itself is highlighted by
  " `s:AddMatches`, so nothing is lost by hiding the marker -- and a heading
  " that reads as a heading is the single biggest difference between "a
  " Markdown file" and "a rendered document".
  syntax match rustyfiPreviewHeadMarker '^#\{1,6}\s' conceal
  " A list bullet becomes a real one. `cchar` needs the match to be exactly
  " what is replaced, so the marker and its space are separate from the item.
  syntax match rustyfiPreviewBullet '^\s*\zs[-*+]\ze\s' conceal cchar=•
endfunction

function! s:Highlights() abort
  " Linked, never coloured literally: the user's colourscheme decides what
  " these look like, and a hard-coded palette would fight every theme.
  highlight default link rustyfiPreviewHeading Title
  highlight default link rustyfiPreviewQuote   Comment
  highlight default link rustyfiPreviewFence   NonText
  highlight default link rustyfiPreviewStale   WarningMsg
endfunction

" Apply the rich layer to the CURRENT window and buffer.
"
" Called with the preview window current, after its filetype is set.
function! rustyfi#rich#apply() abort
  if !get(g:, 'rustyfi_preview_rich', 1)
    return
  endif

  " Layer 1. Prose wants wrapping: `nowrap` on a preview whose content is
  " paragraphs means a CJK line runs off the edge and the reader scrolls
  " horizontally to read a sentence. `linebreak` breaks at spaces rather than
  " mid-word, and `breakindent` keeps a wrapped list item under its own text.
  setlocal wrap linebreak breakindent
  " 'showbreak' is GLOBAL-LOCAL and an empty local value means "use the
  " global one" -- so this cannot be cleared, only replaced. It has to be:
  " a global `showbreak=>` (a common setting, and the one on this machine)
  " puts a `>` at the head of every wrapped line, which in a Markdown pane
  " reads as a blockquote marker on text that is not quoted. A single space
  " is the closest thing to "no marker" the option can express -- `NONE` is
  " not a sentinel here, it sets the literal string NONE and displays it.
  let &l:showbreak = get(g:, 'rustyfi_preview_showbreak', ' ')
  " Conceal is what turns `**a**` into a bold `a`. `concealcursor` matters:
  " without it the line under the cursor un-conceals as you move, which in a
  " read-only preview is flicker with no upside -- there is nothing to edit.
  setlocal conceallevel=2 concealcursor=nvic

  " Layer 2.
  if s:HasTreesitterMarkdown()
    " Errors are swallowed on purpose: a preview that opens plain beats one
    " that opens with a stack trace, and layers 1 and 3 still applied.
    silent! call luaeval("(function() pcall(vim.treesitter.start, tonumber(_A), 'markdown') return 0 end)()", bufnr('%'))
  endif

  " Layer 2b -- conceal what layer 2 leaves visible.
  "
  " Neovim's bundled Markdown queries conceal a code fence and the backticks
  " of an inline span, but NOT an ATX heading's `#` run: measured on the
  " corpus preview, `# 1. インストール` reaches the screen with its marker.
  " A `syntax match` conceal still applies with the treesitter highlighter
  " running -- the two layers answer different questions, colour and
  " visibility -- so this is additive rather than a competing highlighter.
  call s:ConcealMarkers()

  " Layer 3.
  call s:Highlights()
  call s:AddMatches()
endfunction
