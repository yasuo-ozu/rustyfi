use crate::hbox::PureHorzBox;
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
}
