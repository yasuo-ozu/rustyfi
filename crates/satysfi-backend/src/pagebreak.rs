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

/// The line's own `(height-above-baseline, depth-below-baseline)` as-placed,
/// or `None` if `line` carries no real content (only zero-width markers —
/// `HookPageBreak`/`FrameMarker`, or nothing but glue). Used by
/// `fire_hooks`'s block-fragment extent accumulation (§D,
/// docs/plans/hooks-annotations-crossref.md) to derive a frame fragment's
/// rect from the real lines between its `FrameStart`/`FrameEnd` markers,
/// since `PlacedLine` itself doesn't carry the height/depth `chop_page`
/// used when placing it — recomputed here from each box's own dimensions,
/// the same per-box shape `linebreak.rs`'s `justify_line` uses.
pub fn placed_line_extent(line: &PlacedLine) -> Option<(Length, Length)> {
    fn go(bx: &PureHorzBox, height: &mut Length, depth: &mut Length, has_real: &mut bool) {
        match bx {
            PureHorzBox::HookPageBreak { .. }
            | PureHorzBox::FrameMarker { .. }
            // A footnote marker is inert here too (its rendered extent is
            // the separately bottom-placed stack's own `PlacedLine`s, not
            // this referencing line's).
            | PureHorzBox::Footnote { .. } => {}
            PureHorzBox::OuterEmpty { .. } | PureHorzBox::OuterFil | PureHorzBox::FixedEmpty { .. } => {}
            PureHorzBox::InnerString { height: h, depth: d, .. }
            | PureHorzBox::Graphics { height: h, depth: d, .. }
            | PureHorzBox::GraphicsOuter { height: h, depth: d, .. }
            | PureHorzBox::Math { height: h, depth: d, .. }
            | PureHorzBox::EmbeddedBlock { height: h, depth: d, .. }
            | PureHorzBox::Frame { height: h, depth: d, .. } => {
                *has_real = true;
                *height = (*height).max(*h);
                *depth = (*depth).max(*d);
            }
            PureHorzBox::Image { height: h, .. } => {
                *has_real = true;
                *height = (*height).max(*h);
            }
            PureHorzBox::Tabular(tab) => {
                *has_real = true;
                *height = (*height).max(tab.height);
                *depth = (*depth).max(tab.depth);
            }
            PureHorzBox::Discretionary { no_break, .. } => {
                for p in no_break {
                    go(p, height, depth, has_real);
                }
            }
        }
    }
    let mut height = Length::ZERO;
    let mut depth = Length::ZERO;
    let mut has_real = false;
    for (_, bx) in &line.contents {
        go(bx, &mut height, &mut depth, &mut has_real);
    }
    has_real.then_some((height, depth))
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
///
/// **Footnotes** (`add-footnote`; upstream `pageBreak.ml:131-142` +
/// `handlePdf.ml:400-403`): the moment a `VertBox::Line` is COMMITTED to
/// this page (i.e. it fits and is actually placed, not rolled to the next
/// page), every `PureHorzBox::Footnote` marker reachable from its contents
/// is extracted (`collect_footnotes`) and its stacked height
/// (`stack_height`) is added to a running reservation that shrinks the
/// effective bottom limit for every *subsequent* overflow check on this
/// page — so a line whose footnote would no longer fit rolls to the next
/// page taking its footnote with it (upstream's `hgttotalA`/`:158`). After
/// the chop loop, the collected footnote blocks are bottom-aligned in this
/// column via `place_block_at` at `y0 + height - stack_height` (the port of
/// `get_footnote_origin_position`) and appended to the returned lines.
/// Degenerate case: the FIRST real line of a page is always placed even if
/// its own footnote would overflow the column (the progress guarantee below
/// takes priority — bounded, and matches upstream's unconditional advance).
pub fn chop_page(
    origin: (Length, Length),
    height: Length,
    vboxes: &mut Vec<VertBox>,
) -> Vec<PlacedLine> {
    let (x0, y0) = origin;
    let y_limit = y0 + height;

    let mut lines: Vec<PlacedLine> = Vec::new();
    let mut footnotes: Vec<VertBox> = Vec::new();
    let mut footnote_h = Length::ZERO;
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
            VertBox::FrameStart(id) => {
                let pos = prev_baseline.unwrap_or(y0);
                lines.push(PlacedLine {
                    x: x0,
                    baseline_y: pos,
                    contents: vec![(
                        Length::ZERO,
                        PureHorzBox::FrameMarker { id: *id, end: false },
                    )],
                });
                idx += 1;
            }
            VertBox::FrameEnd(id) => {
                let pos = prev_baseline.unwrap_or(y0);
                lines.push(PlacedLine {
                    x: x0,
                    baseline_y: pos,
                    contents: vec![(
                        Length::ZERO,
                        PureHorzBox::FrameMarker { id: *id, end: true },
                    )],
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
                // A committed line's footnotes shrink the usable page
                // bottom the moment it is placed (pageBreak.ml:138,
                // `hgttotalA = … +% hgtnewfootnote`); a line that rolls to
                // the next page takes its footnotes with it (`:158`).
                let mut new_footnotes = Vec::new();
                collect_footnotes(contents, &mut new_footnotes);
                let footnote_h_new = if new_footnotes.is_empty() {
                    footnote_h
                } else {
                    let mut all = footnotes.clone();
                    all.extend(new_footnotes.iter().cloned());
                    stack_height(&all)
                };
                if baseline + *depth > y_limit - footnote_h_new && placed_real_line {
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
                footnotes.extend(new_footnotes);
                footnote_h = footnote_h_new;
                idx += 1;
            }
        }
    }
    vboxes.drain(0..idx);

    // Bottom-align this column's footnotes in its content area:
    // origin = (x0, y0 + height - stack_height) — the port of
    // `get_footnote_origin_position` (handlePdf.ml:400-403) +
    // `add_column_to_page`'s per-column footnote placement.
    if !footnotes.is_empty() {
        let fh = stack_height(&footnotes);
        lines.extend(place_block_at((x0, y0 + height - fh), footnotes));
    }
    lines
}

/// The vertical extent `place_block_at(_, vboxes)` will occupy below its
/// origin: the last line's `baseline + depth`, advancing exactly as
/// `place_block_at` does (`leading.max(height)` between lines; a trailing
/// skip after the last line is not counted, mirroring its unflushed
/// `pending_skip`). The port of `get_height_of_evaled_vert_box_list`
/// (handlePdf.ml:401) — computed with the SAME rule as placement so the
/// bottom reservation always equals the drawn extent. (Tiny faithful-in-
/// spirit deviation from upstream, which reserves `Σ(h+d)` per line instead
/// of this leading-aware advance; they differ only when a footnote line's
/// `leading > height + next depth`, and only by whitespace — see the plan's
/// Risks note.)
fn stack_height(vboxes: &[VertBox]) -> Length {
    let mut prev_baseline: Option<Length> = None;
    let mut pending_skip = Length::ZERO;
    let mut bottom = Length::ZERO;
    for vb in vboxes {
        match vb {
            VertBox::Skip(l) => pending_skip += *l,
            VertBox::ClearPage | VertBox::HookPageBreak(_) => {}
            VertBox::FrameStart(_) | VertBox::FrameEnd(_) => {}
            VertBox::Line {
                height,
                depth,
                leading,
                ..
            } => {
                let baseline = match prev_baseline {
                    None => pending_skip + *height,
                    Some(b) => b + leading.max(*height) + pending_skip,
                };
                pending_skip = Length::ZERO;
                prev_baseline = Some(baseline);
                bottom = baseline + *depth;
            }
        }
    }
    bottom
}

/// Extract every footnote block reachable from one line's contents, in
/// inline order — the port of `PageInfo.embed_page_info`'s footnote pass
/// (pageInfo.ml:13-52): recurses into a `Discretionary`'s rendered
/// `no_break` slot, `Tabular` cells, `EmbeddedBlock` lines, and (matching
/// upstream's `ImHorzFrame` arm, pageInfo.ml:27-30, which recurses into
/// `imhbs` before wrapping the result back up as `EvHorzFrame`) a `Frame`'s
/// own `contents` — a document class's `FootnoteScheme.main` wraps its
/// `add-footnote` call in `Inline.no-break` (`inline.satyh`'s `no-break =
/// inline-frame-outer (0pt,0pt,0pt,0pt) Deco.empty`), which lowers to
/// exactly a `PureHorzBox::Frame`, so without this arm the footnote marker
/// is unreachable and its body silently never renders (the marker itself
/// still renders — line breaking never involves this pass). NOT into a
/// `Footnote`'s own block ("Ignores footnote designation in footnote",
/// pageBreak.ml:133).
fn collect_footnotes(contents: &[(Length, PureHorzBox)], out: &mut Vec<VertBox>) {
    for (_, bx) in contents {
        collect_footnotes_in_box(bx, out);
    }
}

fn collect_footnotes_in_box(bx: &PureHorzBox, out: &mut Vec<VertBox>) {
    match bx {
        PureHorzBox::Footnote { block } => out.extend(block.iter().cloned()),
        PureHorzBox::Discretionary { no_break, .. } => {
            for b in no_break {
                collect_footnotes_in_box(b, out);
            }
        }
        PureHorzBox::Tabular(tab) => {
            for cell in &tab.cells {
                for (_, cbx) in &cell.contents {
                    collect_footnotes_in_box(cbx, out);
                }
            }
        }
        PureHorzBox::EmbeddedBlock { block, .. } => {
            for vb in block {
                if let VertBox::Line { contents, .. } = vb {
                    collect_footnotes(contents, out);
                }
            }
        }
        PureHorzBox::Frame { contents, .. } => {
            for (_, cbx) in contents {
                collect_footnotes_in_box(cbx, out);
            }
        }
        _ => {}
    }
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
            VertBox::FrameStart(id) => {
                let pos = prev_baseline.unwrap_or(y0);
                lines.push(PlacedLine {
                    x: x0,
                    baseline_y: pos,
                    contents: vec![(Length::ZERO, PureHorzBox::FrameMarker { id, end: false })],
                });
            }
            VertBox::FrameEnd(id) => {
                let pos = prev_baseline.unwrap_or(y0);
                lines.push(PlacedLine {
                    x: x0,
                    baseline_y: pos,
                    contents: vec![(Length::ZERO, PureHorzBox::FrameMarker { id, end: true })],
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
