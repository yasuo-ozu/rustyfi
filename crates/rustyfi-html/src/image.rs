//! HTML output backend, Slice 3 (: "Image"). `PureHorzBox::Image` -> an
//! `<img>` data URI. The design doc's per-primitive table calls for "encode
//! RGB8 samples to PNG", but this crate deliberately carries no
//! PNG/image-codec dependency anywhere (the PDF writer's own
//! `write_image_xobjects`, `lib.rs`, writes the same `ImageResource`
//! samples completely uncompressed too — see that function's doc comment).
//! Rather than adding a dependency (off limits this slice, same constraint
//! as base64), this hand-rolls the simplest container format a browser
//! decodes natively straight from raw RGB8: an **uncompressed 24-bit
//! `BI_RGB` Windows BMP** (`image/bmp` — supported by every mainstream
//! browser as an `<img>`/`data:` source, no palette, no compression, a
//! fixed 54-byte header). This is exactly as lossless as the PNG the design
//! doc describes (no compression either way) and an order of magnitude
//! simpler to hand-roll correctly than a PNG encoder (which needs a CRC32 +
//! zlib/DEFLATE stream even at its most minimal).

use rustyfi_backend::ImageResource;

use super::base64;

/// `ImageResource` -> a self-contained `data:` URI, ready to drop straight
/// into an `<img src="...">` attribute.
///
/// A **baseline JPEG passes through byte-for-byte** as `image/jpeg`
/// (`JpegDct::bytes` is the complete original file, kept for exactly this
/// kind of verbatim re-embedding — it is what the PDF writer hands to
/// `/DCTDecode`). Only a source with no such stream — a PNG, or a JPEG
/// variant `sniff_baseline_jpeg_dct` declines — falls back to the
/// uncompressed BMP below.
///
/// The passthrough is not an optimisation detail. The reflow backend renders
/// images for real, and re-encoding a photograph as uncompressed 24-bit BMP
/// and then base64 costs about 4 bytes per pixel: `figbox`'s manual, 39
/// figures, came out as a **21 MB** HTML file. Handing the browser the JPEG
/// it was given brings the same document to 1.5 MB, and is lossless in the
/// literal sense — the bytes are unchanged.
pub(super) fn data_uri(image: &ImageResource) -> String {
    match &image.jpeg_dct {
        Some(jpeg) => {
            let mut out = String::from("data:image/jpeg;base64,");
            out.push_str(&base64::encode(&jpeg.bytes));
            out
        }
        None => {
            let mut out = String::from("data:image/bmp;base64,");
            out.push_str(&base64::encode(&encode_bmp(image)));
            out
        }
    }
}

/// Encode `image`'s row-major, top-to-bottom, 3-bytes-per-pixel RGB8
/// samples (`ImageResource`'s own doc comment, `rustyfi-backend/src/hbox.rs`)
/// as a complete, standalone BMP file: `BITMAPFILEHEADER` (14 bytes) +
/// `BITMAPINFOHEADER` (40 bytes) + row-padded BGR pixel data.
///
/// **Row order.** BMP's canonical/most-compatible layout is BOTTOM-UP (a
/// positive `biHeight` means the FIRST row in the file is the image's
/// bottom row) — a negative height flags top-down instead, which is legal
/// per the format spec but a less universally-supported corner (some older
/// decoders only handle positive heights for uncompressed data). Since
/// `ImageResource::samples` is already top-to-bottom, this writes the
/// source rows in REVERSE order (last source row first) rather than
/// negating `biHeight`, trading a cheap row-index flip for the
/// maximum-compatibility encoding.
fn encode_bmp(image: &ImageResource) -> Vec<u8> {
    let w = image.px_w as usize;
    let h = image.px_h as usize;
    let row_bytes = w * 3;
    // Each row is padded to a multiple of 4 bytes (the BMP spec's DWORD
    // alignment requirement).
    let padded_row = (row_bytes + 3) & !3;
    let pixel_data_len = padded_row * h;
    let pixel_data_offset: u32 = 14 + 40;
    let file_size = pixel_data_offset as usize + pixel_data_len;

    let mut buf = Vec::with_capacity(file_size);

    // BITMAPFILEHEADER.
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&(file_size as u32).to_le_bytes());
    buf.extend_from_slice(&[0, 0, 0, 0]); // bfReserved1/2
    buf.extend_from_slice(&pixel_data_offset.to_le_bytes());

    // BITMAPINFOHEADER (the 40-byte `BITMAPINFOHEADER` variant, the one
    // every decoder supports).
    buf.extend_from_slice(&40u32.to_le_bytes()); // biSize
    buf.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth
    buf.extend_from_slice(&(h as i32).to_le_bytes()); // biHeight (positive: bottom-up)
    buf.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    buf.extend_from_slice(&24u16.to_le_bytes()); // biBitCount
    buf.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    buf.extend_from_slice(&(pixel_data_len as u32).to_le_bytes()); // biSizeImage
    buf.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    buf.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    // Pixel data: BGR triples (BMP's byte order, the mirror of RGB), each
    // row zero-padded to `padded_row` bytes, source rows visited BOTTOM
    // first (see this function's doc comment on row order).
    for src_row in (0..h).rev() {
        let start = src_row * row_bytes;
        for col in 0..w {
            let px = start + col * 3;
            let (r, g, b) = (
                image.samples[px],
                image.samples[px + 1],
                image.samples[px + 2],
            );
            buf.push(b);
            buf.push(g);
            buf.push(r);
        }
        buf.resize(buf.len() + (padded_row - row_bytes), 0);
    }

    debug_assert_eq!(buf.len(), file_size);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_2x2() -> ImageResource {
        // top-to-bottom, RGB8: red, green / blue, white.
        ImageResource {
            samples: vec![
                255, 0, 0, 0, 255, 0, // top row: red, green
                0, 0, 255, 255, 255, 255, // bottom row: blue, white
            ],
            px_w: 2,
            px_h: 2,
            jpeg_dct: None,
            pdf: None,
        }
    }

    #[test]
    fn encodes_a_valid_bmp_header() {
        let bmp = encode_bmp(&tiny_2x2());
        assert_eq!(&bmp[0..2], b"BM", "missing BMP magic");
        let file_size = u32::from_le_bytes(bmp[2..6].try_into().unwrap());
        assert_eq!(file_size as usize, bmp.len());
        let data_offset = u32::from_le_bytes(bmp[10..14].try_into().unwrap());
        assert_eq!(data_offset, 54);
        let width = i32::from_le_bytes(bmp[18..22].try_into().unwrap());
        let height = i32::from_le_bytes(bmp[22..26].try_into().unwrap());
        assert_eq!(width, 2);
        assert_eq!(height, 2, "expected a positive (bottom-up) height");
        let bpp = u16::from_le_bytes(bmp[28..30].try_into().unwrap());
        assert_eq!(bpp, 24);
    }

    #[test]
    fn bottom_up_row_order_puts_the_source_bottom_row_first() {
        let bmp = encode_bmp(&tiny_2x2());
        // Pixel data starts at offset 54. Each 2px row is 6 bytes, padded to
        // 8 (2*3=6, rounded up to a multiple of 4). The FIRST file row must
        // be the source's BOTTOM row (blue, white) in BGR order.
        let px0 = &bmp[54..57];
        assert_eq!(
            px0,
            &[255, 0, 0],
            "first BMP row's first pixel should be blue (BGR)"
        );
        let px1 = &bmp[57..60];
        assert_eq!(
            px1,
            &[255, 255, 255],
            "first BMP row's second pixel should be white (BGR)"
        );
    }

    #[test]
    fn data_uri_has_the_expected_scheme_and_mime() {
        let uri = data_uri(&tiny_2x2());
        assert!(
            uri.starts_with("data:image/bmp;base64,"),
            "unexpected URI: {uri}"
        );
    }

    /// A baseline JPEG is handed to the browser verbatim rather than
    /// re-encoded as an uncompressed BMP — the difference between a 1.9 MB
    /// document and a 21 MB one for a manual with photographs in it. The
    /// bytes must be the ORIGINAL file's, not a re-render of the decoded
    /// samples (which are deliberately mismatched here to catch exactly
    /// that).
    #[test]
    fn a_baseline_jpeg_passes_through_verbatim() {
        let jpeg_bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 0xFF, 0xD9];
        let res = ImageResource {
            jpeg_dct: Some(rustyfi_backend::JpegDct {
                bytes: jpeg_bytes.clone(),
                components: 3,
            }),
            ..tiny_2x2()
        };
        let uri = data_uri(&res);
        assert!(uri.starts_with("data:image/jpeg;base64,"), "{uri}");
        assert_eq!(
            uri,
            format!("data:image/jpeg;base64,{}", super::base64::encode(&jpeg_bytes)),
            "the original JPEG stream must be embedded unchanged"
        );
    }
}
