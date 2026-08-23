//! The reflow document's base stylesheet (Slice 1: "Base document CSS (max-width
//! column, line-height, font stack, dark/light neutral)"). Everything here is
//! FLOW layout — no `position: absolute`/`top`/`left` anywhere (the defining
//! difference from `crate::render_html_fixed`'s faithful `.page`/`.run` rules,
//! which position every run at its exact placed coordinate).
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
//! ## What this stylesheet deliberately does NOT draw
//!
//! `.frame` has no border. A `FrameStart`/`FrameEnd` pair is
//! `block-frame-breakable`, which real packages use for section bodies, list
//! items, figure wrappers and quotation blocks as readily as for anything
//! that actually has a rule around it — the `enumitem` manual alone opens
//! 336 of them. The decoration that would say which is which is a lang-side
//! callback this backend cannot run, so drawing a box around every one
//! turned the page into nested rounded rectangles. It stays a semantic
//! grouping element with margins, and the class remains for anyone who wants
//! to restyle it.

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
:root {{ color-scheme: light dark; }}\n\
body {{\n\
  margin: 0;\n\
  padding: 2rem 1rem;\n\
  font-family: {body_family};\n\
  font-size: {body_size}pt;\n\
  line-height: 1.7;\n\
  background: #fff;\n\
  color: #1a1a1a;\n\
}}\n\
@media (prefers-color-scheme: dark) {{\n\
  body {{ background: #14161a; color: #e8e8e8; }}\n\
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
.embed {{ margin: 0.5em 0; }}\n\
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
@media (prefers-color-scheme: dark) {{\n\
  a.link {{ color: #62a0ea; }}\n\
}}\n\
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
nav.toc {{\n\
  margin: 0 auto 2em auto;\n\
  max-width: {max_width}pt;\n\
  width: 100%;\n\
  box-sizing: border-box;\n\
  border: 1px solid rgba(127,127,127,0.4);\n\
  border-radius: 4px;\n\
  padding: 0.75em 1.5em;\n\
}}\n\
/* No list marker: an outline entry's own text already begins with the\n\
   number the document assigned it, so the UA decimal counter would\n\
   render that number a second time. */\n\
nav.toc ol {{ margin: 0.2em 0; padding: 0 0 0 1.2em; list-style: none; }}\n\
nav.toc > ol {{ padding-left: 0; }}\n\
nav.toc li {{ margin: 0.15em 0; }}\n\
nav.toc a {{ text-decoration: none; }}\n\
nav.toc a:hover {{ text-decoration: underline; }}\n\
table.tabular {{\n\
  border-collapse: collapse;\n\
  margin: 1em auto;\n\
  max-width: 100%;\n\
}}\n\
table.tabular td {{\n\
  border: 1px solid rgba(127,127,127,0.4);\n\
  padding: 0.3em 0.6em;\n\
  vertical-align: baseline;\n\
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

/// One `background-image` rule per image the flow placed more than once
/// (`Ctx::shared_images`, filled during the body walk — so this must be
/// called AFTER it, exactly like `fonts::font_face_rules`). Each image's
/// bytes appear once here instead of once per placement; see `inline.rs`'s
/// `Image` arm for why that matters and what it costs.
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
