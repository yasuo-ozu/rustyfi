//! Box/glue model, typesetting context, line and page breaking — the
//! backend of the SATySFi port (a milestone-1 subset of `src/backend/`).

pub mod context;
pub mod font;
pub mod hbox;
pub mod length;
pub mod linebreak;
pub mod pagebreak;
pub mod vbox;

pub use context::{Context, PageGeometry};
pub use font::{FontKey, FontMetrics};
pub use hbox::{HorzBox, HorzStringInfo, PureHorzBox};
pub use length::Length;
pub use linebreak::break_into_lines;
pub use pagebreak::{break_pages, Page, PlacedLine};
pub use vbox::VertBox;
