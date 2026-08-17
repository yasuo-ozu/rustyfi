# Rustyfi logo

![The Rustyfi logo](rustyfi-logo.png)

The source is [`manual/logo.saty`](../../manual/logo.saty) — it lives beside
`manual.saty` because it is dogfood of the same kind: the manual proves the port
can typeset prose, the logo proves it can typeset vector art. Every mark in the emblem is a
[`satysfi-xpath`](https://github.com/monaqa/satysfi-xpath) path — there is no
imported artwork, no SVG, and no `Gr.*` helper. The port compiles it with its
own binary, so the logo doubles as a standing end-to-end exercise of the
package manager, the `xpath` package, and the PDF writer.

## Building

The document `@require:`s `xpath/xpath`, so the library has to be installed
into a lib-root first. It is **not** reached by a relative `@import:` into the
corpus tree; `scripts/layout_fidelity_corpus/xpath/Satyristes` is the manifest
that makes the vendored sources installable.

```sh
LIBROOT=$(mktemp -d)
cp -r lib-rustyfi/dist "$LIBROOT"/

cargo run --bin rustyfi-rust -- satyrographos install \
    scripts/layout_fidelity_corpus/xpath --lib-root "$LIBROOT"

cargo run --bin rustyfi-rust -- --lib-root "$LIBROOT" --font-dir "$LIBROOT" \
    -o assets/logo/rustyfi-logo.pdf manual/logo.saty
```

`satyrographos list --lib-root "$LIBROOT"` should then report `xpath 0.1.0
(4 files)`. The raster is derived, not authored:

```sh
pdftocairo -png -r 300 -transp -singlefile \
    assets/logo/rustyfi-logo.pdf assets/logo/rustyfi-logo
```

## What the mark says

A **cog** for Rust, and inside it a **page** — the thing a typesetter makes.

There is **no document in it**. Drawing a sheet of paper — even a spare one —
is depiction, and the mark does not need to show the artefact when it can show
the idea that produces it.

What sits inside the gear is the abstraction every typesetter actually works
against: the four metric lines, and **one set line** resting on the baseline
and filling the x-height. Registration ticks mark where the rules end. That is
what typesetting is before it is any particular page — and unlike a page full
of little grey bars, it survives being shrunk to a favicon.

The guides are weighted by authority. The **baseline is solid**, because it is
the one line that is not negotiable. The **x-height is dashed** — a guide, not
a mark. **Cap and descender are dotted**, because they are only reached when a
letter reaches them.

The set line is the only solid mass inside the disc, so it is where the eye
lands, and it is lit the way the ring is: a lighter bar along its top edge and
a darker one along the bottom. An earlier attempt gave it an ascender and a
descender as well; three strokes read as a machine part rather than as type,
and one is better.

Around it, the dashed ring is the measure's guide, and it is not drawn as
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

## The type

A classical emblem wants a contemporary logotype next to it, not a matching
one. So: a **tight lowercase `rustyfi`** carrying all the weight, with three
progressively smaller, progressively wider-tracked lines under it — rule,
monospace kicker, colophon.

The `fi` is set in oxide. It is the wordmark's accent, and in a program whose
whole job is setting type it is also the most famous ligature in the business.
(The port measures runs additively and substitutes no ligatures, so it really
is two glyphs — the joke is in the choice, not in the rendering.)

**The rule under the logotype is measured, not guessed.** `get-natural-metrics`
returns the set width of the very inline boxes being drawn, so the rule spans
the logotype exactly — and, measured a second time against `rusty` alone, it
changes colour at precisely the point where the `fi` begins. Change the size or
the face and it still lines up; nothing here is a hand-tuned constant.

The last line is a **colophon**, which is the correct furniture for a
typesetter's mark: it says the thing typeset itself.

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
| ivory | `#FAF4E6` | the page |
| ink | `#211C1F` | `rusty` |
