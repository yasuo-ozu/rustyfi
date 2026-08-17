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

/// `register-document-information`'s payload (prim-retype-sweep §2.4;
/// upstream `tDOCINFODIC` = `document-information-dictionary`,
/// `dev-0-1-0:src/frontend/primitives.cppo.ml:98-107`): `/Info` dictionary
/// fields. Structural on the language side (`rustyfi-lang`'s
/// `t_doc_info_dictionary()` reuses the `t_pbinfo` closed-record
/// precedent, not a nominal synonym type) — this struct is just the plain
/// Rust value both PDF writers read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocInfo {
    pub title: Option<String>,
    pub subject: Option<String>,
    pub author: Option<String>,
    /// Joined with a single space at emission time (upstream
    /// `String.concat " "`, `documentInformationDictionary.ml`), only if
    /// non-empty.
    pub keywords: Vec<String>,
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
    /// `register-document-information`'s registered value, `None` when
    /// never called (both PDF writers gate the whole `/Info` dict emission
    /// on this — every pre-L5a document stays byte-identical, prim-retype-
    /// sweep §2.4 step 5).
    pub doc_info: Option<DocInfo>,
}
