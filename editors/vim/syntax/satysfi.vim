" syntax/satysfi.vim -- deliberately minimal SATySFi highlighting.
"
" It models the one structural distinction that matters for not lying:
" SATySFi has a PROGRAM mode and a TEXT mode, and `{ ... }` always enters
" inline text while `'< ... >` enters block text.  Keywords are highlighted
" only in program mode, so the English word "in" inside a paragraph is left
" alone.  Anything finer -- per-command argument arity, math structure -- is
" out of scope on purpose; `:RustyfiFmt` and the language server are where
" real understanding of the source lives.
"
" Guarded by b:current_syntax, so if another plugin's satysfi syntax loaded
" first this file is a no-op rather than a fight.

if exists('b:current_syntax')
  finish
endif

syn case match
syn sync minlines=200

" ---- comments -------------------------------------------------------------
syn match satysfiComment "%.*$" contains=@Spell

" ---- headers --------------------------------------------------------------
syn match satysfiHeader "^\s*@\%(require\|import\|stage\)\s*:.*$" contains=satysfiHeaderKey
syn match satysfiHeaderKey "@\%(require\|import\|stage\)\s*:" contained

" ---- literals -------------------------------------------------------------
" A string literal is delimited by a run of backticks and closed by a run of
" the same length.  A Vim regex cannot count, so the three widths that occur
" in practice are spelled out, longest first.
syn region satysfiString start="```" end="```" keepend contains=@Spell
syn region satysfiString start="``"  end="``"  keepend contains=@Spell
syn region satysfiString start="`"   end="`"   keepend contains=@Spell

syn match satysfiNumber "\<0[xX]\x\+\>"
syn match satysfiLength "\<\d\+\%(\.\d\+\)\?\%(pt\|cm\|mm\|inch\|in\|ex\|em\)\>"
syn match satysfiNumber "\<\d\+\%(\.\d\+\)\?\>"

" ---- program mode ---------------------------------------------------------
syn keyword satysfiKeyword let let-rec let-mutable let-inline let-block let-math
syn keyword satysfiKeyword and in fun if then else match with when as
syn keyword satysfiKeyword type module struct sig end val direct constraint open
syn keyword satysfiKeyword before while do controls cycle command not mod
syn keyword satysfiKeyword inline-cmd block-cmd math-cmd
syn keyword satysfiKeyword signature include use of persistent
syn keyword satysfiBoolean true false
syn keyword satysfiType int float bool string unit length
syn keyword satysfiType inline-text block-text inline-boxes block-boxes
syn keyword satysfiType context math math-text math-boxes graphics image
syn keyword satysfiType color script language document code page font
syn keyword satysfiType deco deco-set paren regexp cell pre-path path option list ref

syn match satysfiOperator "->\|<-\|::\|+++\|\*'\|+'\|-'\|/'\|>=\|<=\|==\|<>\|&&\|||"
syn match satysfiOperator "[-+*/^<>=|&!?]"
" &e / ~e -- multi-stage quote and splice.
syn match satysfiStage "[&~]\ze[[:alpha:](]"
syn match satysfiModPath "\<\u[[:alnum:]-]*\ze\."

" ---- command names --------------------------------------------------------
syn match satysfiInlineCmd "\\\%([[:alpha:]][[:alnum:]-]*\.\)*[[:alpha:]][[:alnum:]-]*"
syn match satysfiBlockCmd  "+\%([[:alpha:]][[:alnum:]-]*\.\)*[[:alpha:]][[:alnum:]-]*"
syn match satysfiEscape    "\\[^[:alpha:]]"
syn match satysfiVarRef    "#[[:alpha:]][[:alnum:]-]*;\?"

" ---- text mode ------------------------------------------------------------
syn cluster satysfiTextBody contains=satysfiText,satysfiBlockText,satysfiMath,satysfiInlineCmd,satysfiBlockCmd,satysfiEscape,satysfiVarRef,satysfiComment,satysfiString,@Spell

syn region satysfiText matchgroup=satysfiTextDelim start="{" end="}" contains=@satysfiTextBody fold
syn region satysfiBlockText matchgroup=satysfiTextDelim start="'<" end=">" contains=@satysfiTextBody fold
syn region satysfiMath matchgroup=satysfiMathDelim start="\${" end="}" contains=satysfiMath,satysfiText,satysfiInlineCmd,satysfiVarRef,satysfiComment,satysfiNumber

" ---- links ----------------------------------------------------------------
hi def link satysfiComment     Comment
hi def link satysfiHeader      PreProc
hi def link satysfiHeaderKey   Include
hi def link satysfiString      String
hi def link satysfiNumber      Number
hi def link satysfiLength      Number
hi def link satysfiKeyword     Keyword
hi def link satysfiBoolean     Boolean
hi def link satysfiType        Type
hi def link satysfiOperator    Operator
hi def link satysfiStage       Special
hi def link satysfiModPath     Identifier
hi def link satysfiInlineCmd   Function
hi def link satysfiBlockCmd    Statement
hi def link satysfiEscape      SpecialChar
hi def link satysfiVarRef      Identifier
hi def link satysfiTextDelim   Delimiter
hi def link satysfiMathDelim   Delimiter
hi def link satysfiMath        Constant

let b:current_syntax = 'satysfi'
