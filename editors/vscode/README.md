# rustyfi for Visual Studio Code

Syntax highlighting, formatting, language-server features and a live Markdown
preview for [SATySFi] documents, all driven by the `rustyfi` binary in this
repository.

[SATySFi]: https://github.com/gfngfn/SATySFi

## What it gives you

| Feature | Backed by |
| --- | --- |
| Highlighting for `.saty`, `.satyh`, `.satyg` | a TextMate grammar in this extension |
| Format Document / format on save | `rustyfi fmt -` |
| Diagnostics, hover, go-to-definition, completion, document & workspace symbols | `rustyfi lsp` |
| Live side-by-side preview | `rustyfi <file> --format markdown --unicode-math` |

## Install

The extension is not on the Marketplace. Build it from this directory:

```sh
cd editors/vscode
npm install
npm run compile
```

Then either

- **run it from source** — open `editors/vscode/` in VS Code and press <kbd>F5</kbd>,
  which launches an Extension Development Host with the extension loaded; or
- **build a `.vsix`** and install that:

  ```sh
  npx --yes @vscode/vsce package
  code --install-extension rustyfi-0.1.0.vsix
  ```

You also need the compiler itself:

```sh
cargo build --release -p rustyfi     # produces target/release/rustyfi
```

### Pointing it at a locally-built binary

The extension looks for `rustyfi` in this order:

1. the `rustyfi.serverPath` setting, if set;
2. anything named `rustyfi` on `$PATH`;
3. `target/release/rustyfi` under an open workspace folder.

So if you open **this repository** as your workspace and have run
`cargo build --release -p rustyfi`, step 3 finds it and no configuration is
needed. To use a build from somewhere else, set the path explicitly:

```jsonc
{
  "rustyfi.serverPath": "/home/you/src/rustyfi/target/release/rustyfi"
}
```

A `rustyfi.serverPath` that does not point at an executable is reported as an
error rather than quietly falling back to a different binary — otherwise you
would be running a compiler you did not choose with no indication why.

## Formatting

`Format Document` (and `editor.formatOnSave`) runs `rustyfi fmt -`, feeding it
the **buffer contents** on stdin rather than the file on disk, so it works on
an unsaved document and always formats what is on screen.

### The decline path

`rustyfi fmt` does not always produce a document, and the extension treats
those cases as *leave the buffer alone*:

| Exit | Meaning | What the extension does |
| --- | --- | --- |
| 0 | formatted | applies the edit |
| 2 | usage error | error notification (check your `rustyfi.format.*` settings) |
| 5 | filesystem error | error notification |
| 6 | the document does not lex | **declines** — buffer untouched, note in the status bar |
| 7 | it lexed but did not parse, so only whitespace was tidied | **declines** — buffer untouched |

Exit 6 prints **nothing** on stdout, so a naive provider would replace your
file with an empty string; exit 7 prints a whitespace-only tidy, so a naive
provider would silently throw away the layout you expected. Both are refused,
and there is an additional guard that refuses to apply an empty replacement to
a non-empty document even on exit 0. This is covered by tests that run the
real binary, and each guard has been mutation-tested to confirm the tests
actually fail when it is removed.

Because a document being typed into is unparseable most of the time, a decline
is reported in the status bar rather than as a modal popup.

### Formatting settings

Every option is **unset by default**, and an unset option contributes no flag.
That matters: `rustyfi fmt` resolves each option as *flag → `RUSTYFI_FMT_*`
environment variable → built-in default*, so passing a flag unconditionally
would silently override an environment variable you had deliberately set.

| Setting | Flag | Range | Default |
| --- | --- | --- | --- |
| `rustyfi.format.maxWidth` | `--max-width` | 20–1000 | 100 |
| `rustyfi.format.tabSpaces` | `--tab-spaces` | 1–16 | 2 |
| `rustyfi.format.maxBlankLines` | `--max-blank-lines` | 0–32 | 2 |
| `rustyfi.format.wrapComments` | `--wrap-comments` | boolean | true |
| `rustyfi.format.wrapInlineText` | `--wrap-inline-text` | boolean | true |
| `rustyfi.format.lang` | `--lang` | `auto`, `0.0`, `0.1` | `auto` |

A value outside the documented range is dropped with a warning instead of
being forwarded, because the CLI *refuses* an out-of-range value and writes
nothing — forwarding it would break formatting entirely rather than degrade it.

### Who formats: the CLI or the server?

`rustyfi lsp` **also** advertises a formatting provider. If both registered,
VS Code would ask you to choose a default formatter every time, and only one
of them honours the `rustyfi.format.*` settings above. `rustyfi.format.provider`
picks exactly one:

- `cli` (default) — this extension runs `rustyfi fmt -`, and the server's
  formatting capability is suppressed;
- `lsp` — the server formats, and the settings above do not apply;
- `off` — no formatter is contributed.

## Preview

`rustyfi: Open Preview to the Side` compiles the document with

```
rustyfi <file> --format markdown --unicode-math
```

and renders the Markdown in a webview beside the editor. `--unicode-math`
writes equations as their characters in reading order (`x²`, `∑ₐᵇ`,
`(a+b)/(c+d)`), which is plain text and always renders.
`rustyfi.preview.mathMode` can select `--svg-math`, `--svg-outline-math`,
`--katex` or `--mathml` instead; note that `katex` and `mathml` emit markup
this preview does not typeset, so equations will appear as source.

### What it costs

Measured on this repository's corpus, compiling to Markdown with
`--unicode-math` (not to PDF, which is slower):

| Document | Size | Compile |
| --- | --- | --- |
| `layout-tests/corpus/floatfig/floatfig.saty` | 3.6 KB | **0.13 s** |
| `layout-tests/corpus/latexcmds/doc/latexcmds-doc.saty` | 25 KB | **0.22 s** |
| `layout-tests/corpus/xpath/doc/xpath-doc.saty` | 25 KB | **2.8 – 4.1 s** |

Each figure is a cache-missing run — the file was modified first, which is the
case that matters while typing.

**The debounce default is 300 ms**, chosen from those numbers: it sits just
above the typical compile so an ordinary document feels live (roughly 0.5 s
from last keystroke to fresh preview), while being long enough that a burst of
typing produces one compile rather than one per character. `xpath` is the
outlier at ~4 s — it is interpretation-bound rather than large — and for a
document like that the debounce matters less than the cancellation described
below. Tune with `rustyfi.preview.debounce`.

### How it behaves while you type

- **A failed compile never destroys a good preview.** A document under the
  cursor is broken most of the time, so a preview that blanked on failure
  would be blank most of the time. A failure shows an error banner at the top
  and leaves the last successful render underneath it.
- **Scroll position survives a re-render.** The webview's HTML shell is built
  once and each re-render patches the body, so the panel does not jump to the
  top on every keystroke.
- **In-flight compiles are cancelled**, both when a newer edit supersedes them
  and when the panel closes, so a fast typist does not accumulate compiler
  processes. A compile that outlives its generation has its result discarded
  rather than being allowed to overwrite a fresher one.
- **Unsaved buffers are previewed.** The compiler has no stdin mode, so the
  buffer is written to a temp file — as a **sibling** of the real document
  (`.rustyfi-preview-<hash>.saty`), not in `/tmp`. That is deliberate:
  `@require:` and `@import:` resolve relative to the importing file's own
  directory, so a temp file in `/tmp` would break every relative import. The
  file is removed when the panel closes. An *untitled* buffer has no directory
  at all, so relative imports cannot work there and the preview says so.
- The cross-reference `.satysfi-aux` file is redirected into a temp directory,
  so previewing never litters your source tree nor clobbers the aux file a
  real build wrote.

### Errors in the preview

A failed compile shows the compiler's own message in a banner. Two details are
handled for you: the compiler names the file it was handed, which is the temp
sibling, so that path is rewritten back to your document's real name before it
is shown; and the compiler writes a good deal of layout progress to stderr on
a *successful* run (`page parts 3`, `column end 4`, …), which is never
surfaced because only a non-zero exit produces a banner.

### Preview security

The webview runs under `default-src 'none'` with no remote origin permitted —
**nothing is loaded from a CDN**. The Markdown renderer is written in this
extension (the compiler's Markdown output is a small, known subset, and it is
rendered with a strict escape-everything default).

Figures arrive from the compiler as **raw inline `<svg>`**, so that SVG is
rebuilt through a tag/attribute allowlist: `<script>`, `<foreignObject>` and
`<image>` are dropped with their contents, event-handler attributes are
stripped, and `url(...)` references are permitted only as local fragments. An
SVG too malformed to rebuild is shown as escaped source rather than guessed at.

## All settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `rustyfi.serverPath` | `""` | Path to the binary; empty means auto-discover. |
| `rustyfi.lsp.enable` | `true` | Run `rustyfi lsp`. |
| `rustyfi.trace.server` | `off` | Trace LSP traffic in the output channel. |
| `rustyfi.format.provider` | `cli` | `cli`, `lsp` or `off`. |
| `rustyfi.format.*` | unset | See the formatting table above. |
| `rustyfi.preview.enable` | `true` | Recompile automatically as you type. |
| `rustyfi.preview.debounce` | `300` | Milliseconds of quiet before recompiling. |
| `rustyfi.preview.mathMode` | `unicode-math` | How equations are written. |
| `rustyfi.preview.timeout` | `30000` | Milliseconds before a compile is killed. |
| `rustyfi.libRoot` | `""` | Passed as `--lib-root` to preview compiles. |

## Commands

- `rustyfi: Open Preview to the Side`
- `rustyfi: Refresh Preview`
- `rustyfi: Restart Language Server`
- `rustyfi: Show Output Channel`

## Development

```sh
npm install
npm run compile     # tsc
npm test            # node --test over the compiled tests
npm run watch       # incremental
```

`npm test` runs 81 tests: the pure logic (option mapping, exit-code decisions,
binary discovery, the Markdown renderer and SVG sanitizer), a TextMate
tokenization suite that checks the four lexical areas nest correctly, and
integration tests that drive the real `rustyfi` binary for the exit-code
contract and a preview compile. The integration tests skip themselves if
`target/release/rustyfi` has not been built.

There are **no VS Code integration tests** — those need `@vscode/test-electron`
to download a VS Code build at test time, which this environment cannot do.
Everything that touches the `vscode` API (the providers, the webview panel, the
language client) is therefore **compile-checked but not executed** by the test
suite; the logic underneath it is factored into `src/core/` precisely so it can
be tested without the editor.
