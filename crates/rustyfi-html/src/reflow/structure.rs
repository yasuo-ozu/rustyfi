//! Above-flat structure recovery ("S3" / §2 "the one real lever:
//! `extras.outline`" / §3's `Tabular` row). Everything here is additive to
//! Slices 1/2's flat paragraph-and-frame sequence; nothing in this module
//! changes behavior when its inputs are absent (an empty `extras.outline`, no
//! `Tabular` box in the flow).
//!
//! ## Headings — best-effort, but a STRUCTURAL match, not a heuristic
//!
//! `+section`/`+subsection` names are erased at eval (design doc §2), so
//! there is no "this paragraph is a level-0 heading" tag anywhere in the box
//! tree. The one surviving side-channel is `extras.outline` — populated by
//! `register-outline` — paired with `register-location-frame`/
//! `register-destination`, which real doc classes (`stdjabook.satyh`'s
//! `section-scheme`/`subsection-scheme`) call on the SAME `label` used for
//! the section's own `+register-outline` entry. Both resolve that label
//! through `Interp::dest_name` (`rustyfi-lang/src/primitives.rs`'s
//! `prim_register_outline`/`prim_register_destination`), so an outline
//! entry's `dest_name` and a `register-location-frame`-wrapped heading's
//! `Frame::deco` (via `Ctx::dests`, S2) name the exact same destination —
//! `find_heading_level` below is therefore a STRUCTURAL id match, not a
//! text/font-size guess. It is still "best-effort" in the sense the design
//! doc means: a doc class that never wraps its heading title in a
//! `register-location-frame`-style deco (only calls `register-outline`, or
//! doesn't call either) gets no promoted `<h#>` for that entry — the title
//! stays a plain `<p>`. No font-size/weight heuristic fallback is
//! implemented: guessing "biggest font on this line = a heading" would
//! promote arbitrarily-styled emphasis runs too (design doc §3's own
//! warning about emphasis provenance being unrecoverable), which is worse
//! than leaving an unmatched heading as a paragraph.
//!
//! ## Tables — genuinely recoverable
//!
//! `PureHorzBox::Tabular` (`rustyfi_backend::TabularBox`) keeps every cell's
//! already-typeset content (`TabularCellBox::contents`, a
//! `Vec<(Length, PureHorzBox)>` exactly like a `VertBox::Line`'s), so unlike
//! headings/lists this is a REAL structural recovery, not best-effort:
//! [`render_table`] regroups the solved cell list back into rows/columns and
//! emits a real `<table>`/`<tr>`/`<td>`.
//!
//! ## Lists — RESOLVED in S4, via a new lever (not this module)
//!
//! `itemize`/`enumerate` erase their own structure just as thoroughly as
//! headings do (nesting is flattened, `block-frame-breakable`'s frame marker
//! is shared with unrelated content like `+figure`, and the bullet/number
//! glyph is indistinguishable from arbitrary graphics/text) — WITHOUT
//! outline's side-channel to fall back on. Promoting to `<ul>`/`<li>` from
//! the box tree alone would mean inventing structure the box tree does not
//! expose, exactly the line this module's heading logic refuses to cross.
//!
//! resolves this with a NEW, genuinely additive lever this module does NOT
//! implement: dedicated inert marker boxes
//! (`VertBox::ListMark`/`PureHorzBox::InlineMark`) emitted POSITIONALLY by a
//! modified `lib-rustyfi/dist-v01/packages/itemize.satyh` (list/item
//! boundaries, ordered-vs-unordered, bullet fencing) — the direct analogue
//! of `FrameStart`/`FrameEnd` above, not a side-channel record (a
//! side-channel fails here for the same reason a `DecoId`-keyed table can't
//! disambiguate "this is a list item frame" from a `+figure`'s frame — see
//! the design doc §3's rejection of that option). See `block.rs`'s
//! `VertBox::ListMark` arm (list/item nesting, via a small open-tag stack)
//! and `inline.rs`'s `PureHorzBox::InlineMark` arm (bullet-fence
//! suppression) for the consuming side.

use std::collections::HashMap;
use std::fmt::Write as _;

use rustyfi_backend::{
    graphics_bbox, path_bbox, Color, DecoId, FrameDecoration, GraphicsElem, Length, OutlineEntry,
    PathSeg, PureHorzBox, TabularBox,
};

use super::Ctx;
use crate::recover::{Borders, Rule, RULE_EPS_PT};

/// `dest_name -> level` from `extras.outline` (`register-outline`'s already
/// `Interp::dest_name`-resolved entries) — the lookup table
/// [`find_heading_level`] consults. Owned strings (see `Ctx::outline_by_dest`'s
/// doc comment on why this isn't borrowed).
pub(crate) fn outline_levels(outline: &[OutlineEntry]) -> HashMap<String, i64> {
    crate::recover::outline_levels(outline)
}

/// `register-outline`'s 0-based `level` as an HTML heading tag NUMBER
/// (`<h1>`..`<h6>`) — [`crate::recover::heading_depth`], which is the same
/// 1-based, 6-capped depth Markdown's `#`..`######` uses.
pub(crate) fn heading_tag(level: i64) -> u8 {
    crate::recover::heading_depth(level)
}

/// Does `bx` (or, recursively, one of its `Frame` descendants) carry the
/// `DecoId` of a `register-location-frame`/`register-destination` call whose
/// resolved name matches a `register-outline` entry? Returns that entry's
/// level on the first match (document order of the recursion, i.e. the
/// outermost/leftmost matching `Frame` wins — real doc classes never nest
/// two destination frames for the same heading, so this tie-break is
/// unobserved in practice). See this module's doc comment for why this is a
/// structural match, not a heuristic, and
/// [`crate::recover::find_heading_level`] — which both backends call — for
/// why `InlineFrameMarker` has to be checked as well as `Frame`.
pub(crate) fn find_heading_level(bx: &PureHorzBox, ctx: &Ctx) -> Option<i64> {
    crate::recover::find_heading_level(bx, &ctx.dests, &ctx.outline_by_dest)
}

/// `PureHorzBox::Tabular` → a real `<table>`/`<tr>`/`<td>` (design doc §3's
/// `Tabular` row: "genuinely recoverable"). `extra_attrs` is an already-
/// formatted attribute-fragment string (e.g. a `margin-top` `style=`, from
/// `block.rs`'s pending-`Skip` bookkeeping) spliced onto the `<table>` tag
/// itself, mirroring how `FrameStart`/`EmbeddedBlock` carry their own
/// pending margin.
///
/// Row grouping is [`crate::recover::table_rows`] — recovered from
/// `TabularCellBox::x` alone, shared with the Markdown backend, and
/// documented there; so is which grid lines the table draws ([`Borders`]),
/// which the LaTeX backend needs for exactly the same reason a bordered
/// rendering does.
pub(crate) fn render_table(out: &mut String, tab: &TabularBox, extra_attrs: &str, ctx: &Ctx) {
    if tab.cells.is_empty() {
        return;
    }
    let rows = crate::recover::table_rows(tab);

    let paired;
    let rules: &[GraphicsElem] = if tab.rules.is_empty() {
        paired = ctx
            .tabular_rules
            .borrow()
            .iter()
            .rev()
            .find(|(w, h, _)| {
                (w - tab.width.0).abs() < RULE_EPS_PT && (h - tab.height.0).abs() < RULE_EPS_PT
            })
            .map(|(_, _, r)| r.clone());
        paired.as_deref().unwrap_or(&[])
    } else {
        &tab.rules
    };
    let borders = Borders::solve(&rows, rules);

    let mut table = String::new();
    let mut any_content = false;
    let _ = write!(table, "<table class=\"tabular\"{extra_attrs}>\n");
    for (r, row) in rows.iter().enumerate() {
        table.push_str("<tr>\n");
        for (c, cell) in row.iter().enumerate() {
            let _ = write!(table, "<td{}>", borders.style_for(r, c, row.len()));
            // A cell is a hard flow boundary in both directions: glue left
            // pending by the previous cell must not open this one with a
            // space, and this cell's last character must not decide the
            // spacing of the next.
            ctx.reset_flow();
            let mut cell_html = String::new();
            for (_, bx) in &cell.contents {
                super::inline::emit_inline(&mut cell_html, bx, ctx);
            }
            super::inline::close_run(&mut cell_html, ctx);
            ctx.reset_flow();
            let trimmed = cell_html.trim();
            any_content |= super::text::has_visible_content(trimmed);
            table.push_str(trimmed);
            table.push_str("</td>\n");
        }
        table.push_str("</tr>\n");
    }
    table.push_str("</table>\n");
    if any_content {
        out.push_str(&table);
    }
}

impl crate::recover::Borders {
    /// The ` style="…"` fragment for the cell at `(r, c)`, or the empty
    /// string when no rule touches it. Each cell states only its own top and
    /// left edge, plus the bottom/right of the last row/column — with
    /// `border-collapse: collapse` that draws each shared line exactly once.
    fn style_for(&self, r: usize, c: usize, row_len: usize) -> String {
        let mut decls = String::new();
        let mut edge = |side: &str, rule: Option<Rule>| {
            if let Some(rule) = rule {
                let _ = write!(
                    decls,
                    "border-{side}:{}pt solid {};",
                    rule.width,
                    crate::svg::css_color(rule.color),
                );
            }
        };
        edge("top", self.horizontal(r));
        edge("left", self.vertical(c));
        if r + 1 == self.rows() {
            edge("bottom", self.horizontal(r + 1));
        }
        if c + 1 == row_len {
            edge("right", self.vertical(c + 1));
        }
        if decls.is_empty() {
            String::new()
        } else {
            format!(" style=\"{decls}\"")
        }
    }
}

/// What a framed `<div>` needs in order to show its own decoration.
pub(crate) struct FrameRender {
    /// Appended to `class="frame…"`.
    pub(crate) extra_class: &'static str,
    /// CSS declarations for the `<div>` itself.
    pub(crate) style: String,
    /// Markup to emit as the div's FIRST child, before its content.
    pub(crate) svg: String,
}

impl FrameRender {
    fn none() -> Self {
        FrameRender {
            extra_class: "",
            style: String::new(),
            svg: String::new(),
        }
    }
}

/// Turn a block frame's recorded decoration into something a browser can draw
/// at any width.
///
/// Two shapes, because they want different answers:
///
/// - **A plain filled panel** — one `Fill` covering the frame, which is what
///   `+code`'s grey box and most highlight frames are — becomes a real
///   `background-color`. Exact at every width, and it costs no markup.
/// - **Anything else** — the rounded double stroke around a `stdjabook`
///   title, a rule under a heading — becomes an `<svg>` sized to the div and
///   stretched (`preserveAspectRatio="none"`). Stretching is the honest
///   compromise: the drawing was authored for one specific width, and the
///   reader's column is a different one. A corner radius therefore distorts
///   in proportion to how far the column has moved from the original.
///
/// The frame's own padding rides along in both cases: without it the content
/// sits on top of the border it is supposed to be inside.
pub(crate) fn frame_decoration(deco: &DecoId, ctx: &Ctx) -> FrameRender {
    let Some(frame) = ctx.frame_decos.get(deco) else {
        return FrameRender::none();
    };
    if frame.elems.is_empty() || frame.width.0 <= 0.0 || frame.height.0 <= 0.0 {
        return FrameRender::none();
    }
    if let Some(color) = solid_panel(frame) {
        return FrameRender {
            extra_class: " framed",
            style: format!("background:{};{}", crate::svg::css_color(color), pad_right(frame)),
            svg: String::new(),
        };
    }
    let mut svg = String::new();
    crate::svg::emit_graphics(
        &mut svg,
        &frame.elems,
        frame.width.0,
        frame.height.0,
        0.0,
        0.0,
        frame.height.0,
        &mut |_svg, _bx, _x, _y| {},
    );
    FrameRender {
        extra_class: " framed",
        style: pad_right(frame),
        svg: retarget_svg(&svg, frame),
    }
}

/// The one padding the flow does not already carry — see
/// `FrameDecoration::pads`.
fn pad_right(frame: &FrameDecoration) -> String {
    let r = frame.pads.1 .0;
    if r > 0.5 {
        format!("padding-right:{r}pt;")
    } else {
        String::new()
    }
}

/// Replace `emit_graphics`' opening `<svg>` — absolutely positioned at its
/// caller's anchor point and sized in points — with
/// one that fills the `<div>` instead.
///
/// The `viewBox` is widened to the drawing's OWN extent where that reaches
/// outside the frame box, which it routinely does: a rounded frame's corners
/// bulge 14pt past each edge, and at the box's own width those 14pt hung off
/// the side of the reading column. Fitting the window to the ink keeps the
/// whole decoration inside the element that owns it, at the cost of drawing
/// it a few percent smaller than the frame it surrounds.
fn retarget_svg(svg: &str, frame: &FrameDecoration) -> String {
    let (w, h) = (frame.width.0, frame.height.0);
    let (mut x0, mut y0, mut x1, mut y1) = (0.0f64, 0.0f64, w, h);
    for elem in &frame.elems {
        if let Some(((Length(ex0), Length(ey0)), (Length(ex1), Length(ey1)))) = graphics_bbox(elem)
        {
            x0 = x0.min(ex0);
            y0 = y0.min(ey0);
            x1 = x1.max(ex1);
            y1 = y1.max(ey1);
        }
    }
    // `emit_graphics`' `<g>` maps box-local y-up onto a y-down viewBox by
    // `y_down = height - y_up`, so the widened window's top edge is the
    // drawing's HIGHEST point.
    let view = format!(
        "viewBox=\"{} {} {} {}\"",
        x0,
        h - y1,
        (x1 - x0).max(0.01),
        (y1 - y0).max(0.01),
    );
    let Some(end) = svg.find('>') else {
        return svg.to_string();
    };
    format!(
        "<svg class=\"frame-deco\" preserveAspectRatio=\"none\" {view}{}",
        &svg[end..]
    )
}

/// The colour of a decoration that is nothing but a filled rectangle covering
/// the whole frame, or `None` for anything with an outline, a curve, or more
/// than one element.
fn solid_panel(frame: &FrameDecoration) -> Option<Color> {
    let [GraphicsElem::Fill(color, path)] = frame.elems.as_slice() else {
        return None;
    };
    let ((Length(x0), Length(y0)), (Length(x1), Length(y1))) = path_bbox(path);
    let covers = x0 <= RULE_EPS_PT
        && y0 <= RULE_EPS_PT
        && (x1 - frame.width.0).abs() < 1.0
        && (y1 - frame.height.0).abs() < 1.0;
    // A rectangle has four corners and no curves; anything else drawn edge to
    // edge (a rounded panel, a blob) has to go through the SVG path.
    let rectangular = path.subpaths.len() == 1
        && path.subpaths[0]
            .segs
            .iter()
            .all(|s| matches!(s, PathSeg::Line(_)));
    (covers && rectangular).then_some(*color)
}
