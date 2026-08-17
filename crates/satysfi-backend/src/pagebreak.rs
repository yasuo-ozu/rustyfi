//! Page breaking. `docs/plans/document-page-model.md` Slice 1: a single
//! **stateless** per-page chopper (`chop_page`) plus a fixed-origin
//! placement helper (`place_block_at`), replacing the old whole-document
//! `break_pages` — the per-page loop now lives lang-side
//! (`satysfi-lang/src/primitives.rs`'s `prim_page_break`), the one place
//! that legally holds `&mut Interp` to apply the two scheme closures per
//! page (see that plan's "who drives it" section).

use crate::hbox::PureHorzBox;
use crate::length::Length;
use crate::vbox::VertBox;

/// One typeset line placed on a page, in page coordinates (y grows downward
/// from the paper top; the PDF writer flips it).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedLine {
    pub x: Length,
    pub baseline_y: Length,
    pub contents: Vec<(Length, PureHorzBox)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub lines: Vec<PlacedLine>,
}

/// Fill ONE page's content area (the port of `chop_single_column`,
/// `pageBreak.ml`): consume `vboxes` from the front until the next line
/// would overflow `origin.1 + height`, leaving whatever didn't fit in
/// `vboxes` for the caller's next page. No looping, no closures, no lang
/// knowledge.
///
/// **Termination guarantee**: a page always places at least one *real*
/// line (regardless of `height`) — the overflow check only fires once one
/// has been placed, tracked by `placed_real_line` (not `lines.is_empty()`,
/// since a `HookPageBreak` marker can occupy a `PlacedLine` slot without
/// being real content) — so a degenerate scheme (`height <= 0`, or a line
/// taller than the area) still makes forward progress and the lang-side
/// loop is bounded by the vbox count (docs/plans/document-page-model.md's
/// Risks: "Progress / termination").
///
/// `VertBox::ClearPage` (`clear-page`) ends the page immediately once at
/// least one real line has been placed (mirrors `pageBreak.ml`'s
/// `PBClearPage`); a `clear-page` with nothing yet placed is redundant and
/// swallowed instead (mirrors `pageBreak.ml`'s `omit_redundant_clear`),
/// so a leading `clear-page` doesn't produce a pointless blank page.
///
/// `VertBox::HookPageBreak` (`hook-page-break-block`) is placed as a
/// zero-height marker `PlacedLine` carrying the hook's `PureHorzBox`
/// wrapper at the position it sits in the flow — `fire_hooks` (satysfi-lang)
/// already scans every placed line's contents for this box regardless of
/// whether it came from the inline or block-level hook primitive, so no
/// change to that seam is needed.
pub fn chop_page(
    origin: (Length, Length),
    height: Length,
    vboxes: &mut Vec<VertBox>,
) -> Vec<PlacedLine> {
    let (x0, y0) = origin;
    let y_limit = y0 + height;

    let mut lines: Vec<PlacedLine> = Vec::new();
    let mut prev_baseline: Option<Length> = None;
    let mut pending_skip = Length::ZERO;
    let mut placed_real_line = false;
    let mut idx = 0;

    while idx < vboxes.len() {
        match &vboxes[idx] {
            VertBox::Skip(l) => {
                pending_skip += *l;
                idx += 1;
            }
            VertBox::ClearPage => {
                idx += 1;
                if placed_real_line {
                    break; // ends the page right here; the marker is consumed
                }
                // Redundant: nothing real placed on this page yet — swallow it.
            }
            VertBox::HookPageBreak(id) => {
                let pos = prev_baseline.unwrap_or(y0);
                lines.push(PlacedLine {
                    x: x0,
                    baseline_y: pos,
                    contents: vec![(Length::ZERO, PureHorzBox::HookPageBreak { id: *id })],
                });
                idx += 1;
            }
            VertBox::Line {
                height: h,
                depth,
                leading,
                contents,
            } => {
                let baseline = match prev_baseline {
                    None => y0 + pending_skip + *h,
                    Some(b) => b + leading.max(*h) + pending_skip,
                };
                if baseline + *depth > y_limit && placed_real_line {
                    break; // this line (and everything after) rolls to the next page
                }
                pending_skip = Length::ZERO;
                prev_baseline = Some(baseline);
                placed_real_line = true;
                lines.push(PlacedLine {
                    x: x0,
                    baseline_y: baseline,
                    contents: contents.clone(),
                });
                idx += 1;
            }
        }
    }
    vboxes.drain(0..idx);
    lines
}

/// Solidify block-boxes at a fixed top-down page origin — headers and
/// footers are not page-broken (`write_page`, `handlePdf.ml:465-471`): a
/// header/footer is placed as-is, with no height limit and no overflow
/// check. `chop_page` without the page-end check.
pub fn place_block_at(origin: (Length, Length), vboxes: Vec<VertBox>) -> Vec<PlacedLine> {
    let (x0, y0) = origin;
    let mut lines = Vec::new();
    let mut prev_baseline: Option<Length> = None;
    let mut pending_skip = Length::ZERO;

    for vbox in vboxes {
        match vbox {
            VertBox::Skip(l) => pending_skip += l,
            // No page-breaking happens here (headers/footers aren't
            // page-broken), so `clear-page` has nothing to do.
            VertBox::ClearPage => {}
            VertBox::HookPageBreak(id) => {
                let pos = prev_baseline.unwrap_or(y0);
                lines.push(PlacedLine {
                    x: x0,
                    baseline_y: pos,
                    contents: vec![(Length::ZERO, PureHorzBox::HookPageBreak { id })],
                });
            }
            VertBox::Line {
                height,
                leading,
                contents,
                ..
            } => {
                let baseline = match prev_baseline {
                    None => y0 + pending_skip + height,
                    Some(b) => b + leading.max(height) + pending_skip,
                };
                pending_skip = Length::ZERO;
                prev_baseline = Some(baseline);
                lines.push(PlacedLine {
                    x: x0,
                    baseline_y: baseline,
                    contents,
                });
            }
        }
    }
    lines
}
