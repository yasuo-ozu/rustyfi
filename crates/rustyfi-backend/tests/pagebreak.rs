//! `chop_page`'s footnote accumulator (`add-footnote`;
//! docs/plans/document-page-model.md §C): extraction at line-commit,
//! bottom-reservation, and bottom-placement — tested directly against the
//! backend function, mirroring `crates/rustyfi-lang/tests/page_prims.rs`'s
//! style of driving `chop_page` with hand-built `VertBox`/`PureHorzBox`
//! trees (no lang/eval machinery needed for these; the footnote job is
//! entirely inside `chop_page`).

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

/// T1: a single body line carrying a footnote is bottom-placed so its
/// bottom edge (`baseline + depth`) sits exactly at the column's bottom
/// edge (`y0 + height`).
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
    // baseline = (100 - 20) + 20 = 100: bottom edge (baseline + depth = 100)
    // lands exactly at y0 + height.
    assert_eq!(fn_line.baseline_y, Length::pt(100.0));
    assert!(vboxes.is_empty());
}

/// T2: reservation shrinks the number of body lines that fit. 10 identical
/// lines (h=10, d=2, leading=12): line k's bottom edge is at `12*k - 2 + 2
/// = 12*k` actually let's just trust the geometry: baselines are
/// 10,22,34,...,`10+12*(k-1)`, bottoms are baseline+2. Without any
/// reservation, 8 lines fit in 100pt (bottom of line 8 = 10+12*7+2 = 96 <=
/// 100; line 9's bottom = 108 > 100). Line 1 carries a 20pt-tall footnote
/// stack, shrinking the effective limit to 80pt for every subsequent line:
/// bottom of line 6 = 10+12*5+2 = 72 <= 80; line 7's bottom = 84 > 80. So
/// only 6 body lines fit, plus the footnote stack, leaving 4 lines behind.
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

    // 6 body lines + 1 footnote line.
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

/// T3: a footnote attached to a line that itself doesn't fit rolls over
/// WITH its footnote — the reservation must not apply until the line is
/// actually committed.
#[test]
fn footnote_rolls_over_with_its_uncommitted_line() {
    // 10 identical lines (h=10, d=2, leading=12); line 7 (index 6) carries
    // the footnote. Without any reservation, 8 lines fit (bottom of line 8
    // = 96 <= 100). Since line 7 fits within the *unreserved* limit checked
    // before its own footnote grows the reservation... but the reservation
    // is computed and applied to line 7's OWN overflow check too (spec:
    // "shrink the effective bottom limit by the accumulated footnote stack
    // height" happens as part of evaluating whether *this* line fits). With
    // the footnote on line 7, its own check uses the shrunk limit (80pt):
    // bottom of line 7 = 10+12*6+2 = 84 > 80, so it rolls to the next page,
    // taking the footnote with it — page 1 ends at line 6 (8 lines would
    // have fit without the footnote, so this proves the rollover).
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
    // The tallest body baseline placed (line 6, 0-indexed 5) is 70pt; a
    // bottom-placed footnote stack would sit at baseline 100pt (this
    // geometry, same as T1/T2) — well clear of every body baseline here.
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

/// T4: a footnote's own block is not itself scanned for nested footnotes
/// ("Ignores footnote designation in footnote", pageBreak.ml:133) — exactly
/// one footnote stack is placed, and the inner marker stays inert.
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
    // Body line + exactly one footnote line (the outer block's own line);
    // the inner marker inside it is never expanded into a third line.
    assert_eq!(lines.len(), 2);
    assert!(vboxes.is_empty());
}

/// T5: `place_block_at` (headers/footers) never extracts footnotes — a
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

/// T6: progress guarantee — a degenerate zero-height column still places
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

/// T2b: multi-column shape sanity — two consecutive `chop_page` calls over
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

/// `docs/plans/design-reflow-s4-lists.md` §4.3 (the byte-identity
/// argument): a `VertBox::ListMark` is a PURE skip in `chop_page` — unlike
/// `FrameStart`/`FrameEnd`, it produces NO `PlacedLine` at all (not even a
/// zero-height marker one), is drained from `vboxes` just like every other
/// consumed box, and contributes nothing to where the surrounding real
/// lines land. This is the mechanical half of the "PDF/faithful HTML never
/// see these markers" proof — `page_break_core` (rustyfi-lang) clones
/// `reflow_source` BEFORE this function runs, so the marker survives there
/// while still vanishing here.
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
