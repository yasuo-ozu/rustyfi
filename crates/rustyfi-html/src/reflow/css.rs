//! The reflow document's base stylesheet (Slice 1: "Base document CSS (max-width
//! column, line-height, font stack, dark/light neutral)"). Everything here is
//! FLOW layout, with exactly TWO absolute rules — `svg.frame-deco` and
//! `.dtx`, each commented at its own definition below. Neither positions
//! anything against the page: both are scoped to a box that has already been
//! placed by flow and made `position: relative`, one to paint a frame's
//! decoration over it and one to stack the rows of a `draw-text`
//! construction inside it. `reflow_output_never_uses_absolute_positioning`
//! pins the count, so a third cannot arrive quietly.
//!
//! S4 adds `ul.list`/`ol.list` spacing rules for the real
//! `<ul>`/`<ol>`/`<li>` `block.rs`'s `VertBox::ListMark` arm now emits —
//! bullet/number glyphs and basic indentation are otherwise the browser's
//! own UA stylesheet default for `<ul>`/`<ol>`/`<li>`, so no additional
//! list-style rule is needed; `<em>`/`<strong>` (S4's other new tag pair)
//! likewise need no CSS of their own — the browser default (italic/bold) is
//! exactly the semantic they name.
//!
//! ## The body rule is computed, not fixed
//!
//! `body` carries the document's OWN dominant face and size
//! (`text::BodyStyle::dominant`). That is what lets `inline.rs` write the
//! bulk of the text as bare characters with no `<span>` at all, and lets the
//! runs that do differ state their size as an `em` ratio — so the reader
//! scales the whole document by changing one number rather than fighting a
//! per-run absolute point size.
//!
//! ## Why `.frame` has no border of its own
//!
//! A `FrameStart`/`FrameEnd` pair is `block-frame-breakable`, which real
//! packages use for section bodies, list items, figure wrappers and
//! quotation blocks as readily as for anything that actually has a rule
//! around it — the `enumitem` manual alone opens 336 of them. Drawing a box
//! around every one turned the page into nested rounded rectangles.
//!
//! What distinguishes them is the frame's own decoration, and that IS now
//! drawn: `fire_hooks` records each one box-local
//! (`DocumentValue::reflow_frame_decos`), and `structure::frame_decoration`
//! turns it into a `background` or a scalable `<svg>` under `.frame.framed`.
//! So a frame that draws nothing still draws nothing, and one that draws a
//! title box keeps it.

use std::fmt::Write as _;

use rustyfi_backend::PageGeometry;

use super::Ctx;

/// `geometry.text_width` seeds the reading column's `max-width` (in `pt`,
/// SATySFi's own unit, 1:1 with CSS `pt`) — a nod to the source document's
/// own measure without pinning the layout to it: a narrow viewport still
/// shrinks the column further (the `max-width`/`width:100%` combination),
/// and nothing here prevents the column from being narrower than the
/// viewport on a wide screen either. Purely a readability default, not a
/// geometry replay.
pub(crate) fn stylesheet(geometry: &PageGeometry, ctx: &Ctx) -> String {
    let max_width = geometry.text_width.0.max(1.0);
    let body_size = ctx.body.size;
    // The document's own dominant face, when one is known — already a full
    // stack ending in a generic (`fonts::reflow_font_stack`), so body text
    // renders in the face it was typeset in where the reader has it and in
    // something sensible where they do not, and `inline.rs` need not repeat
    // it on every run.
    let body_family = ctx
        .body
        .font
        .and_then(|f| ctx.font_family_for(f))
        .unwrap_or_else(|| "Georgia, \"Noto Serif\", \"Noto Serif CJK JP\", serif".to_string());
    format!(
        "\
body {{\n\
  margin: 0;\n\
  padding: 2rem 1rem;\n\
  font-family: {body_family};\n\
  font-size: {body_size}pt;\n\
  line-height: 1.7;\n\
  background: #fff;\n\
  color: #1a1a1a;\n\
}}\n\
.doc {{\n\
  max-width: {max_width}pt;\n\
  width: 100%;\n\
  margin: 0 auto;\n\
  box-sizing: border-box;\n\
}}\n\
.para {{\n\
  margin: 0 0 0.9em 0;\n\
  text-align: justify;\n\
  hyphens: auto;\n\
  overflow-wrap: break-word;\n\
}}\n\
.para[data-align=\"center\"] {{ text-align: center; }}\n\
.para[data-align=\"right\"] {{ text-align: right; }}\n\
.frame {{ margin: 0.6em 0; }}\n\
/* A frame that DREW something: the drawing is a sibling SVG stretched\n\
   over the whole box, so the box has to be its containing block.\n\
   No padding of its own: the frame's left padding is already folded into\n\
   the contained lines' own offsets (`indent_left`) and its vertical ones\n\
   arrive as `FramePad` skips, so adding CSS padding here would double all\n\
   three. Only `padding-right`, which nothing else records, is written — per\n\
   frame, from its real value. */\n\
.frame.framed {{ position: relative; }}\n\
/* Content sits ABOVE the drawing. `:not(.frame-deco)` is load-bearing:\n\
   `.frame.framed > *` outranks `svg.frame-deco` on specificity, so without\n\
   it the decoration is pulled into normal flow and every framed block\n\
   renders as an empty box with its content underneath. */\n\
.frame.framed > *:not(.frame-deco) {{ position: relative; }}\n\
.frame.framed > svg.frame-deco {{\n\
  position: absolute;\n\
  left: 0;\n\
  top: 0;\n\
  width: 100%;\n\
  height: 100%;\n\
  overflow: visible;\n\
  pointer-events: none;\n\
}}\n\
/* A `draw-text` run PLACED inside its own math/graphics wrapper\n\
   (`inline.rs`'s `emit_placed_text`): the second — and, by design, last —\n\
   absolute rule in this stylesheet. Like `svg.frame-deco` above it, it is\n\
   scoped to one relatively-positioned inline box and never to the page:\n\
   the wrapper it sits in is `position:relative`, so this places a stacked\n\
   row within one operator, not a paragraph on a canvas. It exists because\n\
   flow has no way to say ABOVE — see `emit_placed_text` for the argument,\n\
   including why the strut is what makes `top` exact. */\n\
.dtx {{ position: absolute; line-height: 0; white-space: nowrap; }}\n\
.dtx > .dtx-strut {{ display: inline-block; width: 0; vertical-align: baseline; }}\n\
/* Math is drawn from the face's own outlines; these are the characters,\n\
   kept selectable behind them (`inline.rs`'s `Phantom`). `fill: none` and\n\
   NOT `visibility: hidden`/`display: none` — those take the text out of the\n\
   selection too, which is the whole thing being avoided. */\n\
.math-glyphs .mphantom {{ fill: none; }}\n\
.embed {{ margin: 0.5em 0; }}\n\
/* A block composed into a drawing (`inline.rs`'s `emit_embedded_block`), at\n\
   the measure the document chose for it. */\n\
.embed-inline {{ display: inline-block; max-width: 100%; text-align: left; }}\n\
.iframe {{ display: inline; }}\n\
.hskip {{ display: inline-block; }}\n\
.clearpage {{\n\
  border: none;\n\
  border-top: 1px dashed rgba(127,127,127,0.5);\n\
  margin: 2em 0;\n\
}}\n\
.reflow-empty {{ font-style: italic; opacity: 0.7; }}\n\
.gfx-placeholder, .pdf-image {{\n\
  display: inline-block;\n\
  vertical-align: middle;\n\
  border: 1px dashed rgba(127,127,127,0.5);\n\
  border-radius: 3px;\n\
}}\n\
.img {{\n\
  max-width: 100%;\n\
  height: auto;\n\
  vertical-align: middle;\n\
}}\n\
span.img {{\n\
  display: inline-block;\n\
  background-repeat: no-repeat;\n\
  background-size: contain;\n\
  background-position: center;\n\
}}\n\
a.link {{ color: #1a5fb4; text-decoration: underline; }}\n\
.heading {{\n\
  margin: 1.4em 0 0.5em 0;\n\
  font-weight: bold;\n\
  line-height: 1.3;\n\
  text-align: left;\n\
  /* The heading's SIZE is the document's own, carried by the run inside\n\
     it as an em ratio of the body. Without this the UA default for\n\
     h1..h6 (2em, 1.5em, ...) multiplies that ratio and a 1.83em section\n\
     title renders at 3.7em. */\n\
  font-size: inherit;\n\
}}\n\
table.tabular {{\n\
  border-collapse: collapse;\n\
  margin: 1em auto;\n\
  max-width: 100%;\n\
}}\n\
/* No border here: which grid lines a table draws is the DOCUMENT's, read\n\
   off `TabularBox::rules` and written per cell (`structure.rs`'s `Borders`).\n\
   A blanket rule turned every booktabs-style table into a full grid.\n\
   No padding either — the cell's own margins arrive as real `hskip` struts\n\
   inside it, so a CSS pad would be added on top of them. */\n\
table.tabular td {{\n\
  padding: 0.15em 0;\n\
  vertical-align: baseline;\n\
}}\n\
/* Fixed-pitch text: upstream's line breaks are the author's and survive as\n\
   a hard break, so justification and hyphenation must not fight them. */\n\
.para.code {{\n\
  text-align: left;\n\
  hyphens: none;\n\
  overflow-x: auto;\n\
}}\n\
ul.list, ol.list {{\n\
  margin: 0 0 1em 0;\n\
  padding: 0 0 0 1.5em;\n\
}}\n\
ul.list ul.list, ul.list ol.list, ol.list ul.list, ol.list ol.list {{\n\
  margin: 0.2em 0;\n\
}}\n\
ul.list li, ol.list li {{ margin: 0.2em 0; }}\n\
/* The in-text footnote anchor is a zero-width link TARGET only — the\n\
   document typesets its own reference marker. */\n\
.fnref {{ display: inline; }}\n\
aside.footnote {{\n\
  margin: 0.6em 0 1.2em 0;\n\
  padding: 0.2em 0 0.2em 1em;\n\
  border-left: 3px solid rgba(127,127,127,0.35);\n\
  font-size: 0.92em;\n\
}}\n\
aside.footnote .fnback {{\n\
  margin-left: 0.4em;\n\
  text-decoration: none;\n\
  color: inherit;\n\
  opacity: 0.55;\n\
}}\n\
aside.footnote .para {{ margin: 0; text-align: left; }}\n\
aside.footnote .para + .para {{ margin-top: 0.3em; }}\n\
"
    )
}

/// The two rules `--katex` needs, and nothing at all in any other mode.
///
/// Emitted as a separate block rather than folded into [`stylesheet`] for one
/// reason that is worth stating plainly: without the mode test, adding these
/// declarations would change the bytes of EVERY `--format html` render,
/// including the ones this flag has nothing to do with. Opt-in has to mean
/// byte-for-byte opt-in, or "did anything else move?" stops being answerable
/// with `sha256sum`.
///
/// `text-align: center` because a displayed equation is centred in the PDF
/// too, and the surrounding `.para` rule justifies — which, on a paragraph
/// whose sole content is one `\[…\]`, stretches nothing and left-aligns it.
pub(crate) fn math_tex_rules(ctx: &Ctx) -> String {
    if ctx.math != crate::MathMode::Katex {
        return String::new();
    }
    "/* --katex: the equation is LaTeX for the reader's own typesetter. */\n\
     .math-tex { white-space: nowrap; }\n\
     .para.math-display { text-align: center; margin: 1em 0; }\n"
        .to_string()
}

/// One `background-image` rule per image the flow placed more than once
/// (`Ctx::shared_images`, filled during the body walk — so this must be
/// called AFTER it). Each image's bytes appear once here instead of once per
/// placement; see `inline.rs`'s `Image` arm for why that matters and what it
/// costs.
pub(crate) fn shared_image_rules(ctx: &Ctx) -> String {
    let mut out = String::new();
    for id in ctx.shared_images.borrow().iter() {
        if let Some(res) = ctx.images.get(*id) {
            let _ = write!(
                out,
                ".shared-img-{id} {{ background-image: url(\"{}\"); }}\n",
                crate::image::data_uri(res),
            );
        }
    }
    out
}
