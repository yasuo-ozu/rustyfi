//! `fire_hooks`' inline-frame (`fire_inline_frame`) and block-frame-fragment
//! firing, against hand-built `DocumentValue`s — no parser, no
//! `page-break` involved. Each deco closure is interned directly into
//! `interp.decos`, exactly what `make_inline_frame`/`prim_block_frame_
//! breakable` do at a higher level.

use rustyfi_backend::{
    Color, DecoId, FontKey, FontMetrics, GraphicsElem, Length, Paddings, Page, PageGeometry,
    PlacedLine, PureHorzBox,
};
use rustyfi_lang::ast::Ast;
use rustyfi_lang::eval::{self, DecoEntry};
use rustyfi_lang::primitives;
use rustyfi_lang::value::{DocumentValue, Value};
use rustyfi_syntax::{RustyfiVersion, Span};
use std::rc::Rc;

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

fn var(name: &str) -> Ast {
    Ast::Var(name.to_string(), Span::default())
}

fn apply_all(name: &str, args: Vec<Ast>) -> Ast {
    args.into_iter()
        .fold(var(name), |f, a| Ast::Apply(Box::new(f), Box::new(a)))
}

fn str_lit(s: &str) -> Ast {
    Ast::Str(s.to_string())
}

fn border_none() -> Ast {
    Ast::Ctor("None".to_string(), None)
}

fn lambda4(body: Ast) -> Ast {
    Ast::Lambda(
        "pt".to_string(),
        Rc::new(Ast::Lambda(
            "w".to_string(),
            Rc::new(Ast::Lambda(
                "h".to_string(),
                Rc::new(Ast::Lambda("d".to_string(), Rc::new(body))),
            )),
        )),
    )
}

/// A deco `fun pt w h d -> [fill (Gray 0.0) …]` — ignores its arguments,
/// always returns a single-element graphics list, so `page_graphics` gains
/// exactly one `GraphicsElem::Fill`.
fn fill_deco() -> Ast {
    let path = apply_all(
        "close-with-line",
        vec![apply_all(
            "line-to",
            vec![
                Ast::Tuple(vec![
                    Ast::Length(Length::pt(10.0)),
                    Ast::Length(Length::pt(0.0)),
                ]),
                apply_all(
                    "start-path",
                    vec![Ast::Tuple(vec![
                        Ast::Length(Length::pt(0.0)),
                        Ast::Length(Length::pt(0.0)),
                    ])],
                ),
            ],
        )],
    );
    let fill = apply_all(
        "fill",
        vec![
            Ast::Ctor("Gray".to_string(), Some(Box::new(Ast::Float(0.0)))),
            path,
        ],
    );
    lambda4(Ast::List(vec![fill]))
}

/// A deco `fun pt w h d -> let () = register-link-to-uri uri pt w h d None
/// in []` — forwards its OWN curried args straight through to
/// `register-link-to-uri`, so the recorded `Annot`'s rect directly reveals
/// what `apply_deco` was called with.
fn link_deco(uri: &str) -> Ast {
    let call = apply_all(
        "register-link-to-uri",
        vec![
            str_lit(uri),
            var("pt"),
            var("w"),
            var("h"),
            var("d"),
            border_none(),
        ],
    );
    lambda4(Ast::Sequential(
        Box::new(call),
        Box::new(Ast::List(Vec::new())),
    ))
}

fn eval_to_value(interp: &mut eval::Interp, ast: &Ast) -> Value {
    let env = primitives::base_env();
    interp
        .eval(&env, ast)
        .expect("deco AST must evaluate to a closure")
}

fn geometry() -> PageGeometry {
    PageGeometry {
        paper_width: Length::pt(400.0),
        paper_height: Length::pt(300.0),
        text_origin: (Length::pt(0.0), Length::pt(0.0)),
        text_width: Length::pt(400.0),
        text_height: Length::pt(300.0),
    }
}

fn doc_with_pages(pages: Vec<Page>) -> DocumentValue {
    DocumentValue {
        geometry: geometry(),
        pages,
        images: Vec::new(),
        extras: Default::default(),
        reflow_source: None,
        reflow_links: Vec::new(),
        reflow_dests: Vec::new(),
    }
}

// Inline frames (`fire_inline_frame`)

#[test]
fn a_frame_with_a_fill_deco_puts_one_element_in_page_graphics() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let deco_v = eval_to_value(&mut interp, &fill_deco());
    interp.decos.push(DecoEntry::Inline { deco: deco_v, version: RustyfiVersion::V0_0 });

    let frame = PureHorzBox::Frame {
        width: Length::pt(30.0),
        height: Length::pt(10.0),
        depth: Length::pt(2.0),
        deco: DecoId(0),
        contents: Vec::new(),
    };
    let page = Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::ZERO, frame)],
        }],
    };
    let doc = doc_with_pages(vec![page]);

    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");
    assert_eq!(interp.page_graphics.len(), 1);
    assert_eq!(interp.page_graphics[0].len(), 1);
    assert!(matches!(
        interp.page_graphics[0][0],
        GraphicsElem::Fill(Color::Gray(_), _)
    ));
}

#[test]
fn a_frame_deco_calling_register_link_to_uri_lands_an_annot_with_the_frames_rect() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let deco_v = eval_to_value(&mut interp, &link_deco("https://example.com/"));
    interp.decos.push(DecoEntry::Inline { deco: deco_v, version: RustyfiVersion::V0_0 });

    let frame = PureHorzBox::Frame {
        width: Length::pt(30.0),
        height: Length::pt(10.0),
        depth: Length::pt(2.0),
        deco: DecoId(0),
        contents: Vec::new(),
    };
    let page = Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::pt(4.0), frame)],
        }],
    };
    let doc = doc_with_pages(vec![page]);

    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");
    assert_eq!(interp.annotations.len(), 1);
    let a = &interp.annotations[0];
    assert_eq!(a.page, 0);
    // x = line.x + dx = 50 + 4 = 54; y = paper_height - baseline_y = 200.
    let x = Length::pt(54.0);
    let y = Length::pt(200.0);
    assert_eq!(
        a.rect,
        (
            x,
            y - Length::pt(2.0),
            x + Length::pt(30.0),
            y + Length::pt(10.0)
        ),
        "rect = (x, y - depth, x + width, y + height), from the deco's OWN curried args"
    );
}

#[test]
fn a_nested_frame_fires_with_its_parents_x_plus_its_own_dx() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let outer_deco = eval_to_value(&mut interp, &link_deco("outer"));
    interp.decos.push(DecoEntry::Inline { deco: outer_deco, version: RustyfiVersion::V0_0 });
    let inner_deco = eval_to_value(&mut interp, &link_deco("inner"));
    interp.decos.push(DecoEntry::Inline { deco: inner_deco, version: RustyfiVersion::V0_0 });

    let inner_frame = PureHorzBox::Frame {
        width: Length::pt(20.0),
        height: Length::pt(8.0),
        depth: Length::pt(1.0),
        deco: DecoId(1),
        contents: Vec::new(),
    };
    let outer_frame = PureHorzBox::Frame {
        width: Length::pt(50.0),
        height: Length::pt(10.0),
        depth: Length::pt(2.0),
        deco: DecoId(0),
        contents: vec![(Length::pt(5.0), inner_frame)],
    };
    let page = Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(100.0),
            baseline_y: Length::pt(50.0),
            contents: vec![(Length::ZERO, outer_frame)],
        }],
    };
    let doc = doc_with_pages(vec![page]);

    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");
    assert_eq!(
        interp.annotations.len(),
        2,
        "outer fires before recursing into inner"
    );
    assert_eq!(
        interp.annotations[0].rect.0,
        Length::pt(100.0),
        "outer: x = line.x + 0"
    );
    assert_eq!(
        interp.annotations[1].rect.0,
        Length::pt(105.0),
        "inner: x = outer's x (100) + its own dx (5)"
    );
}

#[test]
fn a_frame_inside_a_tabular_cell_fires_on_the_cells_own_baseline() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let deco_v = eval_to_value(&mut interp, &link_deco("https://example.com/in-cell"));
    interp.decos.push(DecoEntry::Inline {
        deco: deco_v,
        version: RustyfiVersion::V0_0,
    });

    // A cell's boxes never reach the page flow, so before this recursion a
    // `\href` in an easytable cell produced no `/Link` annotation at all.
    let tabular = PureHorzBox::Tabular(rustyfi_backend::TabularBox {
        width: Length::pt(100.0),
        height: Length::pt(40.0),
        depth: Length::ZERO,
        rules: Vec::new(),
        cells: vec![rustyfi_backend::TabularCellBox {
            x: Length::pt(7.0),
            baseline_y: Length::pt(25.0),
            contents: vec![(
                Length::pt(3.0),
                PureHorzBox::Frame {
                    width: Length::pt(30.0),
                    height: Length::pt(10.0),
                    depth: Length::pt(2.0),
                    deco: DecoId(0),
                    contents: Vec::new(),
                },
            )],
        }],
    });
    let page = Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::pt(2.0), tabular)],
        }],
    };
    let doc = doc_with_pages(vec![page]);
    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");

    assert_eq!(interp.annotations.len(), 1, "the in-cell frame must fire");
    let rect = interp.annotations[0].rect;
    assert_eq!(
        rect.0,
        Length::pt(62.0),
        "x = line.x (50) + tabular dx (2) + cell.x (7) + box dx (3)"
    );
    assert_eq!(
        rect.3,
        Length::pt(235.0),
        "y = paper_height (300) - (line.baseline_y (100) - cell.baseline_y (25)) \
         + h (10): `cell.baseline_y` is measured y-UP from the tabular box's \
         baseline, matching the PDF writer's `ty + cell.baseline_y`"
    );
}

/// A `\href` inside a `draw-text` run — figbox's `textbox` wraps whole
/// tables in one, which is how slydifi's theme table reaches its `\link`s.
/// Element coordinates are box-local PDF y-UP from the box's placed anchor.
#[test]
fn a_frame_inside_a_draw_text_run_fires_at_the_runs_own_point() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let deco_v = eval_to_value(&mut interp, &link_deco("https://example.com/in-graphics"));
    interp.decos.push(DecoEntry::Inline {
        deco: deco_v,
        version: RustyfiVersion::V0_0,
    });

    let text_run = GraphicsElem::Text {
        pt: (Length::pt(6.0), Length::pt(9.0)),
        width: Length::pt(30.0),
        height: Length::pt(10.0),
        depth: Length::pt(2.0),
        transform: None,
        contents: vec![(
            Length::pt(4.0),
            PureHorzBox::Frame {
                width: Length::pt(30.0),
                height: Length::pt(10.0),
                depth: Length::pt(2.0),
                deco: DecoId(0),
                contents: Vec::new(),
            },
        )],
    };
    let gfx = PureHorzBox::Graphics {
        width: Length::pt(40.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        elems: vec![GraphicsElem::Group(vec![text_run])],
        origin_independent: false,
    };
    let page = Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::pt(2.0), gfx)],
        }],
    };
    let doc = doc_with_pages(vec![page]);
    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");

    assert_eq!(interp.annotations.len(), 1, "the in-graphics frame must fire");
    let rect = interp.annotations[0].rect;
    assert_eq!(
        rect.0,
        Length::pt(62.0),
        "x = line.x (50) + graphics dx (2) + run pt.x (6) + box dx (4)"
    );
    assert_eq!(
        rect.3,
        Length::pt(219.0),
        "y = paper_height (300) - line.baseline_y (100) + run pt.y (9) + h (10)"
    );
}

// Inline BREAKABLE frame fragments (`InlineFrameMarker` pairs)

/// Intern a four-closure deco set whose members are distinguishable: each
/// `register-link-to-uri`s a different URI (`"S"`/`"H"`/`"M"`/`"T"`) and
/// forwards its own `pt`/`w`/`h`/`d`, so the resulting `Annot` sequence says
/// both WHICH fragment closures fired, in order, and with what rect.
fn intern_labelled_decoset(interp: &mut eval::Interp) -> DecoId {
    let decoset = ["S", "H", "M", "T"].map(|tag| {
        let ast = link_deco(tag);
        eval_to_value(interp, &ast)
    });
    let id = DecoId(interp.decos.len());
    interp.decos.push(DecoEntry::InlineBreakable {
        pads: Paddings {
            l: Length::ZERO,
            r: Length::ZERO,
            t: Length::ZERO,
            b: Length::ZERO,
        },
        decoset,
        version: RustyfiVersion::V0_0,
    });
    id
}

fn inline_marker(id: DecoId, end: bool) -> PureHorzBox {
    PureHorzBox::InlineFrameMarker {
        id,
        end,
        height: Length::pt(10.0),
        depth: Length::pt(2.0),
    }
}

fn fired(interp: &eval::Interp) -> Vec<(String, (f64, f64, f64, f64))> {
    interp
        .annotations
        .iter()
        .map(|a| {
            let uri = match &a.action {
                rustyfi_backend::AnnotAction::Uri(u) => u.clone(),
                other => panic!("expected a Uri action, got {other:?}"),
            };
            (
                uri,
                (a.rect.0 .0, a.rect.1 .0, a.rect.2 .0, a.rect.3 .0),
            )
        })
        .collect()
}

#[test]
fn a_breakable_frame_that_fits_one_line_fires_only_deco_s() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let id = intern_labelled_decoset(&mut interp);

    let page = Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![
                (Length::ZERO, inline_marker(id, false)),
                (
                    Length::ZERO,
                    PureHorzBox::FixedEmpty {
                        width: Length::pt(30.0),
                    },
                ),
                (Length::pt(30.0), inline_marker(id, true)),
            ],
        }],
    };
    let doc = doc_with_pages(vec![page]);
    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");

    assert_eq!(
        fired(&interp),
        vec![("S".to_string(), (50.0, 198.0, 80.0, 210.0))],
        "an unbroken frame fires decoS ONCE, spanning its two markers \
         (x 50..80) at the line's baseline (300 - 100 = 200, ±d/h)"
    );
}

#[test]
fn a_breakable_frame_split_over_two_lines_fires_deco_h_then_deco_t() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let id = intern_labelled_decoset(&mut interp);

    let page = Page {
        body_lines: usize::MAX,
        lines: vec![
            // Opens here and runs off the end of the line: the fragment stops
            // at the line's last box (50 + 30).
            PlacedLine {
                x: Length::pt(50.0),
                baseline_y: Length::pt(100.0),
                contents: vec![
                    (Length::ZERO, inline_marker(id, false)),
                    (
                        Length::ZERO,
                        PureHorzBox::FixedEmpty {
                            width: Length::pt(30.0),
                        },
                    ),
                ],
            },
            // Resumes at THIS line's own left edge (40) and closes at the end
            // marker's x (40 + 20).
            PlacedLine {
                x: Length::pt(40.0),
                baseline_y: Length::pt(120.0),
                contents: vec![
                    (
                        Length::ZERO,
                        PureHorzBox::FixedEmpty {
                            width: Length::pt(20.0),
                        },
                    ),
                    (Length::pt(20.0), inline_marker(id, true)),
                ],
            },
        ],
    };
    let doc = doc_with_pages(vec![page]);
    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");

    assert_eq!(
        fired(&interp),
        vec![
            ("H".to_string(), (50.0, 198.0, 80.0, 210.0)),
            ("T".to_string(), (40.0, 178.0, 60.0, 190.0)),
        ],
        "a frame that really splits fires the HEAD closure for its first \
         fragment and the TAIL closure for its last — this is what \
         `annot.satyh`'s `register-location-frame` = (decoR, decoR, decoI, \
         decoI) relies on to register a destination exactly once"
    );
}

#[test]
fn a_breakable_frame_spanning_three_lines_fires_h_then_m_then_t() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let id = intern_labelled_decoset(&mut interp);

    let filler = |w: f64| PureHorzBox::FixedEmpty { width: Length::pt(w) };
    let page = Page {
        body_lines: usize::MAX,
        lines: vec![
            PlacedLine {
                x: Length::pt(50.0),
                baseline_y: Length::pt(100.0),
                contents: vec![
                    (Length::ZERO, inline_marker(id, false)),
                    (Length::ZERO, filler(30.0)),
                ],
            },
            // No marker at all on this line — the frame is simply still open,
            // so the whole line is one MIDDLE fragment.
            PlacedLine {
                x: Length::pt(40.0),
                baseline_y: Length::pt(120.0),
                contents: vec![(Length::ZERO, filler(45.0))],
            },
            PlacedLine {
                x: Length::pt(40.0),
                baseline_y: Length::pt(140.0),
                contents: vec![
                    (Length::ZERO, filler(20.0)),
                    (Length::pt(20.0), inline_marker(id, true)),
                ],
            },
        ],
    };
    let doc = doc_with_pages(vec![page]);
    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");

    assert_eq!(
        fired(&interp)
            .into_iter()
            .map(|(u, r)| (u, r.0, r.2))
            .collect::<Vec<_>>(),
        vec![
            ("H".to_string(), 50.0, 80.0),
            ("M".to_string(), 40.0, 85.0),
            ("T".to_string(), 40.0, 60.0),
        ],
        "every interior line of a split frame is a MIDDLE fragment spanning \
         that line's full extent"
    );
}

// Block frame fragments

fn block_page(lines: Vec<PlacedLine>) -> Page {
    Page {
        body_lines: usize::MAX,
        lines,
    }
}

#[test]
fn a_start_line_end_fragment_fires_decos_once_with_the_padded_extent_and_zero_depth() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let deco_v = eval_to_value(&mut interp, &link_deco("frame"));
    interp.decos.push(DecoEntry::Block {
        pads: Paddings {
            l: Length::ZERO,
            r: Length::ZERO,
            t: Length::pt(3.0),
            b: Length::pt(4.0),
        },
        width: Length::pt(200.0),
        decoset: [deco_v, Value::Unit, Value::Unit, Value::Unit],
        version: RustyfiVersion::V0_0,
    });

    let real_line = PlacedLine {
        x: Length::pt(30.0),
        baseline_y: Length::pt(120.0),
        contents: vec![(
            Length::ZERO,
            PureHorzBox::Graphics {
                width: Length::pt(50.0),
                height: Length::pt(8.0),
                depth: Length::pt(2.0),
                elems: Vec::new(),
                origin_independent: false,
            },
        )],
    };
    let page = block_page(vec![
        PlacedLine {
            x: Length::pt(30.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(
                Length::ZERO,
                PureHorzBox::FrameMarker {
                    id: DecoId(0),
                    end: false,
                },
            )],
        },
        real_line,
        PlacedLine {
            x: Length::pt(30.0),
            baseline_y: Length::pt(130.0),
            contents: vec![(
                Length::ZERO,
                PureHorzBox::FrameMarker {
                    id: DecoId(0),
                    end: true,
                },
            )],
        },
    ]);
    let doc = doc_with_pages(vec![page]);

    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");
    assert_eq!(
        interp.annotations.len(),
        1,
        "the fragment must fire decoS exactly once"
    );
    let a = &interp.annotations[0];
    assert_eq!(a.page, 0);

    // Real-line extent: top = baseline(120) - height(8) = 112, bottom =
    // baseline(120) + depth(2) = 122. Padded: frame_top = 112 - 3 = 109,
    // frame_bottom = 122 + 4 = 126 -> h = 17, d = 0 (bottom-left point).
    let paper_h = geometry().paper_height;
    let frame_bottom = Length::pt(126.0);
    let x = Length::pt(30.0);
    let y = paper_h - frame_bottom; // the bottom-left point's y
    assert_eq!(
        a.rect,
        (
            x,
            y - Length::ZERO,
            x + Length::pt(200.0),
            y + Length::pt(17.0)
        ),
        "w = DecoEntry::Block::width (200), h = padded extent (17), d = 0"
    );
}

#[test]
fn a_start_with_no_matching_end_on_the_page_fires_nothing() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let deco_v = eval_to_value(&mut interp, &link_deco("frame"));
    interp.decos.push(DecoEntry::Block {
        pads: Paddings {
            l: Length::ZERO,
            r: Length::ZERO,
            t: Length::ZERO,
            b: Length::ZERO,
        },
        width: Length::pt(200.0),
        decoset: [deco_v, Value::Unit, Value::Unit, Value::Unit],
        version: RustyfiVersion::V0_0,
    });

    let page = block_page(vec![PlacedLine {
        x: Length::pt(30.0),
        baseline_y: Length::pt(100.0),
        contents: vec![(
            Length::ZERO,
            PureHorzBox::FrameMarker {
                id: DecoId(0),
                end: false,
            },
        )],
    }]);
    let doc = doc_with_pages(vec![page]);

    rustyfi_lang::fire_hooks(&mut interp, &doc).expect("fire_hooks must succeed");
    assert!(
        interp.annotations.is_empty(),
        "an unclosed frame at page end must fire nothing (dropped silently, see fire_hooks doc comment)"
    );
}
