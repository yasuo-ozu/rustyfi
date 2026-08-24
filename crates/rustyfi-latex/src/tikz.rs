//! `GraphicsElem` → TikZ, the LaTeX counterpart of `rustyfi-html`'s `svg`.
//!
//! **This is the one place LaTeX is a BETTER target than the other two
//! backends have.** A drawing in the box stream is already a list of vector
//! paths — the same `Fill`/`Stroke`/`DashedStroke`/`Clip` elements the PDF
//! writer strokes and fills — so nothing has to be rasterized, approximated
//! or dropped: TikZ draws exactly those paths, and `\begin{tikzpicture}` is
//! ordinary LaTeX that any engine compiles. The Markdown backend emits an
//! `<svg>` and hopes the reader's renderer passes HTML through; here the
//! figure is a first-class part of the document.
//!
//! ## Coordinates need no transform at all
//!
//! `GraphicsElem`'s `Path` coordinates are box-local and y-**up** from the
//! box's baseline-left origin (`graphics.rs`'s `Point` doc comment), and
//! TikZ's are box-local and y-up too. So unlike the SVG writer — which needs
//! a `<g transform="translate(0,h) scale(1,-1)">` because SVG page space is
//! y-down — this module writes the numbers out as they stand. The single
//! declaration that makes that true is `[x=1bp, y=1bp]` on the
//! `tikzpicture`.
//!
//! **`bp`, not `pt`, and that is not a detail.** A `Length` in this port is
//! 1/72 inch — `length.rs`'s `from_unit` maps `"inch"` to `72.0` — because
//! that is PDF user space, which is what the PDF writer emits into
//! unchanged. TeX's own `pt` is 1/72.27 inch; its `bp` ("big point") is
//! 1/72. So every number this backend writes carries `bp`, and a drawing
//! comes out the size the layout measured rather than 0.37% too large.
//!
//! `baseline=0bp` then puts the picture's own baseline at y=0, which is the
//! box's baseline — so a drawing set into a line of prose sits where the
//! layout put it rather than on the baseline of its own bottom edge.
//!
//! ## Colour is EXACT here, and it is not in the SVG writer
//!
//! `xcolor` has a real `cmyk` model, so a `Cmyk` colour is declared as CMYK
//! and left for the driver to separate. `rustyfi-html`'s `svg::css_color` has to
//! apply the naive `(1-c)(1-k)` conversion because CSS has no device-CMYK at
//! all, and says so; there is nothing to apologise for on this path.

use std::fmt::Write as _;

use rustyfi_backend::{Closing, Color, GraphicsElem, Length, Path, PathSeg, PureHorzBox};

/// The label emitter: a `draw-text` run's nested boxes, rendered as inline
/// LaTeX for a `\node`. Returning an empty string drops the node.
pub(super) type LabelEmitter<'a> = &'a dyn Fn(&[(Length, PureHorzBox)]) -> String;

/// One drawing as a self-contained `tikzpicture`, or `None` when it has no
/// bounding box (nothing to draw).
///
/// The caller decides whether the drawing is big enough to be worth showing —
/// see `inline.rs`'s ink-size threshold, which is what keeps every heading
/// rule and leader dot in the corpus from becoming its own picture.
/// `scale` is `Some(k)` for a drawing too big for the text area — see
/// [`fit_scale`], which is where the reason it cannot simply be left to
/// overflow is.
pub(super) fn graphics_block(
    elems: &[GraphicsElem],
    scale: Option<f64>,
    label: LabelEmitter<'_>,
) -> Option<String> {
    if elems.is_empty() {
        return None;
    }
    let mut body = String::new();
    emit_elems(&mut body, elems, label);
    if body.trim().is_empty() {
        return None;
    }
    // `transform shape` so a `\node`'s TEXT shrinks with the drawing around
    // it: `scale` alone moves the nodes and leaves them full size, which for
    // a figure whose labels are half its content is worse than not scaling.
    let opts = match scale {
        Some(k) => format!(",scale={k:.4},transform shape"),
        None => String::new(),
    };
    Some(format!(
        "\\begin{{tikzpicture}}[x=1bp,y=1bp,baseline=0bp{opts}]\n{body}\\end{{tikzpicture}}"
    ))
}

/// The fraction of the text HEIGHT an oversized drawing is fitted into — see
/// [`fit_scale`], where the missing 10% is accounted for.
const FIT_MARGIN: f64 = 0.9;

/// How much a `w` x `h` drawing has to shrink to fit a `tw` x `th` text area,
/// or `None` when it already does.
///
/// **A drawing that does not fit does not merely overflow — it hangs the page
/// builder.** A `tikzpicture` is one unbreakable box; LaTeX responds to a box
/// taller than `\textheight` by ending the page and trying again on the next
/// one, forever. `slydifi`'s first slide is a full-bleed 405bp drawing on a
/// 405bp page, and before the geometry was corrected it produced 131072 empty
/// pages and then died of `dest_names_size`. The geometry fix removes that
/// particular 67bp shortfall; this removes the general case, which is any
/// document whose figure is simply bigger than its measure.
///
/// Scaled proportionally, and only ever DOWN: a drawing smaller than the
/// measure is left at the size the document chose.
///
/// **The HEIGHT is fitted to [`FIT_MARGIN`] of the area rather than to all of
/// it**, and that slack is what makes the difference between compiling and
/// not. `slydifi`'s background is 405.07bp on a 388.30bp text height: scaled
/// to exactly 1.0000 of it the picture is 389.75pt in a 389.75pt
/// `\textheight` and it STILL loops, because a page holds more than its
/// boxes — `\topskip` above the first, the interline glue below it, and the
/// paragraph that follows all have to go somewhere. Nothing this side of the
/// engine knows those numbers (they depend on the class's `\baselineskip`,
/// which depends on a font this writer never loads), so the honest answer is
/// a margin rather than an arithmetic.
///
/// **The WIDTH takes no such margin**, because there is no loop to avoid
/// horizontally — an over-wide box is an overfull `\hbox` warning and prints
/// anyway. Applying the same 10% there shrank every full-measure rule in the
/// corpus by a tenth: `code-printer`'s title is underlined by a 440bp rule on
/// a 440bp measure, which fits exactly and should be drawn exactly. What
/// makes "exactly" true is `block.rs`'s `\noindent`, without which the
/// paragraph indent would push it into the margin.
pub(super) fn fit_scale(w: f64, h: f64, tw: f64, th: f64) -> Option<f64> {
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let th = th * FIT_MARGIN;
    let mut k: f64 = 1.0;
    if tw > 0.0 && w > tw {
        k = k.min(tw / w);
    }
    if th > 0.0 && h > th {
        k = k.min(th / h);
    }
    (k < 1.0).then_some(k)
}

/// A correctly-sized, clearly-marked stand-in for something this backend
/// cannot draw — an embedded raster image or an imported PDF page.
///
/// **Why a placeholder and not the picture.** A compile produces ONE output
/// path (`-o out.tex`), so writing the images out as sidecar files would be a
/// contract the CLI does not have and would break the moment the `.tex` is
/// moved or mailed. LaTeX has no data-URI equivalent — the Markdown backend's
/// answer — because `\includegraphics` reads a FILE and nothing else. So the
/// figure leaves a named hole at exactly the size it occupied, which keeps
/// the surrounding page layout honest and tells a reader what to go and find.
///
/// Drawn with TikZ rather than an `\fbox`: a framed box has to be sized with
/// a `\parbox` whose content can overflow it, and a node inside a picture of
/// the right size cannot change that size whatever the caption says.
pub(super) fn placeholder(width: f64, height: f64, label: &str) -> String {
    let (w, h) = (width.max(1.0), height.max(1.0));
    format!(
        "\\begin{{tikzpicture}}[x=1bp,y=1bp,baseline=0bp]\n\
         \\draw[gray,dashed] (0,0) rectangle ({w:.3},{h:.3});\n\
         \\node[gray,font=\\scriptsize] at ({:.3},{:.3}) {{{label}}};\n\
         \\end{{tikzpicture}}",
        w / 2.0,
        h / 2.0,
    )
}

fn emit_elems(out: &mut String, elems: &[GraphicsElem], label: LabelEmitter<'_>) {
    for elem in elems {
        match elem {
            // Even-odd fill, matching the PDF writer's `content.fill_even_odd()`
            // (upstream `op_f'`, not nonzero winding).
            GraphicsElem::Fill(color, path) => {
                define_color(out, *color);
                let _ = writeln!(
                    out,
                    "\\fill[rustyficlr,even odd rule] {};",
                    path_tikz(path)
                );
            }
            GraphicsElem::Stroke(width, color, path) => {
                define_color(out, *color);
                let _ = writeln!(
                    out,
                    "\\draw[rustyficlr,line width={:.4}bp] {};",
                    width.0,
                    path_tikz(path)
                );
            }
            // `dashed-stroke`: `Stroke` plus PostScript's dash array, which
            // `dash pattern`/`dash phase` say directly — the TikZ analogue of
            // the PDF writer's `set_dash_pattern`.
            GraphicsElem::DashedStroke(width, dash, color, path) => {
                define_color(out, *color);
                let _ = writeln!(
                    out,
                    "\\draw[rustyficlr,line width={:.4}bp,dash pattern=on {:.4}bp off {:.4}bp,\
                     dash phase={:.4}bp] {};",
                    width.0,
                    dash.0 .0.max(0.01),
                    dash.1 .0.max(0.0),
                    dash.2 .0,
                    path_tikz(path)
                );
            }
            // `draw-text`: content placed at a point inside the drawing.
            //
            // This is the arm SVG cannot have — an HTML element inside
            // `<svg>` ends the parser's foreign-content mode, so
            // `svg::graphics_block` DROPS these and leaves its caller to
            // flow the text separately. A TikZ `\node` is ordinary LaTeX
            // inside an ordinary picture, so the label stays where the
            // document put it: `\overset`, `\underset` and every
            // big-operator-with-limits in `latexcmds` is an
            // `inline-graphics` of two or three of these, and flowing them
            // puts a limit BEFORE its operator.
            //
            // `anchor=base west` because the recorded point is the run's
            // left edge and its BASELINE, and `inner sep=0pt` so the node
            // adds no padding of its own to a position the layout already
            // measured.
            GraphicsElem::Text { pt, contents, .. } => {
                let body = label(contents);
                if !body.trim().is_empty() {
                    let _ = writeln!(
                        out,
                        "\\node[anchor=base west,inner sep=0pt] at ({:.3},{:.3}) {{{body}}};",
                        pt.0 .0, pt.1 .0,
                    );
                }
            }
            // 0.1's `graphics` collection container. No transform of its
            // own, so a plain scope keeps any styling local without moving
            // anything.
            GraphicsElem::Group(inner) => emit_elems(out, inner, label),
            GraphicsElem::Clip(path, inner) => {
                let _ = writeln!(
                    out,
                    "\\begin{{scope}}\n\\clip[even odd rule] {};",
                    path_tikz(path)
                );
                emit_elems(out, inner, label);
                out.push_str("\\end{scope}\n");
            }
            // Not ink: a deferred `register-destination` marker, already
            // consumed by `fire_hooks` into `DocExtras::destinations`. The
            // anchor is emitted by the surrounding writer.
            GraphicsElem::Destination { .. } => {}
        }
    }
}

/// Declare the colour the next path uses, under one reused name.
///
/// `\definecolor` inside a `tikzpicture` is local to it and silently
/// redefines, so one name is enough and the alternative — a unique name per
/// element — would fill the file with `\definecolor{rustyficlr17}` for no
/// gain.
fn define_color(out: &mut String, color: Color) {
    match color {
        Color::Gray(v) => {
            let _ = writeln!(out, "\\definecolor{{rustyficlr}}{{gray}}{{{:.4}}}", clamp(v));
        }
        Color::Rgb(r, g, b) => {
            let _ = writeln!(
                out,
                "\\definecolor{{rustyficlr}}{{rgb}}{{{:.4},{:.4},{:.4}}}",
                clamp(r),
                clamp(g),
                clamp(b)
            );
        }
        // Exact, unlike the SVG writer's — see this module's doc comment.
        Color::Cmyk(c, m, y, k) => {
            let _ = writeln!(
                out,
                "\\definecolor{{rustyficlr}}{{cmyk}}{{{:.4},{:.4},{:.4},{:.4}}}",
                clamp(c),
                clamp(m),
                clamp(y),
                clamp(k)
            );
        }
    }
}

fn clamp(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// One `Path`'s subpaths as a TikZ path specification.
///
/// `PathSeg::Line` → `--`, `PathSeg::Bezier` → `.. controls … and … ..`,
/// `Closing::Line` → `-- cycle`, `Closing::Bezier` → one final curve back to
/// the subpath's own start and then `cycle` — the direct analogue of the PDF
/// writer's `m`/`l`/`c`/`h` and of the SVG writer's `M`/`L`/`C`/`Z`.
///
/// Several subpaths ride in ONE path specification, which TikZ allows: a new
/// coordinate with no operator before it starts a new component, and a single
/// `\fill` over all of them is what makes the even-odd rule mean what the PDF
/// writer means by it (a two-subpath fill is how every ring-shaped figure in
/// the corpus is drawn, and filling them separately would black out the
/// hole).
fn path_tikz(path: &Path) -> String {
    let mut out = String::new();
    for sub in &path.subpaths {
        let _ = write!(out, "({:.3},{:.3})", bp(sub.start.0 .0), bp(sub.start.1 .0));
        for seg in &sub.segs {
            match seg {
                PathSeg::Line(pt) => {
                    let _ = write!(out, " -- ({:.3},{:.3})", bp(pt.0 .0), bp(pt.1 .0));
                }
                PathSeg::Bezier(c1, c2, dest) => {
                    let _ = write!(
                        out,
                        " .. controls ({:.3},{:.3}) and ({:.3},{:.3}) .. ({:.3},{:.3})",
                        bp(c1.0 .0),
                        bp(c1.1 .0),
                        bp(c2.0 .0),
                        bp(c2.1 .0),
                        bp(dest.0 .0),
                        bp(dest.1 .0),
                    );
                }
            }
        }
        match sub.closing {
            Closing::Open => {}
            Closing::Line => out.push_str(" -- cycle"),
            Closing::Bezier(c1, c2) => {
                let _ = write!(
                    out,
                    " .. controls ({:.3},{:.3}) and ({:.3},{:.3}) .. cycle",
                    bp(c1.0 .0),
                    bp(c1.1 .0),
                    bp(c2.0 .0),
                    bp(c2.1 .0),
                );
            }
        }
        out.push(' ');
    }
    out.trim_end().to_string()
}

/// One coordinate, as TikZ will read it.
///
/// **A non-finite coordinate is pinned to 0 rather than printed.** `{:.3}` on
/// an `f64` writes `NaN` or `inf`; TikZ then tries to read that as a
/// dimension and stops, with an error naming neither the drawing nor the
/// document — so one degenerate control point costs the whole file. A
/// `Length` here is the evaluator's own arithmetic, and a division by a
/// zero length is all it takes to produce one. `fit_scale` does not catch
/// these: it guards the drawing's overall BOX, and a `NaN` slips through the
/// bounding box too, because `f64::min(NaN, x)` is `x`.
///
/// Pinned rather than dropped: dropping the path loses the figure, while
/// pinning loses one point of it and leaves something a reader can see is
/// wrong. Both beat a document that does not compile.
fn bp(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyfi_backend::Subpath;

    fn square() -> Path {
        Path {
            subpaths: vec![Subpath {
                start: (Length::pt(0.0), Length::pt(0.0)),
                segs: vec![
                    PathSeg::Line((Length::pt(10.0), Length::pt(0.0))),
                    PathSeg::Bezier(
                        (Length::pt(10.0), Length::pt(5.0)),
                        (Length::pt(5.0), Length::pt(10.0)),
                        (Length::pt(0.0), Length::pt(10.0)),
                    ),
                ],
                closing: Closing::Line,
            }],
        }
    }

    /// The coordinates go out as they stand — no flip, unlike the SVG
    /// writer, because TikZ is y-up too.
    #[test]
    fn a_path_maps_segment_for_segment_with_no_transform() {
        assert_eq!(
            path_tikz(&square()),
            "(0.000,0.000) -- (10.000,0.000) .. controls (10.000,5.000) and (5.000,10.000) .. \
             (0.000,10.000) -- cycle"
        );
    }

    #[test]
    fn a_fill_declares_its_colour_and_the_even_odd_rule() {
        let nothing = |_: &[(Length, PureHorzBox)]| String::new();
        let tex = graphics_block(
            &[GraphicsElem::Fill(Color::Rgb(1.0, 0.0, 0.5), square())],
            None,
            &nothing,
        )
        .expect("a path is something to draw");
        assert!(tex.contains("\\definecolor{rustyficlr}{rgb}{1.0000,0.0000,0.5000}"), "{tex}");
        assert!(tex.contains("\\fill[rustyficlr,even odd rule]"), "{tex}");
        // One unit is one point, and the baseline is the box's own.
        assert!(tex.starts_with("\\begin{tikzpicture}[x=1bp,y=1bp,baseline=0bp]"), "{tex}");
        assert!(tex.ends_with("\\end{tikzpicture}"), "{tex}");
    }

    /// CMYK survives as CMYK. The SVG writer has to convert it and loses
    /// information doing so; `xcolor` has the model natively.
    #[test]
    fn cmyk_is_not_converted_away() {
        let nothing = |_: &[(Length, PureHorzBox)]| String::new();
        let tex = graphics_block(
            &[GraphicsElem::Fill(Color::Cmyk(0.1, 0.2, 0.3, 0.4), square())],
            None,
            &nothing,
        )
        .unwrap();
        assert!(
            tex.contains("\\definecolor{rustyficlr}{cmyk}{0.1000,0.2000,0.3000,0.4000}"),
            "{tex}"
        );
    }

    /// A `draw-text` label keeps its point, which is the whole reason this
    /// backend can draw one at all — see the `Text` arm.
    #[test]
    fn a_draw_text_run_becomes_a_node_at_its_own_point() {
        let label = |_: &[(Length, PureHorzBox)]| "hello".to_string();
        let tex = graphics_block(
            &[GraphicsElem::Text {
                pt: (Length::pt(3.0), Length::pt(-2.0)),
                contents: Vec::new(),
                width: Length::pt(10.0),
                height: Length::pt(8.0),
                depth: Length::ZERO,
                transform: None,
            }],
            None,
            &label,
        )
        .unwrap();
        assert!(
            tex.contains("\\node[anchor=base west,inner sep=0pt] at (3.000,-2.000) {hello};"),
            "{tex}"
        );
    }

    /// A drawing with nothing in it is `None`, not an empty picture — an
    /// empty `tikzpicture` still reserves its own `\baselineskip`.
    #[test]
    fn an_empty_drawing_is_declined() {
        let nothing = |_: &[(Length, PureHorzBox)]| String::new();
        assert!(graphics_block(&[], None, &nothing).is_none());
        // …and so is one whose only element draws nothing.
        assert!(graphics_block(
            &[GraphicsElem::Destination {
                key: "d".into(),
                pt: (Length::ZERO, Length::ZERO)
            }],
            None,
            &nothing
        )
        .is_none());
    }
}
