//! Page breaking: a **stateless** per-page chopper (`chop_page`) plus a
//! fixed-origin placement helper (`place_block_at`). The per-page loop lives
//! lang-side (`primitives.rs`'s `prim_page_break`), the one place that legally
//! holds `&mut Interp` to apply the two scheme closures per page.

use crate::hbox::PureHorzBox;
use crate::length::Length;
use crate::vbox::VertBox;

/// SATySFi's `min_first_line_ascender` (default, `primitives.cppo.ml:516`): a
/// paragraph's top margin is padded so its first line has at least this much
/// ascender slot above the baseline —
/// `margin_top = paragraph_margin_top + max(0, 9pt - hgt)`,
/// `lineBreak.ml:855-857`. The fold happens where upstream does it, in
/// `line-break` (`primitives.rs`'s `prim_line_break`), BEFORE the margin
/// collapse, so a larger preceding bottom margin absorbs the pad.
pub const MIN_FIRST_ASCENDER: Length = Length::pt(9.0);

/// One typeset line placed on a page, in page coordinates (y grows downward
/// from the paper top; the PDF writer flips it).
#[derive(Clone, Debug, PartialEq, syan::visit::Ast)]
#[subast(crate::hbox::PureHorzBox)]
pub struct PlacedLine {
    pub x: Length,
    pub baseline_y: Length,
    pub contents: Vec<(Length, PureHorzBox)>,
}

#[derive(Clone, Debug, Default, PartialEq, syan::visit::Ast)]
#[subast(crate::pagebreak::PlacedLine)]
pub struct Page {
    pub lines: Vec<PlacedLine>,
    /// How many leading entries of `lines` are BODY (column) content. The
    /// page's header and footer are appended after the columns
    /// (`primitives.rs`'s `page_break_core`), so they occupy `lines[body_lines..]`.
    ///
    /// `fire_hooks` needs the split: a `block-frame-breakable` carried across a
    /// page boundary is "open" for the whole of the next page's walk, so
    /// accumulating its fragment extent over EVERY line swallows the header and
    /// footer too — the `decoH` fragment then spans y=30 (header baseline) to
    /// y=788 (footer), painting its background over the whole page.
    ///
    /// `usize::MAX` means "all lines are body" (no header/footer), which is
    /// what hand-built test pages want.
    pub body_lines: usize,
}

/// The line's own `(height-above-baseline, depth-below-baseline)` as-placed,
/// or `None` if `line` carries no real content (only zero-width markers, or
/// nothing but glue). Used by `fire_hooks`'s block-fragment extent
/// accumulation; `PlacedLine` does not carry the height/depth `chop_page`
/// placed it with, so it is recomputed here from each box's own dimensions.
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
            // Folds its padded extent in (that is the only way an inline
            // breakable frame's `paddingT`/`paddingB` reaches this walk — its
            // contents are spliced siblings, already visited above) but never
            // sets `has_real`: a line holding nothing but the bracket has no
            // content to decorate, and claiming otherwise would give a carried
            // block frame a fragment over an empty line.
            PureHorzBox::InlineFrameMarker { height: h, depth: d, .. } => {
                *height = (*height).max(*h);
                *depth = (*depth).max(*d);
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
/// `vboxes` for the caller's next page.
///
/// **Termination guarantee**: a page always places at least one *real*
/// line (regardless of `height`) — the overflow check only fires once one
/// has been placed, tracked by `placed_real_line` (not `lines.is_empty()`,
/// since a `HookPageBreak` marker can occupy a `PlacedLine` slot without
/// being real content) — so a degenerate scheme (`height <= 0`, or a line
/// taller than the area) still makes forward progress and the lang-side
/// loop is bounded by the vbox count.
///
/// `VertBox::ClearPage` (`clear-page`) ends the page immediately once at
/// least one real line has been placed (mirrors `pageBreak.ml`'s
/// `PBClearPage`); a `clear-page` with nothing yet placed is redundant and
/// swallowed instead (mirrors `pageBreak.ml`'s `omit_redundant_clear`),
/// so a leading `clear-page` doesn't produce a pointless blank page.
///
/// `VertBox::HookPageBreak` (`hook-page-break-block`) is placed as a
/// zero-height marker `PlacedLine` at the position it sits in the flow;
/// `fire_hooks` scans every placed line's contents for it regardless of
/// whether it came from the inline or the block-level hook primitive.
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
    // down by its full depth, so its extent is actually spent on the page
    // instead of being overlapped by the following line.
    let mut prev_depth = Length::ZERO;
    let mut pending_skip = Length::ZERO;
    // Additive frame padding (`FramePad`) — stacks ON TOP of `pending_skip`
    // rather than max-collapsing with it (SATySFi frame padding is additive).
    let mut pending_pad = Length::ZERO;
    let mut placed_real_line = false;
    // Distinct from `placed_real_line`: true once ANY line box has been placed,
    // including a zero-extent one (a slydifi frame's `bb-gr` background line).
    // `clear-page` uses THIS (not `placed_real_line`) to decide a page is
    // non-empty, so a frame whose body is empty/all-graphics (section/title
    // slides) still ends its page — while `placed_real_line` stays gated on
    // real height so an overflowing frame BODY isn't forced to roll off a page
    // whose only prior line was the zero-extent background (keeps slides atomic).
    let mut placed_any_line = false;
    // Top pads of the `block-frame-breakable` frames open across this page's
    // boundaries, so the continuation page can RE-APPLY them (the splice after
    // the loop). Split in two, because a frame can straddle any number of pages
    // while its `FrameStart` marker was consumed on some earlier one:
    //
    // * `carried_pads` — frames already open when this page began. Their count
    //   is `unmatched_frame_ends` (a `FrameEnd` in this list whose `FrameStart`
    //   is not), and the previous page spliced exactly that many `FramePad`s at
    //   the front, outermost first — so they can be read straight off, with no
    //   state carried between `chop_page` calls.
    // * `open_pads` — the `FramePad(pad_t)` that `prim_block_frame_breakable`
    //   emits immediately after each `FrameStart` consumed on THIS page.
    //
    // Frames are well nested, so a `FrameEnd` always closes the innermost entry
    // (`open_pads` last, else `carried_pads` last).
    let mut carried_pads: Vec<Length> = vboxes
        .iter()
        .take(unmatched_frame_ends(vboxes))
        .filter_map(|vb| match vb {
            VertBox::FramePad(l) => Some(*l),
            _ => None,
        })
        .collect();
    let mut open_pads: Vec<Option<Length>> = Vec::new();
    let mut idx = 0;

    while idx < vboxes.len() {
        match &vboxes[idx] {
            VertBox::Skip(l) => {
                // Adjacent vertical skips COLLAPSE to their maximum, not sum:
                // these are paragraph/block margins, and SATySFi's block-box
                // model combines adjacent margins by max (like CSS/TeX margin
                // collapsing). Summing them double-counted every block boundary
                // (~+25pt each), the dominant source of the port's
                // over-pagination vs the original SATySFi.
                pending_skip = pending_skip.max(*l);
                idx += 1;
            }
            VertBox::ParagTop(l) => {
                // Same max-collapse as `Skip`. The `min_first_line_ascender`
                // pad is already INSIDE this value (`prim_line_break` folds it
                // in where upstream does, `lineBreak.ml:855-857`, before the
                // collapse); the variant stays distinct only because the HTML
                // reflow walker reads it.
                pending_skip = pending_skip.max(*l);
                idx += 1;
            }
            VertBox::FramePad(l) => {
                // Additive: frame padding stacks on top of the margin.
                pending_pad += *l;
                // Remember it as the innermost open frame's TOP pad: the first
                // `FramePad` after a `FrameStart` is that frame's `pad_t` (the
                // second is its `pad_b`, by which time the slot is filled).
                // Carried frames' pads were read off the front of `vboxes`
                // above, so nothing to do for them here.
                if let Some(slot @ None) = open_pads.last_mut() {
                    *slot = Some(*l);
                }
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
                open_pads.push(None);
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
                // Closes the innermost frame still open (well-nested by
                // construction): one opened on this page if there is one, else
                // the innermost one carried in from an earlier page.
                if open_pads.pop().is_none() {
                    carried_pads.pop();
                }
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
            // A PURE skip — unlike `FrameStart`/`FrameEnd` above, this marker
            // has no downstream (PDF/faithful-HTML) consumer at all, so it
            // doesn't even get a placeholder `PlacedLine`; it simply never
            // reaches placement. It already rode into `reflow_source` (cloned
            // by `page_break_core` BEFORE this function runs) — that clone is
            // the only place `VertBox::ListMark` survives to be read, by the
            // reflow HTML walker.
            VertBox::ListMark(_) => {
                idx += 1;
            }
            VertBox::Line {
                height: h,
                depth,
                leading,
                contents,
            } => {
                // Page-top glue suppression: any `VertBox::Skip`
                // accumulated before the FIRST real line of this page/column
                // (`prev_baseline == None`) is discarded, not added to `y0`
                // — mirroring upstream's page-top glue discard, the same way
                // TeX drops glue/kerns at the top of a page. This keeps a
                // paragraph's `paragraph_top` margin from adding a spurious
                // gap above the first paragraph of a page — without it,
                // wiring `paragraph_top` in would shift every page's content
                // down by 18pt. `HookPageBreak`/`FrameStart`/`FrameEnd`
                // markers above don't consume `pending_skip` and don't set
                // `placed_real_line`, so a marker-then-skip prefix (e.g. a
                // `block-frame-breakable`'s `FrameStart` immediately
                // followed by its `pad_t` skip) is covered by this same
                // branch too — `prev_baseline` is still `None`.
                //
                // FRAME PADDING IS NOT GLUE, so it SURVIVES the page top. A
                // `block-frame-breakable`'s `paddingT` is interior content of
                // the frame: upstream adds it to the running column height in
                // the frame arm of the chopper (`pageBreak.ml:323`
                // `hgttotal +% pads.paddingT`) and never routes it through
                // `squash_margins`, which only ever sees the frame's MARGINS.
                // Discarding it here seated the first line of a page-opening
                // frame at the frame's own top edge: stdjabook's `+make-title`
                // (`block-frame-breakable ctx (20pt, 20pt, 10pt, 10pt)`) put
                // its title 10pt — exactly `paddingT` — above upstream's, and
                // with it every following block on that page. `measure_block`
                // already counted the pad at the top
                // (`pending_skip + pending_pad + height`), so this also makes
                // the extent it RESERVES match the extent actually drawn.
                let baseline = match prev_baseline {
                    None => y0 + pending_pad + *h,
                    // A pending margin goes straight into the advance, with NO
                    // leading floor: SATySFi's inter-block advance is
                    // `prev_depth + margin + height` — the NATURAL content
                    // height plus the (collapsed) paragraph margin. Verified by
                    // a controlled `set-paragraph-margin` sweep against real
                    // SATySFi 0.0.11: gap == 12pt (content) + margin, exactly
                    // linear (X=0→12, 5→17, 10→22, 20→32). The leading grid
                    // (≈18pt) governs only lines WITHIN one paragraph, where
                    // `pending_skip == 0`; between blocks a small margin gives a
                    // gap BELOW the leading (e.g. an itemize item-gap of 10 →
                    // 22pt, not 28). Applying the leading floor to block gaps was
                    // the bug that over-spaced every list/table by ~6pt/row —
                    // and for a large POSITIONING skip (slydifi's ~125pt
                    // bg-graphic offset, which seats a frame body at its true
                    // top) it would leave the line a spurious
                    // `leading - height` short of its target.
                    // NO `min_first_line_ascender` floor is applied HERE.
                    // Upstream folds that pad into the paragraph's own
                    // `margin_top` in `line-break`
                    // (`margin_top = paragraph_margin_top + max(0, 9pt - hgt)`,
                    // `lineBreak.ml:855-857`) and only THEN max-collapses it
                    // against the previous block's bottom margin
                    // (`pageBreak.ml`'s `squash_margins`, `:596-601`), so a
                    // larger predecessor ABSORBS the pad. `prim_line_break`
                    // does the same, which is why the arm below is a plain
                    // `prev_depth + collapsed_margin + height`: clamping the
                    // height to 9pt here, AFTER the collapse, would re-apply a
                    // pad the collapse had already swallowed — 5pt too much at
                    // every stdjabook section heading.
                    Some(b) if pending_skip + pending_pad > Length::ZERO => {
                        b + prev_depth + pending_skip + pending_pad + *h
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

    // A `block-frame-breakable` still open when the page ends gets its
    // `paddingT` AGAIN at the top of the continuation page. Upstream's chopper
    // re-enters its frame arm for the `Midway` fragment and unconditionally
    // adds the pad to the running column height (`pageBreak.ml:322`,
    // `hgttotal_before = hgttotal +% pads.paddingT`), keeping the FULL `pads`
    // for a midway fragment rather than zeroing them (`:352-357`, whose own
    // comment records that zeroing "may be better" — it deliberately does not);
    // `handlePdf.ml:325-330` then lays every
    // fragment's contents out at `ypos -% pads.paddingT` and spans the deco
    // rect from `ypos`. Dropping it seated every continuation page's first line
    // flush against the text-area top: measured against SATySFi 0.0.11, the
    // enumitem manual lost 5pt on each page continuing a `+code` block (its
    // frame's `paddingT`) and 10pt on each page continuing the document's own
    // `+block-frame` — 13 of its 27 pages.
    //
    // Splicing the pads back at the FRONT of the remainder (outermost first) is
    // what makes this stateless: the next `chop_page` recognises them by
    // counting the frames its input leaves unclosed.
    if !vboxes.is_empty() {
        let reopened: Vec<VertBox> = carried_pads
            .iter()
            .copied()
            .chain(open_pads.into_iter().flatten())
            .map(VertBox::FramePad)
            .collect();
        vboxes.splice(0..0, reopened);
    }

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

/// How many `block-frame-breakable` frames are ALREADY OPEN at the head of
/// `vboxes` — i.e. how many `FrameEnd` markers it contains whose matching
/// `FrameStart` it does not, because that `FrameStart` was consumed by an
/// earlier page. Markers are well nested by construction
/// (`prim_block_frame_breakable` always emits a matched pair around its own
/// contents), so a running depth that never goes below its own minimum counts
/// them exactly.
fn unmatched_frame_ends(vboxes: &[VertBox]) -> usize {
    let mut depth: i32 = 0;
    let mut deepest: i32 = 0;
    for vb in vboxes {
        match vb {
            VertBox::FrameStart(_) => depth += 1,
            VertBox::FrameEnd(_) => {
                depth -= 1;
                deepest = deepest.min(depth);
            }
            _ => {}
        }
    }
    (-deepest) as usize
}

/// The vertical extent `place_block_at(_, vboxes)` will occupy below its
/// origin: the last line's `baseline + depth`, advancing exactly as
/// `place_block_at` does (`leading.max(height)` between lines; a trailing
/// skip after the last line is not counted, mirroring its unflushed
/// `pending_skip`). The port of `get_height_of_evaled_vert_box_list`
/// (handlePdf.ml:401) — computed with the SAME rule as placement so the bottom
/// reservation always equals the drawn extent. Upstream instead reserves
/// `Σ(h+d)` per line; the two differ only when a footnote line's
/// `leading > height + next depth`, and only by whitespace.
fn stack_height(vboxes: &[VertBox]) -> Length {
    let mut prev_baseline: Option<Length> = None;
    let mut prev_depth = Length::ZERO;
    let mut pending_skip = Length::ZERO;
    let mut pending_pad = Length::ZERO;
    let mut bottom = Length::ZERO;
    for vb in vboxes {
        match vb {
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
///
/// A `Graphics` (`draw-text`) is excluded ON PURPOSE — that one is fidelity
/// rather than a gap, and the measurement is on the arm itself.
fn collect_footnotes(contents: &[(Length, PureHorzBox)], out: &mut Vec<VertBox>) {
    for (_, bx) in contents {
        collect_footnotes_in_box(bx, out);
    }
}

/// The per-box half of [`collect_footnotes`].
///
/// The match is deliberately **wildcard-free**. A `_ => {}` arm is the exact
/// silent-omission shape [`crate::visit`] exists to abolish: a new
/// box-carrying `PureHorzBox` variant lands in it, collects nothing, and the
/// only symptom is a footnote body that never renders. Naming every variant
/// makes adding one a compile error HERE, so whoever adds it has to decide
/// whether a footnote can ride inside it.
///
/// It stays a hand-written match rather than becoming `crate::visit`'s
/// generated traversal because this walk must follow the RENDERED shape, and
/// the generated descent is unconditional in two places where that is wrong:
///
/// * a `Footnote`'s own `block` — upstream's "Ignores footnote designation
///   in footnote" (pageBreak.ml:133), already pinned by
///   `footnote_in_footnote_is_ignored`;
/// * a `Discretionary`'s `pre_break` / `post_break` slots. A discretionary
///   the paragraph breaker acted on is already gone by the time a line
///   reaches here — `linebreak::line_content` splices the chosen slot in
///   flat. One that survives into a placed line is therefore always an
///   UN-TAKEN candidate (a `fit_cell`-measured cell / `draw-text` run /
///   frame content is laid out unbroken and keeps its discretionaries
///   whole), so `no_break` is the slot that renders and the other two render
///   nothing at all. Collecting from them would bottom-place a footnote the
///   page never shows.
///
/// A `visitor!` closure cannot prune, so the alternative is a hand-written
/// `Visit` impl declining those edges — which buys exhaustiveness over the
/// whole node set at the cost of stating both exclusions as overrides of a
/// default that does the opposite.
///
/// **What the wildcard-free match does not buy, honestly:** exhaustiveness
/// over the OTHER node types this walk reaches through. The `Tabular`,
/// `EmbeddedBlock` and `Frame` arms are open-coded field accesses, so a new
/// node-carrying field on `TabularBox` / `TabularCellBox`, or a second
/// `VertBox` variant that carries a line, is still skipped in silence. Only
/// `PureHorzBox` — where every hole found in this walk so far has been — is
/// covered. `tests/pagebreak.rs`'s `footnote_*` group pins the edges that
/// exist today.
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
        // NOT recursed into, and this one is FIDELITY rather than a gap.
        //
        // A `Graphics`' `elems` can hold a `GraphicsElem::Text` (`draw-text`)
        // whose `contents` are real inline boxes, so a `\footnote` genuinely
        // can sit under one: figbox's `FigBox.frame` / `rotate` / `scale` /
        // `shift` and `\fig-inline` all wrap their content in exactly that
        // shape. Upstream does not collect it either — `embed_page_info`'s
        // `ImHorzInlineGraphics` arm (pageInfo.ml:44-45) returns the box
        // untouched, with no `iter` and no `appendF`, while every arm around
        // it (`ImHorzRising` :22-25, `ImHorzFrame` :27-30,
        // `ImHorzEmbeddedVert` :36-39) recurses. In both engines the marker
        // renders and the body does not.
        //
        // Measured, not inferred: one document setting the same `\footnote`
        // six ways (plain, `\fig-inline`+`textbox`, `\fig-center`+`textbox`,
        // `hconcat`, `frame`, `rotate`) renders character-identical under
        // this port and under upstream 0.0.11 — all six markers present and
        // identically numbered, bodies 1/3/4 rendered, 2/5/6 absent. The
        // three that survive reach the line as ordinary inline boxes or via
        // `line-stack-bottom`'s `EmbeddedBlock`, never through a `draw-text`.
        //
        // So recursing here would make the port DIVERGE from the reference it
        // is measured against (`layout-tests/fidelity.py` gates figbox). If
        // that is ever wanted, want it deliberately;
        // `footnote_under_a_draw_text_is_not_collected_matching_upstream` in
        // `tests/pagebreak.rs` goes red the moment this arm starts to.
        PureHorzBox::Graphics { .. } => {}
        // Leaf-shaped for this pass. `Math`'s `rules` are engine-built
        // fraction bars and radical signs — `GraphicsElem`s with no user
        // content, so no footnote can ride one. A `GraphicsOuter` is replaced
        // by a resolved `Graphics` before placement. The two marker pairs are
        // zero-width and carry no boxes at all: an `InlineFrameMarker`'s
        // frame contents are SPLICED into the enclosing paragraph (see that
        // variant's own note), so they are reached here as siblings of the
        // marker rather than through it.
        PureHorzBox::InnerString { .. }
        | PureHorzBox::OuterEmpty { .. }
        | PureHorzBox::OuterFil
        | PureHorzBox::FixedEmpty { .. }
        | PureHorzBox::Image { .. }
        | PureHorzBox::GraphicsOuter { .. }
        | PureHorzBox::Math { .. }
        | PureHorzBox::HookPageBreak { .. }
        | PureHorzBox::FrameMarker { .. }
        | PureHorzBox::InlineFrameMarker { .. }
        | PureHorzBox::InlineMark(_) => {}
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
            // the same rule `chop_page` applies. `place_block_at` both
            // measures an embedded block's height (via `make_embedded_block`)
            // and renders it (via `place_embedded_block`), so summing made
            // every embedded block over-tall: e.g. a slydifi frame body's
            // `+listing` items, each a `line-break` emitting `Skip(item-gap)`
            // before and after, got `item-gap + item-gap` between consecutive
            // items instead of one collapsed `item-gap`, roughly doubling list
            // spacing and pushing the atomic frame past its page (an extra
            // page per slide).
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
                // Same page/column-top leading-glue suppression as `chop_page`:
                // a solidified block (header/footer/footnote column)
                // placed at a fixed origin discards any leading `VertBox::Skip`
                // before its first real line, so a footer whose content is
                // built via `line-break` (default `paragraph_top` = 18pt, e.g.
                // `stdja-mini`'s page-number footer) stays anchored at its
                // `footer-origin` rather than dropping 18pt below it. The
                // advance itself is `chop_page`'s, unchanged.
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
