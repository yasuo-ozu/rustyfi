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

use rustyfi_backend::{OutlineEntry, PureHorzBox, TabularBox, TabularCellBox};

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
pub(crate) fn find_heading_level(bx: &PureHorzBox, ctx: &Ctx) -> Option<i64> {
    match bx {
        PureHorzBox::Frame { deco, contents, .. } => {
            if let Some(name) = ctx.dests.get(deco) {
                if let Some(level) = ctx.outline_by_dest.get(*name) {
                    return Some(*level);
                }
            }
            contents
                .iter()
                .find_map(|(_, inner)| find_heading_level(inner, ctx))
        }
        _ => None,
    }
}

/// `<nav class="toc">`: a nested `<ol>` walk of `extras.outline`'s flat
/// `(level, text, dest_name)` list (design doc §3 "Navigation (always
/// safe)"), each entry a `<li><a href="#dest_name">text</a></li>` — the
/// `href` targets the SAME `id="dest_name"` anchor `block.rs`'s
/// `FrameStart`/`inline.rs`'s `Frame` arm already place (S2's `ctx.dests`),
/// so this is a real, working in-page jump even when the matching paragraph
/// is NOT promoted to a heading (`find_heading_level` fails to match, or the
/// doc class calls `register-outline` without a destination frame at all).
/// No-op (nothing written) when `outline` is empty — the common case for any
/// doc class that never calls `register-outline`.
///
/// Handles non-contiguous level jumps (upstream never validates that
/// `register-outline` levels increase by exactly 1) by opening/closing one
/// `<ol>` per level step, same as a jump of 1 — no synthetic placeholder
/// entries at the skipped levels, just (unusually, but validly) nested empty
/// `<ol>` wrappers.
pub(crate) fn render_toc(out: &mut String, outline: &[OutlineEntry]) {
    if outline.is_empty() {
        return;
    }
    out.push_str("<nav class=\"toc\">\n");
    let mut open_level: i64 = -1; // no <ol> open yet
    let mut li_open = false;
    for entry in outline {
        let target = entry.level.max(0);
        while open_level < target {
            out.push_str("<ol>\n");
            open_level += 1;
            li_open = false;
        }
        while open_level > target {
            if li_open {
                out.push_str("</li>\n");
                li_open = false;
            }
            out.push_str("</ol>\n");
            open_level -= 1;
        }
        if li_open {
            out.push_str("</li>\n");
        }
        let _ = write!(
            out,
            "<li><a href=\"#{}\">{}</a>",
            crate::escape_html(&entry.dest_name),
            crate::escape_html(&entry.text),
        );
        li_open = true;
    }
    while open_level >= 0 {
        if li_open {
            out.push_str("</li>\n");
            li_open = false;
        }
        out.push_str("</ol>\n");
        open_level -= 1;
    }
    out.push_str("</nav>\n");
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

    let _ = write!(out, "<table class=\"tabular\"{extra_attrs}>\n");
    for row in rows {
        out.push_str("<tr>\n");
        for cell in row {
            out.push_str("<td>");
            let mut cell_html = String::new();
            for (_, bx) in &cell.contents {
                super::inline::emit_inline(&mut cell_html, bx, ctx);
            }
            out.push_str(cell_html.trim());
            out.push_str("</td>\n");
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
}
