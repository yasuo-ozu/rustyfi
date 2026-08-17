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
/// **Termination guarantee**: a page always places at least one line
/// (regardless of `height`) — the overflow check only fires once
/// `lines` is non-empty — so a degenerate scheme (`height <= 0`, or a
/// line taller than the area) still makes forward progress and the
/// lang-side loop is bounded by the vbox count
/// (docs/plans/document-page-model.md's Risks: "Progress / termination").
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
    let mut idx = 0;

    while idx < vboxes.len() {
        match &vboxes[idx] {
            VertBox::Skip(l) => {
                pending_skip += *l;
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
                if baseline + *depth > y_limit && !lines.is_empty() {
                    break; // this line (and everything after) rolls to the next page
                }
                pending_skip = Length::ZERO;
                prev_baseline = Some(baseline);
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
