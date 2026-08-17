use crate::font::FontKey;
use crate::length::Length;

/// Which font/size a string box is set in (`horz_string_info`).
#[derive(Clone, Debug, PartialEq)]
pub struct HorzStringInfo {
    pub font: FontKey,
    pub size: Length,
}

/// A milestone-1 subset of `pure_horz_box` from horzBox.ml, keeping its
/// vocabulary so the full port extends rather than replaces it.
#[derive(Clone, Debug, PartialEq)]
pub enum PureHorzBox {
    /// Fixed text with pre-measured dimensions.
    InnerString {
        info: HorzStringInfo,
        text: String,
        width: Length,
        height: Length,
        depth: Length,
    },
    /// Interword glue.
    OuterEmpty {
        natural: Length,
        shrinkable: Length,
        stretchable: Length,
    },
    /// Infinitely stretchable glue (`inline-fil`).
    OuterFil,
}

impl PureHorzBox {
    pub fn natural_width(&self) -> Length {
        match self {
            PureHorzBox::InnerString { width, .. } => *width,
            PureHorzBox::OuterEmpty { natural, .. } => *natural,
            PureHorzBox::OuterFil => Length::ZERO,
        }
    }

    pub fn is_glue(&self) -> bool {
        matches!(
            self,
            PureHorzBox::OuterEmpty { .. } | PureHorzBox::OuterFil
        )
    }
}

/// `horz_box`: milestone 1 has no discretionaries yet, but the wrapper stays
/// so line-break input keeps the shape lineBreak.ml expects.
#[derive(Clone, Debug, PartialEq)]
pub enum HorzBox {
    Pure(PureHorzBox),
}
