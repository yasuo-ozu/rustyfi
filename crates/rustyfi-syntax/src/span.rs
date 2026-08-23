/// A source location. `line` is 1-based, `col` is a 0-based character column,
/// `byte` is the byte offset into the source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Loc {
    pub line: u32,
    pub col: u32,
    pub byte: usize,
}

/// A half-open source range `[start, end)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: Loc,
    pub end: Loc,
}

impl Span {
    pub(crate) fn new(start: Loc, end: Loc) -> Self {
        Span { start, end }
    }

    /// The smallest span covering both `self` and `other` (Range.unite).
    pub fn unite(self, other: Span) -> Span {
        let dummy = Span::default();
        if self == dummy {
            return other;
        }
        if other == dummy {
            return self;
        }
        let start = if self.start.byte <= other.start.byte {
            self.start
        } else {
            other.start
        };
        let end = if self.end.byte >= other.end.byte {
            self.end
        } else {
            other.end
        };
        Span { start, end }
    }
}

impl syan::span::Span for Span {
    fn migrate(self, other: Self) -> Self {
        self.unite(other)
    }
}

/// The largest `char` boundary at or below `byte`, clamped to `src.len()`.
///
/// Every consumer of a [`Span`] that wants to *slice* the source needs this:
/// a span's byte offsets come from the lexer and are boundaries by
/// construction, but a caller may have widened, clamped or defaulted one, and
/// slicing a `str` off a boundary panics. Rounding down rather than panicking
/// is the useful behaviour on both sides — a diagnostic covering one extra
/// character beats no diagnostic at all.
pub fn floor_char_boundary(src: &str, mut byte: usize) -> usize {
    byte = byte.min(src.len());
    while byte > 0 && !src.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.start.line == self.end.line {
            write!(
                f,
                "line {}, characters {}-{}",
                self.start.line, self.start.col, self.end.col
            )
        } else {
            write!(
                f,
                "line {}, character {} to line {}, character {}",
                self.start.line, self.start.col, self.end.line, self.end.col
            )
        }
    }
}
