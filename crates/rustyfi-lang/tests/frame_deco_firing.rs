//! `docs/plans/hooks-annotations-crossref.md` §D: `fire_hooks`' inline-frame
//! (`fire_inline_frame`) and block-frame-fragment firing, driven directly
//! against hand-built `DocumentValue`s (mirroring `tests/hooks_crossref.rs`'s
//! style) — no parser, no `page-break` involved. Each deco closure is a
//! small hand-built `Ast::Lambda` chain (curried `pt -> w -> h -> d -> …`)
//! evaluated once to a `Value::Closure` and interned directly into
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
use rustyfi_syntax::Span;
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

// ============================================================================
// Inline frames (`fire_inline_frame`)
// ============================================================================

#[test]
fn a_frame_with_a_fill_deco_puts_one_element_in_page_graphics() {
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let deco_v = eval_to_value(&mut interp, &fill_deco());
    interp.decos.push(DecoEntry::Inline { deco: deco_v });

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
    interp.decos.push(DecoEntry::Inline { deco: deco_v });

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
    interp.decos.push(DecoEntry::Inline { deco: outer_deco });
    let inner_deco = eval_to_value(&mut interp, &link_deco("inner"));
    interp.decos.push(DecoEntry::Inline { deco: inner_deco });

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

// ============================================================================
// Block frame fragments (§C3)
// ============================================================================

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
