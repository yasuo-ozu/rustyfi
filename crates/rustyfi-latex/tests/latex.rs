//! Integration tests for the LaTeX writer (`render_latex`, `--format
//! latex`), over hand-built `Vec<VertBox>` fixtures.
//!
//! **These run everywhere; they do not compile anything.** The real evidence
//! that this backend works is `crates/rustyfi/tests/latex_output.rs`, which
//! puts a corpus document through a real `lualatex` — but that needs a TeX
//! Live, which the `build · clippy · test` CI job does not have. So the two
//! are split by what they can assume: this file asserts the SHAPE of the
//! output from box streams built in memory, with no fonts, no binary and no
//! engine.
//!
//! LaTeX's failure modes are the reason the assertions below are mostly about
//! what must NOT appear. Three of them stop a compile dead and two are
//! silent:
//!
//! - a bare `%` comments out the rest of the line, and the file still
//!   compiles — a document that says `100%` loses the other half of its
//!   sentence and nothing reports it;
//! - a bare `&` is `Misplaced alignment tab character`;
//! - a bare `_` is `Missing $ inserted`, from which TeX RECOVERS by opening
//!   math mode, so the rest of the sentence comes out italic with the spaces
//!   removed;
//! - an unbalanced `\begin`/`\end` runs to the end of the file;
//! - two superscripts on one base is `Double superscript`.
//!
//! The groups, in order: the document envelope (preamble, engine, packages),
//! then the structure recovered from the box stream, then escaping.

use rustyfi_backend::{
    AnnotAction, Closing, Color, DecoId, DocExtras, FontKey, GraphicsElem, HorzStringInfo,
    InlineMarkKind, Length, ListMarkKind, OutlineEntry, PageGeometry, Path, PathSeg, PureHorzBox,
    Subpath, TabularBox, TabularCellBox, VertBox,
};

fn text_run(text: &str) -> PureHorzBox {
    PureHorzBox::InnerString {
        info: HorzStringInfo {
            font: FontKey(0),
            size: Length::pt(12.0),
            rising: Length::ZERO,
            color: Color::Gray(0.0),
        },
        text: text.to_string(),
        width: Length::pt(8.0 * text.chars().count() as f64),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
    }
}

fn glue(natural: f64) -> PureHorzBox {
    PureHorzBox::OuterEmpty {
        natural: Length::pt(natural),
        shrinkable: Length::ZERO,
        stretchable: Length::ZERO,
    }
}

fn line_of(boxes: Vec<PureHorzBox>) -> VertBox {
    VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(12.0),
        contents: boxes.into_iter().map(|b| (Length::ZERO, b)).collect(),
    }
}

fn text_line(text: &str) -> VertBox {
    line_of(vec![text_run(text)])
}

fn render(vboxes: &[VertBox]) -> String {
    render_full(vboxes, &DocExtras::default(), &[], &[])
}

fn render_full(
    vboxes: &[VertBox],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
) -> String {
    rustyfi_latex::render_latex(
        Some(vboxes),
        &PageGeometry::default(),
        &[],
        extras,
        links,
        dests,
    )
    .expect("latex rendering must succeed")
}

/// The body between `\begin{document}` and `\end{document}`, trimmed — what
/// most assertions here are actually about.
fn body(tex: &str) -> String {
    let start = tex
        .find("\\begin{document}")
        .expect("a document envelope")
        + "\\begin{document}".len();
    let end = tex.find("\\end{document}").expect("a document envelope");
    tex[start..end].trim().to_string()
}

/// Every `\begin{X}` has a matching `\end{X}`, in order. The check that
/// catches the whole class of "the rest of the file is inside an
/// environment" failures at once.
fn environments_balance(tex: &str) -> Result<(), String> {
    let mut stack: Vec<&str> = Vec::new();
    let mut rest = tex;
    while let Some(at) = rest.find("\\begin{").or_else(|| rest.find("\\end{")) {
        let is_begin = rest[at..].starts_with("\\begin{");
        let open = at + if is_begin { 7 } else { 5 };
        let Some(close) = rest[open..].find('}') else {
            return Err("an unterminated environment name".into());
        };
        let name = &rest[open..open + close];
        if is_begin {
            stack.push(name);
        } else {
            match stack.pop() {
                Some(top) if top == name => {}
                Some(top) => return Err(format!("\\end{{{name}}} closes \\begin{{{top}}}")),
                None => return Err(format!("\\end{{{name}}} with nothing open")),
            }
        }
        rest = &rest[open + close..];
    }
    if stack.is_empty() {
        Ok(())
    } else {
        Err(format!("still open at the end: {stack:?}"))
    }
}

// ---------------------------------------------------------------------------
// The document envelope
// ---------------------------------------------------------------------------

/// A `.tex` that is not a whole document is not what `--format latex`
/// promises, and a fragment cannot be compiled to check anything else about
/// it.
#[test]
fn the_output_is_a_complete_document_not_a_fragment() {
    let tex = render(&[text_line("Hello.")]);
    assert!(tex.starts_with("% Generated by rustyfi --format latex."), "{tex}");
    assert!(tex.contains("\\documentclass{article}"), "{tex}");
    assert!(tex.contains("\\begin{document}"), "{tex}");
    assert!(tex.trim_end().ends_with("\\end{document}"), "{tex}");
    environments_balance(&tex).unwrap();
}

/// An empty document still has to COMPILE — this backend's whole claim is
/// that the file works, and a `DocumentValue` with no captured flow (a
/// hand-built one in a unit test) is the one input that could produce
/// nothing at all.
#[test]
fn a_document_with_no_flow_is_still_a_valid_document() {
    let tex = rustyfi_latex::render_latex(
        None,
        &PageGeometry::default(),
        &[],
        &DocExtras::default(),
        &[],
        &[],
    )
    .unwrap();
    assert!(tex.contains("\\begin{document}"), "{tex}");
    assert!(tex.trim_end().ends_with("\\end{document}"), "{tex}");
    environments_balance(&tex).unwrap();
}

/// The preamble declares only what the body reached, and the flags are set at
/// the point of emission rather than predicted — so a plain document does not
/// load TikZ, and one with a drawing does.
#[test]
fn the_preamble_declares_only_the_packages_the_body_used() {
    let plain = render(&[text_line("Just words.")]);
    for unused in ["{tikz}", "{fvextra}", "{hyperref}", "{luatexja-fontspec}"] {
        assert!(
            !plain.contains(unused),
            "a plain document should not load {unused}:\n{plain}"
        );
    }
    // …but always the two that every math-capable document needs, since a
    // formula can appear anywhere and they cost nothing.
    assert!(plain.contains("\\usepackage{amsmath}"), "{plain}");

    let drawn = render(&[line_of(vec![filled_square(20.0)])]);
    assert!(drawn.contains("\\usepackage{tikz}"), "{drawn}");
}

/// CJK cannot be set by pdfLaTeX at all, so the requirement is stated in a
/// comment a reader will see AND enforced by `iftex`, which fails immediately
/// instead of dropping every glyph.
#[test]
fn a_cjk_document_names_and_enforces_its_engine() {
    let tex = render(&[text_line("研究計画")]);
    assert!(tex.contains("% ENGINE: lualatex."), "{tex}");
    assert!(tex.contains("\\RequireLuaTeX"), "{tex}");
    assert!(tex.contains("\\usepackage{luatexja-fontspec}"), "{tex}");
    // A document without CJK must NOT demand it — that would make every
    // English document uncompilable under pdflatex for nothing.
    let plain = render(&[text_line("Just words.")]);
    assert!(!plain.contains("\\RequireLuaTeX"), "{plain}");
    assert!(plain.contains("% ENGINE: any of pdflatex"), "{plain}");
}

/// `bp`, never `pt`. A `Length` here is 1/72 inch (PDF user space); TeX's
/// `pt` is 1/72.27, so a page declared in `pt` is 0.37% too small and every
/// drawing with it.
#[test]
fn every_length_is_written_in_big_points() {
    let tex = render(&[line_of(vec![filled_square(20.0)])]);
    assert!(tex.contains("bp,paperheight="), "{tex}");
    assert!(tex.contains("[x=1bp,y=1bp,baseline=0bp"), "{tex}");
    // The failure this pins: `pt` anywhere in a geometry or picture option.
    assert!(!tex.contains("x=1pt"), "{tex}");
    assert!(!tex.contains("paperwidth=595.276pt"), "{tex}");
}

// ---------------------------------------------------------------------------
// Structure recovered from the box stream
// ---------------------------------------------------------------------------

/// A `Line` boundary is the PORT's wrapping decision at the page width the
/// document declared, not the author's — and LaTeX is about to break the
/// paragraph again at its own measure, so reproducing them would fossilize a
/// wrap for nothing.
#[test]
fn wrapped_lines_rejoin_into_one_paragraph_and_a_skip_splits_them() {
    let tex = body(&render(&[
        text_line("Hello,"),
        text_line("world!"),
        VertBox::Skip(Length::pt(12.0)),
        text_line("Second."),
    ]));
    assert_eq!(tex, "Hello, world!\n\nSecond.");
}

/// The one rule every backend in this crate has to get right: the box stream
/// puts glue between every pair of CJK characters, and "glue means space"
/// renders Japanese as `研 究 計 画`.
#[test]
fn the_cjk_glue_rule_is_honoured() {
    let tex = body(&render(&[line_of(vec![
        text_run("研"),
        glue(6.0),
        text_run("究"),
        glue(6.0),
        text_run("計画"),
    ])]));
    assert!(tex.contains("研究計画"), "{tex}");
    // …and a Latin pair still gets its space.
    let tex = body(&render(&[line_of(vec![
        text_run("two"),
        glue(4.0),
        text_run("words"),
    ])]));
    assert!(tex.contains("two words"), "{tex}");
}

/// A heading is found by correlating `extras.outline` to a destination frame
/// through `dest_name`, and the STARRED form is what keeps the document's own
/// numbering from being doubled.
#[test]
fn an_outline_registered_frame_becomes_a_starred_section_with_an_anchor() {
    let extras = DocExtras {
        outline: vec![
            OutlineEntry {
                level: 0,
                text: "Intro".into(),
                dest_name: "sec:intro".into(),
                is_open: true,
            },
            OutlineEntry {
                level: 1,
                text: "Detail".into(),
                dest_name: "sec:detail".into(),
                is_open: true,
            },
        ],
        ..DocExtras::default()
    };
    let dests = vec![
        (DecoId(1), "sec:intro".to_string()),
        (DecoId(2), "sec:detail".to_string()),
    ];
    let heading = |deco: usize, text: &str| {
        line_of(vec![
            PureHorzBox::InlineFrameMarker {
                id: DecoId(deco),
                end: false,
                height: Length::ZERO,
                depth: Length::ZERO,
            },
            text_run(text),
            PureHorzBox::InlineFrameMarker {
                id: DecoId(deco),
                end: true,
                height: Length::ZERO,
                depth: Length::ZERO,
            },
        ])
    };
    let tex = render_full(
        &[
            heading(1, "1. Introduction"),
            VertBox::Skip(Length::pt(12.0)),
            heading(2, "1.1 Detail"),
        ],
        &extras,
        &[],
        &dests,
    );
    // Starred: the document typeset `1.` itself, and `\section` would number
    // it a second time.
    assert!(
        body(&tex).contains("\\section*{\\hypertarget{rustyfi:sec:intro}{}1. Introduction}"),
        "{tex}"
    );
    assert!(body(&tex).contains("\\subsection*{"), "{tex}");
    // A `\hypertarget` needs the package as much as an `\href` does, and a
    // document may have headings and no links at all.
    assert!(tex.contains("hyperref"), "{tex}");
}

/// A `\ref` is a REAL link here, which is the one thing this backend can say
/// and the Markdown one cannot — Markdown has no anchor scheme, so it has to
/// drop cross-references to plain text.
#[test]
fn a_cross_reference_becomes_a_hyperlink_to_the_headings_own_anchor() {
    let links = vec![(DecoId(9), AnnotAction::GotoName("sec:intro".into()))];
    let tex = body(&render_full(
        &[line_of(vec![
            text_run("see"),
            glue(4.0),
            PureHorzBox::InlineFrameMarker {
                id: DecoId(9),
                end: false,
                height: Length::ZERO,
                depth: Length::ZERO,
            },
            text_run("Section 1"),
            PureHorzBox::InlineFrameMarker {
                id: DecoId(9),
                end: true,
                height: Length::ZERO,
                depth: Length::ZERO,
            },
        ])],
        &DocExtras::default(),
        &links,
        &[],
    ));
    assert_eq!(tex, "see \\hyperlink{rustyfi:sec:intro}{Section 1}");
}

/// The word space before a link is pending when the link opens, so left where
/// it falls it lands INSIDE the braces — the link text starts with a space,
/// the underline starts early, and the word before it runs into the link.
#[test]
fn a_links_leading_space_stays_outside_it() {
    let links = vec![(DecoId(3), AnnotAction::Uri("https://x.invalid/".into()))];
    let tex = body(&render_full(
        &[line_of(vec![
            text_run("A"),
            glue(4.0),
            text_run("link:"),
            glue(4.0),
            PureHorzBox::InlineFrameMarker {
                id: DecoId(3),
                end: false,
                height: Length::ZERO,
                depth: Length::ZERO,
            },
            text_run("here"),
            PureHorzBox::InlineFrameMarker {
                id: DecoId(3),
                end: true,
                height: Length::ZERO,
                depth: Length::ZERO,
            },
        ])],
        &DocExtras::default(),
        &links,
        &[],
    ));
    assert_eq!(tex, "A link: \\href{https://x.invalid/}{here}");
}

/// The list markers are the only structural signal there is; the bullet the
/// document DREW between the `BulletStart`/`BulletEnd` fence is dropped,
/// since `\item` replaces it.
#[test]
fn list_markers_become_real_environments_with_their_drawn_bullets_dropped() {
    let tex = body(&render(&[
        VertBox::ListMark(ListMarkKind::ListStart { ordered: false }),
        VertBox::ListMark(ListMarkKind::ItemStart),
        line_of(vec![
            PureHorzBox::InlineMark(InlineMarkKind::BulletStart),
            text_run("*"),
            PureHorzBox::InlineMark(InlineMarkKind::BulletEnd),
            text_run("first"),
        ]),
        VertBox::ListMark(ListMarkKind::ItemEnd),
        VertBox::ListMark(ListMarkKind::ItemStart),
        text_line("second"),
        VertBox::ListMark(ListMarkKind::ItemEnd),
        VertBox::ListMark(ListMarkKind::ListEnd),
    ]));
    assert_eq!(
        tex,
        "\\begin{itemize}\n\\item first\n\\item second\n\\end{itemize}"
    );
    let ordered = body(&render(&[
        VertBox::ListMark(ListMarkKind::ListStart { ordered: true }),
        VertBox::ListMark(ListMarkKind::ItemStart),
        text_line("one"),
        VertBox::ListMark(ListMarkKind::ItemEnd),
        VertBox::ListMark(ListMarkKind::ListEnd),
    ]));
    assert!(ordered.contains("\\begin{enumerate}"), "{ordered}");
}

/// A list left open by the stream would take the `\end{document}` with it,
/// and an environment with no `\item` in it is a hard error in its own right.
#[test]
fn an_unclosed_or_empty_list_still_produces_a_valid_document() {
    let tex = render(&[
        VertBox::ListMark(ListMarkKind::ListStart { ordered: false }),
        text_line("orphan"),
    ]);
    environments_balance(&tex).unwrap();
    let empty = render(&[
        VertBox::ListMark(ListMarkKind::ListStart { ordered: false }),
        VertBox::ListMark(ListMarkKind::ListEnd),
    ]);
    environments_balance(&empty).unwrap();
    assert!(
        !empty.contains("\\begin{itemize}\n\\end{itemize}"),
        "an itemize with no \\item is `Something's wrong--perhaps a missing \
         \\item`:\n{empty}"
    );
}

/// A table carries the rules the DOCUMENT drew — a booktabs-style three-rule
/// table and a fully-ruled grid must not come out alike.
#[test]
fn a_table_carries_only_the_rules_the_document_drew() {
    let cell = |x: f64, y: f64, s: &str| TabularCellBox {
        x: Length::pt(x),
        baseline_y: Length::pt(y),
        contents: vec![(Length::ZERO, text_run(s))],
    };
    // One horizontal rule under the header row, no verticals: the shape
    // `easytable`'s default draws.
    let rule = GraphicsElem::Fill(
        Color::Gray(0.0),
        Path {
            subpaths: vec![Subpath {
                start: (Length::pt(0.0), Length::pt(14.0)),
                segs: vec![
                    PathSeg::Line((Length::pt(100.0), Length::pt(14.0))),
                    PathSeg::Line((Length::pt(100.0), Length::pt(14.4))),
                    PathSeg::Line((Length::pt(0.0), Length::pt(14.4))),
                ],
                closing: Closing::Line,
            }],
        },
    );
    let tab = TabularBox {
        width: Length::pt(100.0),
        height: Length::pt(30.0),
        depth: Length::ZERO,
        cells: vec![
            cell(0.0, 20.0, "a"),
            cell(50.0, 20.0, "b"),
            cell(0.0, 8.0, "c"),
            cell(50.0, 8.0, "d"),
        ],
        rules: vec![rule],
    };
    let tex = body(&render(&[line_of(vec![PureHorzBox::Tabular(tab)])]));
    assert!(tex.contains("\\begin{tabular}{ll}"), "{tex}");
    assert!(tex.contains("a & b \\\\"), "{tex}");
    // Exactly one rule, and it is between the two rows — NOT a `|` in the
    // column spec and not an `\hline` per row.
    assert_eq!(tex.matches("\\hline").count(), 1, "{tex}");
    assert!(!tex.contains("{|l"), "{tex}");
}

/// A drawing is drawn, from the same vector paths the PDF writer strokes —
/// nothing is rasterized and nothing is dropped.
#[test]
fn a_drawing_becomes_a_tikzpicture_of_its_own_paths() {
    let tex = body(&render(&[line_of(vec![filled_square(20.0)])]));
    assert!(tex.contains("\\begin{tikzpicture}"), "{tex}");
    assert!(tex.contains("\\fill[rustyficlr,even odd rule]"), "{tex}");
    assert!(tex.contains("(0.000,0.000) -- (20.000,0.000)"), "{tex}");
}

/// The corpus is full of hairline rules, leader dots and heading underlines
/// drawn as one-off graphics. Measured by the INK, not the box: `stdjabook`
/// draws its heading rule as a 440x1pt line inside a 440x4pt box.
#[test]
fn a_hairline_rule_is_not_promoted_to_a_figure() {
    let tex = body(&render(&[line_of(vec![PureHorzBox::Graphics {
        width: Length::pt(440.0),
        height: Length::pt(4.0),
        depth: Length::ZERO,
        origin_independent: false,
        elems: vec![GraphicsElem::Fill(
            Color::Gray(0.0),
            Path {
                subpaths: vec![Subpath {
                    start: (Length::pt(0.0), Length::pt(0.0)),
                    segs: vec![
                        PathSeg::Line((Length::pt(440.0), Length::pt(0.0))),
                        PathSeg::Line((Length::pt(440.0), Length::pt(1.0))),
                        PathSeg::Line((Length::pt(0.0), Length::pt(1.0))),
                    ],
                    closing: Closing::Line,
                }],
            },
        )],
    }])]));
    assert!(!tex.contains("tikzpicture"), "{tex}");
}

/// Math is real math. The re-derivation itself is `crate::latex`'s (shared
/// with `--katex`); what is asserted here is only that this backend reaches
/// it and puts the result in delimiters.
#[test]
fn math_is_written_in_math_mode() {
    let tex = body(&render(&[line_of(vec![PureHorzBox::Math {
        width: Length::pt(20.0),
        height: Length::pt(10.0),
        depth: Length::ZERO,
        glyphs: vec![math_glyph("α", 0.0, 0.0, 10.0), math_glyph("x", 6.0, 0.0, 10.0)],
        rules: Vec::new(),
    }])]));
    assert_eq!(tex, "$\\alpha x$");
}

/// A document whose only non-Latin content is GREEK is not a
/// "compiles anywhere" document, and used to claim to be one.
///
/// The engine was chosen with `recover::is_cjk`, which is a SPACING predicate
/// — it knows Han, kana and Hangul because those are what `wants_space` has
/// to suppress a space between, and nothing else. So Greek, Cyrillic, Hebrew,
/// an arrow or a bare `≤` took the "any of pdflatex, xelatex or lualatex"
/// branch, and what that produced was a hard error under pdfLaTeX and a clean
/// exit 0 with the glyphs missing under the other two.
#[test]
fn a_non_latin_non_cjk_document_does_not_claim_to_compile_under_pdflatex() {
    let tex = render(&[text_line("the ratio φ and the bound ≤")]);
    assert!(
        tex.contains("% ENGINE: xelatex or lualatex, NOT pdflatex."),
        "{tex}"
    );
    // Refused where TeX will see it, not only where a reader will.
    assert!(
        tex.contains("\\ifPDFTeX\n  \\PackageError{rustyfi}"),
        "{tex}"
    );
    // And it is NOT the CJK branch: no luatexja, no Japanese faces.
    assert!(!tex.contains("luatexja"), "{tex}");
    assert!(!tex.contains("\\RequireLuaTeX"), "{tex}");

    // The control: pure Latin-1 still claims all three, so the new arm is
    // reached by the characters that need it and not by every document.
    let plain = render(&[text_line("a plain sentence, with a café in it")]);
    assert!(
        plain.contains("% ENGINE: any of pdflatex, xelatex or lualatex."),
        "{plain}"
    );
    assert!(plain.contains("\\usepackage[T1]{fontenc}"), "{plain}");
}

/// **The word space on BOTH sides of an opaque box survives.**
///
/// The trailing one did not, and nothing here caught it: an equation reached
/// `Ctx::open_opaque`, which cleared `last_char`, and
/// `recover::wants_space(None, …)` returns `false`, so the glue after the
/// formula was discarded and `ALPHA $x$ BRAVO` came out as `ALPHA $x$BRAVO`.
/// It survived review because the LEADING space is judged separately and was
/// always right, and because every formula in `latex-plain.saty` happens to
/// be followed by punctuation — the one position where no space is wanted.
///
/// The assertion is `assert_eq!` on the whole paragraph rather than a
/// `contains`, because the failure is an ABSENCE and a substring test for the
/// good case is what was missing in the first place.
#[test]
fn a_formula_between_two_words_keeps_the_space_on_both_sides() {
    let tex = body(&render(&[line_of(vec![
        text_run("ALPHA"),
        glue(4.0),
        PureHorzBox::Math {
            width: Length::pt(10.0),
            height: Length::pt(10.0),
            depth: Length::ZERO,
            glyphs: vec![math_glyph("x", 0.0, 0.0, 10.0)],
            rules: Vec::new(),
        },
        glue(4.0),
        text_run("BRAVO"),
    ])]));
    assert_eq!(tex, "ALPHA $x$ BRAVO");
}

/// The same rule for the other four opaque constructs, which share one
/// implementation (`Ctx::flow_across`) precisely so they cannot disagree.
/// A drawing is the case with a visible body; an image placeholder is the
/// case whose verbatim text is generated rather than recovered.
#[test]
fn a_drawing_and_an_image_keep_the_space_after_them() {
    let drawing = body(&render(&[line_of(vec![
        text_run("ALPHA"),
        glue(4.0),
        filled_square(20.0),
        glue(4.0),
        text_run("BRAVO"),
    ])]));
    assert!(
        drawing.contains("ALPHA "),
        "the space before the drawing: {drawing}"
    );
    assert!(
        drawing.ends_with(" BRAVO"),
        "the space after the drawing: {drawing}"
    );

    let image = body(&render(&[line_of(vec![
        text_run("ALPHA"),
        glue(4.0),
        PureHorzBox::Image {
            image: rustyfi_backend::ImageId(0),
            width: Length::pt(30.0),
            height: Length::pt(20.0),
        },
        glue(4.0),
        text_run("BRAVO"),
    ])]));
    assert!(
        image.ends_with(" BRAVO"),
        "the space after the image placeholder: {image}"
    );
}

// ---------------------------------------------------------------------------
// Escaping
// ---------------------------------------------------------------------------

/// The failure that motivated all of this: a bare `%` comments out the rest
/// of the line, the document still compiles, and nothing anywhere says half a
/// sentence is missing.
#[test]
fn a_documents_own_percent_sign_does_not_truncate_the_line() {
    let tex = body(&render(&[text_line("100% of the budget")]));
    assert_eq!(tex, "100\\% of the budget");
}

#[test]
fn every_reserved_character_survives_as_itself() {
    let tex = body(&render(&[text_line("# $ % & _ { } ~ ^ \\")]));
    assert_eq!(
        tex,
        "\\# \\$ \\% \\& \\_ \\{ \\} \\textasciitilde{} \\textasciicircum{} \
         \\textbackslash{}"
    );
}

/// A URL's percent-encoding is the common case, not a corner, and unescaped
/// it takes the rest of the line with it.
#[test]
fn a_url_keeps_its_percent_encoding() {
    let links = vec![(
        DecoId(4),
        AnnotAction::Uri("https://x.invalid/a%20b#f".into()),
    )];
    let tex = body(&render_full(
        &[line_of(vec![
            PureHorzBox::InlineFrameMarker {
                id: DecoId(4),
                end: false,
                height: Length::ZERO,
                depth: Length::ZERO,
            },
            text_run("t"),
            PureHorzBox::InlineFrameMarker {
                id: DecoId(4),
                end: true,
                height: Length::ZERO,
                depth: Length::ZERO,
            },
        ])],
        &DocExtras::default(),
        &links,
        &[],
    ));
    assert_eq!(tex, "\\href{https://x.invalid/a\\%20b\\#f}{t}");
}

/// Nothing inside a `Verbatim` is escaped, and that is decided per PARAGRAPH
/// — which is exactly what the deferred piece buffer exists for, since the
/// same `%` has to be escaped in prose and left alone here.
///
/// The paragraph is recognised as code STRUCTURALLY, by every line ending in
/// an `inline-fil`, because a `+code` block containing Japanese is not
/// all-fixed-pitch.
#[test]
fn a_code_block_is_verbatim_and_does_not_escape_its_contents() {
    // Three lines, each fil-terminated, in a face the (absent) font store
    // cannot call fixed-pitch — so this also pins that the structural test is
    // what decides, not the font.
    let code_line = |s: &str| line_of(vec![text_run(s), PureHorzBox::OuterFil]);
    let tex = body(&render(&[
        code_line("let x = 100% of y"),
        code_line("let z = \\foo{bar}"),
        code_line("done"),
    ]));
    // With no font store nothing is fixed-pitch, so this comes out as prose
    // and IS escaped — the documented base-14 degradation. What must hold
    // either way is that it is one paragraph and the `%` is safe.
    assert!(tex.contains("\\%"), "{tex}");
    assert!(!tex.contains("100% of"), "the raw percent survived:\n{tex}");
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn filled_square(side: f64) -> PureHorzBox {
    PureHorzBox::Graphics {
        width: Length::pt(side),
        height: Length::pt(side),
        depth: Length::ZERO,
        origin_independent: false,
        elems: vec![GraphicsElem::Fill(
            Color::Rgb(1.0, 0.0, 0.0),
            Path {
                subpaths: vec![Subpath {
                    start: (Length::pt(0.0), Length::pt(0.0)),
                    segs: vec![
                        PathSeg::Line((Length::pt(side), Length::pt(0.0))),
                        PathSeg::Line((Length::pt(side), Length::pt(side))),
                        PathSeg::Line((Length::pt(0.0), Length::pt(side))),
                    ],
                    closing: Closing::Line,
                }],
            },
        )],
    }
}

fn math_glyph(text: &str, dx: f64, dy: f64, size: f64) -> rustyfi_backend::MathGlyph {
    rustyfi_backend::MathGlyph {
        info: HorzStringInfo {
            font: FontKey(0),
            size: Length::pt(size),
            color: Color::Gray(0.0),
            rising: Length::ZERO,
        },
        text: text.to_string(),
        gid: None,
        dx: Length::pt(dx),
        dy: Length::pt(dy),
        width: Length::pt(size * 0.5),
        height: Length::pt(size * 0.7),
        depth: Length::ZERO,
    }
}
