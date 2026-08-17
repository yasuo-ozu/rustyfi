//! JPEG DCTDecode passthrough slice: unit coverage for
//! `ImageResource::sniff_baseline_jpeg_dct`, independent of the lang-side
//! `load-image` primitive that calls it in practice
//! (`crates/rustyfi-lang/tests/images.rs` covers that, and
//! `crates/rustyfi-pdf/tests/image.rs` covers the full round trip through
//! the PDF writer's `/Filter /DCTDecode` embedding).

use std::path::Path;

use rustyfi_backend::ImageResource;

/// The checked-in fixture: a tiny 8x4 baseline (SOF0), 8-bit, 3-component
/// (YCbCr/RGB) JPEG generated with Pillow — same bytes as
/// `rustyfi-pdf/tests/fixtures/dot.jpg` / `rustyfi-lang/tests/fixtures/dot.jpg`
/// (duplicated per this workspace's existing per-crate fixture convention;
/// see `rustyfi-lang/tests/images.rs`'s `fixture_path` doc comment for why).
fn jpeg_bytes() -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dot.jpg"))
        .expect("fixture must be readable")
}

#[test]
fn a_baseline_jpeg_is_recognized_with_its_true_component_count() {
    let dct = ImageResource::sniff_baseline_jpeg_dct(jpeg_bytes())
        .expect("dot.jpg is a baseline (SOF0) JPEG and must be recognized");
    assert_eq!(dct.components, 3, "dot.jpg is a 3-component YCbCr/RGB JPEG");
}

#[test]
fn the_recognized_bytes_are_the_original_file_untouched() {
    let original = jpeg_bytes();
    let dct = ImageResource::sniff_baseline_jpeg_dct(original.clone())
        .expect("dot.jpg is a baseline (SOF0) JPEG and must be recognized");
    assert_eq!(dct.bytes, original, "sniffing must not alter the original bytes");
}

#[test]
fn a_png_is_not_mistaken_for_a_jpeg() {
    let png = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("rustyfi-pdf/tests/fixtures/dot.png"),
    )
    .expect("the workspace's dot.png fixture must be readable");
    assert!(png.starts_with(b"\x89PNG"), "sanity: this must actually be a PNG");
    assert!(
        ImageResource::sniff_baseline_jpeg_dct(png).is_none(),
        "a PNG must never be recognized as a JPEG"
    );
}

#[test]
fn empty_bytes_are_rejected_without_panicking() {
    assert!(ImageResource::sniff_baseline_jpeg_dct(Vec::new()).is_none());
}

#[test]
fn a_truncated_jpeg_header_is_rejected_without_panicking() {
    // Just the SOI marker, nothing else: no SOF marker was ever seen, so
    // this must cleanly return `None`, not panic on an out-of-bounds index.
    assert!(ImageResource::sniff_baseline_jpeg_dct(vec![0xFF, 0xD8]).is_none());
    assert!(ImageResource::sniff_baseline_jpeg_dct(vec![0xFF, 0xD8, 0xFF]).is_none());
}

#[test]
fn a_progressive_jpeg_sof2_is_rejected() {
    // SOI, then a minimal SOF2 (progressive DCT) segment: length=8,
    // precision=8, height=1, width=1, components=3 (one component triplet
    // omitted since sniff_baseline_jpeg_dct only reads up through the
    // component count byte).
    let bytes = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xC2, // SOF2 (progressive DCT) -- not baseline/extended-sequential
        0x00, 0x08, // segment length = 8
        0x08, // precision
        0x00, 0x01, // height = 1
        0x00, 0x01, // width = 1
        0x03, // components = 3
    ];
    assert!(
        ImageResource::sniff_baseline_jpeg_dct(bytes).is_none(),
        "SOF2 (progressive) must fall back to decode/re-encode, not DCTDecode passthrough"
    );
}

#[test]
fn a_four_component_cmyk_jpeg_is_rejected() {
    // Same shape as the SOF2 test above but a baseline SOF0 with 4
    // components (CMYK/YCCK) -- out of scope per `sniff_baseline_jpeg_dct`'s
    // doc comment (Adobe APP14 transform ambiguity).
    let bytes = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xC0, // SOF0 (baseline)
        0x00, 0x08, // segment length = 8
        0x08, // precision
        0x00, 0x01, // height = 1
        0x00, 0x01, // width = 1
        0x04, // components = 4 (CMYK/YCCK)
    ];
    assert!(
        ImageResource::sniff_baseline_jpeg_dct(bytes).is_none(),
        "4-component (CMYK/YCCK) JPEGs must fall back, not be embedded as DeviceCMYK"
    );
}
