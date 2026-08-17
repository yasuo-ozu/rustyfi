//! Page breaking. Milestone 1: single column, break when the text area
//! overflows (a subset of pageBreak.ml's `main`).

use crate::context::PageGeometry;
use crate::hbox::PureHorzBox;
use crate::length::Length;
use crate::vbox::VertBox;

/// One typeset line placed on a page, in page coordinates (y grows downward
/// from the paper top; the PDF writer flips it).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedLine {
    pub x: Length,
    pub baseline_y: Length,
    pub contents: Vec<(Length, PureHorzBox)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub lines: Vec<PlacedLine>,
}

/// Flow vertical boxes into pages. `leading` is the baseline-to-baseline
/// distance (taken from the context that produced the lines).
pub fn break_pages(geom: &PageGeometry, leading: Length, vboxes: Vec<VertBox>) -> Vec<Page> {
    let (x0, y0) = geom.text_origin;
    let y_limit = y0 + geom.text_height;

    let mut pages = Vec::new();
    let mut page = Page::default();
    let mut prev_baseline: Option<Length> = None;
    let mut pending_skip = Length::ZERO;

    for vbox in vboxes {
        match vbox {
            VertBox::Skip(l) => pending_skip += l,
            VertBox::Line {
                height,
                depth,
                contents,
            } => {
                let mut baseline = match prev_baseline {
                    None => y0 + pending_skip + height,
                    Some(b) => b + leading.max(height) + pending_skip,
                };
                if baseline + depth > y_limit && !page.lines.is_empty() {
                    pages.push(std::mem::take(&mut page));
                    baseline = y0 + height;
                }
                pending_skip = Length::ZERO;
                prev_baseline = Some(baseline);
                page.lines.push(PlacedLine {
                    x: x0,
                    baseline_y: baseline,
                    contents,
                });
            }
        }
    }
    if !page.lines.is_empty() || pages.is_empty() {
        pages.push(page);
    }
    pages
}
