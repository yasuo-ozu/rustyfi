//! End-to-end proof that CJK glyph rendering works through the real
//! pipeline (docs/plans/text-rendering.md, Group D closure): a genuine
//! Japanese sentence (kanji + hiragana + katakana), compiled through the
//! real loader/elaborator/typechecker/evaluator/line-breaker/page-breaker,
//! then rendered via the CID-keyed PDF writer (`render_pdf_ttf`) under a
//! `TtfFontStore` built from the real IPAex font
//! (`lib-rustyfi/dist/fonts/ipaexm.ttf`, fetched by
//! `scripts/download-fonts.sh`) — an IPA-licensed, `glyf`-outline TrueType
//! face, embeddable by this port's CID writer, unlike the host's CFF `.ttc`
//! system CJK fonts (`cid.rs`'s module doc: only `glyf` outlines can become
//! `FontFile2`).
//!
//! Skipped (not failed) when the font isn't present on disk — this repo
//! never commits font binaries (`lib-rustyfi/dist/fonts/*.ttf` is
//! gitignored); run `sh scripts/download-fonts.sh` to fetch it.
//!
//! Proves, none of which the base-14/Latin-only path can prove:
//!  (a) `get-initial-context` (called by stdja-mini's plain `document`,
//!      with NO `set-font` anywhere in the fixture) picks up the CJK
//!      `default-font.rustyfi-hash` `scripts` block automatically, routing
//!      `HanIdeographic`/`Kana` script text to the `ipaexm` `FontKey`;
//!  (b) the rendered PDF actually embeds the font (`FontFile2`);
//!  (c) the CJK codepoints reach REAL glyphs — the CID-keyed content
//!      stream's `Tj` operands carry the exact glyph ids IPAex's own cmap
//!      names for the sentence's kanji/kana (computed in-test via
//!      `ttf-parser`'s `Face::glyph_index`, never hardcoded);
//!  (d) (when `pdftotext` is available) the embedded `ToUnicode` CMap
//!      round-trips: the extracted text contains the original Japanese
//!      sentence.

use std::path::{Path, PathBuf};
use std::process::Command;

use rustyfi_pdf::{FontFlags, FontRegistry};

/// This repo's `lib-rustyfi/`, resolved from the crate manifest dir exactly
/// as `tests/e2e.rs`/`tests/fonts.rs` do.
fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

fn ipaexm_path() -> PathBuf {
    lib_root().join("dist/fonts/ipaexm.ttf")
}

/// Load `entry` and its full `@require:`/`@import:` dependency graph
/// against `lib_root()`, then concatenate the dependency-ordered library
/// preludes ahead of the entry document's own prelude — the same
/// `merge_program` shape `tests/e2e.rs`'s `load_and_merge` uses.
fn as_v006(cst: rustyfi_loader::LoadedCst) -> rustyfi_syntax::cst::File {
    match cst {
        rustyfi_loader::LoadedCst::V0_0(f) => f,
        rustyfi_loader::LoadedCst::V0_1(_) => {
            unreachable!("this test's load_and_merge is the V0_0-only path")
        }
    }
}

fn load_and_merge(entry: &Path) -> rustyfi_syntax::cst::File {
    let program = rustyfi_loader::load(
        entry,
        &rustyfi_loader::LoadOptions {
            lib_root: Some(lib_root()),
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("failed to load {}: {e}", entry.display()));

    let mut files = program.files;
    let entry_file = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(entry_file.cst);
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(as_v006(lib.cst).prelude);
    }
    prelude.extend(entry_cst.prelude);
    rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw,
        body: entry_cst.body,
        eoi: entry_cst.eoi,
    }
}

/// The Japanese sentence the fixture's body renders: hiragana ("の", "で",
/// "す"), katakana ("テ", "ス", "ト"), and kanji ("日", "本", "語", "文") —
/// `char_script`'s `HanIdeographic`/`Kana` buckets, both of which
/// `download-fonts.sh`'s `default-font.rustyfi-hash` `scripts` block points
/// at `ipaexm`.
const SENTENCE: &str = "日本語のテスト文です";

/// Decode one `Tj` string operand: either a literal string `(...)`
/// (ASCII-only content, PDF-escaped per `pdf_writer::object::Str::write`:
/// `\n`/`\r`/`\t`/`\b`/`\f`/`\(`/`\)`/`\\`/`\OOO` octal) or a hex string
/// `<...>` (used whenever any byte is non-ASCII — the common case for a
/// CJK glyph id's high byte). Mirrors `Str::write`'s encoding in reverse.
fn decode_pdf_string(s: &[u8]) -> Option<Vec<u8>> {
    if s.len() >= 2 && s.first() == Some(&b'<') && s.last() == Some(&b'>') {
        let hex = &s[1..s.len() - 1];
        let mut digits: Vec<u8> = hex
            .iter()
            .copied()
            .filter(|b| b.is_ascii_hexdigit())
            .collect();
        if digits.len() % 2 == 1 {
            digits.push(b'0');
        }
        let mut raw = Vec::with_capacity(digits.len() / 2);
        for pair in digits.chunks(2) {
            let hi = (pair[0] as char).to_digit(16)? as u8;
            let lo = (pair[1] as char).to_digit(16)? as u8;
            raw.push((hi << 4) | lo);
        }
        Some(raw)
    } else if s.len() >= 2 && s.first() == Some(&b'(') && s.last() == Some(&b')') {
        let inner = &s[1..s.len() - 1];
        let mut raw = Vec::new();
        let mut i = 0;
        while i < inner.len() {
            if inner[i] == b'\\' && i + 1 < inner.len() {
                match inner[i + 1] {
                    b'n' => {
                        raw.push(b'\n');
                        i += 2;
                    }
                    b'r' => {
                        raw.push(b'\r');
                        i += 2;
                    }
                    b't' => {
                        raw.push(b'\t');
                        i += 2;
                    }
                    b'b' => {
                        raw.push(0x08);
                        i += 2;
                    }
                    b'f' => {
                        raw.push(0x0c);
                        i += 2;
                    }
                    b'(' => {
                        raw.push(b'(');
                        i += 2;
                    }
                    b')' => {
                        raw.push(b')');
                        i += 2;
                    }
                    b'\\' => {
                        raw.push(b'\\');
                        i += 2;
                    }
                    d @ b'0'..=b'7' => {
                        let mut val = (d - b'0') as u32;
                        let mut n = 1;
                        i += 2;
                        while n < 3
                            && i < inner.len()
                            && inner[i].is_ascii_digit()
                            && inner[i] < b'8'
                        {
                            val = val * 8 + (inner[i] - b'0') as u32;
                            i += 1;
                            n += 1;
                        }
                        raw.push(val as u8);
                    }
                    other => {
                        raw.push(other);
                        i += 2;
                    }
                }
            } else {
                raw.push(inner[i]);
                i += 1;
            }
        }
        Some(raw)
    } else {
        None
    }
}

/// Every `Tj` operand's decoded byte string from a raw (uncompressed)
/// PDF, by scanning for lines ending in `" Tj"` — safe here because
/// `pdf-writer`'s `Content` writes exactly one operator per `\n`-terminated
/// line (`Operation::drop`, pdf-writer's `content.rs`) and this crate's
/// content streams are never Flate-compressed (`lib.rs`'s module doc).
fn extract_tj_payloads(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for line in bytes.split(|&b| b == b'\n') {
        let Some(operand) = line.strip_suffix(b" Tj") else {
            continue;
        };
        if let Some(decoded) = decode_pdf_string(operand) {
            out.push(decoded);
        }
    }
    out
}

/// Every gid a decoded `Tj` payload carries, chunked as big-endian `u16`s
/// (Identity-H: the CID-keyed writer's content bytes ARE glyph ids, 2 bytes
/// each — `cid.rs`'s `encode_glyph_run`).
fn gids_in(payload: &[u8]) -> Vec<u16> {
    payload
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect()
}

#[test]
fn cjk_sentence_renders_real_glyphs_end_to_end() {
    let font_path = ipaexm_path();
    if !font_path.is_file() {
        eprintln!(
            "skipping cjk_sentence_renders_real_glyphs_end_to_end: {} not found \
             — run `sh scripts/download-fonts.sh` to fetch the IPAex TrueType \
             face this test needs",
            font_path.display()
        );
        return;
    }

    // Real production font-config path (docs/plans/text-rendering.md §1a):
    // `FontRegistry::discover` finds `lib-rustyfi/dist/hash/*.rustyfi-hash`
    // (written by `download-fonts.sh`), whose `scripts` block points
    // `han-ideographic`/`kana` at `ipaexm`.
    let registry = FontRegistry::discover(Some(&lib_root()), None, &FontFlags::default())
        .expect("font config discovery must succeed")
        .unwrap_or_else(|| {
            panic!(
                "lib-rustyfi/dist/hash/fonts.rustyfi-hash must be present after \
                 download-fonts.sh (checked {})",
                lib_root().join("dist/hash/fonts.rustyfi-hash").display()
            )
        });
    let store = registry.build_store().expect("build_store must succeed");

    let cjk_key = store
        .abbrev_key("ipaexm")
        .expect("ipaexm must be a registered abbrev");
    let face = store.face(cjk_key).expect("parse the ipaexm face");

    // Expected glyph ids, computed empirically from the REAL font (never
    // hardcoded).
    let expected_gids: Vec<(char, u16)> = SENTENCE
        .chars()
        .map(|c| {
            let gid = face
                .glyph_index(c)
                .unwrap_or_else(|| panic!("ipaexm's cmap must cover {c:?}"))
                .0;
            (c, gid)
        })
        .collect();
    for (c, gid) in &expected_gids {
        assert_ne!(*gid, 0, "expected a real (non-.notdef) glyph id for {c:?}");
    }

    // Compile-time sanity check (before touching the PDF bytes): the
    // font_scheme overlay must have actually routed both CJK script buckets
    // to `cjk_key`, not left them at the Latin default — the D1a/D1b wiring
    // this whole test exists to prove.
    assert_eq!(
        store.script_default(0), // HanIdeographic
        Some((cjk_key, 0.88, 0.0)),
        "han-ideographic script must resolve to ipaexm per default-font.rustyfi-hash"
    );
    assert_eq!(
        store.script_default(1), // Kana
        Some((cjk_key, 0.88, 0.0)),
        "kana script must resolve to ipaexm per default-font.rustyfi-hash"
    );

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cjk.saty");
    let merged = load_and_merge(&fixture);
    let doc = rustyfi_lang::compile_document_cst(&merged, &store)
        .expect("cjk fixture must compile end-to-end");
    assert!(!doc.pages.is_empty(), "expected at least one page");

    let bytes = rustyfi_pdf::render_pdf_ttf(&doc.geometry, &doc.pages, &store, &doc.images)
        .expect("PDF rendering must succeed");
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    // (b) the PDF actually embeds the font.
    assert!(
        bytes.windows(9).any(|w| w == b"FontFile2"),
        "expected an embedded TrueType font (FontFile2) in the CJK PDF"
    );

    // (c) the CJK codepoints reach REAL glyphs: every character's IPAex gid
    // (computed above via ttf-parser, not hardcoded) appears among the
    // content stream's decoded `Tj` payloads.
    let payloads = extract_tj_payloads(&bytes);
    assert!(
        !payloads.is_empty(),
        "expected at least one Tj payload in the content stream"
    );
    let all_gids: Vec<u16> = payloads.iter().flat_map(|p| gids_in(p)).collect();
    for (c, gid) in &expected_gids {
        assert!(
            all_gids.contains(gid),
            "expected the content stream to carry glyph id {gid} for {c:?} \
             (U+{:04X}); decoded Tj gids: {all_gids:?}",
            *c as u32
        );
    }

    // (d) ToUnicode round-trip via pdftotext, when available.
    let Ok(which) = Command::new("which").arg("pdftotext").output() else {
        eprintln!("skipping pdftotext round-trip: `which` not available");
        return;
    };
    if !which.status.success() {
        eprintln!("skipping pdftotext round-trip: pdftotext not on PATH");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("rustyfi-cjk-{}.pdf", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();
    let out = Command::new("pdftotext")
        .arg(&tmp)
        .arg("-")
        .output()
        .expect("run pdftotext");
    let _ = std::fs::remove_file(&tmp);
    assert!(out.status.success(), "pdftotext failed: {out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(SENTENCE),
        "pdftotext output missing the Japanese sentence (ToUnicode round-trip failed): {text:?}"
    );
}
