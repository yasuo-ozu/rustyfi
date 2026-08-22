//! Box/glue model, typesetting context, line and page breaking — the port of
//! upstream `src/backend/`.

pub mod context;
pub mod doc;
pub mod font;
pub mod graphics;
pub mod hbox;
pub mod length;
pub mod linebreak;
pub mod math;
pub mod pagebreak;
pub mod tabular;
pub mod vbox;
pub mod visit;

pub use context::{
    Context, HyphenLang, Language, MathCmdId, MathScriptLevel, PageGeometry, PaperSize, Script,
    ScriptFont,
};
pub use doc::{Annot, AnnotAction, DocExtras, DocInfo, NamedDest, OutlineEntry};
pub use font::{
    char_script, FontKey, FontMetrics, MathConstants, MathCorner, MathVariantGlyph,
    VertVariantPolicy,
};
pub use graphics::{
    graphics_bbox, linear_transform_graphics, linear_transform_path, path_bbox, shift_graphics,
    shift_path, Closing, Color, Dash, GraphicsElem, Path, PathSeg, Point, PrePath, Subpath,
};
pub use hbox::{
    DecoId, GraphicsFnId, HookId, HorzBox, HorzStringInfo, ImageId, ImageResource, ImportedObjects,
    InlineMarkKind, JpegDct, ObjRepr, PdfPageResource, PureHorzBox, FORCED_BREAK_PENALTY,
    NO_BREAK_PENALTY,
};
pub use length::Length;
pub use linebreak::{
    break_into_lines, break_opportunities, fit_cell, measure_block, natural_metrics, BreakKind,
};
pub use math::{default_math_variant_char, MathCharClass, MathGlyph, MathKind};
pub use pagebreak::{
    chop_page, place_block_at, placed_line_extent, Page, PlacedLine, MIN_FIRST_ASCENDER,
};
pub use tabular::{Cell, Paddings, TabularBox, TabularCellBox};
pub use vbox::{ListMarkKind, VertBox};
