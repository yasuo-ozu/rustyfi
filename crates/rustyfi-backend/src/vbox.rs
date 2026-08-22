use crate::hbox::{DecoId, HookId, PureHorzBox};
use crate::length::Length;

/// A milestone-1 subset of `vert_box`: a typeset line or vertical space.
#[derive(Clone, Debug, PartialEq)]
pub enum VertBox {
    /// One typeset line: boxes with their x offsets from the line start.
    Line {
        height: Length,
        depth: Length,
        /// Baseline-to-baseline distance to the *next* line: the `leading`
        /// a `context` was set to when this line was assembled. `page-break`
        /// takes no context, so the property rides the line itself, set by
        /// `break_into_lines` from `ctx.leading`.
        leading: Length,
        contents: Vec<(Length, PureHorzBox)>,
    },
    /// Fixed vertical space (`block-skip`, frame padding, a paragraph's
    /// *bottom* margin) — NOT padded by `min_first_line_ascender`.
    Skip(Length),
    /// A paragraph's TOP margin (`prim_line_break` prepends this from
    /// `ctx.paragraph_top`). Distinguished from `Skip` because SATySFi pads
    /// only the paragraph `margin_top` up to `min_first_line_ascender`
    /// (`lineBreak.ml:857`) — `block-skip`s and frame pads get no such pad.
    /// Accumulates into `pending_skip` exactly like `Skip`; the difference is
    /// only that its presence enables the ascender pad on the following line.
    ParagTop(Length),
    /// A `block-frame-breakable` frame's internal top/bottom padding. Unlike
    /// `Skip` (a margin, which max-COLLAPSES with adjacent margins), frame
    /// padding is ADDITIVE — SATySFi adds it to the running height inside the
    /// frame (`pageBreak.ml:323` `hgttotal +% pads.paddingT`, `:380`
    /// `+% pads.paddingB`), so it stacks ON TOP of the surrounding
    /// paragraph margin rather than being absorbed by it.
    FramePad(Length),
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
    /// `block-frame-breakable`'s frame-extent markers: the frame's
    /// indented contents sit between a `FrameStart(id)`/`FrameEnd(id)` pair;
    /// `chop_page`/`place_block_at` place each as a zero-height marker
    /// `PlacedLine` (the `HookPageBreak` pattern) and `fire_hooks` derives
    /// each page fragment's rect from the real lines between them. The
    /// frame's pads/width/deco-set live lang-side (`DecoEntry::Block`).
    FrameStart(DecoId),
    FrameEnd(DecoId),
    /// an INERT reflow marker for list (`itemize`/`enumerate`) structure,
    /// emitted by the `list-mark` primitive from
    /// `lib-rustyfi/dist-v01/packages/itemize.satyh`'s
    /// `listing`/`listing-item`/`listing-item-breakable`/`enumerate`/
    /// `enumerate-item`. Zero height/depth, contributes nothing to any
    /// measurement (`measure_block`) or placement (`chop_page`/
    /// `place_block_at`) — it never reaches a `PlacedLine`, so PDF and
    /// faithful HTML are byte-identical whether or not a document's stdlib
    /// emits these. Read only by the reflow HTML walker (the `html-support`
    /// branch's `rustyfi-html/src/reflow/block.rs`'s `walk_vboxes`), which
    /// uses the Start/End nesting to rebuild real `<ul>`/`<ol>`/`<li>`
    /// structure.
    ListMark(ListMarkKind),
}

/// The marker kind a `VertBox::ListMark` carries. `ordered` on `ListStart`
/// is the ONE piece of real data these markers carry (`listing` vs
/// `enumerate` are distinct stdlib commands, so this is an exact bit, not a
/// heuristic); nesting depth is recovered structurally by the reflow
/// walker's stack from how markers are nested in the flat box stream, not
/// stored here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListMarkKind {
    /// Opens a `<ul>` (`ordered = false`) or `<ol>` (`ordered = true`).
    ListStart { ordered: bool },
    /// Closes the innermost open list.
    ListEnd,
    /// Opens one `<li>`.
    ItemStart,
    /// Closes the innermost open `<li>`.
    ItemEnd,
}
