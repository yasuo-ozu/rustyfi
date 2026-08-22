//! `inline-graphics-outer`: a fil-stretchy graphics box whose
//! callback needs the RESOLVED width, unknown until line layout. Driven as a
//! raw `Ast`-apply chain, so the returned `Value`/backend structures can be
//! inspected directly rather than through a typechecked source string — no
//! parser, no `|>` (unsupported by this port's frontend), just
//! `start-path`/`line-to`/`close-with-line`/`fill` application chains
//! building a `0..w`-wide rectangle so the resolved width is directly
//! checkable against the fill path's own bbox.

use rustyfi_backend::{path_bbox, Context, FontKey, FontMetrics, Length, PureHorzBox};
use rustyfi_lang::ast::Ast;
use rustyfi_lang::eval;
use rustyfi_lang::primitives;
use rustyfi_lang::value::Value;
use rustyfi_syntax::Span;

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

fn app1(f: Ast, a: Ast) -> Ast {
    Ast::Apply(Box::new(f), Box::new(a))
}

fn app2(name: &str, a: Ast, b: Ast) -> Ast {
    app1(app1(var(name), a), b)
}

fn len(pt: f64) -> Ast {
    Ast::Length(Length::pt(pt))
}

fn point(x: Ast, y: Ast) -> Ast {
    Ast::Tuple(vec![x, y])
}

fn gray(g: f64) -> Ast {
    Ast::Ctor("Gray".to_string(), Some(Box::new(Ast::Float(g))))
}

/// `fun w pt -> [ fill (Gray 0.) (start-path (0,0) |> line-to (w,0) |>
/// line-to (w,10) |> line-to (0,10) |> close-with-line) ]` — a `0..w`-wide,
/// `0..10pt`-tall rectangle, built as an application chain (no `|>`).
fn rect_of_width_w_closure() -> Ast {
    let path = app1(
        var("close-with-line"),
        app2(
            "line-to",
            point(len(0.0), len(10.0)),
            app2(
                "line-to",
                point(var("w"), len(10.0)),
                app2(
                    "line-to",
                    point(var("w"), len(0.0)),
                    app1(var("start-path"), point(len(0.0), len(0.0))),
                ),
            ),
        ),
    );
    let fill_call = app2("fill", gray(0.0), path);
    Ast::Lambda(
        "w".to_string(),
        std::rc::Rc::new(Ast::Lambda(
            "pt".to_string(),
            std::rc::Rc::new(Ast::List(vec![fill_call])),
        )),
    )
}

#[test]
fn inline_graphics_outer_resolves_to_a_graphics_box_with_the_lines_slack_width() {
    let mut env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);

    let ctx = Context::initial(Length::pt(200.0));
    let paragraph_width = ctx.paragraph_width;
    env.define("ctx", Value::Context(Box::new(ctx)));

    // `inline-graphics-outer 5pt 0pt (fun w pt -> [...])`
    let ib_ast = app1(
        app1(app1(var("inline-graphics-outer"), len(5.0)), len(0.0)),
        rect_of_width_w_closure(),
    );
    // `line-break true true ctx ib`
    let line_break_ast = app1(
        app1(
            app1(app1(var("line-break"), Ast::Bool(true)), Ast::Bool(true)),
            var("ctx"),
        ),
        ib_ast,
    );

    let v = interp
        .eval(&env, &line_break_ast)
        .expect("evaluation should succeed");
    let Value::BlockBoxes(lines) = v else {
        panic!("expected block-boxes, got {v:?}")
    };
    // line-break brackets the formed paragraph with paragraph_top/bottom
    // margin Skips, so the single fil-only line sits between two
    // VertBox::Skip — find it rather than assuming index 0.
    let contents = lines
        .iter()
        .find_map(|vb| match vb {
            rustyfi_backend::VertBox::Line { contents, .. } => Some(contents),
            _ => None,
        })
        .expect("a single fil-only box should fit on one line");
    assert_eq!(contents.len(), 1);
    let (_, bx) = &contents[0];
    match bx {
        PureHorzBox::Graphics { width, elems, .. } => {
            // The marker was replaced (not left as `GraphicsOuter`), and the
            // sole fil on the line took ALL the slack: natural width is 0,
            // target is `paragraph_width`, so the resolved width equals it
            // exactly.
            assert!(
                (width.0 - paragraph_width.0).abs() < 1e-6,
                "expected resolved width == paragraph_width ({paragraph_width:?}), got {width:?}"
            );
            assert_eq!(elems.len(), 1, "expected the callback's one fill element");
            let rustyfi_backend::GraphicsElem::Fill(_, path) = &elems[0] else {
                panic!("expected a Fill element, got {:?}", elems[0]);
            };
            // The callback actually received the RESOLVED width (not 0):
            // the rectangle's own x-extent equals it.
            let (pmin, pmax) = path_bbox(path);
            let x_extent = pmax.0 - pmin.0;
            assert!(
                (x_extent.0 - paragraph_width.0).abs() < 1e-6,
                "expected the fill path's x-extent to equal the resolved width \
                 {paragraph_width:?} (proving the callback saw the real width, not 0), \
                 got {x_extent:?}"
            );
        }
        other => panic!("expected a resolved Graphics box, got {other:?}"),
    }
}
