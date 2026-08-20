//! Box-tree / render-layout snapshot harness (next-work survey item #3:
//! "no golden/snapshot infrastructure — the suite can stay green while box
//! positions drift"). This is the missing safety net: `typecheck_corpus.rs`
//! covers typecheck OUTPUT, and the e2e capstones (`tests/e2e.rs` etc.) only
//! grep a handful of `pdftotext` substrings out of the final PDF — neither
//! would notice a systematic shift in line spacing, box widths, or page
//! geometry as long as the same *words* still appear somewhere.
//!
//! ## Design decision: WHAT to snapshot
//!
//! Three candidates were on the table:
//!
//! 1. **Raw PDF bytes.** Rejected: not deterministic across environments/
//!    font-subsetting/compression-timestamp changes, and a byte diff on a
//!    binary blob isn't reviewable.
//! 2. **`pdftotext` output.** Already covered by the e2e tests, and by
//!    construction throws away all geometry (position, size, wrapping) —
//!    exactly the signal this harness exists to catch.
//! 3. **The post-page-break placed-box model** (`rustyfi_backend::pagebreak
//!    ::{Page, PlacedLine}` and the `PureHorzBox`/`VertBox` tree they carry)
//!    — the richest layout signal available, and the same IR both PDF
//!    writers and the HTML writer consume. **Chosen.**
//!
//! So this harness compiles each fixture down to a `DocumentValue` (stopping
//! *before* either PDF writer runs — see `compile_v006_fixture`/
//! `compile_v01_fixture` below) and serializes `doc.geometry` + every
//! `Page`/`PlacedLine`/`PureHorzBox` to deterministic, human-readable text
//! (`serialize_document`), one line per box/line/page, with:
//!
//! - Lengths rounded to 3 decimal places (`fmt_len`) — base-14 metrics are
//!   fixed rational AFM tables, so real drift shows up well above the 3rd
//!   decimal; this just absorbs the last-bit float noise a `derive(Debug)`
//!   dump would otherwise bake into the golden file.
//! - No addresses, no timestamps, no wall-clock-anything. The only "opaque
//!   IDs" that appear (`FontKey`/`ImageId`/`DecoId`/…) are small `usize`
//!   resource-table indices assigned in a fixed order by a fully
//!   deterministic single-threaded compile of a fixed input — reproducible,
//!   not random.
//! - Base-14 metrics only (`rustyfi_pdf::Base14Metrics`), so this test needs
//!   no installed font (not gated on DejaVu like the TTF-based e2e
//!   capstones) and runs identically everywhere.
//!
//! Fixtures (small and curated, not the whole corpus): `minimal.saty` and
//! `phase2.saty` (0.0.6, `@require: stdja-mini` — text wrapping, `\emph`,
//! `let-inline`/`let-rec`/`match`), and `v01-minimal.saty` (0.1,
//! `@require: v01-mini` — proves the harness also covers the 0.1 lowering
//! path). All three already exist under `tests/fixtures/` for the e2e PDF
//! tests; this file reuses them rather than adding new ones.
//!
//! ## Update mechanism
//!
//! Run with `UPDATE_SNAPSHOTS=1` to (re)write the `.snap` baseline files
//! under `tests/snapshots/` from the current compiler's output — e.g.:
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test -p rustyfi --test layout_snapshot
//! ```
//!
//! A normal `cargo test` run instead compares freshly-serialized output
//! against the committed `.snap` file and panics with a line-level diff
//! (`diff_report`) on any mismatch.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rustyfi_backend::{Length, PureHorzBox, VertBox};
use rustyfi_lang::value::DocumentValue;
use rustyfi_loader::{LoadOptions, LoadedCst};
use rustyfi_pdf::Base14Metrics;
use rustyfi_syntax::RustyfiVersion;

fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn snapshots_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

// ---------------------------------------------------------------------
// Compiling a fixture down to a `DocumentValue` (no PDF writer involved).
// ---------------------------------------------------------------------

fn as_v006(cst: LoadedCst) -> rustyfi_syntax::cst::File {
    match cst {
        LoadedCst::V0_0(f) => f,
        LoadedCst::V0_1(_) => unreachable!("compile_v006_fixture is the V0_0-only path"),
    }
}

/// Load `name` (+ its `@require:`/`@import:` graph) against this repo's
/// `lib-rustyfi/`, merge dependency preludes ahead of the entry's own
/// (exactly `rustyfi`'s private `merge_program`, reproduced the same way
/// `tests/e2e.rs`'s `load_and_merge` already does), and compile with
/// deterministic base-14 metrics — never touching `rustyfi-pdf`'s writers.
fn compile_v006_fixture(name: &str) -> std::rc::Rc<DocumentValue> {
    let entry = fixtures_dir().join(name);
    let program = rustyfi_loader::load(
        &entry,
        &LoadOptions {
            lib_root: Some(repo_lib_root()),
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
    let merged = rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw,
        body: entry_cst.body,
        eoi: entry_cst.eoi,
    };

    let metrics = Base14Metrics;
    rustyfi_lang::compile_document_cst(&merged, &metrics)
        .unwrap_or_else(|e| panic!("{name} must compile: {e}"))
}

/// V0.1 analogue of [`compile_v006_fixture`]: load through the real loader
/// with `version: V0_1` (resolves `@require: v01-mini` under
/// `lib-rustyfi/dist-v01/packages/`, exactly like `tests/xver_capstone.rs`
/// does), then `compile_document_v1` directly on the loader's file list —
/// no manual prelude-splicing needed on this path.
fn compile_v01_fixture(name: &str) -> std::rc::Rc<DocumentValue> {
    let entry = fixtures_dir().join(name);
    let program = rustyfi_loader::load(
        &entry,
        &LoadOptions {
            lib_root: Some(repo_lib_root()),
            version: RustyfiVersion::V0_1,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("failed to load {}: {e}", entry.display()));

    let metrics = Base14Metrics;
    rustyfi_lang::compile_document_v1(&program.files, &metrics)
        .unwrap_or_else(|e| panic!("{name} must compile: {e}"))
}

// ---------------------------------------------------------------------
// Deterministic serialization of the placed-box tree.
// ---------------------------------------------------------------------

/// Round to 3 decimal places and normalize `-0.0` to `0.0`, so the snapshot
/// text is stable across runs regardless of last-bit float noise in how a
/// particular sum of glue/advance widths happened to associate.
fn round3(l: Length) -> f64 {
    let v = (l.0 * 1000.0).round() / 1000.0;
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

fn fmt_len(l: Length) -> String {
    format!("{:.3}", round3(l))
}

/// Serialize one `PureHorzBox`, one line, indented by nesting depth. The
/// match is intentionally exhaustive (no wildcard arm) so a future new
/// `PureHorzBox` variant forces this serializer to decide how to represent
/// it, rather than silently going invisible to the snapshot — mirroring the
/// exhaustive-match discipline `pagebreak.rs`'s own `placed_line_extent`/
/// `collect_footnotes_in_box` already use over this same enum.
fn write_box(out: &mut String, indent: usize, dx: Length, bx: &PureHorzBox) {
    let pad = "  ".repeat(indent);
    match bx {
        PureHorzBox::InnerString {
            info,
            text,
            width,
            height,
            depth,
        } => {
            let _ = writeln!(
                out,
                "{pad}dx={} InnerString font={} size={} rising={} w={} h={} d={} text={:?}",
                fmt_len(dx),
                info.font.0,
                fmt_len(info.size),
                fmt_len(info.rising),
                fmt_len(*width),
                fmt_len(*height),
                fmt_len(*depth),
                text
            );
        }
        PureHorzBox::OuterEmpty {
            natural,
            shrinkable,
            stretchable,
        } => {
            let _ = writeln!(
                out,
                "{pad}dx={} OuterEmpty natural={} shrink={} stretch={}",
                fmt_len(dx),
                fmt_len(*natural),
                fmt_len(*shrinkable),
                fmt_len(*stretchable)
            );
        }
        PureHorzBox::OuterFil => {
            let _ = writeln!(out, "{pad}dx={} OuterFil", fmt_len(dx));
        }
        PureHorzBox::FixedEmpty { width } => {
            let _ = writeln!(
                out,
                "{pad}dx={} FixedEmpty w={}",
                fmt_len(dx),
                fmt_len(*width)
            );
        }
        PureHorzBox::Image {
            width,
            height,
            image,
        } => {
            let _ = writeln!(
                out,
                "{pad}dx={} Image w={} h={} image_id={}",
                fmt_len(dx),
                fmt_len(*width),
                fmt_len(*height),
                image.0
            );
        }
        PureHorzBox::Discretionary {
            penalty,
            pre_break,
            post_break,
            no_break,
        } => {
            let _ = writeln!(
                out,
                "{pad}dx={} Discretionary penalty={penalty}",
                fmt_len(dx)
            );
            if !pre_break.is_empty() {
                let _ = writeln!(out, "{pad}  pre_break:");
                for b in pre_break {
                    write_box(out, indent + 2, Length::ZERO, b);
                }
            }
            if !post_break.is_empty() {
                let _ = writeln!(out, "{pad}  post_break:");
                for b in post_break {
                    write_box(out, indent + 2, Length::ZERO, b);
                }
            }
            if !no_break.is_empty() {
                let _ = writeln!(out, "{pad}  no_break:");
                for b in no_break {
                    write_box(out, indent + 2, Length::ZERO, b);
                }
            }
        }
        PureHorzBox::Graphics {
            width,
            height,
            depth,
            elems,
            origin_independent: _,
        } => {
            let _ = writeln!(
                out,
                "{pad}dx={} Graphics w={} h={} d={} elems={}",
                fmt_len(dx),
                fmt_len(*width),
                fmt_len(*height),
                fmt_len(*depth),
                elems.len()
            );
        }
        PureHorzBox::GraphicsOuter {
            height,
            depth,
            width,
            fn_id,
        } => {
            let _ = writeln!(
                out,
                "{pad}dx={} GraphicsOuter w={} h={} d={} fn_id={}",
                fmt_len(dx),
                fmt_len(*width),
                fmt_len(*height),
                fmt_len(*depth),
                fn_id.0
            );
        }
        PureHorzBox::Math {
            width,
            height,
            depth,
            glyphs,
            rules,
        } => {
            let _ = writeln!(
                out,
                "{pad}dx={} Math w={} h={} d={} glyphs={} rules={}",
                fmt_len(dx),
                fmt_len(*width),
                fmt_len(*height),
                fmt_len(*depth),
                glyphs.len(),
                rules.len()
            );
            for g in glyphs {
                let _ = writeln!(
                    out,
                    "{pad}  glyph dx={} dy={} font={} size={} w={} h={} d={} text={:?}",
                    fmt_len(g.dx),
                    fmt_len(g.dy),
                    g.info.font.0,
                    fmt_len(g.info.size),
                    fmt_len(g.width),
                    fmt_len(g.height),
                    fmt_len(g.depth),
                    g.text
                );
            }
        }
        PureHorzBox::HookPageBreak { id } => {
            let _ = writeln!(out, "{pad}dx={} HookPageBreak id={}", fmt_len(dx), id.0);
        }
        PureHorzBox::Tabular(tab) => {
            let _ = writeln!(
                out,
                "{pad}dx={} Tabular w={} h={} d={} cells={} rules={}",
                fmt_len(dx),
                fmt_len(tab.width),
                fmt_len(tab.height),
                fmt_len(tab.depth),
                tab.cells.len(),
                tab.rules.len()
            );
            for cell in &tab.cells {
                let _ = writeln!(
                    out,
                    "{pad}  cell x={} baseline_y={}",
                    fmt_len(cell.x),
                    fmt_len(cell.baseline_y)
                );
                for (cdx, cbx) in &cell.contents {
                    write_box(out, indent + 2, *cdx, cbx);
                }
            }
        }
        PureHorzBox::EmbeddedBlock {
            width,
            height,
            depth,
            block,
            ..
        } => {
            let _ = writeln!(
                out,
                "{pad}dx={} EmbeddedBlock w={} h={} d={} lines={}",
                fmt_len(dx),
                fmt_len(*width),
                fmt_len(*height),
                fmt_len(*depth),
                block.len()
            );
            for vb in block {
                write_vbox(out, indent + 1, vb);
            }
        }
        PureHorzBox::Frame {
            width,
            height,
            depth,
            deco,
            contents,
        } => {
            let _ = writeln!(
                out,
                "{pad}dx={} Frame w={} h={} d={} deco_id={}",
                fmt_len(dx),
                fmt_len(*width),
                fmt_len(*height),
                fmt_len(*depth),
                deco.0
            );
            for (cdx, cbx) in contents {
                write_box(out, indent + 1, *cdx, cbx);
            }
        }
        PureHorzBox::FrameMarker { id, end } => {
            let _ = writeln!(
                out,
                "{pad}dx={} FrameMarker id={} end={}",
                fmt_len(dx),
                id.0,
                end
            );
        }
        PureHorzBox::Footnote { block } => {
            let _ = writeln!(
                out,
                "{pad}dx={} Footnote lines={}",
                fmt_len(dx),
                block.len()
            );
            for vb in block {
                write_vbox(out, indent + 1, vb);
            }
        }
        // an inert reflow marker (zero width/height/depth, contributes
        // nothing to geometry — see `linebreak.rs`'s/`hbox.rs`'s
        // `InlineMark` arms). No current fixture emits one (none call
        // `\emph`/`\bold`/ `\listing`/`\enumerate` under this harness's
        // base-14 setup), but the snapshot format still names the kind
        // explicitly rather than silently going invisible, matching this
        // function's own exhaustive discipline (its doc comment above).
        PureHorzBox::InlineMark(kind) => {
            let _ = writeln!(out, "{pad}dx={} InlineMark kind={kind:?}", fmt_len(dx));
        }
    }
}

/// `VertBox` sibling of [`write_box`] — only reachable inside an
/// `EmbeddedBlock`/`Footnote`'s nested `block`, but exhaustive for the same
/// reason.
fn write_vbox(out: &mut String, indent: usize, vb: &VertBox) {
    let pad = "  ".repeat(indent);
    match vb {
        VertBox::Line {
            height,
            depth,
            leading,
            contents,
        } => {
            let _ = writeln!(
                out,
                "{pad}VLine h={} d={} leading={}",
                fmt_len(*height),
                fmt_len(*depth),
                fmt_len(*leading)
            );
            for (ldx, lbx) in contents {
                write_box(out, indent + 1, *ldx, lbx);
            }
        }
        VertBox::Skip(l) => {
            let _ = writeln!(out, "{pad}VSkip {}", fmt_len(*l));
        }
        VertBox::ClearPage => {
            let _ = writeln!(out, "{pad}VClearPage");
        }
        VertBox::HookPageBreak(id) => {
            let _ = writeln!(out, "{pad}VHookPageBreak id={}", id.0);
        }
        VertBox::FrameStart(id) => {
            let _ = writeln!(out, "{pad}VFrameStart id={}", id.0);
        }
        VertBox::FrameEnd(id) => {
            let _ = writeln!(out, "{pad}VFrameEnd id={}", id.0);
        }
        // same inert-marker treatment as `PureHorzBox::InlineMark` above
        // — zero height, contributes nothing to geometry
        // (`measure_block`'s `ListMark` arm).
        VertBox::ListMark(kind) => {
            let _ = writeln!(out, "{pad}VListMark kind={kind:?}");
        }
        VertBox::ParagTop(l) => {
            let _ = writeln!(out, "{pad}VParagTop {}", fmt_len(*l));
        }
        VertBox::FramePad(l) => {
            let _ = writeln!(out, "{pad}VFramePad {}", fmt_len(*l));
        }
    }
}

/// The whole-document snapshot: geometry, then every page's every placed
/// line's every box, then a one-line summary of the non-geometric extras
/// (annotations/destinations/outline/page-graphics/doc-info) so a future
/// fixture that exercises those isn't silently uncovered.
fn serialize_document(name: &str, doc: &DocumentValue) -> String {
    let mut out = String::new();
    let g = &doc.geometry;
    let _ = writeln!(out, "FIXTURE {name}");
    let _ = writeln!(
        out,
        "GEOMETRY paper={}x{} text_origin=({},{}) text_size={}x{}",
        fmt_len(g.paper_width),
        fmt_len(g.paper_height),
        fmt_len(g.text_origin.0),
        fmt_len(g.text_origin.1),
        fmt_len(g.text_width),
        fmt_len(g.text_height)
    );
    let _ = writeln!(out, "PAGES {}", doc.pages.len());
    for (pi, page) in doc.pages.iter().enumerate() {
        let _ = writeln!(out, "-- page {pi} lines={} --", page.lines.len());
        for (li, line) in page.lines.iter().enumerate() {
            let _ = writeln!(
                out,
                "LINE {li} x={} baseline_y={}",
                fmt_len(line.x),
                fmt_len(line.baseline_y)
            );
            for (dx, bx) in &line.contents {
                write_box(&mut out, 1, *dx, bx);
            }
        }
    }
    let _ = writeln!(
        out,
        "EXTRAS annotations={} destinations={} outline={} page_graphics_pages={} doc_info={}",
        doc.extras.annotations.len(),
        doc.extras.destinations.len(),
        doc.extras.outline.len(),
        doc.extras.page_graphics.len(),
        if doc.extras.doc_info.is_some() {
            "some"
        } else {
            "none"
        }
    );
    out
}

// ---------------------------------------------------------------------
// Compare-or-update against the committed `.snap` baseline.
// ---------------------------------------------------------------------

/// A small, dependency-free line-oriented diff: reports the total line
/// counts plus the first ~20 mismatching lines (1-based, both sides) — no
/// `insta`/`similar`/etc. dependency, just enough to localize a layout
/// drift without dumping the whole (possibly large) snapshot twice.
fn diff_report(expected: &str, actual: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "expected {} lines, got {} lines",
        exp_lines.len(),
        act_lines.len()
    );
    let max = exp_lines.len().max(act_lines.len());
    let mut shown = 0;
    for i in 0..max {
        let e = exp_lines.get(i).copied();
        let a = act_lines.get(i).copied();
        if e != a {
            let _ = writeln!(out, "  line {}: expected {e:?}", i + 1);
            let _ = writeln!(out, "  line {}:      got {a:?}", i + 1);
            shown += 1;
            if shown >= 20 {
                let _ = writeln!(out, "  ... (more differences omitted)");
                break;
            }
        }
    }
    out
}

/// Compare `actual` against the committed `tests/snapshots/{fixture_name}
/// .snap` baseline, or (re)write it when `UPDATE_SNAPSHOTS=1` is set in the
/// environment.
fn check_snapshot(fixture_name: &str, actual: &str) {
    let path = snapshots_dir().join(format!("{fixture_name}.snap"));

    if std::env::var("UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
        std::fs::create_dir_all(snapshots_dir()).expect("create tests/snapshots/");
        std::fs::write(&path, actual)
            .unwrap_or_else(|e| panic!("write snapshot {}: {e}", path.display()));
        eprintln!("updated snapshot {}", path.display());
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing snapshot baseline {} ({e}) — run with UPDATE_SNAPSHOTS=1 to create it",
            path.display()
        )
    });
    if actual != expected {
        panic!(
            "layout snapshot mismatch for {fixture_name} (baseline: {}).\n\
             If this is an INTENTIONAL layout change, rerun with UPDATE_SNAPSHOTS=1 to accept \
             it; otherwise this is a layout regression.\n{}",
            path.display(),
            diff_report(&expected, actual)
        );
    }
}

// ---------------------------------------------------------------------
// The fixtures.
// ---------------------------------------------------------------------

#[test]
fn layout_snapshot_minimal() {
    let doc = compile_v006_fixture("minimal.saty");
    let actual = serialize_document("minimal.saty", &doc);
    check_snapshot("minimal", &actual);
}

#[test]
fn layout_snapshot_phase2() {
    let doc = compile_v006_fixture("phase2.saty");
    let actual = serialize_document("phase2.saty", &doc);
    check_snapshot("phase2", &actual);
}

#[test]
fn layout_snapshot_v01_minimal() {
    let doc = compile_v01_fixture("v01-minimal.saty");
    let actual = serialize_document("v01-minimal.saty", &doc);
    check_snapshot("v01-minimal", &actual);
}
