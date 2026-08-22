//! `chop_page`'s footnote accumulator (`add-footnote`): extraction at
//! line-commit, bottom-reservation, and bottom-placement, driven with
//! hand-built `VertBox`/`PureHorzBox` trees — no lang/eval machinery, since
//! the footnote job is entirely inside `chop_page`.

use rustyfi_backend::{chop_page, Length, ListMarkKind, PlacedLine, PureHorzBox, VertBox};

fn line(h: f64, d: f64, lead: f64, contents: Vec<(Length, PureHorzBox)>) -> VertBox {
    VertBox::Line {
        height: Length::pt(h),
        depth: Length::pt(d),
        leading: Length::pt(lead),
        contents,
    }
}

fn plain_line(h: f64, d: f64, lead: f64) -> VertBox {
    line(h, d, lead, vec![])
}

fn fnote(block: Vec<VertBox>) -> PureHorzBox {
    PureHorzBox::Footnote { block }
}

/// A footnote is bottom-placed so its bottom edge (`baseline + depth`)
/// sits exactly at the column's bottom edge (`y0 + height`).
#[test]
fn footnote_is_bottom_placed_at_the_column_edge() {
    let mut vboxes = vec![line(
        10.0,
        2.0,
        12.0,
        vec![(Length::ZERO, fnote(vec![plain_line(20.0, 0.0, 20.0)]))],
    )];
    let lines = chop_page((Length::ZERO, Length::ZERO), Length::pt(100.0), &mut vboxes);
    assert_eq!(lines.len(), 2, "expected the body line + one footnote line");

    let body = &lines[0];
    assert_eq!(body.baseline_y, Length::pt(10.0));

    let fn_line = &lines[1];
    assert_eq!(fn_line.x, Length::ZERO, "footnote shares the column's x origin");
    // baseline = (100 - 20) + 20 = 100, so bottom edge = y0 + height.
    assert_eq!(fn_line.baseline_y, Length::pt(100.0));
    assert!(vboxes.is_empty());
}

/// Reservation shrinks the number of body lines that fit. 10 identical
/// lines (h=10, d=2, leading=12): baselines `10+12*(k-1)`, bottoms
/// baseline+2. Unreserved, 8 fit in 100pt (line 8's bottom = 10+12*7+2 = 96
/// <= 100; line 9's = 108 > 100). Line 1 carries a 20pt footnote stack,
/// shrinking the limit to 80pt: line 6's bottom = 72 <= 80, line 7's = 84 >
/// 80. So 6 body lines + the footnote stack, leaving 4 behind.
#[test]
fn footnote_reservation_shrinks_the_column() {
    let mut vboxes: Vec<VertBox> = (0..10)
        .map(|i| {
            if i == 0 {
                line(
                    10.0,
                    2.0,
                    12.0,
                    vec![(Length::ZERO, fnote(vec![plain_line(20.0, 0.0, 20.0)]))],
                )
            } else {
                plain_line(10.0, 2.0, 12.0)
            }
        })
        .collect();
    let lines = chop_page((Length::ZERO, Length::ZERO), Length::pt(100.0), &mut vboxes);

    assert_eq!(lines.len(), 7, "expected 6 body lines + 1 footnote line");
    for body in &lines[..6] {
        assert!(
            body.contents.is_empty() || body.baseline_y <= Length::pt(62.0),
            "sanity: all placed lines should be within the shrunk column"
        );
    }
    let fn_line = lines.last().unwrap();
    assert_eq!(fn_line.baseline_y, Length::pt(100.0));

    assert_eq!(vboxes.len(), 4, "4 lines should roll over to the next page");
}

/// A footnote attached to a line that itself doesn't fit rolls over
/// WITH its footnote — the reservation must not apply until the line is
/// actually committed.
#[test]
fn footnote_rolls_over_with_its_uncommitted_line() {
    // 10 identical lines (h=10, d=2, leading=12); line 7 (index 6) carries
    // the footnote. Unreserved, 8 fit (line 8's bottom = 96 <= 100). The
    // reservation applies to line 7's OWN overflow check too, so that check
    // uses the shrunk 80pt limit: line 7's bottom = 10+12*6+2 = 84 > 80, and
    // it rolls to the next page taking the footnote with it. Page 1 ends at
    // line 6, where 8 would have fit without the footnote.
    let mut vboxes: Vec<VertBox> = (0..10)
        .map(|i| {
            if i == 6 {
                line(
                    10.0,
                    2.0,
                    12.0,
                    vec![(Length::ZERO, fnote(vec![plain_line(20.0, 0.0, 20.0)]))],
                )
            } else {
                plain_line(10.0, 2.0, 12.0)
            }
        })
        .collect();

    let page1 = chop_page((Length::ZERO, Length::ZERO), Length::pt(100.0), &mut vboxes);
    assert_eq!(page1.len(), 6, "page 1 should hold 6 lines, none of them footnote lines");
    // The tallest body baseline placed is 70pt; a bottom-placed footnote
    // stack would sit at 100pt in this geometry.
    assert!(
        page1.iter().all(|l| l.baseline_y <= Length::pt(70.0)),
        "no footnote line should have been placed on page 1"
    );
    assert_eq!(vboxes.len(), 4, "lines 7..10 roll to page 2");

    let page2 = chop_page((Length::ZERO, Length::ZERO), Length::pt(100.0), &mut vboxes);
    // 4 body lines + 1 footnote line.
    assert_eq!(page2.len(), 5);
    assert!(vboxes.is_empty());
}

/// A footnote's own block is not scanned for nested footnotes ("Ignores
/// footnote designation in footnote", pageBreak.ml:133).
#[test]
fn footnote_in_footnote_is_ignored() {
    let inner = fnote(vec![plain_line(5.0, 0.0, 5.0)]);
    let outer_block = vec![line(20.0, 0.0, 20.0, vec![(Length::ZERO, inner)])];
    let mut vboxes = vec![line(
        10.0,
        2.0,
        12.0,
        vec![(Length::ZERO, fnote(outer_block))],
    )];
    let lines = chop_page((Length::ZERO, Length::ZERO), Length::pt(100.0), &mut vboxes);
    // Body line + one footnote line; the inner marker never becomes a third.
    assert_eq!(lines.len(), 2);
    assert!(vboxes.is_empty());
}

/// `place_block_at` (headers/footers) never extracts footnotes — a
/// `Footnote` box riding a header/footer line is inert, contributing no
/// extra lines.
#[test]
fn place_block_at_does_not_extract_footnotes() {
    let block = vec![line(
        10.0,
        2.0,
        12.0,
        vec![(Length::ZERO, fnote(vec![plain_line(20.0, 0.0, 20.0)]))],
    )];
    let lines = rustyfi_backend::place_block_at((Length::ZERO, Length::ZERO), block);
    assert_eq!(lines.len(), 1, "no extra bottom lines for the footnote");
}

/// Progress guarantee — a degenerate zero-height column still places
/// its first real line (and that line's footnote), fully draining vboxes.
#[test]
fn progress_guarantee_holds_with_a_footnote_on_the_first_line() {
    let mut vboxes = vec![line(
        10.0,
        2.0,
        12.0,
        vec![(Length::ZERO, fnote(vec![plain_line(20.0, 0.0, 20.0)]))],
    )];
    let lines = chop_page((Length::ZERO, Length::ZERO), Length::ZERO, &mut vboxes);
    assert_eq!(lines.len(), 2, "body line + footnote line both placed");
    assert!(vboxes.is_empty(), "the loop must still make progress");
}

/// Multi-column shape sanity — two consecutive `chop_page` calls over
/// one shared `&mut Vec`, at shifted x-origins, each place their own
/// footnotes at their own column's x.
#[test]
fn footnotes_place_per_column_at_their_shifted_x_origin() {
    let mut vboxes = vec![
        line(
            10.0,
            2.0,
            12.0,
            vec![(Length::ZERO, fnote(vec![plain_line(20.0, 0.0, 20.0)]))],
        ),
        line(
            10.0,
            2.0,
            12.0,
            vec![(Length::ZERO, fnote(vec![plain_line(20.0, 0.0, 20.0)]))],
        ),
    ];
    // Column 1 takes the first line + footnote; leave enough content for
    // column 2 by capping the height so only one line fits per call.
    let col1 = chop_page((Length::ZERO, Length::ZERO), Length::pt(30.0), &mut vboxes);
    let col2 = chop_page((Length::pt(250.0), Length::ZERO), Length::pt(30.0), &mut vboxes);

    let col1_fn: Vec<&PlacedLine> = col1.iter().filter(|l| l.baseline_y > Length::pt(11.0)).collect();
    let col2_fn: Vec<&PlacedLine> = col2.iter().filter(|l| l.baseline_y > Length::pt(11.0)).collect();
    assert_eq!(col1_fn.len(), 1);
    assert_eq!(col2_fn.len(), 1);
    assert_eq!(col1_fn[0].x, Length::ZERO);
    assert_eq!(col2_fn[0].x, Length::pt(250.0));
    assert!(vboxes.is_empty());
}

/// A `VertBox::ListMark` is a PURE skip in `chop_page` — unlike
/// `FrameStart`/`FrameEnd`, it produces NO
/// `PlacedLine` at all (not even a zero-height marker one), is drained
/// from `vboxes` just like every other consumed box, and contributes
/// nothing to where the surrounding real lines land. This is the
/// mechanical half of the "PDF/faithful HTML never see these markers"
/// proof — `page_break_core` (rustyfi-lang) clones `reflow_source` BEFORE
/// this function runs, so the marker survives there while still vanishing
/// here.
#[test]
fn list_mark_is_a_pure_skip_that_produces_no_placed_line() {
    let mut vboxes = vec![
        VertBox::ListMark(ListMarkKind::ListStart { ordered: false }),
        VertBox::ListMark(ListMarkKind::ItemStart),
        plain_line(10.0, 2.0, 12.0),
        VertBox::ListMark(ListMarkKind::ItemEnd),
        VertBox::ListMark(ListMarkKind::ListEnd),
    ];
    let lines = chop_page((Length::ZERO, Length::ZERO), Length::pt(100.0), &mut vboxes);
    assert_eq!(lines.len(), 1, "the four ListMarks must produce ZERO PlacedLines");
    assert!(vboxes.is_empty(), "chop_page must still drain every marker");
    let baseline_with_marks = lines[0].baseline_y;

    // Byte-identity check: the SAME real line, with the surrounding
    // ListMarks stripped out entirely, must land at the EXACT same
    // baseline — the markers contribute no leading/skip/height either.
    let mut without_marks = vec![plain_line(10.0, 2.0, 12.0)];
    let lines_without = chop_page((Length::ZERO, Length::ZERO), Length::pt(100.0), &mut without_marks);
    assert_eq!(lines_without.len(), 1);
    assert_eq!(
        baseline_with_marks, lines_without[0].baseline_y,
        "ListMark markers must not shift the real line's placement"
    );
}

/// A margin (`Skip`/`ParagTop`) before the first line of a page is glue and is
/// dropped; a `block-frame-breakable`'s `FramePad` is interior CONTENT of the
/// frame and is not. Upstream adds `pads.paddingT` to the running column
/// height inside its frame arm (`pageBreak.ml:323` `hgttotal +%
/// pads.paddingT`), a path `squash_margins` — which only ever sees a frame's
/// MARGINS — never touches.
///
/// Measured: stdjabook's `+make-title` is a `block-frame-breakable ctx (20pt,
/// 20pt, 10pt, 10pt)` and always opens page 1, so dropping its `paddingT` put
/// the enumitem manual's title (and every block below it on that page) exactly
/// 10pt above real SATySFi 0.0.11's. With the pad kept, the title's first word
/// lands within 0.001pt of upstream's.
#[test]
fn page_top_discards_margins_but_keeps_frame_padding() {
    // Margin-only prefix: dropped, so the line sits at `y0 + height`.
    let mut margin_only = vec![
        VertBox::ParagTop(Length::pt(18.0)),
        plain_line(10.0, 2.0, 18.0),
    ];
    let origin = (Length::ZERO, Length::pt(100.0));
    let lines = chop_page(origin, Length::pt(400.0), &mut margin_only);
    assert_eq!(lines[0].baseline_y, Length::pt(110.0));

    // Same prefix plus a frame's 10pt top padding: the pad is kept, so the
    // line sits at `y0 + paddingT + height`.
    let mut with_pad = vec![
        VertBox::ParagTop(Length::pt(18.0)),
        VertBox::FramePad(Length::pt(10.0)),
        plain_line(10.0, 2.0, 18.0),
    ];
    let lines = chop_page(origin, Length::pt(400.0), &mut with_pad);
    assert_eq!(
        lines[0].baseline_y,
        Length::pt(120.0),
        "a frame's paddingT is frame content, not page-top glue"
    );
}

/// …and a frame still OPEN when the page ends gets that `paddingT` AGAIN at
/// the top of the continuation page. Upstream re-enters its frame arm for the
/// `Midway` fragment and adds the pad unconditionally (`pageBreak.ml:322`,
/// `hgttotal_before = hgttotal +% pads.paddingT`), keeping the full `pads`
/// rather than zeroing them (`:352-357`); `handlePdf.ml:325-330` lays every
/// fragment out at `ypos -% paddingT`.
///
/// Measured against real SATySFi 0.0.11 on `layout-tests/measure/fixtures/
/// p06-frame-across-page.saty` (one frame, 13pt top pad, body spanning a page
/// break): the continuation page's first baseline was 99.31 against upstream's
/// 112.31 — exactly 13.00pt too high — and is now 112.31.
#[test]
fn a_frame_open_across_a_page_break_re_pads_the_continuation_page() {
    use rustyfi_backend::DecoId;
    let id = DecoId(0);
    // Frame with a 10pt top pad whose three lines cannot all fit one page.
    let mut vboxes = vec![
        VertBox::FrameStart(id),
        VertBox::FramePad(Length::pt(10.0)),
        plain_line(8.0, 2.0, 20.0),
        plain_line(8.0, 2.0, 20.0),
        plain_line(8.0, 2.0, 20.0),
        VertBox::FramePad(Length::pt(4.0)),
        VertBox::FrameEnd(id),
    ];
    let origin = (Length::ZERO, Length::pt(100.0));
    // 40pt of column: pad(10) + height(8) = 118 for the first line, +20 leading
    // for the second = 138, which still fits `y0 + 40 = 140` with depth 2; the
    // third would not.
    let page1 = chop_page(origin, Length::pt(40.0), &mut vboxes);
    let body: Vec<&PlacedLine> = page1.iter().filter(|l| l.contents.is_empty()).collect();
    assert_eq!(body.len(), 2, "two of the frame's three lines fit");
    assert_eq!(body[0].baseline_y, Length::pt(118.0));

    // The remainder now leads with the re-applied pad, so the continuation's
    // first line sits at `y0 + paddingT + height` — NOT flush at `y0 + height`.
    assert_eq!(vboxes.first(), Some(&VertBox::FramePad(Length::pt(10.0))));
    let page2 = chop_page(origin, Length::pt(400.0), &mut vboxes);
    let body2: Vec<&PlacedLine> = page2.iter().filter(|l| l.contents.is_empty()).collect();
    assert_eq!(
        body2[0].baseline_y,
        Length::pt(118.0),
        "a continuation fragment re-applies the frame's own paddingT"
    );
}

/// The same, one page further on: a frame spanning THREE pages re-pads each
/// continuation. `chop_page` is stateless, so it has to recognise the pad it
/// spliced in itself — it does, by counting the frames its input leaves
/// unclosed (`unmatched_frame_ends`), which is what keeps this working past
/// the first continuation.
#[test]
fn a_frame_spanning_three_pages_re_pads_every_continuation() {
    use rustyfi_backend::DecoId;
    let id = DecoId(0);
    let mut vboxes = vec![VertBox::FrameStart(id), VertBox::FramePad(Length::pt(10.0))];
    for _ in 0..6 {
        vboxes.push(plain_line(8.0, 2.0, 20.0));
    }
    vboxes.push(VertBox::FramePad(Length::pt(4.0)));
    vboxes.push(VertBox::FrameEnd(id));

    let origin = (Length::ZERO, Length::pt(100.0));
    for page in 0..3 {
        let lines = chop_page(origin, Length::pt(40.0), &mut vboxes);
        // Body lines carry no contents here; the frame's own Start/End markers
        // do (a zero-width `FrameMarker`), so they are what gets filtered out.
        let first = lines
            .iter()
            .find(|l| l.contents.is_empty())
            .expect("each page places lines");
        assert_eq!(
            first.baseline_y,
            Length::pt(118.0),
            "page {page} of a 3-page frame must start below the re-applied pad"
        );
    }
}

// The inter-block advance's `min_first_line_ascender` handling.
//
// These two encode the faithful upstream rule (the pad belongs in the
// paragraph's own `margin_top`, BEFORE the collapse). On its own it LOOKED
// like a regression: removing the ~5pt-per-stdjabook-section-heading surplus
// made the port run FURTHER AHEAD of upstream's pagination, dropping enumitem's
// text_match 0.8891 -> 0.8608, because a separate space DEFICIT in that
// document was being masked by exactly this over-spacing.
//
// That deficit was `chop_page` dropping a `block-frame-breakable`'s `paddingT`
// on every CONTINUATION page (upstream re-applies it per fragment,
// `pageBreak.ml:322`), which cost the enumitem manual 5pt on each page
// continuing a `+code` block and 10pt on each page continuing its own
// `+block-frame` — 13 of 27 pages. With both changes in,
// enumitem is back to 0.8865 and slydifi (0.8511 -> 0.8748), easytable
// (0.8683 -> 0.8751) and figbox (0.8805 -> 0.8940, and page-count PARITY at
// last) all improve.

/// SATySFi's page accumulator adds, per box, exactly `hgt + (-dpt)` for a
/// line and `vskip` for a skip (`pageBreak.ml:132-137`, `:201-203`), so the
/// baseline-to-baseline advance across a block boundary is EXACTLY
/// `prev_depth + collapsed_margin + height`. No floor of any kind is applied
/// here — in particular not `min_first_line_ascender`.
///
/// `min_first_line_ascender` (9pt, `primitives.cppo.ml:516`) is real, but it
/// applies ONE LAYER UP and to a DIFFERENT quantity: `lineBreak.ml:855-857`
/// folds it into the paragraph's own `margin_top`
/// (`paragraph_margin_top + max(0, min_first_ascender - hgt)`), and only THEN
/// does `pageBreak.ml`'s `squash_margins` (`:596-601`) max-collapse that
/// padded top against the previous block's bottom margin. Padding the HEIGHT
/// here instead — i.e. AFTER the collapse — is a different function whenever
/// the previous block's bottom margin wins that collapse: the pad then rides
/// ON TOP of a margin that upstream had already absorbed it into.
///
/// Measured against real SATySFi 0.0.11 on a two-block probe
/// (`set-paragraph-margin 2pt 20pt`, then `set-paragraph-margin 2pt 2pt` over
/// a 4pt line whose height is 2.7466pt): upstream advances 22.747pt, this port
/// advanced 29.0pt — `9 - 2.7466 = 6.253pt` too much, exactly the surplus this
/// test pins.
#[test]
fn inter_block_advance_is_prev_depth_plus_margin_plus_height() {
    // Two blocks, as `line-break` emits them. The second block's `ParagTop`
    // ALREADY carries the `min_first_line_ascender` pad folded in
    // (2pt margin + max(0, 9 - 3) = 8pt) — which is what `lineBreak.ml` hands
    // to `squash_margins`.
    let mut vboxes = vec![
        VertBox::ParagTop(Length::pt(2.0)),
        plain_line(10.0, 1.0, 18.0),
        VertBox::Skip(Length::pt(20.0)),
        VertBox::ParagTop(Length::pt(8.0)),
        plain_line(3.0, 1.0, 18.0),
        VertBox::Skip(Length::pt(2.0)),
    ];
    let lines = chop_page((Length::ZERO, Length::ZERO), Length::pt(400.0), &mut vboxes);
    assert_eq!(lines.len(), 2);
    // max(20, 8) = 20 -> advance = prev_depth(1) + 20 + height(3) = 24.
    // The pre-fix code computed 1 + 20 + max(3, 9) = 30.
    assert_eq!(
        lines[1].baseline_y - lines[0].baseline_y,
        Length::pt(24.0),
        "inter-block advance must be prev_depth + collapsed_margin + height, \
         with the ascender pad already inside the margin — not re-applied to \
         the height after the collapse"
    );
}

/// The companion case, where the padded top margin WINS the collapse: the
/// same short line, but after a block whose bottom margin is small. Here the
/// pad really does show up in the gap — `max(2, 8) = 8` — and the old and the
/// new rule agree, which is why the surplus above went unnoticed.
#[test]
fn inter_block_advance_keeps_the_pad_when_the_padded_top_margin_wins() {
    let mut vboxes = vec![
        VertBox::ParagTop(Length::pt(2.0)),
        plain_line(10.0, 1.0, 18.0),
        VertBox::Skip(Length::pt(2.0)),
        VertBox::ParagTop(Length::pt(8.0)),
        plain_line(3.0, 1.0, 18.0),
        VertBox::Skip(Length::pt(2.0)),
    ];
    let lines = chop_page((Length::ZERO, Length::ZERO), Length::pt(400.0), &mut vboxes);
    assert_eq!(lines.len(), 2);
    // max(2, 8) = 8 -> advance = 1 + 8 + 3 = 12: the short line still gets its
    // 9pt ascender slot above the previous line's depth.
    assert_eq!(lines[1].baseline_y - lines[0].baseline_y, Length::pt(12.0));
}
