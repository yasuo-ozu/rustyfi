//! Document-level PDF extras: link annotations, named destinations, the
//! outline (bookmark) tree, and per-page decoration-graphics overlays.
//! Pure data — accumulated lang-side (`Interp`, during `fire_hooks`),
//! carried on `DocumentValue::extras`, emitted by both PDF writers.
//! Upstream: annotation.ml / namedDest.ml / outline.ml.

use crate::graphics::{Color, GraphicsElem};
use crate::length::Length;

/// One `/Annots` entry (upstream `Annotation.t` + its rect/border payload).
#[derive(Clone, Debug, PartialEq)]
pub struct Annot {
    /// 0-based page index the annotation is attached to.
    pub page: usize,
    /// `(x1, y1, x2, y2)` in PDF points, y-up, lower-left/upper-right —
    /// already `annotation.ml:22`'s `(x, y - dpt, x + wid, y + hgt)`.
    pub rect: (Length, Length, Length, Length),
    pub action: AnnotAction,
    /// `/Border` width + `/C` color (upstream `borderopt`); `None` ⇒
    /// `/Border [0 0 0]` (upstream emits a zero border, NOT the PDF default).
    pub border: Option<(Length, Color)>,
}

/// The link action (upstream `Pdfaction.Uri` / `Pdfaction.GotoName`).
#[derive(Clone, Debug, PartialEq)]
pub enum AnnotAction {
    Uri(String),
    GotoName(String),
}

/// One named destination (upstream `NamedDest`: `(name, (x, y), pageno)`),
/// emitted as `/Dests { name: [page /XYZ x y 0] }`.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedDest {
    pub page: usize, // 0-based
    pub name: String,
    pub x: Length, // PDF points, y-up
    pub y: Length,
}

/// One outline entry (upstream `Outline`: `(level, text, key, isopen)` with
/// the key already resolved to a `/Dests` name via `NamedDest.get`).
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineEntry {
    pub level: i64,
    pub text: String,
    pub dest_name: String,
    pub is_open: bool,
}

/// Everything the PDF writers need beyond `pages`/`images`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocExtras {
    pub annotations: Vec<Annot>,
    pub destinations: Vec<NamedDest>,
    pub outline: Vec<OutlineEntry>,
    /// One overlay per page (may be shorter than `pages`; missing = empty):
    /// deco graphics fired at placement time, absolute PDF y-up coordinates,
    /// drawn UNDER the page's text (background fills/borders).
    pub page_graphics: Vec<Vec<GraphicsElem>>,
}
