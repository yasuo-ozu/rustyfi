use satysfi_backend::*;

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
            VertBox::Skip(_) => "<skip>".into(),
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

#[test]
fn wraps_at_glue() {
    let m = Mono;
    // 60pt wide: each word "aaaa" is 24pt, space 6pt → two words + space =
    // 54pt fits, third word forces a break.
    let c = ctx(60.0);
    let mut boxes = Vec::new();
    for i in 0..5 {
        if i > 0 {
            boxes.push(space(&c));
        }
        boxes.push(word(&m, &c, "aaaa"));
    }
    boxes.push(fil());
    let v = break_into_lines(&c, boxes);
    assert_eq!(lines_of(&v), vec!["aaaa aaaa", "aaaa aaaa", "aaaa"]);
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

// -- Phase 6: Knuth–Plass optimal line breaking -----------------------------

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

#[test]
fn page_break_overflows_to_next_page() {
    let geom = PageGeometry {
        paper_width: Length::pt(100.0),
        paper_height: Length::pt(100.0),
        text_origin: (Length::pt(10.0), Length::pt(10.0)),
        text_width: Length::pt(80.0),
        text_height: Length::pt(45.0),
    };
    let line = VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(3.0),
        contents: vec![],
    };
    // Leading 18pt, limit y=55: baselines at 19, 37, then 55+3 > 55 → next page.
    let pages = break_pages(&geom, Length::pt(18.0), vec![line.clone(), line.clone(), line]);
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].lines.len(), 2);
    assert_eq!(pages[1].lines.len(), 1);
    assert_eq!(pages[0].lines[0].baseline_y, Length::pt(19.0));
    assert_eq!(pages[1].lines[0].baseline_y, Length::pt(19.0));
}

#[test]
#[ignore]
fn perf_thousand_words() {
    let m = Mono;
    let c = ctx(400.0);
    let mut texts: Vec<String> = Vec::new();
    for i in 0..1000 {
        texts.push(format!("w{}", i % 7));
    }
    let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    let mut boxes = words(&m, &c, &text_refs);
    boxes.push(fil());
    let start = std::time::Instant::now();
    let v = break_into_lines(&c, boxes);
    let elapsed = start.elapsed();
    eprintln!("1000-word paragraph: {:?}, {} lines", elapsed, v.len());
}
