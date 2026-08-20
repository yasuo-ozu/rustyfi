//! Page breaking. Slice 1: a single **stateless** per-page chopper
//! (`chop_page`) plus a fixed-origin placement helper
//! (`place_block_at`), replacing the old whole-document `break_pages` —
//! the per-page loop now lives lang-side
//! (`rustyfi-lang/src/primitives.rs`'s `prim_page_break`), the one place
//! that legally holds `&mut Interp` to apply the two scheme closures per
//! page (see that plan's "who drives it" section).

use crate::hbox::PureHorzBox;
use crate::length::Length;
use crate::vbox::VertBox;

/// SATySFi's `min_first_line_ascender` (default, `primitives.ml:546`): a
/// paragraph's top margin is padded so its first line has at least this much
/// ascender slot above the baseline (`lineBreak.ml:857`). Applied to every
/// inter-block advance.
const MIN_FIRST_ASCENDER: Length = Length::pt(9.0);

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
    /// How many leading entries of `lines` are BODY (column) content. The
    /// page's header and footer are appended after the columns
    /// (`primitives.rs`'s `page_break_core`), so they occupy `lines[body_lines..]`.
    ///
    /// `fire_hooks` needs the split: a `block-frame-breakable` carried across a
    /// page boundary is "open" for the whole of the next page's walk, so
    /// accumulating its fragment extent over EVERY line swallowed the header
    /// and footer too — every carried frame's `decoH` fragment came out
    /// spanning y=30 (the header baseline) to y=788 (the footer), painting its
    /// background over the entire page instead of over its own content.
    ///
    /// `usize::MAX` means "all lines are body" (no header/footer), which is
    /// what hand-built test pages want.
    pub body_lines: usize,
}

/// The line's own `(height-above-baseline, depth-below-baseline)` as-placed,
/// or `None` if `line` carries no real content (only zero-width markers —
/// `HookPageBreak`/`FrameMarker`, or nothing but glue). Used by
/// `fire_hooks`'s block-fragment extent accumulation (§D) to derive a frame
/// fragment's rect from the real lines between its `FrameStart`/`FrameEnd`
/// markers, since `PlacedLine` itself doesn't carry the height/depth
/// `chop_page` used when placing it — recomputed here from each box's own
/// dimensions, the same per-box shape `linebreak.rs`'s `justify_line` uses.
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
            // inert, zero contribution — same treatment as
            // `HookPageBreak`/`FrameMarker` above.
            PureHorzBox::InlineMark(_) => {}
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
/// loop is bounded by the vbox count (Risks: "Progress / termination").
///
/// `VertBox::ClearPage` (`clear-page`) ends the page immediately once at
/// least one real line has been placed (mirrors `pageBreak.ml`'s
/// `PBClearPage`); a `clear-page` with nothing yet placed is redundant and
/// swallowed instead (mirrors `pageBreak.ml`'s `omit_redundant_clear`),
/// so a leading `clear-page` doesn't produce a pointless blank page.
///
/// `VertBox::HookPageBreak` (`hook-page-break-block`) is placed as a
/// zero-height marker `PlacedLine` carrying the hook's `PureHorzBox`
/// wrapper at the position it sits in the flow — `fire_hooks` (rustyfi-lang)
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
    // The previous committed line's depth. The baseline-to-baseline advance is
    // `leading.max(prev_depth + height)` (SATySFi's vertical stacking, `Types`/
    // `PageBreak.solidify`): the leading (line-skip) governs normal text — where
    // `leading` (≈18pt) always exceeds `prev_depth + height` (≈2.5 + 9) so this
    // is a no-op — but a DEEP box (e.g. a multi-row figbox `vconcat` whose
    // top-anchored `EmbeddedBlock` has a large depth) forces the next baseline
    // down by its full depth, so its vertical extent is actually spent on the
    // page rather than being overlapped by the following line (which made the
    // pager under-count pages around tall figures).
    let mut prev_depth = Length::ZERO;
    let mut pending_skip = Length::ZERO;
    // Additive frame padding (`FramePad`) — stacks ON TOP of `pending_skip`
    // rather than max-collapsing with it (SATySFi frame padding is additive).
    let mut pending_pad = Length::ZERO;
    // Whether the pending margin includes a paragraph TOP (`ParagTop`), which
    // is the only kind SATySFi pads up to `min_first_line_ascender`.
    let mut pending_parag = false;
    let mut placed_real_line = false;
    // Distinct from `placed_real_line`: true once ANY line box has been placed,
    // including a zero-extent one (a slydifi frame's `bb-gr` background line).
    // `clear-page` uses THIS (not `placed_real_line`) to decide a page is
    // non-empty, so a frame whose body is empty/all-graphics (section/title
    // slides) still ends its page — while `placed_real_line` stays gated on
    // real height so an overflowing frame BODY isn't forced to roll off a page
    // whose only prior line was the zero-extent background (keeps slides atomic).
    let mut placed_any_line = false;
    let mut idx = 0;

    while idx < vboxes.len() {
        match &vboxes[idx] {
            VertBox::Skip(l) => {
                // Adjacent vertical skips COLLAPSE to their maximum, not sum:
                // these are paragraph/block margins (`line-break` wraps each
                // block in a Skip(paragraph_top) … Skip(paragraph_bottom), so
                // between two blocks the previous block's bottom margin meets
                // the next block's top margin). SATySFi's block-box margin
                // model combines adjacent margins by max (like CSS/TeX margin
                // collapsing); summing them double-counted every block boundary
                // (~+25pt each), the dominant source of the port's
                // over-pagination vs the original SATySFi.
                pending_skip = pending_skip.max(*l);
                idx += 1;
            }
            VertBox::ParagTop(l) => {
                // Same max-collapse as `Skip`, but flags the boundary as a
                // paragraph top so the following line gets the
                // `min_first_line_ascender` pad (SATySFi pads `margin_top` only).
                pending_skip = pending_skip.max(*l);
                pending_parag = true;
                idx += 1;
            }
            VertBox::FramePad(l) => {
                // Additive: frame padding stacks on top of the margin.
                pending_pad += *l;
                idx += 1;
            }
            VertBox::ClearPage => {
                idx += 1;
                if placed_any_line {
                    break; // ends the page right here; the marker is consumed
                }
                // Redundant: nothing placed on this page yet — swallow it.
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
            // (the byte-identity argument): a PURE skip — unlike
            // `FrameStart`/`FrameEnd` above, this marker has no downstream
            // (PDF/faithful-HTML) consumer at all, so it doesn't even get a
            // placeholder `PlacedLine`; it simply never reaches placement.
            // It already rode into `reflow_source` (cloned by
            // `page_break_core` BEFORE this function runs, `primitives.rs`'s
            // `page_break_core` doc comment) — that clone is the only place
            // `VertBox::ListMark` survives to be read, by the reflow HTML
            // walker.
            VertBox::ListMark(_) => {
                idx += 1;
            }
            VertBox::Line {
                height: h,
                depth,
                leading,
                contents,
            } => {
                // FIX 3's page-top glue suppression: any `VertBox::Skip`
                // accumulated before the FIRST real line of this page/column
                // (`prev_baseline == None`) is discarded, not added to `y0`
                // — mirroring upstream's page-top glue discard (glue at the
                // very top of a page/column doesn't accumulate; the OCaml
                // page breaker drops leading glue the same way TeX drops
                // glue/kerns at the top of a page). This is what keeps a
                // paragraph's `paragraph_top` margin from adding a spurious
                // gap above the first paragraph of a page — without it,
                // wiring `paragraph_top` in would shift every page's content
                // down by 18pt. `HookPageBreak`/`FrameStart`/`FrameEnd`
                // markers above don't consume `pending_skip` and don't set
                // `placed_real_line`, so a marker-then-skip prefix (e.g. a
                // `block-frame-breakable`'s `FrameStart` immediately
                // followed by its `pad_t` skip) is covered by this same
                // branch too — `prev_baseline` is still `None`.
                let baseline = match prev_baseline {
                    None => y0 + *h,
                    // When the pending margin EXCEEDS the leading it is a
                    // positioning skip (e.g. slydifi's ~125pt bg-graphic
                    // offset that seats a frame body at its true top), not an
                    // inter-line gap. SATySFi's stacking rule folds the skip
                    // into the advance (`max(leading, prev_depth+skip+height)`)
                    // rather than stacking leading on top of it; for a large
                    // skip that means `prev_depth + skip + height`, so the line
                    // sits at the skip's target instead of a spurious
                    // `leading - height` lower. Small inter-paragraph margins
                    // (skip <= leading) keep the additive model the flowing
                    // corpus docs are calibrated to.
                    // SATySFi's inter-block advance is `prev_depth + margin +
                    // height` — the NATURAL content height plus the (collapsed)
                    // paragraph margin, with NO leading floor. Verified by a
                    // controlled `set-paragraph-margin` sweep against real
                    // SATySFi 0.0.11: gap == 12pt (content) + margin, exactly
                    // linear (X=0→12, 5→17, 10→22, 20→32). The leading grid
                    // (≈18pt) governs only lines WITHIN one paragraph, where
                    // `pending_skip == 0`; between blocks a small margin gives a
                    // gap BELOW the leading (e.g. an itemize item-gap of 10 →
                    // 22pt, not 28). Applying the leading floor to block gaps was
                    // the bug that over-spaced every list/table by ~6pt/row.
                    // SATySFi pads the paragraph's top margin up to
                    // `min_first_line_ascender` (default 9pt, primitives.ml:546):
                    // `margin_top = paragraph_margin_top + max(0, 9pt - hgt)`
                    // (lineBreak.ml:857). Folded into the advance this is
                    // `prev_depth + margin + max(hgt, 9pt)` — a short first line
                    // (body ascender ≈8.5pt, or a tiny caption/rule line) still
                    // gets at least a 9pt ascender slot above its baseline. The
                    // matching `min_last_descender` field is dead in 0.0.11 (it
                    // is assigned but never read), so only the TOP is padded.
                    Some(b) if pending_skip + pending_pad > Length::ZERO => {
                        let asc = if pending_parag { (*h).max(MIN_FIRST_ASCENDER) } else { *h };
                        b + prev_depth + pending_skip + pending_pad + asc
                    }
                    Some(b) => b + leading.max(prev_depth + *h),
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
                pending_pad = Length::ZERO;
                pending_parag = false;
                prev_baseline = Some(baseline);
                prev_depth = *depth;
                placed_any_line = true;
                // Only a line with real vertical extent gates the overflow-roll
                // (a zero-extent frame-background line must not force the body
                // that follows to roll — see `placed_any_line`).
                if *h + *depth > Length::ZERO {
                    placed_real_line = true;
                }
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
    let mut prev_depth = Length::ZERO;
    let mut pending_skip = Length::ZERO;
    let mut pending_pad = Length::ZERO;
    let mut bottom = Length::ZERO;
    for vb in vboxes {
        match vb {
            // Collapse adjacent margins (max, not sum) and advance by the same
            // faithful `prev_depth + margin + height` rule `place_block_at` uses,
            // so the reserved extent equals the drawn extent.
            VertBox::Skip(l) | VertBox::ParagTop(l) => pending_skip = pending_skip.max(*l),
            VertBox::FramePad(l) => pending_pad += *l,
            VertBox::ClearPage | VertBox::HookPageBreak(_) => {}
            VertBox::FrameStart(_) | VertBox::FrameEnd(_) => {}
            VertBox::ListMark(_) => {}
            VertBox::Line {
                height,
                depth,
                leading,
                ..
            } => {
                let baseline = match prev_baseline {
                    None => pending_skip + pending_pad + *height,
                    Some(b) if pending_skip + pending_pad > Length::ZERO => {
                        b + prev_depth + pending_skip + pending_pad + *height
                    }
                    Some(b) => b + leading.max(prev_depth + *height),
                };
                pending_skip = Length::ZERO;
                pending_pad = Length::ZERO;
                prev_baseline = Some(baseline);
                prev_depth = *depth;
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
    // The previous line's depth: the next baseline must clear it, otherwise a
    // line carrying a DEEP inline box (e.g. a `+fig-center` figure — a
    // `vmargin`-wrapped `EmbeddedBlock` whose table content hangs ~150pt below
    // its baseline) would be overlapped by the following paragraph/list. Same
    // `leading.max(prev_depth + …)` rule `chop_page` uses; a no-op for ordinary
    // text (leading > depth+height) and for `line-stack` rows (whose `leading`
    // already bakes in `prev_depth`), so it only bites for genuinely deep boxes.
    let mut prev_depth = Length::ZERO;
    let mut pending_skip = Length::ZERO;
    let mut pending_pad = Length::ZERO;

    for vbox in vboxes {
        match vbox {
            // Collapse adjacent vertical margins to their max, NOT their sum —
            // the same rule `chop_page` applies (commit "collapse adjacent
            // vertical margins"). `place_block_at` both measures an embedded
            // block's height (via `make_embedded_block`) and renders it (via
            // `place_embedded_block`), so summing here made every embedded
            // block over-tall: e.g. a slydifi frame body's `+listing` items,
            // each a `line-break` emitting `Skip(item-gap)` before and after,
            // got `item-gap + item-gap` between consecutive items instead of
            // one collapsed `item-gap`, roughly doubling list spacing and
            // pushing the atomic frame past its page (an extra page per slide).
            VertBox::Skip(l) | VertBox::ParagTop(l) => pending_skip = pending_skip.max(l),
            // Additive frame padding (see `chop_page`).
            VertBox::FramePad(l) => pending_pad += l,
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
            // Same pure skip as `chop_page`'s matching arm —
            // headers/footers/footnote columns don't page-break, so a
            // list marker here (should one ever appear — no bundled
            // stdlib emits one outside the main flow) simply contributes
            // nothing.
            VertBox::ListMark(_) => {}
            VertBox::Line {
                height,
                depth,
                leading,
                contents,
            } => {
                // Same page/column-top leading-glue suppression as `chop_page`
                // (FIX 3): a solidified block (header/footer/footnote column)
                // placed at a fixed origin discards any leading `VertBox::Skip`
                // before its first real line, so a footer whose content is
                // built via `line-break` (default `paragraph_top` = 18pt, e.g.
                // `stdja-mini`'s page-number footer) stays anchored at its
                // `footer-origin` rather than dropping 18pt below it. The
                // advance folds the paragraph margin into the max AND clears
                // the previous line's depth, so a deep inline box (a
                // `+fig-center` figure) is not overlapped by the next line.
                // Faithful inter-block advance (see `chop_page`): natural
                // content height plus the collapsed margin, no leading floor —
                // `prev_depth + margin + height`. The leading grid governs only
                // margin-free lines within one paragraph (`pending_skip == 0`).
                let baseline = match prev_baseline {
                    None => y0 + height,
                    Some(b) if pending_skip + pending_pad > Length::ZERO => {
                        b + prev_depth + pending_skip + pending_pad + height
                    }
                    Some(b) => b + leading.max(prev_depth + height),
                };
                pending_skip = Length::ZERO;
                pending_pad = Length::ZERO;
                prev_baseline = Some(baseline);
                prev_depth = depth;
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
