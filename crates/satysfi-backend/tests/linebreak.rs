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
