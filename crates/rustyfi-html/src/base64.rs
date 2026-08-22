//! HTML output backend, Slice 3 (: "real fonts + math"). A hand-rolled
//! standard (RFC 4648 §4) base64 encoder — used by [`super::fonts`] (the
//! embedded `@font-face` TTF bytes) and [`super::image`] (the `<img>` data
//! URI). Deliberately NOT a crate dependency: the project constraint for
//! this slice is "no new dependency, no manifest edit" (see `html.rs`'s
//! module doc on why this whole feature lives inside `rustyfi-pdf` rather
//! than a new crate), and a padded 3-bytes-in/4-chars-out encoder is a
//! dozen lines, well under the bar for pulling in `base64`/similar.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard (padded, `+`/`/`) base64 — the variant every browser's
/// `data:` URI parser expects.
pub(super) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut chunks = bytes.chunks_exact(3);
    for c in &mut chunks {
        let n = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | (c[2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push(ALPHABET[(n & 0x3F) as usize] as char);
    }
    match chunks.remainder() {
        [] => {}
        &[b0] => {
            let n = (b0 as u32) << 16;
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            out.push('=');
            out.push('=');
        }
        &[b0, b1] => {
            let n = ((b0 as u32) << 16) | ((b1 as u32) << 8);
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        _ => unreachable!("chunks_exact(3)'s remainder is always 0, 1, or 2 bytes"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_vectors() {
        // RFC 4648 §10's own worked examples.
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn matches_the_classic_man_vector() {
        // The other commonly-cited worked example (Wikipedia's base64 page).
        assert_eq!(encode(b"Man"), "TWFu");
        assert_eq!(encode(b"Ma"), "TWE=");
        assert_eq!(encode(b"M"), "TQ==");
    }
}
