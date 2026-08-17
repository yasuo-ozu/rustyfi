use crate::hbox::PureHorzBox;
use crate::length::Length;

/// A milestone-1 subset of `vert_box`: a typeset line or vertical space.
#[derive(Clone, Debug, PartialEq)]
pub enum VertBox {
    /// One typeset line: boxes with their x offsets from the line start.
    Line {
        height: Length,
        depth: Length,
        contents: Vec<(Length, PureHorzBox)>,
    },
    /// Fixed vertical space.
    Skip(Length),
}
