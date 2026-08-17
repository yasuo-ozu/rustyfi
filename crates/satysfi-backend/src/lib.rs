//! Box/glue model, typesetting context, line and page breaking — the
//! backend of the SATySFi port (a milestone-1 subset of `src/backend/`).

pub mod context;
pub mod font;
pub mod graphics;
pub mod hbox;
pub mod length;
pub mod linebreak;
pub mod math;
pub mod pagebreak;
pub mod tabular;
pub mod vbox;

pub use context::{Context, PageGeometry, PaperSize};
pub use font::{FontKey, FontMetrics};
pub use graphics::{
    graphics_bbox, linear_transform_graphics, linear_transform_path, shift_graphics, shift_path,
    Closing, Color, Dash, GraphicsElem, Path, PathSeg, Point, PrePath, Subpath,
};
pub use hbox::{
    HookId, HorzBox, HorzStringInfo, ImageId, ImageResource, PureHorzBox, FORCED_BREAK_PENALTY,
};
pub use length::Length;
pub use linebreak::{
    break_into_lines, break_opportunities, fit_cell, measure_block, natural_metrics, BreakKind,
};
pub use math::{MathGlyph, MathKind};
pub use pagebreak::{chop_page, place_block_at, Page, PlacedLine};
pub use tabular::{Cell, Paddings, TabularBox, TabularCellBox};
pub use vbox::VertBox;
