//! Base-14 Helvetica metrics (advance widths transcribed from the freely
//! redistributable Adobe Core-14 AFM files), covering ASCII 32–126. This is
//! the milestone-1 `FontMetrics` provider — no font files are parsed at all.

use satysfi_backend::{FontKey, FontMetrics, Length};

pub const FONT_REGULAR: FontKey = FontKey(0);
pub const FONT_BOLD: FontKey = FontKey(1);
pub const FONT_OBLIQUE: FontKey = FontKey(2);

/// PostScript base font names, indexed by `FontKey`.
pub const BASE_FONT_NAMES: [&str; 3] = ["Helvetica", "Helvetica-Bold", "Helvetica-Oblique"];

/// Helvetica advance widths for chars 32..=126, in 1/1000 em.
/// (Helvetica-Oblique shares these.)
#[rustfmt::skip]
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, // ' ' ! " # $ % & ' ( )
    389, 584, 278, 333, 278, 278,                     // * + , - . /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0-9
    278, 278, 584, 584, 584, 556, 1015,               // : ; < = > ? @
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, // A-J
    667, 556, 833, 722, 778, 667, 778, 722, 667, 611, // K-T
    722, 667, 944, 667, 667, 611,                     // U-Z
    278, 278, 278, 469, 556, 333,                     // [ \ ] ^ _ `
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, // a-j
    500, 222, 833, 556, 556, 556, 556, 333, 500, 278, // k-t
    556, 500, 722, 500, 500, 500,                     // u-z
    334, 260, 334, 584,                               // { | } ~
];

/// Helvetica-Bold advance widths for chars 32..=126, in 1/1000 em.
#[rustfmt::skip]
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, // ' ' ! " # $ % & ' ( )
    389, 584, 278, 333, 278, 278,                     // * + , - . /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0-9
    333, 333, 584, 584, 584, 611, 975,                // : ; < = > ? @
    722, 722, 722, 722, 667, 611, 778, 722, 278, 556, // A-J
    722, 611, 833, 722, 778, 667, 778, 722, 667, 611, // K-T
    722, 667, 944, 667, 667, 611,                     // U-Z
    333, 278, 333, 584, 556, 333,                     // [ \ ] ^ _ `
    556, 611, 556, 611, 556, 333, 611, 611, 278, 278, // a-j
    556, 278, 889, 611, 611, 611, 611, 389, 556, 333, // k-t
    611, 556, 778, 556, 556, 500,                     // u-z
    389, 280, 389, 584,                               // { | } ~
];

/// Font-wide vertical metrics from the AFMs, in 1/1000 em.
const ASCENDER: f64 = 718.0;
const DESCENDER: f64 = 207.0;

pub struct Base14Metrics;

impl Base14Metrics {
    fn widths(font: FontKey) -> &'static [u16; 95] {
        match font {
            FONT_BOLD => &HELVETICA_BOLD,
            _ => &HELVETICA,
        }
    }
}

impl FontMetrics for Base14Metrics {
    fn advance(&self, font: FontKey, c: char, size: Length) -> Option<Length> {
        let code = c as u32;
        if !(32..=126).contains(&code) {
            return None;
        }
        let per_mille = Self::widths(font)[(code - 32) as usize] as f64;
        Some(size * (per_mille / 1000.0))
    }

    fn ascender(&self, _font: FontKey, size: Length) -> Length {
        size * (ASCENDER / 1000.0)
    }

    fn descender(&self, _font: FontKey, size: Length) -> Length {
        size * (DESCENDER / 1000.0)
    }
}
