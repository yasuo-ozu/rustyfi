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
    PathSeg, PureHorzBox, TabularBox, TabularCellBox,
};

use super::Ctx;

/// `dest_name -> level` from `extras.outline` (`register-outline`'s already
/// `Interp::dest_name`-resolved entries) — the lookup table
/// [`find_heading_level`] consults. Owned strings (see `Ctx::outline_by_dest`'s
/// doc comment on why this isn't borrowed).
pub(crate) fn outline_levels(outline: &[OutlineEntry]) -> HashMap<String, i64> {
    outline
        .iter()
        .map(|entry| (entry.dest_name.clone(), entry.level))
        .collect()
}

/// `register-outline`'s `level` is 0-based (`+section` registers level 0,
/// `+subsection` level 1 — `stdjabook.satyh:548`/`:573`); HTML's heading
/// tags are 1-based and capped at 6. A deeper-than-`<h6>` outline (unusual,
/// but upstream never validates outline depth) collapses onto `<h6>` rather
/// than emitting an invalid `<h7>`.
pub(crate) fn heading_tag(level: i64) -> u8 {
    (level.max(0) as u64 + 1).min(6) as u8
}

/// Does `bx` (or, recursively, one of its `Frame` descendants) carry the
/// `DecoId` of a `register-location-frame`/`register-destination` call whose
/// resolved name matches a `register-outline` entry? Returns that entry's
/// level on the first match (document order of the recursion, i.e. the
/// outermost/leftmost matching `Frame` wins — real doc classes never nest
/// two destination frames for the same heading, so this tie-break is
/// unobserved in practice). See this module's doc comment for why this is a
/// structural match, not a heuristic.
///
/// **`InlineFrameMarker` is checked too, and that is what makes this work at
/// all on a real document.** `inline-frame-breakable` splices its contents
/// between a marker PAIR rather than building a `Frame`, so that the frame
/// can split across a line break — and that is how every bundled doc class
/// writes a section title (`stdjabook.satyh:551`, `stdjareport.satyh:445`:
/// `inline-frame-breakable no-pads (Annot.register-location-frame label)`).
/// Matching only `Frame`, as this did, meant no heading in any
/// `stdjabook`/`stdjareport` document was ever promoted: the `latexcmds`
/// manual's seven `+section`s all came out as `<p>`, with the `<nav>`
/// linking to anchors on paragraphs. Only the START marker is consulted —
/// the `end: true` twin carries the same `DecoId` and would match a second
/// time for nothing.
pub(crate) fn find_heading_level(bx: &PureHorzBox, ctx: &Ctx) -> Option<i64> {
    match bx {
        PureHorzBox::InlineFrameMarker { id, end: false, .. } => level_of_deco(id, ctx),
        PureHorzBox::Frame { deco, contents, .. } => level_of_deco(deco, ctx).or_else(|| {
            contents
                .iter()
                .find_map(|(_, inner)| find_heading_level(inner, ctx))
        }),
        _ => None,
    }
}

/// `DecoId` -> destination name (S2's `ctx.dests`) -> outline level
/// (`ctx.outline_by_dest`), the two-hop structural lookup both arms of
/// [`find_heading_level`] share.
fn level_of_deco(deco: &DecoId, ctx: &Ctx) -> Option<i64> {
    let name = ctx.dests.get(deco)?;
    ctx.outline_by_dest.get(*name).copied()
}

/// `PureHorzBox::Tabular` → a real `<table>`/`<tr>`/`<td>` (design doc §3's
/// `Tabular` row: "genuinely recoverable"). `extra_attrs` is an already-
/// formatted attribute-fragment string (e.g. a `margin-top` `style=`, from
/// `block.rs`'s pending-`Skip` bookkeeping) spliced onto the `<table>` tag
/// itself, mirroring how `FrameStart`/`EmbeddedBlock` carry their own
/// pending margin.
///
/// Row grouping is recovered from `TabularCellBox::x` alone: `TabularBox`
/// does not carry the solver's `xs`/`ys` grid-line lists (those exist only
/// on the transient `tabular::Solved` the lang-side rule callback consumes,
/// `rustyfi-backend/src/tabular.rs`'s `Solved` vs. `TabularBox`), but
/// `tabular::solidify_tabular` pushes cells in strict row-major order (outer
/// loop over rows, inner over columns, `Cell::Empty` slots producing no
/// entry at all) — so within one row, `x` (each cell's box-local left edge)
/// is monotonically non-decreasing (later columns start further right); a
/// new row begins exactly when `x` fails to increase. This recovers exact
/// row/column-order grouping for the common case (no `Empty`-gap-heavy
/// spans); a pathological grid whose first visible cell in a row happens to
/// sit further right than the previous row's last visible cell would
/// mis-group — accepted as the "best-effort" edge of an otherwise genuine
/// recovery (see this module's doc comment: unlike lists, this is real
/// recovery, not a guess, for the overwhelming common case).
pub(crate) fn render_table(out: &mut String, tab: &TabularBox, extra_attrs: &str, ctx: &Ctx) {
    if tab.cells.is_empty() {
        return;
    }
    let mut rows: Vec<Vec<&TabularCellBox>> = Vec::new();
    let mut last_x: Option<f64> = None;
    for cell in &tab.cells {
        let x = cell.x.0;
        let starts_new_row = match last_x {
            None => true,
            Some(lx) => x <= lx,
        };
        if starts_new_row {
            rows.push(Vec::new());
        }
        rows.last_mut().expect("just pushed if empty").push(cell);
        last_x = Some(x);
    }

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

/// Which grid lines a table actually draws, recovered from
/// `TabularBox::rules`.
///
/// A stylesheet cannot know this. `rules` is whatever the document's own rule
/// callback drew, and the conventions differ completely: `easytable`'s
/// default draws three horizontal rules and no verticals (the booktabs look),
/// while a `\easytable` with explicit column separators draws a full grid.
/// Giving every cell the same border made the first render as the second, and
/// no table in the corpus looked like its PDF.
///
/// The rules are ordinary graphics — thin filled rectangles or strokes — so
/// each one's bounding box says where it lies, and its position against the
/// cell origins says which boundary it is. Rules the geometry cannot place
/// (a diagonal, a decorative flourish) are simply not reproduced; they draw
/// nothing rather than something wrong.
struct Borders {
    /// `horizontal[r]` is the rule ABOVE row `r`; the extra last entry is the
    /// rule below the final row.
    horizontal: Vec<Option<Rule>>,
    /// `vertical[c]` is the rule LEFT of column `c`; the extra last entry is
    /// the rule right of the final column.
    vertical: Vec<Option<Rule>>,
}

/// One recovered grid line: how thick, and in what colour.
#[derive(Clone, Copy)]
struct Rule {
    width: f64,
    color: Color,
}

/// A rule thinner than this (pt) is invisible in a browser anyway; a
/// coordinate closer than this to a boundary counts as being on it.
const RULE_EPS_PT: f64 = 0.05;

impl Borders {
    fn solve(rows: &[Vec<&TabularCellBox>], rules: &[GraphicsElem]) -> Self {
        let ncols = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut borders = Borders {
            horizontal: vec![None; rows.len() + 1],
            vertical: vec![None; ncols + 1],
        };
        // Row baselines DESCEND (`Solved::ys` runs from the table's height
        // down to 0), so a rule sits above row `r` when its y is above that
        // row's baseline and below the previous row's.
        let baselines: Vec<f64> = rows
            .iter()
            .map(|row| row.first().map_or(0.0, |c| c.baseline_y.0))
            .collect();
        let lefts: Vec<f64> = (0..ncols)
            .map(|c| {
                rows.iter()
                    .filter_map(|row| row.get(c).map(|cell| cell.x.0))
                    .fold(f64::INFINITY, f64::min)
            })
            .collect();
        for elem in rules {
            collect_rule(elem, &baselines, &lefts, &mut borders);
        }
        borders
    }

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
        edge("top", self.horizontal.get(r).copied().flatten());
        edge("left", self.vertical.get(c).copied().flatten());
        if r + 1 == self.horizontal.len() - 1 {
            edge("bottom", self.horizontal[r + 1]);
        }
        if c + 1 == row_len {
            edge("right", self.vertical.get(c + 1).copied().flatten());
        }
        if decls.is_empty() {
            String::new()
        } else {
            format!(" style=\"{decls}\"")
        }
    }
}

/// Place one rule graphic on the grid, recursing through `Group`/`Clip` so a
/// united rule set is read the same way a flat one is.
fn collect_rule(
    elem: &GraphicsElem,
    baselines: &[f64],
    lefts: &[f64],
    borders: &mut Borders,
) {
    let (color, stroke_w) = match elem {
        GraphicsElem::Fill(c, _) => (*c, None),
        GraphicsElem::Stroke(w, c, _) | GraphicsElem::DashedStroke(w, _, c, _) => (*c, Some(w.0)),
        GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => {
            for e in inner {
                collect_rule(e, baselines, lefts, borders);
            }
            return;
        }
        _ => return,
    };
    let Some((lo, hi)) = graphics_bbox(elem) else {
        return;
    };
    let (Length(x0), Length(y0)) = lo;
    let (Length(x1), Length(y1)) = hi;
    let (w, h) = (x1 - x0, y1 - y0);
    if w >= h {
        // Horizontal: above row `r` = the number of rows whose baseline is
        // above this rule's own centre line.
        let y = (y0 + y1) / 2.0;
        let above = baselines.iter().filter(|b| **b > y + RULE_EPS_PT).count();
        let rule = Rule {
            width: stroke_w.unwrap_or(h).max(RULE_EPS_PT),
            color,
        };
        if let Some(slot) = borders.horizontal.get_mut(above) {
            *slot = Some(rule);
        }
    } else {
        let x = (x0 + x1) / 2.0;
        let left_of = lefts.iter().filter(|l| **l < x - RULE_EPS_PT).count();
        let rule = Rule {
            width: stroke_w.unwrap_or(w).max(RULE_EPS_PT),
            color,
        };
        if let Some(slot) = borders.vertical.get_mut(left_of) {
            *slot = Some(rule);
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
