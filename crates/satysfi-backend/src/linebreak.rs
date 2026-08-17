//! Paragraph breaking. Milestone 1: greedy first-fit with glue justification.
//! The input model (a flat `Vec<HorzBox>` of strings and glue) matches what
//! lineBreak.ml consumes, so the phase-6 Knuth–Plass port replaces only this
//! function's internals.

use crate::context::Context;
use crate::hbox::{HorzBox, PureHorzBox};
use crate::length::Length;
use crate::vbox::VertBox;

/// Break a paragraph's boxes into justified lines.
pub fn break_into_lines(ctx: &Context, boxes: Vec<HorzBox>) -> Vec<VertBox> {
    let pure: Vec<PureHorzBox> = boxes.into_iter().map(|HorzBox::Pure(p)| p).collect();
    let width = ctx.paragraph_width;

    // Split greedily at glue: take boxes until the natural width would
    // overflow, then break at the last glue seen.
    let mut lines: Vec<Vec<PureHorzBox>> = Vec::new();
    let mut current: Vec<PureHorzBox> = Vec::new();
    let mut current_width = Length::ZERO;
    let mut last_glue: Option<usize> = None;

    for bx in pure {
        let w = bx.natural_width();
        if current_width + w > width && last_glue.is_some() && !bx.is_glue() {
            let glue_idx = last_glue.unwrap();
            let rest: Vec<PureHorzBox> = current.drain(glue_idx..).skip(1).collect();
            lines.push(std::mem::take(&mut current));
            current = rest;
            current_width = current.iter().map(|b| b.natural_width()).fold(
                Length::ZERO,
                |acc, w| acc + w,
            );
            last_glue = None;
        }
        if bx.is_glue() {
            // A break opportunity — but never at the very start of a line.
            if current.is_empty() {
                continue;
            }
            last_glue = Some(current.len());
        }
        current_width += bx.natural_width();
        current.push(bx);
    }
    if !current.is_empty() {
        lines.push(current);
    }

    let line_count = lines.len();
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| layout_line(ctx, line, width, i + 1 == line_count))
        .collect()
}

/// Assign x offsets, justifying interior lines by distributing slack into
/// glue (`OuterFil` absorbs everything; otherwise stretchables share it).
fn layout_line(ctx: &Context, mut line: Vec<PureHorzBox>, width: Length, is_last: bool) -> VertBox {
    // Trailing glue never justifies anything.
    while line.last().is_some_and(|b| b.is_glue() && !matches!(b, PureHorzBox::OuterFil)) {
        line.pop();
    }

    let natural: Length = line
        .iter()
        .map(|b| b.natural_width())
        .fold(Length::ZERO, |acc, w| acc + w);
    let slack = width - natural;

    let fil_count = line
        .iter()
        .filter(|b| matches!(b, PureHorzBox::OuterFil))
        .count();
    let stretch_total: Length = line
        .iter()
        .map(|b| match b {
            PureHorzBox::OuterEmpty { stretchable, .. } => *stretchable,
            _ => Length::ZERO,
        })
        .fold(Length::ZERO, |acc, w| acc + w);

    let mut x = Length::ZERO;
    let mut contents = Vec::with_capacity(line.len());
    let mut height = Length::ZERO;
    let mut depth = Length::ZERO;

    for bx in line {
        let advance = match &bx {
            PureHorzBox::InnerString {
                width,
                height: h,
                depth: d,
                ..
            } => {
                height = height.max(*h);
                depth = depth.max(*d);
                *width
            }
            PureHorzBox::OuterEmpty {
                natural,
                stretchable,
                ..
            } => {
                let mut adv = *natural;
                if fil_count == 0 && slack.is_positive() && stretch_total.is_positive() && !is_last
                {
                    adv += slack * (*stretchable / stretch_total);
                }
                adv
            }
            PureHorzBox::OuterFil => {
                if fil_count > 0 && slack.is_positive() {
                    slack * (1.0 / fil_count as f64)
                } else {
                    Length::ZERO
                }
            }
        };
        contents.push((x, bx));
        x += advance;
    }

    // An all-glue line still needs sane metrics.
    if height == Length::ZERO && depth == Length::ZERO {
        height = ctx.font_size * 0.75;
        depth = ctx.font_size * 0.25;
    }

    VertBox::Line {
        height,
        depth,
        contents,
    }
}
