use crate::hbox::{DecoId, HookId, PureHorzBox};
use crate::length::Length;

/// A milestone-1 subset of `vert_box`: a typeset line or vertical space.
#[derive(Clone, Debug, PartialEq)]
pub enum VertBox {
    /// One typeset line: boxes with their x offsets from the line start.
    Line {
        height: Length,
        depth: Length,
        /// Baseline-to-baseline distance to the *next* line (the
        /// `leading` a `context` was set to when this line was
        /// assembled — `docs/plans/document-page-model.md` §"The
        /// `leading` refactor": `page-break` no longer takes a context,
        /// so this moves the property onto the line itself, set by
        /// `break_into_lines` from `ctx.leading`).
        leading: Length,
        contents: Vec<(Length, PureHorzBox)>,
    },
    /// Fixed vertical space.
    Skip(Length),
    /// `clear-page`'s marker (`primitives.cppo.ml`'s `VertClearPage`,
    /// `horzBox.ml:346`) — forces the current page to end right here:
    /// `chop_page` closes the page as soon as at least one real `Line` has
    /// been placed, leaving everything from this marker onward for the next
    /// page. Contributes zero height (`measure_block`), mirroring
    /// `pageBreak.ml`'s `PBClearPage` (`solidify`'s
    /// `ImVertFixedEmpty(Fixed, Length.zero)`).
    ClearPage,
    /// `hook-page-break-block`'s marker (`vminst.ml:632`
    /// `BackendHookPageBreakBlock` / `horzBox.ml:347`'s `VertHookPageBreak`)
    /// — the block-level analog of `PureHorzBox::HookPageBreak`: an opaque
    /// index into a lang-side hook table, fired by `fire_hooks` with the
    /// page's `pbinfo` and the point where it sits in the flow. Contributes
    /// zero height, same as `ClearPage`.
    HookPageBreak(HookId),
    /// `block-frame-breakable`'s frame-extent markers (§D): the frame's
    /// indented contents sit between a `FrameStart(id)`/`FrameEnd(id)` pair;
    /// `chop_page`/`place_block_at` place each as a zero-height marker
    /// `PlacedLine` (the `HookPageBreak` pattern) and `fire_hooks` derives
    /// each page fragment's rect from the real lines between them. The
    /// frame's pads/width/deco-set live lang-side (`DecoEntry::Block`).
    FrameStart(DecoId),
    FrameEnd(DecoId),
}
