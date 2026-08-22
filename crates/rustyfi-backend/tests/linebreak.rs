use rustyfi_backend::*;

/// Every glyph is 6pt wide at size 12 (half an em).
struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, _c: char, size: Length) -> Option<Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

fn ctx(width: f64) -> Context {
    Context::initial(Length::pt(width))
}

fn word(m: &Mono, c: &Context, text: &str) -> HorzBox {
    HorzBox::Pure(PureHorzBox::InnerString {
        info: HorzStringInfo {
            font: c.font,
            size: c.font_size,
            rising: Length::ZERO,
            color: Color::Gray(0.0),
        },
        text: text.into(),
        width: m.text_width(c.font, text, c.font_size).unwrap(),
        height: m.ascender(c.font, c.font_size),
        depth: m.descender(c.font, c.font_size),
    })
}

fn space(c: &Context) -> HorzBox {
    let w = c.font_size * 0.5;
    HorzBox::Pure(PureHorzBox::OuterEmpty {
        natural: w,
        shrinkable: w * 0.25,
        stretchable: w * 0.5,
    })
}

fn fil() -> HorzBox {
    HorzBox::Pure(PureHorzBox::OuterFil)
}

fn lines_of(v: &[VertBox]) -> Vec<String> {
    v.iter()
        .map(|vb| match vb {
            VertBox::Line { contents, .. } => contents
                .iter()
                .filter_map(|(_, b)| match b {
                    PureHorzBox::InnerString { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
            VertBox::Skip(_) | VertBox::ParagTop(_) | VertBox::FramePad(_) => "<skip>".into(),
            VertBox::ClearPage => "<clear-page>".into(),
            VertBox::HookPageBreak(_) => "<hook>".into(),
            VertBox::FrameStart(_) => "<frame-start>".into(),
            VertBox::FrameEnd(_) => "<frame-end>".into(),
            VertBox::ListMark(_) => "<list-mark>".into(),
        })
        .collect()
}

/// Build `word sp word sp ... word` (no trailing glue) from plain text
/// pieces, for tests that assemble their own paragraphs.
fn words(m: &Mono, c: &Context, texts: &[&str]) -> Vec<HorzBox> {
    let mut boxes = Vec::new();
    for (i, t) in texts.iter().enumerate() {
        if i > 0 {
            boxes.push(space(c));
        }
        boxes.push(word(m, c, t));
    }
    boxes
}

// -- A test-local recompute of the documented Knuth–Plass scoring formula
// (linebreak.rs: `badness` / `demerits`), used by the tests below to prove
// a *specific* partition's total cost against another candidate's, rather
// than just trusting whatever `break_into_lines` returns.
const BADNESS_INF: f64 = 10_000.0;
const LINE_PENALTY: f64 = 10.0;

fn local_badness(natural: f64, target: f64, stretch: f64, shrink: f64, has_fil: bool) -> f64 {
    let slack = target - natural;
    if slack.abs() < 1e-9 {
        return 0.0;
    }
    if slack > 0.0 {
        if has_fil {
            return 0.0;
        }
        if stretch <= 0.0 {
            return BADNESS_INF;
        }
        (100.0 * (slack / stretch).abs().powi(3)).min(BADNESS_INF)
    } else {
        if shrink <= 0.0 {
            return BADNESS_INF;
        }
        (100.0 * (slack / shrink).abs().powi(3)).min(BADNESS_INF)
    }
}

fn local_demerits(b: f64) -> f64 {
    (LINE_PENALTY + b) * (LINE_PENALTY + b)
}

/// Total demerits of a `[(word_count, is_last)]`-shaped partition of
/// same-width-12pt-word text, given the standard `space()` glue (natural
/// 6, shrink 1.5, stretch 3) between words and no other glue.
fn partition_cost(word_counts: &[usize], target: f64) -> f64 {
    let n = word_counts.len();
    word_counts
        .iter()
        .enumerate()
        .map(|(i, &count)| {
            let count = count as f64;
            let natural = count * 12.0 + (count - 1.0) * 6.0;
            let stretch = (count - 1.0) * 3.0;
            let shrink = (count - 1.0) * 1.5;
            let is_last = i + 1 == n;
            local_demerits(local_badness(natural, target, stretch, shrink, is_last))
        })
        .sum()
}

#[test]
fn single_short_line() {
    let m = Mono;
    let c = ctx(200.0);
    let boxes = vec![word(&m, &c, "hi"), space(&c), word(&m, &c, "there"), fil()];
    let v = break_into_lines(&c, boxes);
    assert_eq!(lines_of(&v), vec!["hi there"]);
}

/// A break at glue happens when — and only when — the shorter line is
/// REPRESENTABLE. Same five 24pt words and same 6pt (+3 / -1.5) spaces both
/// times; only the column moves, by one point, across `ratio_stretch_limit`.
///
/// The intuitive expectation — `["aaaa aaaa", "aaaa aaaa", "aaaa"]` at a 60pt
/// column, because two words "fit" and a third must therefore wrap — is WRONG.
/// Upstream SATySFi does not do that, and the reason is exact rather than a
/// matter of cost: two words plus one space is 54pt natural carrying 3pt of
/// stretch, so filling a 60pt column needs adjustment ratio
/// `(60 - 54) / 3 = 2.0` EXACTLY, and `calculate_ratios`
/// classifies `ratio_raw >= ratio_stretch_limit` (= 2.0) as `LBTooShort`
/// (`lineBreak.ml:507`, `:534`) — a class for which `update_graph` adds NO
/// EDGE AT ALL (`lineBreak.ml:1014-1015`). A one-word line is `LBTooShort`
/// too (zero stretch, nonzero shortfall, `lineBreak.ml:528-531`). So the only
/// path upstream's graph has from `beginning` to `final` is via the single
/// `LBTooLong` edge a source node is permitted before it is dropped outright
/// (`is_already_too_long` / `RemovalSet`, `lineBreak.ml:1017-1027`): three
/// words on line 1, the remaining two plus the `inline-fil` on line 2.
///
/// This port lands on the same two lines by a different route. Its limits are
/// INCLUSIVE (`badness` compares `<=`, see the note there), so ratio 2.0 is
/// scored rather than deleted — but scored at `10000 * 2³ = 80_000`, which
/// makes `[2, 2, 1]` cost 160_030 against `[3, 2]`'s 140_020. Either way the
/// wrap loses; the port merely charges for the partition upstream refuses to
/// build at all.
///
/// Shrink the column by 1pt and the same two-word line has ratio `5/3 < 2`,
/// which is `LBPermissible`; the three-line break becomes a real path and the
/// cheaper one in both engines. That is the control below: this breaker DOES
/// wrap at glue, and 60pt is exactly where the `ratio_stretch_limit` cliff
/// falls for these metrics — not a packing bug.
#[test]
fn wraps_at_glue() {
    let m = Mono;
    let build = |c: &Context| {
        let mut boxes = Vec::new();
        for i in 0..5 {
            if i > 0 {
                boxes.push(space(c));
            }
            boxes.push(word(&m, c, "aaaa"));
        }
        boxes.push(fil());
        boxes
    };

    // 59pt: the two-word line stretches at ratio 5/3, inside the limit, so the
    // wrap is representable — and preferred.
    let c = ctx(59.0);
    let v = break_into_lines(&c, build(&c));
    assert_eq!(lines_of(&v), vec!["aaaa aaaa", "aaaa aaaa", "aaaa"]);

    // 60pt: ratio is exactly 2.0, `LBTooShort`, so that partition does not
    // exist for upstream and is `BADNESS_DROPPED` here. One overfull line wins.
    let c = ctx(60.0);
    let v = break_into_lines(&c, build(&c));
    assert_eq!(lines_of(&v), vec!["aaaa aaaa aaaa", "aaaa aaaa"]);
}

#[test]
fn interior_lines_justify() {
    let m = Mono;
    let c = ctx(60.0);
    let boxes = vec![
        word(&m, &c, "aaaa"),
        space(&c),
        word(&m, &c, "bbbb"),
        space(&c),
        word(&m, &c, "cccc"),
        fil(),
    ];
    let v = break_into_lines(&c, boxes);
    assert_eq!(lines_of(&v), vec!["aaaa bbbb", "cccc"]);
    // First line: the interior space stretches so `bbbb` ends at 60pt.
    let VertBox::Line { contents, .. } = &v[0] else {
        panic!()
    };
    let (x_last, PureHorzBox::InnerString { width, .. }) = &contents[2] else {
        panic!()
    };
    assert!(
        (*x_last + *width - Length::pt(60.0)).0.abs() < 1e-9,
        "line not justified: ends at {}",
        *x_last + *width
    );
}

#[test]
fn last_line_stays_ragged() {
    let m = Mono;
    let c = ctx(200.0);
    let boxes = vec![word(&m, &c, "hi"), space(&c), word(&m, &c, "yo"), fil()];
    let v = break_into_lines(&c, boxes);
    let VertBox::Line { contents, .. } = &v[0] else {
        panic!()
    };
    // The space keeps its natural 6pt: "yo" starts at 12 + 6 = 18pt.
    assert_eq!(contents[2].0, Length::pt(18.0));
}

// -- Knuth–Plass optimal line breaking --------------------------------------

/// Classic case where greedy's "pack as many words as fit" choice is
/// suboptimal: 5 identical 12pt words with standard glue, target 63pt.
/// Greedy packs 3 words onto line 1 (natural 48, ratio 2.5 stretch,
/// badness 1562.5) leaving 2 words for the ragged last line. Knuth–Plass
/// instead pulls a 4th word onto line 1 (natural 66, slightly overfull,
/// shrinking at ratio -2/3, badness ~29.6) leaving a single trailing word —
/// far lower total demerits even though line 1 is no longer underfull.
#[test]
fn kp_finds_lower_cost_split_than_greedy_packing() {
    let m = Mono;
    let c = ctx(63.0);
    let mut boxes = words(&m, &c, &["aa", "aa", "aa", "aa", "aa"]);
    boxes.push(fil());
    let v = break_into_lines(&c, boxes);

    assert_eq!(lines_of(&v), vec!["aa aa aa aa", "aa"]);

    let kp_total = partition_cost(&[4, 1], 63.0);
    let greedy_total = partition_cost(&[3, 2], 63.0);
    assert!(
        kp_total < greedy_total,
        "expected KP's split ({kp_total}) to beat greedy's ({greedy_total})"
    );
    // Sanity-check the magnitude of the win against a hand-derived value.
    assert!((kp_total - 1670.5075445816178).abs() < 1e-6);
    assert!((greedy_total - 2472856.25).abs() < 1e-6);
}

/// Even distribution: with words (12, 30, 12, 30, 12, 12, 12) at target
/// 45pt, Knuth–Plass settles on 3 lines of similarly-moderate looseness
/// (badness 800, 800, 100) rather than the much worse single-word-per-line
/// packing greedy would produce (five lines pegged at the badness cap).
#[test]
fn kp_distributes_looseness_evenly_across_three_lines() {
    let m = Mono;
    let c = ctx(45.0);
    let mut boxes = words(&m, &c, &["aa", "bbbbb", "cc", "ddddd", "ee", "ff", "gg"]);
    boxes.push(fil());
    let v = break_into_lines(&c, boxes);

    assert_eq!(lines_of(&v), vec!["aa bbbbb", "cc ddddd", "ee ff gg"]);

    // Recompute each line's badness directly (mixed word widths, so
    // `partition_cost`'s uniform-12pt assumption doesn't apply here).
    let line_natural = [12.0 + 6.0 + 30.0, 12.0 + 6.0 + 30.0, 12.0 * 3.0 + 6.0 * 2.0];
    let line_stretch = [3.0, 3.0, 2.0 * 3.0];
    let line_shrink = [1.5, 1.5, 2.0 * 1.5];
    let badnesses: Vec<f64> = (0..3)
        .map(|i| {
            local_badness(
                line_natural[i],
                45.0,
                line_stretch[i],
                line_shrink[i],
                i == 2,
            )
        })
        .collect();
    assert!((badnesses[0] - 800.0).abs() < 1e-6);
    assert!((badnesses[1] - 800.0).abs() < 1e-6);
    assert!((badnesses[2] - 100.0).abs() < 1e-6);
    // The first two interior lines are similarly loose, unlike a 2-loose +
    // 1-tight split.
    assert!((badnesses[0] - badnesses[1]).abs() < 1e-9);

    let kp_total: f64 = badnesses.iter().map(|&b| local_demerits(b)).sum();
    // Five single-word lines (what greedy would produce here) each pin the
    // badness cap.
    let greedy_total = local_demerits(BADNESS_INF) * 5.0 + local_demerits(0.0);
    assert!(kp_total < greedy_total);
}

/// A line that's only slightly too wide (natural 48pt against a 46pt
/// target, well within the 3pt of shrink available) is kept as one line
/// and compressed, rather than broken early into a badly underfull pair.
#[test]
fn kp_prefers_shrinking_over_breaking_early() {
    let m = Mono;
    let c = ctx(46.0);
    let mut boxes = words(&m, &c, &["aa", "bb", "cc"]);
    boxes.push(fil());
    let v = break_into_lines(&c, boxes);

    assert_eq!(lines_of(&v), vec!["aa bb cc"]);

    let VertBox::Line { contents, .. } = &v[0] else {
        panic!("expected a single line")
    };
    // contents: [aa, space, bb, space, cc, fil]; the two spaces shrink from
    // 6pt to 5pt each so the line ends exactly at the 46pt target.
    let (x_last, PureHorzBox::InnerString { width, .. }) = &contents[4] else {
        panic!("expected the last word at index 4")
    };
    assert!(
        (*x_last + *width - Length::pt(46.0)).0.abs() < 1e-6,
        "line not compressed to target: ends at {}",
        *x_last + *width
    );
}

/// A single word wider than the whole paragraph (with no glue to break at)
/// must still typeset as one overfull line instead of panicking or looping.
#[test]
fn kp_tolerates_an_overfull_unbreakable_word() {
    let m = Mono;
    let c = ctx(50.0);
    let boxes = vec![word(&m, &c, "abcdefghijklmnopqrst"), fil()];
    let v = break_into_lines(&c, boxes);

    assert_eq!(v.len(), 1);
    assert_eq!(lines_of(&v), vec!["abcdefghijklmnopqrst"]);
}

/// Regression: even once lines can shrink, a trailing `inline-fil` still
/// leaves the *last* line's interior glue at natural width (ragged, not
/// justified) — this time checked on the last of several lines rather than
/// a single-line paragraph.
#[test]
fn kp_last_line_with_fil_keeps_natural_spacing() {
    let m = Mono;
    let c = ctx(60.0);
    let mut boxes = words(&m, &c, &["aaaa", "bbbb", "cccc", "dddd"]);
    boxes.push(fil());
    let v = break_into_lines(&c, boxes);

    assert_eq!(lines_of(&v), vec!["aaaa bbbb", "cccc dddd"]);

    let VertBox::Line { contents, .. } = &v[1] else {
        panic!("expected a second line")
    };
    // "cccc" is 24pt wide; the interior space keeps its natural 6pt even
    // though 6pt of slack remains at the end of this (last, ragged) line.
    assert_eq!(contents[2].0, Length::pt(30.0));
}

/// `break_into_lines` is a pure function of its input: running it twice on
/// the same paragraph must produce identical output. We also document (see
/// `linebreak.rs`'s DP loop) the tie-break rule the search relies on for
/// determinism: candidate transitions are compared on `(total_demerits,
/// line_count)`, so an exact-cost tie is broken toward *fewer* lines, and
/// remaining ties keep whichever candidate the backward scan visits first.
/// A bit-for-bit demerits tie between two genuinely different partitions is
/// a measure-zero coincidence in practice (balanced splits strictly
/// dominate skewed ones once badness is convex), so rather than engineer
/// one, this test pins down determinism directly.
#[test]
fn kp_is_deterministic() {
    let m = Mono;
    let c = ctx(45.0);
    let mut boxes = words(&m, &c, &["aa", "bbbbb", "cc", "ddddd", "ee", "ff", "gg"]);
    boxes.push(fil());
    let v1 = break_into_lines(&c, boxes.clone());
    let v2 = break_into_lines(&c, boxes);
    assert_eq!(v1, v2);
}

// The page-model split — `chop_page` / `place_block_at`.

fn leaded_line(height_pt: f64, depth_pt: f64, leading_pt: f64) -> VertBox {
    VertBox::Line {
        height: Length::pt(height_pt),
        depth: Length::pt(depth_pt),
        leading: Length::pt(leading_pt),
        contents: vec![],
    }
}

#[test]
fn chop_page_splits_across_two_pages_by_height() {
    let line = leaded_line(9.0, 3.0, 18.0);
    let mut remaining = vec![line.clone(), line.clone(), line];
    // origin y=10, height=45 -> y_limit=55: baselines at 19, 37, then 55+3>55 -> stop.
    let placed = chop_page(
        (Length::pt(10.0), Length::pt(10.0)),
        Length::pt(45.0),
        &mut remaining,
    );
    assert_eq!(placed.len(), 2);
    assert_eq!(
        remaining.len(),
        1,
        "the 3rd line must roll over, unconsumed"
    );
    assert_eq!(placed[0].baseline_y, Length::pt(19.0));
    assert_eq!(placed[1].baseline_y, Length::pt(37.0));

    // A fresh `chop_page` call (a new page's origin) picks the leftover line
    // back up as that page's own first line.
    let placed2 = chop_page(
        (Length::pt(10.0), Length::pt(10.0)),
        Length::pt(45.0),
        &mut remaining,
    );
    assert_eq!(placed2.len(), 1);
    assert!(remaining.is_empty());
    assert_eq!(placed2[0].baseline_y, Length::pt(19.0));
}

/// Termination guard (progress): a degenerate content scheme with
/// `text-height <= 0` must still place >=1
/// line per non-empty page, or the lang-side per-page loop never ends.
#[test]
fn chop_page_still_makes_progress_at_zero_height() {
    let line = leaded_line(9.0, 3.0, 18.0);
    let mut remaining = vec![line.clone(), line];
    let placed = chop_page((Length::ZERO, Length::ZERO), Length::ZERO, &mut remaining);
    assert_eq!(
        placed.len(),
        1,
        "a degenerate height must still place >=1 line"
    );
    assert_eq!(remaining.len(), 1);
}

/// `chop_page` over `[FrameStart, Line, FrameEnd]` places 3
/// `PlacedLine`s — the two markers zero-width/contentless, the real line's
/// own geometry unaffected by their presence — and a marker-only page still
/// terminates (same guarantee `HookPageBreak` already has, `pagebreak.rs`'s
/// `placed_real_line`).
#[test]
fn chop_page_places_frame_markers_as_zero_width_lines_around_an_unaffected_real_line() {
    let line = leaded_line(9.0, 3.0, 18.0);
    let mut vboxes = vec![
        VertBox::FrameStart(DecoId(0)),
        line,
        VertBox::FrameEnd(DecoId(0)),
    ];
    let placed = chop_page((Length::pt(10.0), Length::pt(10.0)), Length::pt(1000.0), &mut vboxes);
    assert_eq!(placed.len(), 3);
    assert!(vboxes.is_empty());

    assert_eq!(placed[0].contents.len(), 1);
    assert_eq!(placed[0].contents[0].0, Length::ZERO);
    assert_eq!(
        placed[0].contents[0].1,
        PureHorzBox::FrameMarker { id: DecoId(0), end: false }
    );

    // The real line's own geometry is unaffected by the markers around it —
    // same baseline it would get with no markers at all (`y0 + height`,
    // since it's the first REAL line placed).
    assert_eq!(placed[1].baseline_y, Length::pt(19.0));
    assert_eq!(placed[1].contents.len(), 0);

    assert_eq!(placed[2].contents.len(), 1);
    assert_eq!(
        placed[2].contents[0].1,
        PureHorzBox::FrameMarker { id: DecoId(0), end: true }
    );
}

// A graphics box: a `PureHorzBox::Graphics` measures like `Image` did
// for width, but — unlike `Image` — carries a real depth, and is never a
// legal line-break point.

fn graphics_box(width: f64, height: f64, depth: f64) -> PureHorzBox {
    PureHorzBox::Graphics {
        width: Length::pt(width),
        height: Length::pt(height),
        depth: Length::pt(depth),
        elems: vec![],
        origin_independent: false,
    }
}

#[test]
fn graphics_box_natural_width_and_is_not_glue() {
    let gbox = graphics_box(20.0, 20.0, 2.0);
    assert_eq!(gbox.natural_width(), Length::pt(20.0));
    assert!(!gbox.is_glue());
}

#[test]
fn graphics_box_contributes_height_and_depth_to_its_line() {
    let c = ctx(100.0);
    let line = vec![HorzBox::Pure(graphics_box(20.0, 20.0, 2.0))];
    let lines = break_into_lines(&c, line);
    assert_eq!(lines.len(), 1);
    match &lines[0] {
        VertBox::Line {
            height,
            depth,
            contents,
            ..
        } => {
            assert_eq!(*height, Length::pt(20.0));
            assert_eq!(*depth, Length::pt(2.0));
            assert_eq!(contents.len(), 1);
            assert_eq!(contents[0].0, Length::ZERO);
            assert_eq!(contents[0].1.natural_width(), Length::pt(20.0));
        }
        _ => panic!("expected a Line, got something else"),
    }
}

// A math box: a `PureHorzBox::Math` carries its own outer
// width/height/depth (computed once by `read_math`, rustyfi-lang) so the
// line breaker never re-enters the math engine — it just measures like
// `Graphics` (real height *and* depth), and is never a legal line-break
// point.

fn math_box(width: f64, height: f64, depth: f64) -> PureHorzBox {
    PureHorzBox::Math {
        width: Length::pt(width),
        height: Length::pt(height),
        depth: Length::pt(depth),
        glyphs: vec![],
        rules: vec![],
    }
}

#[test]
fn math_box_natural_width_and_is_not_glue() {
    let mbox = math_box(30.0, 12.0, 3.0);
    assert_eq!(mbox.natural_width(), Length::pt(30.0));
    assert!(!mbox.is_glue());
    assert!(!mbox.is_break_point());
}

#[test]
fn math_box_contributes_height_and_depth_to_its_line() {
    let c = ctx(100.0);
    let line = vec![HorzBox::Pure(math_box(30.0, 12.0, 3.0))];
    let lines = break_into_lines(&c, line);
    assert_eq!(lines.len(), 1);
    match &lines[0] {
        VertBox::Line {
            height,
            depth,
            contents,
            ..
        } => {
            assert_eq!(*height, Length::pt(12.0));
            assert_eq!(*depth, Length::pt(3.0));
            assert_eq!(contents.len(), 1);
            assert_eq!(contents[0].0, Length::ZERO);
            assert_eq!(contents[0].1.natural_width(), Length::pt(30.0));
        }
        _ => panic!("expected a Line, got something else"),
    }
}

// An inline frame: a `PureHorzBox::Frame`'s outer width/height/depth
// (padding already folded in by `make_inline_frame`) drive line metrics
// exactly like `Graphics`/ `Math` above, and it's never a legal line-break
// point (the atomic model: contents are pre-fit, the frame never splits).

fn frame_box(width: f64, height: f64, depth: f64) -> PureHorzBox {
    PureHorzBox::Frame {
        width: Length::pt(width),
        height: Length::pt(height),
        depth: Length::pt(depth),
        deco: DecoId(0),
        contents: vec![(
            Length::ZERO,
            PureHorzBox::InnerString {
                info: HorzStringInfo { font: FontKey(0), size: Length::pt(12.0), rising: Length::ZERO, color: Color::Gray(0.0) },
                text: "x".into(),
                width: Length::pt(width),
                height: Length::pt(height),
                depth: Length::pt(depth),
            },
        )],
    }
}

#[test]
fn frame_box_natural_width_is_the_outer_width_and_is_not_glue() {
    let fbox = frame_box(24.0, 9.0, 3.0);
    assert_eq!(fbox.natural_width(), Length::pt(24.0));
    assert!(!fbox.is_glue());
    assert!(!fbox.is_break_point());
}

#[test]
fn frame_box_grows_line_height_and_depth_by_its_own_padded_metrics() {
    let c = ctx(100.0);
    let line = vec![HorzBox::Pure(frame_box(24.0, 9.0, 3.0))];
    let lines = break_into_lines(&c, line);
    assert_eq!(lines.len(), 1);
    match &lines[0] {
        VertBox::Line { height, depth, contents, .. } => {
            assert_eq!(*height, Length::pt(9.0));
            assert_eq!(*depth, Length::pt(3.0));
            assert_eq!(contents.len(), 1);
            assert_eq!(contents[0].0, Length::ZERO);
            assert_eq!(contents[0].1.natural_width(), Length::pt(24.0));
        }
        _ => panic!("expected a Line, got something else"),
    }
}

// Page-break hooks — a `PureHorzBox::HookPageBreak` is a
// zero-width/height/depth marker, never a legal line-break point;
// `break_pages`/the PDF writers place it like any other content but render
// nothing for it (the lang-side `fire_hooks` post-pass is the only thing
// that ever reads its `HookId`).

fn hook_box(id: usize) -> PureHorzBox {
    PureHorzBox::HookPageBreak { id: HookId(id) }
}

#[test]
fn hook_box_measures_zero_and_is_never_glue_or_a_break_point() {
    let hbox = hook_box(0);
    assert_eq!(hbox.natural_width(), Length::ZERO);
    assert!(!hbox.is_glue());
    assert!(!hbox.is_break_point());
}

#[test]
fn hook_box_contributes_nothing_to_its_line_but_is_still_placed() {
    let c = ctx(100.0);
    // A hook alone (plus a fil so the line isn't empty-underfull-of-nothing)
    // must still lay out as an ordinary (if degenerate) line, with the hook
    // box itself present in `contents` at offset zero so a lang-side
    // post-pass can find it.
    let line = vec![HorzBox::Pure(hook_box(0)), fil()];
    let lines = break_into_lines(&c, line);
    assert_eq!(lines.len(), 1);
    match &lines[0] {
        VertBox::Line { contents, .. } => {
            assert_eq!(contents.len(), 2);
            assert_eq!(contents[0].0, Length::ZERO);
            assert_eq!(contents[0].1, hook_box(0));
        }
        _ => panic!("expected a Line, got something else"),
    }
}
