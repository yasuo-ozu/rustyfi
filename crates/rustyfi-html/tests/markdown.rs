//! Integration tests for the Markdown writer (`render_markdown`,
//! `--format markdown`), over hand-built `Vec<VertBox>` fixtures.
//!
//! Markdown's failure mode is not a crash or invalid output — it is output
//! that PARSES, and parses as something the document never said. A literal
//! asterisk becoming emphasis, a line beginning with a hyphen becoming a
//! bullet, a code block losing its indentation, `code-printer` losing its
//! hyphen: every one of those is silently wrong and looks fine. So the
//! assertions here are mostly about what must NOT appear.
//!
//! The groups, in the order they run: structure recovered from the box stream
//! (headings, lists, tables, links, emphasis, footnotes, images), then the
//! text-quality rules that decide whether the result reads as prose at all
//! (line rejoining, the CJK rule, hyphens, code blocks), then escaping.

use rustyfi_backend::{
    Closing, GraphicsElem, Path, PathSeg, Subpath,
    AnnotAction, Color, DecoId, DocExtras, FontKey, HorzStringInfo, InlineMarkKind, Length,
    ListMarkKind, MathGlyph, OutlineEntry, PureHorzBox, TabularBox, TabularCellBox, VertBox,
};

fn styled_run(text: &str, width: f64, rising: f64) -> PureHorzBox {
    PureHorzBox::InnerString {
        info: HorzStringInfo {
            font: FontKey(0),
            size: Length::pt(12.0),
            rising: Length::pt(rising),
            color: Color::Gray(0.0),
        },
        text: text.to_string(),
        width: Length::pt(width),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
    }
}

fn text_run(text: &str) -> PureHorzBox {
    styled_run(text, 8.0 * text.chars().count() as f64, 0.0)
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
    rustyfi_html::render_markdown(
        Some(vboxes),
        &[],
        extras,
        links,
        dests,
        rustyfi_html::MathMode::SvgOutline,
    )
    .expect("markdown rendering must succeed")
}

/// The same, in a chosen math mode — the one axis this backend has three
/// answers for. Every test above this line is about structure rather than
/// math and takes the default.
fn render_math(vboxes: &[VertBox], math: rustyfi_html::MathMode) -> String {
    rustyfi_html::render_markdown(Some(vboxes), &[], &DocExtras::default(), &[], &[], math)
        .expect("markdown rendering must succeed")
}

// ---------------------------------------------------------------------------
// Structure recovered from the box stream
// ---------------------------------------------------------------------------

/// A `Line` boundary is the PORT's wrapping decision at the page width the
/// document declared, not the author's. Reproducing it would hard-wrap the
/// Markdown at a width the reader never chose.
#[test]
fn wrapped_lines_rejoin_into_one_paragraph_and_a_skip_splits_them() {
    let md = render(&[
        text_line("Hello,"),
        text_line("world!"),
        VertBox::Skip(Length::pt(12.0)),
        text_line("Second."),
    ]);
    assert_eq!(md.trim(), "Hello, world!\n\nSecond.");
}

/// The heading is found by correlating `extras.outline` to a destination
/// frame through `dest_name` — a structural id match, not a font-size guess.
#[test]
fn an_outline_registered_frame_promotes_its_paragraph_to_a_heading() {
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
                height: Length::pt(9.0),
                depth: Length::ZERO,
            },
            text_run(text),
            PureHorzBox::InlineFrameMarker {
                id: DecoId(deco),
                end: true,
                height: Length::pt(9.0),
                depth: Length::ZERO,
            },
        ])
    };
    let md = render_full(
        &[
            heading(1, "Intro"),
            VertBox::Skip(Length::pt(6.0)),
            heading(2, "Detail"),
            VertBox::Skip(Length::pt(6.0)),
            text_line("Body."),
        ],
        &extras,
        &[],
        &dests,
    );
    assert!(md.contains("# Intro"), "{md}");
    assert!(md.contains("## Detail"), "{md}");
    // An unregistered paragraph stays a paragraph — no heuristic promotion.
    assert!(!md.contains("# Body"), "{md}");
}

#[test]
fn list_marks_become_nested_markdown_lists_and_the_drawn_bullet_is_dropped() {
    let bullet_item = |text: &str| {
        vec![
            VertBox::ListMark(ListMarkKind::ItemStart),
            line_of(vec![
                PureHorzBox::InlineMark(InlineMarkKind::BulletStart),
                text_run("\u{2022}"),
                PureHorzBox::InlineMark(InlineMarkKind::BulletEnd),
                glue(3.0),
                text_run(text),
            ]),
            VertBox::ListMark(ListMarkKind::ItemEnd),
        ]
    };
    let mut vboxes = vec![
        text_line("Before."),
        VertBox::Skip(Length::pt(6.0)),
        VertBox::ListMark(ListMarkKind::ListStart { ordered: false }),
    ];
    vboxes.extend(bullet_item("alpha"));
    vboxes.push(VertBox::ListMark(ListMarkKind::ItemStart));
    vboxes.push(text_line("beta"));
    vboxes.push(VertBox::ListMark(ListMarkKind::ListStart { ordered: true }));
    vboxes.extend(bullet_item("one"));
    vboxes.extend(bullet_item("two"));
    vboxes.push(VertBox::ListMark(ListMarkKind::ListEnd));
    vboxes.push(VertBox::ListMark(ListMarkKind::ItemEnd));
    vboxes.push(VertBox::ListMark(ListMarkKind::ListEnd));

    let md = render(&vboxes);
    assert_eq!(
        md.trim(),
        "Before.\n\n- alpha\n- beta\n  1. one\n  2. two",
        "{md}"
    );
    // The drawn bullet glyph is replaced by the marker, not printed beside it.
    assert!(!md.contains('\u{2022}'), "{md}");
}

/// An ordered list numbers itself, because the numeral the document DREW is
/// inside the bullet fence and is dropped with it.
#[test]
fn a_tabular_becomes_a_gfm_pipe_table_with_a_delimiter_row() {
    let cell = |x: f64, y: f64, text: &str| TabularCellBox {
        x: Length::pt(x),
        baseline_y: Length::pt(y),
        contents: vec![(Length::ZERO, text_run(text))],
    };
    let tab = TabularBox {
        width: Length::pt(120.0),
        height: Length::pt(40.0),
        depth: Length::ZERO,
        cells: vec![
            cell(0.0, 30.0, "h1"),
            cell(60.0, 30.0, "h2"),
            cell(0.0, 10.0, "a"),
            cell(60.0, 10.0, "b"),
        ],
        rules: Vec::new(),
    };
    let md = render(&[line_of(vec![PureHorzBox::Tabular(tab)])]);
    assert_eq!(
        md.trim(),
        "| h1 | h2 |\n| --- | --- |\n| a | b |",
        "{md}"
    );
}

/// A cell wide enough to WRAP holds a whole nested block rather than an
/// inline run. Left to `emit_inline`, whose `EmbeddedBlock` arm is inert
/// because only the block walker can close a paragraph, the cell comes out
/// empty — `easytable`'s own `lw 120pt` example lost the entire column it
/// exists to demonstrate.
#[test]
fn a_table_cell_holding_a_wrapped_block_is_not_empty() {
    let cell = |x: f64, contents: Vec<PureHorzBox>| TabularCellBox {
        x: Length::pt(x),
        baseline_y: Length::pt(10.0),
        contents: contents.into_iter().map(|b| (Length::ZERO, b)).collect(),
    };
    let tab = TabularBox {
        width: Length::pt(180.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        cells: vec![
            cell(0.0, vec![text_run("narrow")]),
            cell(
                60.0,
                vec![PureHorzBox::EmbeddedBlock {
                    width: Length::pt(120.0),
                    height: Length::pt(20.0),
                    depth: Length::ZERO,
                    block: vec![text_line("a wrapped"), text_line("cell")],
                    anchor_last: false,
                    breakable: false,
                }],
            ),
        ],
        rules: Vec::new(),
    };
    let md = render(&[line_of(vec![PureHorzBox::Tabular(tab)])]);
    assert!(md.contains("| narrow | a wrapped cell |"), "{md}");
}

/// `easytable` overlays a rules-only grid of EMPTY cells on the real table.
/// This backend draws no rules at all, so that half holds nothing whatever
/// and must not come out as an empty grid above every table.
#[test]
fn a_phantom_table_with_no_text_in_it_is_dropped() {
    let tab = TabularBox {
        width: Length::pt(120.0),
        height: Length::pt(40.0),
        depth: Length::ZERO,
        cells: vec![
            TabularCellBox {
                x: Length::ZERO,
                baseline_y: Length::pt(30.0),
                contents: vec![],
            },
            TabularCellBox {
                x: Length::pt(60.0),
                baseline_y: Length::pt(30.0),
                contents: vec![],
            },
        ],
        rules: Vec::new(),
    };
    let md = render(&[text_line("x"), line_of(vec![PureHorzBox::Tabular(tab)])]);
    assert_eq!(md.trim(), "x", "{md}");
}

#[test]
fn emphasis_marks_become_asterisks_and_a_uri_link_becomes_a_markdown_link() {
    let md = render_full(
        &[line_of(vec![
            PureHorzBox::InlineMark(InlineMarkKind::EmphStart { strong: false }),
            text_run("em"),
            PureHorzBox::InlineMark(InlineMarkKind::EmphEnd),
            glue(3.0),
            PureHorzBox::InlineMark(InlineMarkKind::EmphStart { strong: true }),
            text_run("bold"),
            PureHorzBox::InlineMark(InlineMarkKind::EmphEnd),
            glue(3.0),
            PureHorzBox::InlineFrameMarker {
                id: DecoId(7),
                end: false,
                height: Length::pt(9.0),
                depth: Length::ZERO,
            },
            text_run("here"),
            PureHorzBox::InlineFrameMarker {
                id: DecoId(7),
                end: true,
                height: Length::pt(9.0),
                depth: Length::ZERO,
            },
        ])],
        &DocExtras::default(),
        &[(DecoId(7), AnnotAction::Uri("https://example.invalid/".into()))],
        &[],
    );
    assert_eq!(
        md.trim(),
        "*em* **bold** [here](https://example.invalid/)",
        "{md}"
    );
}

/// A `\ref` resolves to a NAMED DESTINATION, and Markdown has no anchor
/// scheme — a renderer invents heading anchors from the heading's own words,
/// so `[Section 3](#sec:intro)` would be a link that goes nowhere. The
/// cross-reference text the document typeset is already what a reader needs.
#[test]
fn an_in_document_reference_stays_plain_text_rather_than_becoming_a_dead_link() {
    let md = render_full(
        &[line_of(vec![
            PureHorzBox::InlineFrameMarker {
                id: DecoId(3),
                end: false,
                height: Length::pt(9.0),
                depth: Length::ZERO,
            },
            text_run("Section 3"),
            PureHorzBox::InlineFrameMarker {
                id: DecoId(3),
                end: true,
                height: Length::pt(9.0),
                depth: Length::ZERO,
            },
        ])],
        &DocExtras::default(),
        &[(DecoId(3), AnnotAction::GotoName("sec:intro".into()))],
        &[],
    );
    assert_eq!(md.trim(), "Section 3", "{md}");
    assert!(!md.contains('#'), "{md}");
}

/// GFM has a real footnote construct, which fits better than the HTML
/// backend's in-flow `<aside>`. The document's OWN typeset marker — a raised
/// numeral immediately after the box — has to go, or the text reads `[^1]1`.
#[test]
fn a_footnote_becomes_a_gfm_reference_and_the_documents_own_marker_is_dropped() {
    let md = render(&[line_of(vec![
        text_run("Claim"),
        PureHorzBox::Footnote {
            block: vec![text_line("The note body.")],
        },
        // `stdjabook`'s `\footnote` marker: `\*#it-num;`, two RAISED runs.
        styled_run("*", 4.0, 3.0),
        styled_run("1", 4.0, 3.0),
        text_run(" continues."),
    ])]);
    assert!(md.contains("Claim[^1]"), "{md}");
    assert!(md.contains("[^1]: The note body."), "{md}");
    assert!(!md.contains("[^1]*"), "typeset marker survived:\n{md}");
    assert!(!md.contains("[^1]1"), "typeset marker survived:\n{md}");
}

// ---------------------------------------------------------------------------
// Text quality
// ---------------------------------------------------------------------------

/// The box stream puts glue between every pair of CJK characters so a
/// Japanese line has something to justify with. Rendering those as spaces
/// gives `研 究 計 画`, which is the bug `recover::wants_space` exists to
/// prevent — including ACROSS a rejoined line break, which is where a naive
/// rejoin puts one back.
#[test]
fn japanese_gains_no_spaces_between_its_characters_or_across_a_rejoin() {
    let md = render(&[
        line_of(vec![
            text_run("研"),
            glue(0.0),
            text_run("究"),
            glue(5.28),
            text_run("計"),
        ]),
        line_of(vec![text_run("画")]),
    ]);
    assert_eq!(md.trim(), "研究計画", "{md}");
}

/// A script boundary and a real word space DO take one.
#[test]
fn a_word_space_and_a_script_boundary_still_separate() {
    let md = render(&[line_of(vec![
        text_run("を"),
        glue(2.64),
        text_run("LaTeX"),
        glue(3.5),
        text_run("で"),
    ])]);
    assert_eq!(md.trim(), "を LaTeX で", "{md}");
}

/// `InlineMarkKind::BreakHyphen` is the ONLY thing that tells the breaker's
/// hyphen from the author's — the splice produces an ordinary `InnerString`.
/// Guessing from the text's shape deleted real hyphens and rendered
/// `code-printer` as `codeprinter`.
#[test]
fn the_breakers_hyphen_is_deleted_and_an_authored_one_is_not() {
    let broken = render(&[
        line_of(vec![
            text_run("hyphen"),
            PureHorzBox::InlineMark(InlineMarkKind::BreakHyphen),
            text_run("-"),
        ]),
        text_line("ation"),
    ]);
    assert_eq!(broken.trim(), "hyphenation", "{broken}");

    // The identical text SHAPE, with no marker: the author's hyphen, at a
    // break UAX#14 allowed. It stays, and the halves gain no space.
    let authored = render(&[
        line_of(vec![text_run("code"), text_run("-")]),
        text_line("printer"),
    ]);
    assert_eq!(authored.trim(), "code-printer", "{authored}");
}

/// The whole reason Markdown does better than HTML here: a fence keeps
/// whitespace, so a code block's indentation — which arrives as an
/// `inline-skip` in exact multiples of the character advance — survives.
#[test]
fn a_code_block_keeps_its_line_breaks_and_its_indentation() {
    // A fixed-pitch run: 6pt per character, so a 12pt skip is two columns.
    let mono = |text: &str| PureHorzBox::InnerString {
        info: HorzStringInfo {
            font: FontKey(1),
            size: Length::pt(10.0),
            rising: Length::ZERO,
            color: Color::Gray(0.0),
        },
        text: text.to_string(),
        width: Length::pt(6.0 * text.chars().count() as f64),
        height: Length::pt(7.0),
        depth: Length::pt(2.0),
    };
    let code_line = |indent: f64, text: &str| {
        line_of(vec![
            PureHorzBox::FixedEmpty {
                width: Length::pt(indent),
            },
            mono(text),
            PureHorzBox::OuterFil,
        ])
    };
    let vboxes = vec![
        code_line(0.0, "if x:"),
        code_line(12.0, "return 1"),
        code_line(0.0, "done"),
    ];
    let Some(md) = markdown_with_mono_font(&vboxes) else {
        return;
    };
    assert_eq!(
        md.trim(),
        "```\nif x:\n  return 1\ndone\n```",
        "{md}"
    );
}

/// A code block whose text is not ALL fixed-pitch — which is every `+code`
/// block containing Japanese, since a fixed-pitch Latin face has no CJK
/// glyphs — is still a code block. The signal is that every line ends with
/// an `inline-fil`, which a wrapped prose line never does.
#[test]
fn a_code_block_containing_japanese_is_still_a_code_block() {
    let mono = |text: &str| PureHorzBox::InnerString {
        info: HorzStringInfo {
            font: FontKey(1),
            size: Length::pt(10.0),
            rising: Length::ZERO,
            color: Color::Gray(0.0),
        },
        text: text.to_string(),
        width: Length::pt(6.0 * text.chars().count() as f64),
        height: Length::pt(7.0),
        depth: Length::pt(2.0),
    };
    let vboxes = vec![
        line_of(vec![mono("title = {"), text_run("パッケージ"), mono("}")
            , PureHorzBox::OuterFil]),
        line_of(vec![mono("done"), PureHorzBox::OuterFil]),
    ];
    let Some(md) = markdown_with_mono_font(&vboxes) else {
        return;
    };
    assert!(md.starts_with("```"), "{md}");
    assert!(md.contains("title = {パッケージ}\ndone"), "{md}");
}

/// Ordinary justified prose ends only its LAST line with a fil. A two-line
/// paragraph is the case an "all but the last" test would get wrong, so it is
/// the one pinned.
#[test]
fn a_two_line_prose_paragraph_with_inline_code_is_not_a_code_block() {
    let mono = |text: &str| PureHorzBox::InnerString {
        info: HorzStringInfo {
            font: FontKey(1),
            size: Length::pt(10.0),
            rising: Length::ZERO,
            color: Color::Gray(0.0),
        },
        text: text.to_string(),
        width: Length::pt(6.0 * text.chars().count() as f64),
        height: Length::pt(7.0),
        depth: Length::pt(2.0),
    };
    let vboxes = vec![
        line_of(vec![text_run("Use"), glue(3.0), mono("point")]),
        line_of(vec![mono("list"), glue(3.0), text_run("here."), PureHorzBox::OuterFil]),
    ];
    let Some(md) = markdown_with_mono_font(&vboxes) else {
        return;
    };
    assert!(!md.starts_with("```"), "{md}");
    // And the code span the line break split comes back as ONE span.
    assert_eq!(md.trim(), "Use `point list` here.", "{md}");
}

fn math_glyph(text: &str, dx: f64, dy: f64, size: f64) -> MathGlyph {
    MathGlyph {
        info: HorzStringInfo {
            font: FontKey(0),
            size: Length::pt(size),
            rising: Length::ZERO,
            color: Color::Gray(0.0),
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

/// `x² + 1`, with the glyphs deliberately OUT of document order: `dx` is what
/// decides reading order, and every mode below shares that recovery.
fn x_squared_plus_one() -> VertBox {
    line_of(vec![PureHorzBox::Math {
        width: Length::pt(30.0),
        height: Length::pt(10.0),
        depth: Length::pt(2.0),
        glyphs: vec![
            math_glyph("2", 6.0, 5.0, 7.0),
            math_glyph("x", 0.0, 0.0, 10.0),
            math_glyph("+", 12.0, 0.0, 10.0),
            math_glyph("1", 20.0, 0.0, 10.0),
        ],
        rules: Vec::new(),
    }])
}

/// `--unicode-math`: the characters, in reading order, with Unicode's own
/// script forms. Math is flattened to positioned glyphs during evaluation, so
/// this is the honest maximum for a plain-text vocabulary — see
/// `markdown/math.rs`.
#[test]
fn unicode_math_renders_characters_in_reading_order_with_unicode_scripts() {
    let md = render_math(
        &[x_squared_plus_one()],
        rustyfi_html::MathMode::Unicode,
    );
    assert_eq!(md.trim(), "x² + 1", "{md}");
    // The whole point of this mode: no markup at all, so a renderer that
    // strips HTML still shows the equation.
    assert!(!md.contains("<svg"), "{md}");
    assert!(!md.contains('$'), "{md}");
}

/// **With no font store, both drawing modes fall back to characters.**
///
/// A drawing needs a face — to outline, or at minimum to NAME so the reader's
/// browser picks something with the right advances. Base-14 mode has neither,
/// so a `<svg>` would be `<text>` at absolute coordinates in whatever the
/// reader defaults to, and under the sanitizing pipeline a `.md` is usually
/// read through, nothing at all. Reading-order characters are strictly better,
/// and are what this backend wrote before any of these modes existed.
///
/// Not a corner case: it is CI's own state on `build · clippy · test`, which
/// does not run `download-fonts.sh`.
#[test]
fn a_render_with_no_font_store_writes_characters_rather_than_an_empty_drawing() {
    for mode in [
        rustyfi_html::MathMode::SvgOutline,
        rustyfi_html::MathMode::SvgText,
    ] {
        let md = render_math(&[x_squared_plus_one()], mode);
        assert_eq!(md.trim(), "x² + 1", "{mode:?}: {md}");
        assert!(!md.contains("<svg"), "{mode:?}: {md}");
    }
}

/// The trailing space after an inline equation survives.
///
/// **The bug this pins was in every drawing and LaTeX mode at once.** An
/// equation is not an opaque box — `Ctx::open_opaque`, right for an image,
/// clears `last_char`, and the glue that follows is then judged by
/// `wants_space(None, …)`, which returns `false` and drops the space. The word
/// after every equation ran into it: `ここで 𝑙 は線の太さ` came out as
/// `…</svg>は線の太さ`. The space BEFORE always survived, which is what made
/// it easy to miss.
///
/// Asserted in all four modes, because the fix is per-arm and three of the
/// arms had to be changed.
#[test]
fn the_space_after_an_inline_equation_is_not_swallowed() {
    for mode in [
        rustyfi_html::MathMode::SvgOutline,
        rustyfi_html::MathMode::SvgText,
        rustyfi_html::MathMode::Katex,
        rustyfi_html::MathMode::Unicode,
    ] {
        let md = render_math(
            &[line_of(vec![
                text_run("see"),
                glue(4.0),
                PureHorzBox::Math {
                    width: Length::pt(10.0),
                    height: Length::pt(10.0),
                    depth: Length::pt(2.0),
                    glyphs: vec![math_glyph("y", 0.0, 0.0, 10.0)],
                    rules: Vec::new(),
                },
                glue(4.0),
                text_run("then"),
            ])],
            mode,
        );
        // The word after the equation must not be glued to it. Checked on the
        // rendered text rather than on a marker, so it holds however the
        // equation itself is written.
        assert!(
            md.contains(" then"),
            "{mode:?} lost the space before `then`:\n{md}"
        );
        assert!(md.contains("see "), "{mode:?} lost the space after `see`:\n{md}");
    }
}

/// `--katex`: LaTeX in `$…$`, and `$$…$$` when the equation is the whole
/// paragraph.
#[test]
fn katex_writes_latex_in_dollar_delimiters() {
    // Alone in its paragraph: displayed.
    let md = render_math(&[x_squared_plus_one()], rustyfi_html::MathMode::Katex);
    assert_eq!(md.trim(), "$$x^{2}+1$$", "{md}");

    // With prose beside it, the same equation is inline — same LaTeX, one
    // dollar. This is the distinction `Para::sole_math` exists to make.
    let md = render_math(
        &[line_of(vec![
            text_run("see"),
            glue(4.0),
            PureHorzBox::Math {
                width: Length::pt(30.0),
                height: Length::pt(10.0),
                depth: Length::pt(2.0),
                glyphs: vec![
                    math_glyph("x", 0.0, 0.0, 10.0),
                    math_glyph("2", 6.0, 5.0, 7.0),
                ],
                rules: Vec::new(),
            },
        ])],
        rustyfi_html::MathMode::Katex,
    );
    assert!(md.contains("$x^{2}$"), "{md}");
    assert!(!md.contains("$$"), "inline math must not be displayed: {md}");
}

/// Two equations side by side must not run their delimiters together.
///
/// One construction routinely produces several math boxes in a row —
/// `latexcmds`' Schrödinger equation is five — and written flush, the closing
/// `$` of one and the opening `$` of the next form a literal `$$` that every
/// renderer understanding display math reads as the start of one. Measured on
/// that document: `$h$$\frac{1}{2m}…` swallowed the rest of the formula into a
/// display block that never closed.
#[test]
fn two_adjacent_equations_do_not_run_their_delimiters_together() {
    let one_math = |ch: &str, dx: f64| PureHorzBox::Math {
        width: Length::pt(10.0),
        height: Length::pt(10.0),
        depth: Length::pt(2.0),
        glyphs: vec![math_glyph(ch, dx, 0.0, 10.0)],
        rules: Vec::new(),
    };
    let md = render_math(
        // Two math boxes with nothing between them, plus a text run so the
        // paragraph is not display math.
        &[line_of(vec![
            text_run("x"),
            one_math("a", 0.0),
            one_math("b", 0.0),
        ])],
        rustyfi_html::MathMode::Katex,
    );
    assert!(md.contains("$a$"), "{md}");
    assert!(md.contains("$b$"), "{md}");
    assert!(
        !md.contains("$$"),
        "adjacent inline equations formed a display delimiter:\n{md}"
    );
}

/// A formula is not one box. `latexcmds`' Schrödinger equation reaches this
/// backend as FOUR, because each `\underset`-style construction splits the
/// run — and they are pieces of one DISPLAYED equation, so they belong in one
/// `$$…$$`. Four separate display blocks would be four centred lines where the
/// document has one; four inline `$…$` would set a displayed equation in the
/// middle of a paragraph.
#[test]
fn a_paragraph_of_nothing_but_equations_is_one_display_block() {
    let one_math = |ch: &str| PureHorzBox::Math {
        width: Length::pt(10.0),
        height: Length::pt(10.0),
        depth: Length::pt(2.0),
        glyphs: vec![math_glyph(ch, 0.0, 0.0, 10.0)],
        rules: Vec::new(),
    };
    let md = render_math(
        &[line_of(vec![
            PureHorzBox::OuterFil,
            one_math("a"),
            one_math("b"),
            one_math("c"),
            PureHorzBox::OuterFil,
        ])],
        rustyfi_html::MathMode::Katex,
    );
    assert_eq!(md.trim(), "$$a b c$$", "{md}");
    assert_eq!(md.matches("$$").count(), 2, "one block, not three:\n{md}");
}

/// A DISPLAYED equation is centred in the drawing modes.
///
/// The document asked for it — a display equation is a `line-break` between
/// two `inline-fil`s — and the HTML backend already honours it
/// (`data-align="center"`). Markdown dropped it, on the general rule that
/// Markdown has no alignment; but a drawn equation is ALREADY raw HTML, so a
/// wrapper around it costs nothing a renderer that keeps the `<svg>` would
/// notice. See `para.rs`'s `centred`, including why the attribute is `align`
/// and not `style`.
///
/// Needs the bundled faces: with no font store both drawing modes degrade to
/// characters, and characters are not wrapped.
#[test]
fn a_displayed_equation_is_centred_in_the_drawing_modes() {
    let Some(store) = mono_font_store() else {
        return;
    };
    for mode in [
        rustyfi_html::MathMode::SvgOutline,
        rustyfi_html::MathMode::SvgText,
    ] {
        let md = rustyfi_html::render_markdown_ttf_with(
            Some(&[
                text_line("Before."),
                VertBox::Skip(Length::pt(12.0)),
                x_squared_plus_one(),
                VertBox::Skip(Length::pt(12.0)),
                line_of(vec![
                    text_run("see"),
                    glue(4.0),
                    PureHorzBox::Math {
                        width: Length::pt(10.0),
                        height: Length::pt(10.0),
                        depth: Length::pt(2.0),
                        glyphs: vec![math_glyph("y", 0.0, 0.0, 10.0)],
                        rules: Vec::new(),
                    },
                ]),
            ]),
            &store,
            &[],
            &DocExtras::default(),
            &[],
            &[],
            mode,
        )
        .expect("markdown rendering must succeed");
        // The wrapper opens its own line, so the whole thing is ONE CommonMark
        // HTML block and the drawing inside it is passed through raw.
        assert!(
            md.contains("<div align=\"center\">\n<svg"),
            "{mode:?} did not centre the display equation:\n{md}"
        );
        assert!(md.contains("\n</div>"), "{mode:?}: {md}");
        // Exactly one: prose is NOT centred, and neither is the equation set
        // inside a sentence.
        assert_eq!(
            md.matches("<div align=").count(),
            1,
            "{mode:?} centred something that was not a displayed equation:\n{md}"
        );
        assert!(md.contains("Before."), "{mode:?}: {md}");
        assert!(md.contains("see <svg"), "{mode:?}: {md}");
    }
}

/// The two non-drawing modes are left exactly as they were, and each for its
/// own reason.
///
/// `--katex` writes `$$…$$`, which a KaTeX reader centres itself
/// (`katex.css`'s `.katex-display { … text-align: center; }`) and which GitHub
/// only reads as math while the `$$` block stands alone — a `<div>` around it
/// would turn the equation into literal text. `--unicode-math` is the
/// plain-text mode: putting HTML in it would defeat the one property it exists
/// for.
#[test]
fn katex_and_unicode_display_equations_are_not_wrapped() {
    for mode in [
        rustyfi_html::MathMode::Katex,
        rustyfi_html::MathMode::Unicode,
    ] {
        let md = render_math(&[x_squared_plus_one()], mode);
        assert!(!md.contains("<div"), "{mode:?}: {md}");
    }
}

// ---------------------------------------------------------------------------
// `+align`: a `TabularBox` that is an equation, not a table
// ---------------------------------------------------------------------------
//
// The `math` package builds `+align` out of a `tabular`
// (`lib-rustyfi/dist/packages/math.satyh:541-574`), so an aligned equation and
// a spreadsheet arrive at this backend in the same box. The tests below are in
// two halves and the SECOND half is the important one: three shapes that must
// keep rendering as tables, so that "not a table" can never quietly become
// "no table anywhere".

/// One `+align` cell, built the way `math.satyh` builds it: the zero-width
/// padding skips, and an `inline-fil` on the side the content moves AWAY from
/// — leading in an even column (right-aligned), trailing in an odd one
/// (left-aligned).
fn align_cell(x: f64, y: f64, col: usize, text: &str) -> TabularCellBox {
    let even = col.is_multiple_of(2);
    math_cell(x, y, text, even, !even)
}

/// The same, with the two fils chosen explicitly — `(true, true)` is a CENTRED
/// cell, which is what a matrix is built out of.
fn math_cell(x: f64, y: f64, text: &str, fil_before: bool, fil_after: bool) -> TabularCellBox {
    let mut contents = vec![(Length::ZERO, PureHorzBox::FixedEmpty { width: Length::ZERO })];
    if fil_before {
        contents.push((Length::ZERO, PureHorzBox::OuterFil));
    }
    contents.push((
        Length::ZERO,
        PureHorzBox::Math {
            width: Length::pt(5.0 * text.chars().count() as f64),
            height: Length::pt(10.0),
            depth: Length::pt(2.0),
            // Advanced by exactly one glyph width, so the glyphs abut and
            // `math.rs`'s inter-atom spacing rule adds nothing — what is under
            // test here is the GRID, and a stray space in the recovered text
            // would only obscure it.
            glyphs: text
                .chars()
                .enumerate()
                .map(|(i, c)| math_glyph(&c.to_string(), 5.0 * i as f64, 0.0, 10.0))
                .collect(),
            rules: Vec::new(),
        },
    ));
    if fil_after {
        contents.push((Length::ZERO, PureHorzBox::OuterFil));
    }
    contents.push((Length::ZERO, PureHorzBox::FixedEmpty { width: Length::ZERO }));
    TabularCellBox {
        x: Length::pt(x),
        baseline_y: Length::pt(y),
        contents,
    }
}

/// A two-row, two-column `+align`, exactly as `math.satyh` emits it: no rules,
/// every cell one equation, columns right/left.
fn aligned_equation() -> VertBox {
    let tab = TabularBox {
        width: Length::pt(120.0),
        height: Length::pt(40.0),
        depth: Length::ZERO,
        cells: vec![
            align_cell(0.0, 30.0, 0, "x+y"),
            align_cell(60.0, 30.0, 1, "=ab"),
            align_cell(0.0, 10.0, 0, "x-y"),
            align_cell(60.0, 10.0, 1, "=cd"),
        ],
        rules: Vec::new(),
    };
    line_of(vec![
        PureHorzBox::OuterFil,
        PureHorzBox::Tabular(tab),
        PureHorzBox::OuterFil,
    ])
}

/// **The defect.** `+align` came out as a GFM pipe table in every mode, and
/// the delimiter row landed after the FIRST row — so a two-row alignment
/// rendered as a ONE-row table whose column heading was an equation.
///
/// In the modes that draw or typeset the equation there must be no table at
/// all: those modes have already decided the alignment is the equation's
/// business, and a grid around it is an artefact of how `math.satyh` happens
/// to build the block.
#[test]
fn an_aligned_equation_is_not_a_table_in_the_drawing_modes() {
    for mode in [
        rustyfi_html::MathMode::SvgOutline,
        rustyfi_html::MathMode::SvgText,
        rustyfi_html::MathMode::Katex,
    ] {
        let md = render_math(&[aligned_equation()], mode);
        assert!(
            !md.contains('|'),
            "{mode:?} still wrote a pipe table:\n{md}"
        );
        assert!(
            !md.contains("---"),
            "{mode:?} still wrote a delimiter row:\n{md}"
        );
        // Both halves of both equations survive whatever the mode wrote them
        // as — "not a table" must not have become "not there".
        for part in ["x", "y", "a", "b", "c", "d"] {
            assert!(md.contains(part), "{mode:?} lost `{part}`:\n{md}");
        }
    }
}

/// `--katex` writes a real `\begin{aligned}`, which is not a guess: a cell
/// boundary in `+align` IS an alignment point, and the classifier has already
/// established that the columns run right/left — the pattern `aligned` is
/// defined as.
///
/// Asserted as SHAPE, never as transcription. What `crate::latex` writes for a
/// given run of glyphs is that module's business and improves over time; what
/// this test is for is that the grid becomes an environment, that the cell
/// boundary becomes the `&`, and that the row boundary becomes the `\\`.
#[test]
fn katex_writes_an_aligned_environment_with_the_cell_boundary_as_the_ampersand() {
    let md = render_math(&[aligned_equation()], rustyfi_html::MathMode::Katex);
    let md = md.trim();
    // ONE display block for the whole alignment, not one per row: the rows of
    // an `align` are one environment, and `\\` is what separates them.
    assert_eq!(md.matches("$$").count(), 2, "{md}");
    let body = md
        .strip_prefix("$$\\begin{aligned} ")
        .and_then(|b| b.strip_suffix(" \\end{aligned}$$"))
        .unwrap_or_else(|| panic!("not one `aligned` environment:\n{md}"));
    let rows: Vec<&str> = body.split(" \\\\ ").collect();
    assert_eq!(rows.len(), 2, "one `\\\\` per `+align` row:\n{md}");
    for row in rows {
        let halves: Vec<&str> = row.split(" & ").collect();
        assert_eq!(halves.len(), 2, "one `&` per cell boundary:\n{md}");
        assert!(
            halves.iter().all(|h| !h.trim().is_empty()),
            "neither half of a row is empty:\n{md}"
        );
    }
}

/// A drawing mode writes one BLOCK PER ROW, and the row's two cells are joined
/// into it — a row of `+align` is one equation split at the alignment point,
/// so `${x + y}` and `${= a^2}` belong in the same block.
///
/// Needs the bundled faces: with no font store both drawing modes degrade to
/// characters (`a_render_with_no_font_store_writes_characters_rather_than_an_empty_drawing`),
/// and this is the test that the DRAWING really comes out per row. Skips
/// loudly rather than failing on a checkout that has not run
/// `download-fonts.sh` — which is CI's own `build · clippy · test`.
#[test]
fn a_drawn_aligned_equation_is_one_block_per_row() {
    let Some(store) = mono_font_store() else {
        return;
    };
    let md = rustyfi_html::render_markdown_ttf_with(
        Some(&[aligned_equation()]),
        &store,
        &[],
        &DocExtras::default(),
        &[],
        &[],
        rustyfi_html::MathMode::SvgText,
    )
    .expect("markdown rendering must succeed");
    assert!(!md.contains('|'), "{md}");
    let blocks: Vec<&str> = md.trim().split("\n\n").collect();
    assert_eq!(blocks.len(), 2, "one block per row, got:\n{md}");
    for block in &blocks {
        assert_eq!(
            block.matches("<svg").count(),
            2,
            "each row keeps both of its cells:\n{md}"
        );
        // An aligned equation is display math too, so each row is centred on
        // the same terms as any other displayed equation.
        assert!(
            block.starts_with("<div align=\"center\">\n") && block.ends_with("\n</div>"),
            "each row is a centred block:\n{md}"
        );
    }
}

/// **The control that matters more than the fix.** A real table — text in the
/// cells — is still a GFM pipe table, with its first row as the header, in
/// every math mode. Nothing about the `+align` recovery may reach it.
#[test]
fn a_real_table_is_still_a_table_in_every_math_mode() {
    let cell = |x: f64, y: f64, text: &str| TabularCellBox {
        x: Length::pt(x),
        baseline_y: Length::pt(y),
        contents: vec![(Length::ZERO, text_run(text))],
    };
    let tab = TabularBox {
        width: Length::pt(120.0),
        height: Length::pt(40.0),
        depth: Length::ZERO,
        cells: vec![
            cell(0.0, 30.0, "h1"),
            cell(60.0, 30.0, "h2"),
            cell(0.0, 10.0, "a"),
            cell(60.0, 10.0, "b"),
        ],
        // No rules — `easytable`'s content half has none either, so this also
        // pins that the rule-less clause alone never claims a table.
        rules: Vec::new(),
    };
    for mode in [
        rustyfi_html::MathMode::SvgOutline,
        rustyfi_html::MathMode::SvgText,
        rustyfi_html::MathMode::Katex,
        rustyfi_html::MathMode::Unicode,
    ] {
        let md = render_math(
            &[line_of(vec![PureHorzBox::Tabular(tab.clone())])],
            mode,
        );
        assert_eq!(
            md.trim(),
            "| h1 | h2 |\n| --- | --- |\n| a | b |",
            "{mode:?}: {md}"
        );
    }
}

/// A grid of CENTRED equations is a MATRIX, not an alignment, and stays a
/// table.
///
/// The boundary is deliberate and it is where the signal stops being exact: a
/// matrix's meaning is carried by delimiters drawn outside the `tabular`,
/// where this classifier cannot see them, and `\begin{aligned}` would silently
/// restyle it. `satysfi-base`'s `math-ext.satyh:1585` and `azmath`'s
/// `matrices.satyh` both build one this way.
#[test]
fn a_grid_of_centred_equations_is_a_matrix_and_stays_a_table() {
    let tab = TabularBox {
        width: Length::pt(120.0),
        height: Length::pt(40.0),
        depth: Length::ZERO,
        cells: vec![
            math_cell(0.0, 30.0, "a", true, true),
            math_cell(60.0, 30.0, "b", true, true),
            math_cell(0.0, 10.0, "c", true, true),
            math_cell(60.0, 10.0, "d", true, true),
        ],
        rules: Vec::new(),
    };
    let md = render_math(
        &[line_of(vec![PureHorzBox::Tabular(tab)])],
        rustyfi_html::MathMode::Katex,
    );
    // Shape, not transcription: three lines, the delimiter after the FIRST —
    // the ordinary table rendering, untouched by any of this.
    let lines: Vec<&str> = md.trim().lines().collect();
    assert_eq!(lines.len(), 3, "still a header, a delimiter and a row:\n{md}");
    assert!(lines[0].starts_with("| $"), "{md}");
    assert_eq!(lines[1], "| --- | --- |", "{md}");
    assert!(lines[2].starts_with("| $"), "{md}");
    assert!(!md.contains("aligned"), "{md}");
}

/// A grid that DRAWS its own lines is asserting that it is a grid, whatever is
/// in the cells — so an otherwise `+align`-shaped tabular with rules stays a
/// table. `+align` passes `(fun _ _ -> [])`.
#[test]
fn a_ruled_grid_of_equations_stays_a_table() {
    let rule = GraphicsElem::Fill(
        Color::Gray(0.0),
        Path {
            subpaths: vec![Subpath {
                start: (Length::ZERO, Length::ZERO),
                segs: vec![PathSeg::Line((Length::pt(120.0), Length::ZERO))],
                closing: Closing::Line,
            }],
        },
    );
    let tab = TabularBox {
        width: Length::pt(120.0),
        height: Length::pt(40.0),
        depth: Length::ZERO,
        cells: vec![
            align_cell(0.0, 30.0, 0, "x"),
            align_cell(60.0, 30.0, 1, "=y"),
        ],
        rules: vec![rule],
    };
    let md = render_math(
        &[line_of(vec![PureHorzBox::Tabular(tab)])],
        rustyfi_html::MathMode::Katex,
    );
    assert!(md.contains('|'), "a ruled grid must stay a table:\n{md}");
}

/// `--unicode-math` is exempt from the diversion — it writes characters, and a
/// two-column text table is a defensible way to SHOW an alignment in plain
/// text — but the header-row promotion is a bug in any mode, so the grid gets
/// an EMPTY header row and no equation is promoted to a column heading.
///
/// GFM has no headerless table (the delimiter row is mandatory and always
/// follows row one), so an empty header is the only way to say it.
#[test]
fn unicode_math_keeps_the_grid_but_promotes_no_equation_to_a_header() {
    let md = render_math(&[aligned_equation()], rustyfi_html::MathMode::Unicode);
    let lines: Vec<&str> = md.trim().lines().collect();
    assert_eq!(
        lines.len(),
        4,
        "an empty header, the delimiter, and BOTH equations as data rows:\n{md}"
    );
    assert_eq!(lines[0], "|  |  |", "the header row is empty:\n{md}");
    assert_eq!(lines[1], "| --- | --- |", "{md}");
    // Shape, not transcription: what the cells SAY is `markdown::math`'s
    // business. What this pins is that neither row is the header and that both
    // cells of each survived.
    for line in &lines[2..] {
        assert!(!line.contains("---"), "no second delimiter row:\n{md}");
        let cells: Vec<&str> = line.trim_matches('|').split('|').collect();
        assert_eq!(cells.len(), 2, "{md}");
        assert!(cells.iter().all(|c| !c.trim().is_empty()), "{md}");
    }
}

/// The block structure must not depend on whether `download-fonts.sh` has been
/// run. With no font store a drawing mode degrades to characters, but that is
/// a degradation of one EQUATION's rendering, not of the document's shape — so
/// `+align` is still diverted, on the flag the user passed rather than on what
/// could be drawn.
#[test]
fn a_font_less_drawing_mode_still_diverts_the_alignment() {
    let md = render_math(&[aligned_equation()], rustyfi_html::MathMode::SvgText);
    assert!(!md.contains('|'), "{md}");
    let blocks: Vec<&str> = md.trim().split("\n\n").collect();
    assert_eq!(blocks.len(), 2, "one block per row:\n{md}");
    // Each row still carries BOTH of its cells — `b` and `d` are the second
    // cell of each row, and the letters are all any writer of characters can
    // drop or keep.
    assert!(blocks[0].contains('b'), "{md}");
    assert!(blocks[1].contains('d'), "{md}");
    // Characters, so nothing to centre: the wrapper is for drawings.
    assert!(!md.contains("<div"), "{md}");
}

// ---------------------------------------------------------------------------
// Escaping
// ---------------------------------------------------------------------------

/// The failure mode this whole format has to avoid: document text that
/// PARSES as markup. Each of these renders back as itself.
#[test]
fn document_text_that_looks_like_markup_is_escaped() {
    let md = render(&[
        text_line("a * b and _c_ and [d] and |e| and <f>"),
        VertBox::Skip(Length::pt(6.0)),
        text_line("- not a bullet"),
        VertBox::Skip(Length::pt(6.0)),
        text_line("# not a heading"),
        VertBox::Skip(Length::pt(6.0)),
        text_line("1. not an item"),
    ]);
    assert!(md.contains("a \\* b and \\_c\\_ and \\[d\\] and \\|e\\| and \\<f>"), "{md}");
    assert!(md.contains("\\- not a bullet"), "{md}");
    assert!(md.contains("\\# not a heading"), "{md}");
    assert!(md.contains("1\\. not an item"), "{md}");
}

/// Mid-sentence, those same characters are ordinary punctuation. Escaping
/// them everywhere would litter a manual full of version numbers and issue
/// references with backslashes nobody needs.
#[test]
fn punctuation_that_is_only_special_at_a_line_start_is_left_alone_mid_line() {
    let md = render(&[text_line("see issue #3, item 2. of 5")]);
    assert_eq!(md.trim(), "see issue #3, item 2. of 5", "{md}");
}

// ---------------------------------------------------------------------------
// Additivity
// ---------------------------------------------------------------------------

/// A hand-built `DocumentValue` that never populated `reflow_source` renders
/// nothing rather than panicking — the same guarantee the HTML backend makes.
#[test]
fn a_missing_reflow_source_renders_an_empty_document() {
    let md = rustyfi_html::render_markdown(
        None,
        &[],
        &DocExtras::default(),
        &[],
        &[],
        rustyfi_html::MathMode::SvgOutline,
    )
    .expect("must succeed");
    assert_eq!(md, "");
}

/// Render under a font store so `recover::is_monospace` has a family name to
/// read. Without one no face can be fixed-pitch and a code block degrades to
/// prose — the same degradation the HTML backend takes, and never what the
/// code-block tests mean to measure.
fn markdown_with_mono_font(vboxes: &[VertBox]) -> Option<String> {
    let store = mono_font_store()?;
    Some(
        rustyfi_html::render_markdown_ttf_with(
            Some(vboxes),
            &store,
            &[],
            &DocExtras::default(),
            &[],
            &[],
            rustyfi_html::MathMode::SvgOutline,
        )
        .expect("markdown rendering must succeed"),
    )
}

/// A two-face store: `FontKey(0)` proportional, `FontKey(1)` fixed-pitch.
///
/// Built from the repo's own bundled files, because
/// `crate::fonts::is_monospace_family` reads the family name out of the
/// FILE's `name` table — a synthesized stub would have no name to classify,
/// and the test would pass or fail for the wrong reason. The store's three
/// slots are regular/bold/oblique; LM Mono is installed in the bold slot
/// purely because that is the slot `FontKey(1)` resolves to.
/// `None` when the bundled faces are absent. `download-fonts.sh` fetches
/// them, and CI runs it for the fidelity and real-package jobs but NOT for
/// `build · clippy · test` — so a hard `expect` here fails the very job that
/// runs the unit tests, on a checkout that is perfectly valid. Every other
/// font-needing test in the workspace skips instead (`reflow.rs`,
/// `math_table.rs`, `ttf.rs`, ...).
fn mono_font_store() -> Option<rustyfi_pdf::TtfFontStore> {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi/dist/fonts");
    let (Ok(serif), Ok(mono)) = (
        std::fs::read(dir.join("Junicode.ttf")),
        std::fs::read(dir.join("lmmono10-regular.otf")),
    ) else {
        eprintln!(
            "skipping: the bundled faces are not in {} \u{2014} run download-fonts.sh",
            dir.display(),
        );
        return None;
    };
    let store = rustyfi_pdf::TtfFontStore::from_bytes(serif, Some(mono), None, "test fonts")
        .expect("both bundled faces must parse");
    assert!(
        store
            .file_family_name(store.file_index(FontKey(1)))
            .is_some_and(|f| f.to_ascii_lowercase().contains("mono")),
        "FontKey(1) must resolve to a face whose family name reads as fixed-pitch",
    );
    Some(store)
}

/// A drawing is emitted as the drawing, not as a named hole.
///
/// Three properties, and all three are load-bearing:
///
/// - it is an `<svg>` carrying the real path, so a reader sees the figure;
/// - it is on ONE line, because a Markdown paragraph is one line and a raw
///   `<svg>` split across lines has blank lines and `<br>` inserted into the
///   middle of it by the reader's own parser;
/// - it carries no `position:absolute`, which the reflow backend's
///   `svg::emit_graphics` does — in a Markdown file there is no positioned
///   ancestor, so an absolute drawing lands on top of the prose.
#[test]
fn a_drawing_becomes_a_one_line_svg_with_no_positioning() {
    let square = Path {
        subpaths: vec![Subpath {
            start: (Length::pt(0.0), Length::pt(0.0)),
            segs: vec![
                PathSeg::Line((Length::pt(30.0), Length::pt(0.0))),
                PathSeg::Line((Length::pt(30.0), Length::pt(20.0))),
            ],
            closing: Closing::Line,
        }],
    };
    let md = render(&[line_of(vec![PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(30.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        elems: vec![GraphicsElem::Fill(Color::Rgb(1.0, 0.0, 0.0), square)],
    }])]);
    assert!(md.contains("<svg"), "no <svg> for a drawing:\n{md}");
    assert!(md.contains("<path"), "the <svg> carries no path:\n{md}");
    assert!(
        md.contains("rgb(255,0,0)"),
        "the drawing lost its colour:\n{md}",
    );
    assert!(
        !md.contains("[graphic]"),
        "the named hole survived alongside the drawing:\n{md}",
    );
    assert!(
        !md.contains("position:absolute"),
        "a Markdown file has no positioned ancestor:\n{md}",
    );
    let svg = md
        .lines()
        .find(|l| l.contains("<svg"))
        .expect("the <svg> is on a line of its own");
    assert!(
        svg.contains("</svg>"),
        "the <svg> is split across lines, so a Markdown parser will break it:\n{md}",
    );
}

/// ...but a hairline is not a drawing. The corpus draws rules, leader dots and
/// underline strokes as one-off graphics, and an `<svg>` for each would bury
/// the figures that matter under punctuation.
#[test]
fn a_hairline_rule_is_still_dropped_rather_than_drawn() {
    // A FILLED 200pt x 0.4pt bar, not a zero-height line: the drawing has to
    // have real ink for the size threshold to be what rejects it. With a
    // degenerate bbox this test would pass with the threshold removed, which
    // is the whole thing it exists to catch.
    let hair = Path {
        subpaths: vec![Subpath {
            start: (Length::pt(0.0), Length::pt(0.0)),
            segs: vec![
                PathSeg::Line((Length::pt(200.0), Length::pt(0.0))),
                PathSeg::Line((Length::pt(200.0), Length::pt(0.4))),
                PathSeg::Line((Length::pt(0.0), Length::pt(0.4))),
            ],
            closing: Closing::Line,
        }],
    };
    let md = render(&[line_of(vec![PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(200.0),
        height: Length::pt(0.4),
        depth: Length::ZERO,
        elems: vec![GraphicsElem::Fill(Color::Rgb(0.0, 0.0, 0.0), hair)],
    }])]);
    assert!(
        !md.contains("<svg"),
        "a 0.4pt rule was drawn as a figure:\n{md}",
    );
}
