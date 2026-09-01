# rustyfi.vim — Vim and Neovim support for SATySFi

Filetype detection, minimal syntax and indent, the `rustyfi fmt` formatter,
the `rustyfi lsp` language server, and a **live preview that renders into a
scratch buffer** rather than a browser.

One codebase for both editors: the shared surface is Vimscript, and the
Neovim-only language-server glue is Lua that the Vimscript path never needs.

| | verified on |
|---|---|
| Vim | 8.2.1522, 9.2.0106 |
| Neovim | 0.9.4, 0.11.6 |

Minimum supported: **Vim 8.2** and **Neovim 0.9**. `plugin/rustyfi.vim`
refuses to load below those and says so.

---

## The preview

    rustyfi <file> --format markdown --unicode-math

`--unicode-math` writes equations as their characters in reading order — `𝑥²`,
`∑ᵇₐ`, `(𝑎 + 𝑏)/(𝑐 + 𝑑)` — which is the one output form that is plain text.
That is why the preview can be a buffer: no webview, no browser, it reads in a
terminal over ssh.

    :RustyfiPreview          " open a split beside the document
    :RustyfiPreviewToggle
    :RustyfiPreviewRefresh   " force a render now
    :RustyfiPreviewErrors    " full compiler output for the last failure
    :RustyfiPreviewClose     " or just `q` in the preview window

Behaviour worth knowing:

* **It renders the buffer, not the file.** The buffer is written to a dotfile
  *beside the real document* — `.<name>.rustyfi-preview.saty` — and that file
  is compiled. It has to be in that directory: `@import:` resolves relative to
  the input file, and `@require:` discovers its package root by walking up
  from it. A scratch file in `$TMPDIR` fails with *cannot resolve `@import:`*.
  The dotfile is removed when the preview closes.
* **A failed compile never destroys a good preview.** The last successful
  render stays on screen and the window's statusline changes to
  `[rustyfi preview] stale: <first error line>`. Nothing is echoed and the pane
  is not cleared, because a document mid-edit is broken most of the time.
* **Async, and cancelling.** `jobstart()` on Neovim, `job_start()` on Vim,
  behind one interface in `autoload/rustyfi/job.vim`. Starting a render kills
  the one in flight, and a late callback from a killed job is dropped by
  sequence number, so it cannot overwrite the pane a newer render is filling.
* Opening the preview does not move the cursor out of your document, and
  re-rendering preserves the preview window's scroll position.

### What it costs

Measured on this repository's corpus with `--format markdown --unicode-math`,
wall clock, cold compile cache:

| document | lines | render |
|---|---|---|
| `latexcmds/doc/latexcmds-doc.saty` | 403 | 0.20 s |
| `enumitem/doc/enumitem.saty` | 1492 | 0.47–0.71 s |
| `easytable/doc/easytable.saty` | 991 | 0.89–1.34 s |
| `figbox/doc/manual.saty` | 909 | 0.99–1.31 s |
| `xpath/doc/xpath-doc.saty` | 486 | 2.7–3.9 s |
| `manual/manual.saty` | 160 | 0.12 s |

So: a fifth of a second for a small document, around a second for a typical
one, and up to four seconds for one that is mostly vector graphics.

**The debounce default is 500 ms**, from those numbers. A debounce shorter
than the render only queues work; one longer than a typing pause stops feeling
live. 500 ms sits just past a normal inter-keystroke gap, so a burst of typing
produces exactly one render, and because the in-flight job is killed when the
next one starts a slow document self-throttles instead of backing up. If your
documents look like `xpath`, raise it:

```vim
let g:rustyfi_preview_debounce = 1500
```

Renders are triggered by `TextChanged`, `TextChangedI`, `InsertLeave` and
`BufWritePost` on the source buffer.

---

## Formatting

    :RustyfiFmt              " format this buffer
    :RustyfiFmtCheck         " report whether it would change, with a diff
    :RustyfiFmtOnSaveToggle  " per-buffer format-on-save

The buffer is piped through `rustyfi fmt -` (stdin → stdout), so an unsaved
buffer formats what you can actually see, and the file on disk is not touched.

**The cursor stays put.** The replacement trims the common prefix and suffix
and rewrites only the middle, so the cursor does not move at all when the
change is elsewhere in the file, and moves by exactly the line delta when it
is above the cursor. Marks and folds outside the changed region survive.

**Declines are declines.** `rustyfi fmt`'s exit codes are acted on:

| exit | meaning | what the plugin does |
|---|---|---|
| 0 | clean | apply |
| 1 | `--check` would reformat | reported by `:RustyfiFmtCheck` |
| 2 | usage | error, buffer untouched |
| 5 | filesystem | error, buffer untouched |
| 6 | file does not lex — **declined** | error naming the reason, buffer untouched |
| 7 | lexed but did not parse — whitespace tidied only | warning, buffer untouched |

Exit 6 writes *nothing* to stdout, so applying it would blank the buffer;
there is also a belt-and-braces check that refuses any empty output over a
non-empty buffer whatever the exit code claimed. Exit 7 does produce text, but
it is the old whitespace tidy rather than the formatter's layout, so it is not
applied unless you ask:

```vim
let g:rustyfi_fmt_accept_partial = 1
```

Format on save, globally:

```vim
let g:rustyfi_fmt_on_save = 1
```

---

## Language server

`rustyfi lsp` speaks LSP over stdio and advertises hover, go-to-definition,
completion, document symbols, workspace symbols, formatting and diagnostics.

### Neovim 0.11+

This plugin ships `lsp/rustyfi.lua`, which Neovim's runtime loader picks up:

```lua
vim.lsp.enable('rustyfi')
```

### Neovim 0.9 / 0.10, or with nvim-lspconfig

```lua
require('rustyfi.lsp').setup({
  -- all optional
  cmd_bin  = '/path/to/rustyfi',
  lib_root = '/path/to/lib-rustyfi',
  on_attach = function(client, bufnr) ... end,
  capabilities = require('cmp_nvim_lsp').default_capabilities(),
})
```

It uses `vim.lsp.config` on 0.11+, registers with **nvim-lspconfig** if that is
installed, and otherwise starts the server itself from a `FileType` autocommand.

**Does this belong upstream in nvim-lspconfig?** Yes, and as a *new* entry.
lspconfig already ships `satysfi_ls`, which is the OCaml
`satysfi-language-server` — a different binary with a different command line.
`rustyfi lsp` is a second server for the same filetype, so the upstream shape
is a sibling entry (`rustyfi`), not a patch to the existing one. Until such an
entry exists, the Lua above is the whole configuration.

### Vim 8 — vim-lsp

Install [`prabirshrestha/vim-lsp`](https://github.com/prabirshrestha/vim-lsp)
and there is nothing else to do: this plugin registers the server on
vim-lsp's `User lsp_setup` event, and `:LspDefinition`, `:LspHover`,
`:LspDocumentSymbol` and the rest work over `.saty` buffers.

**Why vim-lsp and not ALE.** ALE is a linting framework whose LSP support is
expressed per-linter; wiring `rustyfi lsp` through it gives you diagnostics
and completion, but the rest of the server's surface is reachable only through
ALE's own partial commands. vim-lsp is a full client — one
`lsp#register_server()` call and every capability above is available. If you
already run ALE and do not want a second client:

```vim
call ale#linter#Define('satysfi', {
\   'name': 'rustyfi',
\   'lsp': 'stdio',
\   'executable': 'rustyfi',
\   'command': '%e lsp',
\   'project_root': {b -> ale#path#FindNearestFile(b, 'Satyristes')},
\})
let g:ale_linters = {'satysfi': ['rustyfi']}
```

(That recipe is offered as-is; the shipped, tested integration is vim-lsp's.)

---

## Install

The plugin root is `editors/vim/` inside this repository.

**vim-plug**

```vim
Plug 'yasuo-ozu/satysfi-rust-converted', {'rtp': 'editors/vim'}
```

**lazy.nvim**

```lua
{
  'yasuo-ozu/satysfi-rust-converted',
  ft = 'satysfi',
  config = function()
    require('rustyfi.lsp').setup({})   -- or vim.lsp.enable('rustyfi') on 0.11+
  end,
}
```

lazy.nvim adds the repository root to the runtimepath, not `editors/vim`; add

```lua
vim.opt.runtimepath:append(vim.fn.stdpath('data') .. '/lazy/satysfi-rust-converted/editors/vim')
```

or use `dir = '<checkout>/editors/vim'` with a local clone.

**packer / paq / dein** — the same `rtp` / `subdir` idea; point them at
`editors/vim`.

**Manually** (`:packadd`-free, Vim 8 / Neovim native packages):

```sh
git clone https://github.com/yasuo-ozu/satysfi-rust-converted ~/src/rustyfi
mkdir -p ~/.vim/pack/rustyfi/start          # Vim
ln -s ~/src/rustyfi/editors/vim ~/.vim/pack/rustyfi/start/rustyfi
mkdir -p ~/.local/share/nvim/site/pack/rustyfi/start   # Neovim
ln -s ~/src/rustyfi/editors/vim ~/.local/share/nvim/site/pack/rustyfi/start/rustyfi
```

Then `:helptags` the doc directory once (plugin managers do this for you):

```vim
:helptags ~/.vim/pack/rustyfi/start/rustyfi/doc
```

The plugin needs the `rustyfi` executable. It looks for it on `$PATH`, and
failing that walks up from the current file for a `target/release/rustyfi` or
`target/debug/rustyfi` — so it works inside a checkout with nothing installed.
Override with `let g:rustyfi_bin = '/path/to/rustyfi'`.

---

## Configuration

| variable | default | meaning |
|---|---|---|
| `g:rustyfi_bin` | `'rustyfi'` | executable; auto-discovery is skipped when set |
| `g:rustyfi_quiet` | `0` | suppress informational messages |
| `g:rustyfi_fmt_on_save` | `0` | format every `.saty`/`.satyh`/`.satyg` on write |
| `g:rustyfi_fmt_args` | `[]` | extra flags for `rustyfi fmt`, e.g. `['--max-width', '80']` |
| `g:rustyfi_fmt_accept_partial` | `0` | apply the whitespace tidy on exit 7 |
| `g:rustyfi_fmt_check_show_diff` | `1` | open the `--check` diff in a scratch window |
| `g:rustyfi_preview_debounce` | `500` | milliseconds |
| `g:rustyfi_preview_split` | `'vertical'` | `'vertical'`, `'horizontal'` or `'tab'` |
| `g:rustyfi_preview_width` | `62` | columns, vertical split |
| `g:rustyfi_preview_height` | `15` | lines, horizontal split |
| `g:rustyfi_preview_args` | `[]` | extra compiler flags, e.g. `['--lang', '0.1']` |
| `g:rustyfi_preview_filetype` | `'markdown'` | filetype of the preview buffer |
| `g:rustyfi_lib_root` | unset | `--lib-root` for the preview and the language server |
| `g:rustyfi_lsp_args` | `[]` | extra flags for `rustyfi lsp` |
| `g:rustyfi_lsp_init_options` | `{}` | extra `initializationOptions` |
| `g:rustyfi_no_indent` | `0` | do not install the indent expression |

Full reference: `:help rustyfi`.

---

## Filetype, syntax, indent

`.saty`, `.satyh` and `.satyg` are detected as filetype **`satysfi`** — the
established name, used by `qnighy/satysfi.vim` and listed by nvim-lspconfig's
`satysfi_ls`. Detection uses `setf`, which is a no-op when a filetype is
already set, and the syntax file bails on `b:current_syntax`, so loading this
alongside another SATySFi plugin is not a collision.

The syntax file is deliberately small. It does model the one distinction that
matters for not lying: SATySFi has a program mode and a text mode, `{ … }`
always enters inline text and `'< … >` block text, so keywords are highlighted
only where they are keywords and the English word *in* inside a paragraph is
left alone. Nothing finer.

The indenter is a bracket-depth heuristic. `:RustyfiFmt` is the authority on
layout; the indenter only aims to put the cursor somewhere sane while you
type. `let g:rustyfi_no_indent = 1` turns it off.

---

## Tests

```sh
editors/vim/test/run.sh
```

Real headless editors driving real buffers. It runs `test/run.vim` (88
assertions: filetype detection, the formatter round trip through a buffer,
cursor preservation, the exit-6 and exit-7 decline paths leaving the buffer
intact, format-on-save, the preview producing non-empty unicode-math output
for a document that `@import:`s a neighbour, a broken edit keeping the last
good render, scratch-file cleanup, all three split modes, binary discovery, the
syntax file's program-mode/text-mode split, the indenter, and the job
abstraction) in every editor it finds, and `test/lsp.vim` (7 assertions,
Neovim only) in both language-server setup routes.

`RUSTYFI_EXTRA_VIM` takes a space-separated list of extra editor binaries,
which is how the version matrix at the top of this file was produced.

## Known gaps

* The `:RustyfiFmtCheck` diff comes from `--check` on stdin, so its hunk
  headers name `<stdin>` rather than your file.
* The preview writes a dotfile into the document's directory. If that
  directory is read-only the preview reports it and does nothing; there is no
  fallback, because a fallback location cannot resolve `@import:`.
* No `:RustyfiBuild`. Compiling to PDF is `:!rustyfi %` and there is no value
  in wrapping it.
