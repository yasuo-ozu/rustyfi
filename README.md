# satysfi-rust-converted

A native Rust clone of [SATySFi](https://github.com/gfngfn/SATySFi) (reference:
upstream **v0.0.6**), using the [syan2](../syan2) parser framework for the
grammar.

## Status: milestone 1 + phase 2 (a+b)

A `.saty` document compiles to a real PDF with wrapped, justified text —
including binary operators (full v0.0.6 precedence ladder), `if`/`match`
with patterns and guards, local `let`/`let-rec`(+`and`), tuples, variant
constructors, user-defined commands via `let-inline`/`let-block`,
`let-mutable`/`<-`/`while`/`before`/`!`, record field access and functional
update, itemize (`*` bullets → `Item` constructor trees), math syntax
(parsed and quoted; typesetting is phase 7), modules (`module`/`struct`/
`open`, untyped name-mangling), `#var;` text embeds, and **multi-file
loading** — `@import:`/`@require:` resolve, dedupe, and topologically order
libraries via the `satysfi-loader` crate (safegraph-backed):

```console
$ cargo run -p satysfi-cli -- crates/satysfi-cli/tests/fixtures/minimal.saty -o out.pdf
```

What works:

- **Lexer** (`satysfi-syntax`): full case-for-case port of the v0.0.6
  `lexer.mll` — five mode states (program / vertical / horizontal / active /
  math) with the same stack discipline, eagerly lexing the whole file.
- **Parser**: syan2 derives over custom `WithSpan<Token, Span>` atoms; a
  `#[recurse]` grammar module for the Expr ↔ text SCC. Milestone-1 subset:
  headers, top-level `let`, `fun`, application, records, lists, string/int/
  float/length literals, inline text with `\cmd` and block text with `+cmd`.
  Token-level `Unparse` round-trip is tested.
- **Elaboration** (`satysfi-lang`): CST → `Ast` with scope resolution; the
  function signature is the seam where the phase-3 HM typechecker slots in.
- **Evaluator**: tree-walker with closures and a `PrimDef` registry shaped
  for incremental porting of the vminst primitive inventory (`read-inline`,
  `read-block`, `line-break`, `page-break` under their real names;
  `document`, `+p`, `\emph` are milestone-1 natives pending real stdlib
  loading).
- **Backend** (`satysfi-backend`): `Length`, horzBox-vocabulary box/glue
  model, greedy first-fit line breaking with glue justification (same input
  model as the future Knuth–Plass port), single-column page breaking.
- **PDF** (`satysfi-pdf`): `pdf-writer` output with base-14 Helvetica
  (regular/bold/oblique) and hardcoded AFM advance tables — WinAnsi/ASCII
  text only until real font loading (phase 5).

## Layout

```
crates/
  satysfi-syntax/    Span, Token, mode-stack lexer, ParseStream, grammar (CST)
  satysfi-backend/   Length, boxes/glue, Context, FontMetrics seam, line/page break
  satysfi-lang/      Ast, elaborate (typecheck seam), Value, evaluator, primitives
  satysfi-loader/    @require/@import resolution, dependency graph, load order
  satysfi-pdf/       pdf-writer backend + base-14 metrics
  satysfi-cli/       satysfi-rust <in.saty> -o <out.pdf> [--lib-root <dir>]
```

Requires a checkout of `syan2` at `../syan2` (path dependency).

## Roadmap

1. ✅ Thin end-to-end slice
2. ◕ Full surface language — done through phase 2b (binops, if/match,
   let-rec, tuples, ctors, commands, mutables, fields, items, math syntax,
   modules, multi-file loading); remaining: command macros (`\cmd@`),
   optional args at runtime (parse-and-reject today), `(| e with |)` inside
   signatures, `Mod.(…)`, tabular `|` separators, stages
3. ◕ Real typechecker — HM inference (Rémy levels, let-polymorphism, value
   restriction), row-polymorphic records, user variants, command-argument
   checking inside quoted text; a mandatory pipeline stage. Remaining:
   module signature enforcement, math command types, exhaustiveness, stages
4. Full primitive inventory + loading the real `dist/` stdlib
5. ◔ Real fonts — TrueType metrics + CID/Type0 embedding with ToUnicode
   done (`TtfFontStore`/`render_pdf_ttf`; CLI still defaults to base-14);
   remaining: CLI font selection, subsetting, shaping/kerning, CFF,
   Unicode line-break classes
6. ◔ Paragraph/page model — Knuth–Plass line breaking done (drop-in DP,
   badness/demerits per lineBreak.ml); remaining: discretionaries/
   hyphenation, full page model, graphics
7. Math mode; 8. images/annotations/cross-refs; 9. polish/perf
