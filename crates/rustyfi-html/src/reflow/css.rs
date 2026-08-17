//! The reflow document's base stylesheet (`docs/plans/design-reflowable-html.md`
//! §6 Slice 1: "Base document CSS (max-width column, line-height, font
//! stack, dark/light neutral)"). Everything here is FLOW layout — no
//! `position: absolute`/`top`/`left` anywhere (the defining difference from
//! `crate::render_html`'s faithful `.page`/`.run` rules, which position
//! every run at its exact placed coordinate).
//!
//! S4 (`docs/plans/design-reflow-s4-lists.md` §6.2) adds `ul.list`/`ol.list`
//! spacing rules for the real `<ul>`/`<ol>`/`<li>` `block.rs`'s
//! `VertBox::ListMark` arm now emits — bullet/number glyphs and basic
//! indentation are otherwise the browser's own UA stylesheet default for
//! `<ul>`/`<ol>`/`<li>`, so no additional list-style rule is needed;
//! `<em>`/`<strong>` (S4's other new tag pair) likewise need no CSS of their
//! own — the browser default (italic/bold) is exactly the semantic they
//! name.

use rustyfi_backend::PageGeometry;

/// `geometry.text_width` seeds the reading column's `max-width` (in `pt`,
/// SATySFi's own unit, 1:1 with CSS `pt`) — a nod to the source document's
/// own measure without pinning the layout to it: a narrow viewport still
/// shrinks the column further (the `max-width`/`width:100%` combination),
/// and nothing here prevents the column from being narrower than the
/// viewport on a wide screen either. Purely a readability default, not a
/// geometry replay.
pub(crate) fn stylesheet(geometry: &PageGeometry) -> String {
    let max_width = geometry.text_width.0.max(1.0);
    format!(
        "\
:root {{ color-scheme: light dark; }}\n\
body {{\n\
  margin: 0;\n\
  padding: 2rem 1rem;\n\
  font-family: Georgia, \"Noto Serif\", \"Noto Serif CJK JP\", serif;\n\
  line-height: 1.6;\n\
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
  margin: 0 0 1em 0;\n\
  text-align: justify;\n\
  overflow-wrap: break-word;\n\
}}\n\
.frame {{\n\
  margin: 1em 0;\n\
  padding: 0.75em 1em;\n\
  border: 1px solid rgba(127,127,127,0.4);\n\
  border-radius: 4px;\n\
}}\n\
.embed {{ margin: 0.5em 0; }}\n\
.iframe {{ display: inline; }}\n\
.clearpage {{\n\
  border: none;\n\
  border-top: 1px dashed rgba(127,127,127,0.5);\n\
  margin: 2em 0;\n\
}}\n\
.reflow-empty {{ font-style: italic; opacity: 0.7; }}\n\
.gfx-placeholder, .image-placeholder, \
.footnote-placeholder {{\n\
  display: inline-block;\n\
  padding: 0 0.2em;\n\
  font-style: italic;\n\
  opacity: 0.6;\n\
}}\n\
a.link {{ color: #1a5fb4; text-decoration: underline; }}\n\
@media (prefers-color-scheme: dark) {{\n\
  a.link {{ color: #62a0ea; }}\n\
}}\n\
.heading {{\n\
  margin: 1em 0 0.5em 0;\n\
  font-weight: bold;\n\
  line-height: 1.3;\n\
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
nav.toc ol {{ margin: 0.2em 0; padding: 0 0 0 1.2em; }}\n\
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
"
    )
}
