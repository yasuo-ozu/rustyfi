//! One generated, exhaustive traversal of the placed-box tree.
//!
//! # Why this exists
//!
//! The box tree is a mutually recursive graph — a `PureHorzBox::Tabular`
//! holds cells that hold boxes, a `PureHorzBox::EmbeddedBlock` holds
//! `VertBox`es that hold boxes, a `GraphicsElem::Text` (a `draw-text` run)
//! holds boxes, and `GraphicsElem::Group`/`Clip` hold more graphics — and
//! consumers all over the port walk it looking for one thing: which images
//! are used, which decorations to fire, where a marker sits. Every one of
//! those walks used to be hand-written, and the recurring bug in this port is
//! a hand-written walk that forgets one container. That failure is **silent**:
//! the walker returns fewer results, the page renders with content quietly
//! missing, and nothing anywhere reports an error.
//!
//! The traversal below is derived from the type definitions themselves, so it
//! cannot forget a variant. Add a variant carrying boxes and every consumer
//! written against this module descends into it on the next build.
//!
//! # Using it
//!
//! Every visited type gains inherent `visit` / `visit_mut` methods taking a
//! closure (or a tuple of closures, one per node type, for a single pass):
//!
//! ```
//! use rustyfi_backend::{Length, PureHorzBox, VertBox};
//! # let page: Vec<VertBox> = Vec::new();
//! let mut n = 0usize;
//! for vb in &page {
//!     vb.visit(|_: &PureHorzBox| n += 1);
//! }
//! # let _ = (n, Length::ZERO);
//! ```
//!
//! A visit is **pre-order and inclusive**: `b.visit(|x: &PureHorzBox| ..)`
//! sees `b` itself before its children. Descent is unconditional, so a
//! consumer that wants to prune (say, to skip an inert `Footnote` marker's
//! payload) must implement the generated [`Visit`] trait by hand and decline
//! to call back into `visit_pure_horz_box` for that arm — a closure cannot.
//!
//! # The one trap
//!
//! `visitor!` follows a field only if the field's *peeled* head type is named
//! in the owning type's `#[subast(..)]` list. When those two fall out of step
//! — a field is rewrapped in a container the peel cannot see through, or a
//! new box-carrying field is added and the list is not extended — the field
//! is silently reclassified a leaf and the generated body for it is empty.
//! syan's `#[derive(Ast)]` does have a "`#[subast]` entry matches no field"
//! lint, but it goes through `proc_macro_error::emit_warning!`, which is a
//! no-op on stable Rust; and nothing at all checks the other direction, that
//! a *declared* entry is genuinely reached.
//!
//! `tests/visit_reachability.rs` is that missing check, done at runtime: it
//! plants a uniquely identifiable box under **every** recursive field of
//! every visited type and asserts each one is reached. Extend it whenever a
//! box type grows a field that holds another box.

syan::visit::visitor!(
    crate::pagebreak::Page,
    crate::pagebreak::PlacedLine,
    crate::vbox::VertBox,
    crate::hbox::PureHorzBox,
    crate::tabular::TabularBox,
    crate::tabular::TabularCellBox,
    crate::graphics::GraphicsElem
);
