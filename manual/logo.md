# Rustyfi logo

![The Rustyfi logo](logo.png)

The source is [`logo.saty`](logo.saty) — one file, built by [`Makefile`](Makefile) — it lives beside
`manual.saty` because it is dogfood of the same kind: the manual proves the port
can typeset prose, the logo proves it can typeset vector art. Every mark in the emblem is a
[`satysfi-xpath`](https://github.com/monaqa/satysfi-xpath) path — there is no
imported artwork, no SVG, and no `Gr.*` helper. The port compiles it with its
own binary, so the logo doubles as a standing end-to-end exercise of the
package manager, the `xpath` package, and the PDF writer.

## Building

```sh
make -C manual          # manual.pdf, logo.pdf, logo.png
make -C manual logo     # just the mark
```

The manual builds against the port's own bundled packages and nothing else; the
logo `@require:`s `xpath/xpath`, which is not bundled, so the Makefile assembles
a scratch lib-root and INSTALLS `xpath` into it with the port's own
Satyrographos — deliberately, rather than reaching into the corpus tree with a
relative `@import:`. `scripts/layout_fidelity_corpus/xpath/Satyristes` is the
manifest that makes that possible.

Both documents are compiled `--no-cache --no-aux`: these are reference
artifacts, and the compile cache is keyed on the SOURCE, so an engine change
would otherwise be masked by a stale render.

## What the mark says

A **cog** for Rust, and inside it a **page** — the thing a typesetter makes.

There is **no document in it**. Drawing a sheet of paper — even a spare one —
is depiction, and the mark does not need to show the artefact when it can show
the idea that produces it.

What sits inside the gear is the **logotype itself**, centred: a tight
lowercase `rustyfi`. The mark is one object — there is no wordmark under the
emblem, no rule, no kicker, no colophon — because one object is what a mark has
to be to survive being put in a corner at 32 px.

The logotype is set upright, in the face the port registers as `lmodern`.

That name is worth checking rather than trusting: in
`lib-rustyfi/dist/hash/fonts.rustyfi-hash`, `lmodern` points at
`latinmodern-math.otf` — the same file `lmmath` points at. Upstream's
`lmroman10-regular.otf` is not in this port's roster at all, so what the mark is
actually set in is Latin Modern Math's upright text alphabet. It is the right
look — high-contrast modern letterforms from the same engraving era as the
guilloche — but the abbrev does not say what it is.

A calligraphic (`mathcal`) logotype was built and then removed. Two things it
established are worth keeping even though the result is gone:

* SATySFi 0.0.6 has no `set-math-char-class` — it arrived with the 0.1 math
  split — so the script alphabet has to be reached as Unicode instead, U+1D4B6..
  MATHEMATICAL SCRIPT SMALL A onwards, needing no math mode at all.
* Those codepoints render from `dejavu-math` and come out EMPTY from `lmmath`:
  zero-width boxes, the word absent, the braces closed over the gap. Since
  `lmmath` and `lmodern` are the same file, that is not a font-choice problem —
  it is this OTF/CFF face's Mathematical Alphanumeric glyphs not surviving the
  port's text path, where DejaVu's TrueType ones do.

The type is **outlined, and given relief, the hard way**. Neither SATySFi nor
this port exposes a PDF text render mode, so there is no stroked type to ask
for. The letters are drawn as two rings of sixteen `draw-text` copies each,
plus one more on top in the disc's own centre tone:

  * a pale ring at 2.0 pt, offset DOWN AND RIGHT — away from the light;
  * a dark contour at 0.9 pt, offset UP AND LEFT — towards it;
  * the fill, unoffset.

Concentric rings would give a halo, which is type sitting on a pattern. Offset
rings give relief, and the relief agrees with the light already falling on the
metal. It is still a fake — at poster size the sixteen corners would show — but
at every size this mark is used it reads as engraved type, and it lets the
guilloche run right up to the letterforms.

**The mark speaks SATySFi.** The logotype is flanked by `{` and `}` — the
inline-text delimiters, the mode marker you cannot write a document without —
set larger and lighter than the word so they read as brackets rather than as
letters. `(| |)` opens a record at the top and `< >` a block-text run at the
bottom, so the three GROUPS a document is built out of are all named. Four
sigils sit on the diagonals like the legend on a coin: `\` opens an inline
command, `+` a block one, `$` math mode, `@` a file header (`@require:`).

`[ ]` is deliberately absent. A list is a data literal, not a mode, and the
inscription is about the modes — adding it would make the ring a punctuation
sampler rather than a statement.

Those four come through `embed-string` on backtick STRING literals rather than
being written as inline text, because three of them are illegal there: `@`, `;`
and `$` are mode-switching tokens and the lexer is right to refuse them
(`illegal token '@' in an inline text area`). Quoting the character as data is
how you typeset a character the surrounding syntax reserves — which is itself a
fair thing for this mark to be demonstrating.

The mark is centred on its page by measurement, not by assumption: with the
graphics box centred by `inline-fil` it still landed 5.75 pt up and to the left,
so `text-origin` carries exactly that correction, verified by measuring the
rendered ink's bounding box (equal margins on all four sides).

Around it, the dashed ring is the type area's guide, and it is not drawn as
its own circle — it is `XPath.offset-path` applied to the disc outline, which is
the operation `satysfi-xpath` has and SATySFi's built-in path API does not.

Three other things are worth knowing if you edit it:

- **Everything rotational is one path.** The sixteen teeth, sixty bezel ticks,
  seventy-two beads and forty-eight guilloche petals are each a single shape
  rotated by `XPath.linear-transform-path` and folded together with
  `XPath.unite-path` (`rosette`), so each whole ring of copies is one `fill`.
- **The guilloche is engine-turning, twice over.** Forty-eight circles whose
  centres sit on a ring of their own radius, so every one of them passes through
  the middle — the ornament a banknote or a watch dial is finished with, and
  structurally the same rotate-and-unite as the teeth. A second rosette of
  thirty-one petals at a smaller radius sits over it: two counts that share no
  factor interfere, and that moire is what stops the pattern reading as a flat
  texture. Both hairline, so neither competes with the text on top.
- **The light is painted.** SATySFi has no gradients, so the bevel is two
  `band`s of concentric fills interpolating one colour into another, plus a
  specular highlight at the upper left and an opposing shadow at the lower
  right. A single stroked arc has butt ends that read as a seam rather than as
  light, so each is an `arc-stack`: six nested arcs, each shorter, narrower and
  closer to the bright end, tapering in both width and colour. Use one arc
  instead of six and you get two visible seams at 3 and 9 o'clock; delete the
  stacks and the bands, and the ring goes flat. The teeth are lit the same way,
  by a shadow copy of the whole cog offset down-right and a lighter copy offset
  up-left, with the real one on top. The page is not a flat fill either — it
  lifts towards the middle, the way a sheet does under a light.

## Palette

| | | |
|---|---|---|
| rim | `#571C08` | outer rim, cog shadow |
| oxide deep | `#6B2610` | cog, bezel ticks |
| oxide dark | `#99380F` | margin guide, title rule, kicker |
| oxide | `#C24D14` | ring band, heading rule, accent bar, the `fi` |
| tooth lit | `#A1441A` | the teeth's lit edge |
| ember | `#F7AE42` | bevel, beading |
| guilloche | `#E3D6B8` / `#EDE3CC` | the two engine-turnings |
| relief | `#F2D499` | the logotype's lit ring |
| brace | `#D16B29` | the inline-text delimiters |
| ivory | `#FAF4E6` | the page |
| ink | `#211C1F` | measuring only (the logotype is outlined, not filled) |
