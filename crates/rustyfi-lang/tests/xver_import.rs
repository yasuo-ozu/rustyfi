//! Slices X1 + X2a: a `V0_1` document `@require:`-ing a `V0_0` package
//! end-to-end.
//!
//! Driven through the REAL loader (`rustyfi_loader::load`, `LoadOptions {
//! version: V0_1, .. }`) so the per-file detection rule (`load_legacy`'s Q4
//! worklist logic), the forked-name guard (`compile_document_v1_with_trials`'s
//! dep loop, `collect_free_globals`), and — new in X2a — the
//! `Ast::VersionScope` splice wrapping are all exercised for real, not
//! bypassed:
//!
//! - [`positive_case_list_and_option_render`] (X1): a `V0_1` entry requiring
//!   the REAL vendored `list.satyg`/`option.satyg` (copied byte-for-byte from
//!   `lib-rustyfi/dist/packages/` into a temp `lib_root` — see
//!   [`TempDir::copy_real_package`]) — `List.map`/`List.fold-left`/
//!   `Option.from` all get used, and the resulting document actually
//!   renders (a real page with real placed content), proving the splice +
//!   guard pass a genuinely version-neutral dependency through untouched.
//! - [`xver_page_break_internal_renders`] (X2a headline): a `V0_0`
//!   package whose exported command (a NEUTRAL-typed
//!   `block-boxes -> document`) calls `page-break` INTERNALLY with the
//!   `V0_0` `page` ADT — a shape X1 hard-rejected (`page-break` is
//!   version-forked, `primitives::forked_prim_names`) — now renders
//!   end-to-end through `compile_document_v1`, proving the
//!   `Ast::VersionScope` mechanism (compile-time fold cursor +
//!   `Interp::version` + the typecheck env swap) makes the previously-
//!   rejected case work.
//! - [`xver_forked_prim_coexist`] (X2a soundness core): the SAME package,
//!   but the `V0_1` entry ALSO calls `page-break` directly (with `V0_1`'s
//!   `length * length` geometry) alongside the package's internal `V0_0`
//!   `page`-ADT call, in the SAME compiled program — this only
//!   type-checks/evaluates at all if the two calls resolved to their OWN
//!   version's distinct `page-break` (mismatched arity/type would be a
//!   `TypeError`, not a silent mis-render), so a successful render is
//!   itself the coexistence proof.
//! - [`negative_case_forked_type_on_boundary_is_still_rejected`] (X2a
//!   negative, repurposed from X1): X2a removes only the VALUE half of the
//!   forked-name guard; the TYPE half stays conservative — a `V0_0` package
//!   that textually names a forked TYPE on an export-boundary surface site
//!   (here, a `type` declaration's body — see `v1::xver_adapt`'s module doc
//!   comment for why that site stays unconditionally checked even after
//!   X2b's narrowing) is still rejected with
//!   `CompileError::CrossVersionUnsupportedName`. X1's original negative test
//!   used `page-break` (a forked VALUE) for this — that shape now SUCCEEDS
//!   under X2a (see `xver_page_break_internal_renders` above), so this test
//!   is repurposed to the type-half shape the guard still covers.
//! - [`xver_internal_forked_type_ascription_renders`] (X2b headline,
//! "Slice X2" §X2.3/X2.4):
//!   the guard's TYPE half is narrowed to the export boundary only — a
//!   `V0_0` package whose exported command (still the NEUTRAL
//!   `block-boxes -> document`) uses an INTERNAL, explicitly `page`-typed
//!   local `let-rec` (`Expr::LetRecIn`'s own ascription — a shape the
//!   pre-X2b guard rejected via `collect_free_globals`'s over-conservative
//!   walk, even though the ascribed value never escapes the export) now
//!   compiles and renders end-to-end, proving the walk correctly stopped
//!   treating an internal ascription as export-boundary text.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_backend::{FontKey, FontMetrics, GraphicsElem, Length};
use rustyfi_lang::CompileError;
use rustyfi_loader::{LoadOptions, RustyfiVersion};

/// This repo's root, resolved relative to this crate's own manifest
/// directory (`crates/rustyfi-lang/../..`) — same helper every other
/// `v01_*`/`xver_*` integration test in this crate reproduces locally (no
/// shared test-support library target exists here).
fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// A small fixture tree under a unique temp directory (cleaned up on drop) —
/// mirrors `crates/rustyfi-loader/tests/loader.rs`'s own `TempDir` helper.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustyfi-lang-xver-test-{tag}-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            n
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// Write `content` to `rel` (a `/`-separated path relative to the temp
    /// root), creating any needed parent directories.
    fn write(&self, rel: &str, content: &str) {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&p, content).expect("write fixture file");
    }

    /// Copy the REAL vendored `lib-rustyfi/dist/packages/<name>` byte-for-
    /// byte into this temp tree's own `dist/packages/<name>` — so
    /// `@require: <name>` against `self.path()` as `lib_root` resolves to
    /// the actual frozen 0.0.6 corpus content, without needing a SECOND
    /// `lib_root` alongside this test's own V0_1 helper library (which must
    /// live in the SAME tree so `@import:` can reach it — see this file's
    /// module doc comment).
    fn copy_real_package(&self, name: &str) {
        let real = repo(&format!("lib-rustyfi/dist/packages/{name}"));
        let content = fs::read_to_string(&real).unwrap_or_else(|e| panic!("read {real:?}: {e}"));
        self.write(&format!("dist/packages/{name}"), &content);
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A real (ASCII-only) `FontMetrics` — the positive-case fixture renders
/// actual digits (`arabic`'s output), so `advance` must return `Some` for
/// ASCII — same shape as every other `v01_*` test's `Mono` stub.
struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, c: char, size: Length) -> Option<Length> {
        if c.is_ascii() {
            Some(size * 0.5)
        } else {
            None
        }
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

/// A tiny V0_1 helper library providing `get-initial-context`'s mandatory
/// `[math] inline-cmd` argument — `V0_1`'s `FileV1::Document` has no prelude
/// of its own (every `let` chains its own `in`), so a document-scaffolding
/// command like `\math` must come from a required/imported library, exactly
/// like the real `v01-mini.satyh`/`stdja-mini.satyh` fixtures do. Reached via
/// `@import:` (a same-directory SIBLING of the entry), NOT `@require:` — so
/// it is NOT a `@require:`-resolved corpus target and correctly keeps
/// `opts.version` (`V0_1`) under the X1 Q4 per-file detection rule, even
/// though (like every 0.0.6-style `module … = struct … end` head) plain
/// content sniffing alone would be ambiguous (`version.rs`'s own doc
/// comment: a bare `module` head is deliberately not a signal).
const XVER_HELPER_SRC: &str =
    "module XverHelper = struct\n  val inline ctx \\math m = embed-math ctx (read-math ctx m)\nend\n";

/// The positive-case V0_1 entry: `@require:`s the real vendored
/// `list`/`option` (version-neutral, X1's target subset), `@import:`s the
/// local math-command helper above, and actually calls `List.map`/
/// `List.fold-left`/`Option.from` — end-to-end use, not just a parse probe.
fn positive_entry_src() -> String {
    "@require: list\n\
     @require: option\n\
     @import: xver-helper\n\
     \n\
     let mapped = List.map (fun x -> x + 1) [1, 2, 3] in\n\
     let summed = List.fold-left (fun a b -> a + b) 0 mapped in\n\
     let combined = Option.from 0 (Some summed) in\n\
     let ctx = get-initial-context 440pt (command \\XverHelper.math) in\n\
     let content pbinfo = (| text-origin = (72pt, 100pt), text-height = 640pt |) in\n\
     let parts pbinfo =\n\
       (| header-origin = (72pt, 72pt),  header-content = block-nil,\n\
          footer-origin = (72pt, 800pt), footer-content = block-nil |)\n\
     in\n\
     let body =\n\
       line-break true true ctx\n\
         (inline-fil ++ (read-inline ctx (embed-string (arabic combined))) ++ inline-fil)\n\
     in\n\
     page-break (210mm, 297mm) content parts body\n"
        .to_string()
}

#[test]
fn positive_case_list_and_option_render() {
    let dir = TempDir::new("positive");
    dir.copy_real_package("list.satyg");
    dir.copy_real_package("option.satyg");
    dir.write("xver-helper.satyh", XVER_HELPER_SRC);
    let entry_src = positive_entry_src();
    dir.write("entry.saty", &entry_src);

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the positive xver fixture should succeed: {e}"));

    // The two REQUIRE'd corpus packages (list, option — option is also
    // pulled in transitively by list.satyg's own `@require: option`, so
    // both are deduplicated to ONE node each) must have been detected as
    // `V0_0` (Q4's provenance rule), while the `@import:`-reached helper
    // and the entry itself stay `V0_1`.
    let mut saw_v006 = 0;
    for f in &program.files[..program.files.len() - 1] {
        match f.version {
            RustyfiVersion::V0_0 => {
                saw_v006 += 1;
                assert!(
                    matches!(f.cst, rustyfi_loader::LoadedCst::V0_0(_)),
                    "a V0_0-tagged file must carry a V0_0-parsed cst: {:?}",
                    f.path
                );
            }
            RustyfiVersion::V0_1 => {
                assert!(
                    matches!(f.cst, rustyfi_loader::LoadedCst::V0_1(_)),
                    "a V0_1-tagged file must carry a V0_1-parsed cst: {:?}",
                    f.path
                );
            }
            other => panic!("unexpected version tag {other} on {:?}", f.path),
        }
    }
    assert_eq!(
        saw_v006, 2,
        "list.satyg + option.satyg should both be V0_0-tagged deps"
    );
    let entry_file = program
        .files
        .last()
        .expect("loader always yields the entry last");
    assert!(
        matches!(entry_file.version, RustyfiVersion::V0_1),
        "the entry must always stay V0_1"
    );

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v1_with_trials(&program.files, &mono)
        .unwrap_or_else(|e| panic!("compile_document_v1_with_trials should succeed: {e}"));

    // `mapped = [2;3;4]`, `summed = 9` (List.map/List.fold-left, from the
    // spliced list.satyg), `combined = Option.from 0 (Some 9) = 9` (from the
    // spliced option.satyg) — a real page with real placed content proves
    // the whole pipeline (splice -> elaborate -> typecheck -> eval ->
    // layout) actually ran on the cross-version-linked bindings, not merely
    // that compilation returned `Ok`.
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the footer line (rendering `combined` = 9) must have been placed"
    );
}

// ============================================================================
// X2a: a V0_0 package that uses a version-forked primitive (`page-break`)
// INTERNALLY, behind a neutral-typed (`block-boxes -> document`) export.
// ============================================================================

/// The `V0_0` package: two locally-defined `page-content-info`/
/// `page-parts` closures plus one exported `make-doc`, all bare (unqualified
/// — no `module .. = struct .. end` wrapper; `negative_case_forked_primitive_
/// is_rejected` — X1's original version of this file — already established
/// that a bare top-level `let` splices unqualified) top-level bindings.
/// `page-break A4Paper …` is the `V0_0`-ONLY shape: `page-break`'s first
/// argument is the `page` ADT (`A4Paper`/`UserDefinedPaper`) under `V0_0`,
/// vs. a `length * length` tuple under `V0_1` (`page_prims.rs`'s
/// `page_break_first_arg_is_page_adt_under_v006`/`_length_pair_under_v01`) —
/// exactly the shape X1 hard-rejected and X2a now makes sound.
const XVER_PAGEBREAK_PKG_SRC: &str = "\
@stage: persistent

let xver-content pbinfo = (| text-origin = (0pt, 0pt); text-height = 400pt |)
let xver-parts pbinfo =
  (| header-origin = (0pt, 0pt); header-content = block-nil;
     footer-origin = (0pt, 0pt); footer-content = block-nil |)
let make-doc body = page-break A4Paper xver-content xver-parts body
";

/// Build (and load) the shared X2a fixture tree: the `V0_0`
/// `xver-pagebreak` package (above) at `dist/packages/`, the `V0_1`
/// `xver-helper` math-command helper (reused from the X1 positive case,
/// reached via `@import:` so it stays `V0_1` under the Q4 rule), and an
/// entry whose body is `entry_tail` — so both X2a tests below share the same
/// scaffolding and differ only in what the entry does with `make-doc`.
fn load_xver_pagebreak_fixture(tag: &str, entry_tail: &str) -> Vec<rustyfi_loader::LoadedFile> {
    let dir = TempDir::new(tag);
    dir.write("dist/packages/xver-pagebreak.satyg", XVER_PAGEBREAK_PKG_SRC);
    dir.write("xver-helper.satyh", XVER_HELPER_SRC);
    let entry_src = format!(
        "@require: xver-pagebreak\n\
         @import: xver-helper\n\
         \n\
         let ctx = get-initial-context 440pt (command \\XverHelper.math) in\n\
         let body = line-break true true ctx (read-inline ctx {{Hello, cross-version world!}}) in\n\
         {entry_tail}\n"
    );
    dir.write("entry.saty", &entry_src);

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts).unwrap_or_else(|e| {
        panic!("loading the X2a page-break xver fixture ({tag}) should succeed: {e}")
    });
    assert!(
        program
            .files
            .iter()
            .any(|f| matches!(f.cst, rustyfi_loader::LoadedCst::V0_0(_))),
        "xver-pagebreak.satyg should have been detected as V0_0 (it's a @require: corpus target)"
    );
    // `dir` (and its temp files) would be removed on drop right here if kept
    // local — the loader has already read everything it needs into
    // `program.files`, so returning just the files (not `dir`) is fine; drop
    // it explicitly for clarity.
    drop(dir);
    program.files
}

/// X2a headline (X2.6's `xver_page_break_internal_renders`): the case X1
/// hard-rejected (`negative_case_forked_primitive_is_rejected` used to
/// pin exactly this shape) now renders end-to-end.
#[test]
fn xver_page_break_internal_renders() {
    let files = load_xver_pagebreak_fixture("headline", "make-doc body");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v1_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a V0_0 dependency using page-break internally behind a neutral \
                 export should now compile+render under X2a: {e}"
            )
        });

    // `A4Paper` (V0_0's page ADT) — one page, and the `line-break`d body
    // was actually placed onto it (not silently dropped).
    assert_eq!(
        doc.pages.len(),
        1,
        "one A4 page from the V0_0 page-break call"
    );
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the line-break'd body must have been placed on the page make-doc built"
    );
}

/// X2a soundness core (X2.6's `xver_forked_prim_coexist`): the SAME `V0_0`
/// dependency's internal `page-break A4Paper …` call, PLUS the `V0_1` entry
/// ALSO calling `page-break` directly with `V0_1`'s `(length * length)`
/// geometry — in the SAME compiled program. If both calls had resolved to
/// the SAME `PrimDef` (the X1-era bug this whole slice fixes), one of the
/// two shapes (`A4Paper` vs. a `(210mm, 297mm)` tuple) would be a
/// `TypeError` against the OTHER version's `page-break` signature
/// (`page_prims.rs`'s `page_break_first_arg_is_page_adt_under_v006`/
/// `_length_pair_under_v01` pin the two shapes are genuinely incompatible) —
/// so a successful render IS the coexistence proof.
#[test]
fn xver_forked_prim_coexist() {
    let entry_tail = "\
         let v01-content pbinfo = (| text-origin = (72pt, 100pt), text-height = 640pt |) in\n\
         let v01-parts pbinfo =\n\
           (| header-origin = (72pt, 72pt), header-content = block-nil,\n\
              footer-origin = (72pt, 800pt), footer-content = block-nil |)\n\
         in\n\
         let v01-pagebreak-probe =\n\
           page-break (210mm, 297mm) v01-content v01-parts block-nil\n\
         in\n\
         make-doc body";
    let files = load_xver_pagebreak_fixture("coexist", entry_tail);

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v1_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "the V0_0 dependency's internal `page-break A4Paper ..` and the V0_1 \
                 entry's own direct `page-break (length * length) ..` call must both \
                 resolve to their OWN version's PrimDef and coexist: {e}"
            )
        });

    // The FINAL result is still `make-doc body` (the V0_0 A4 page) — the
    // probe's own document is computed (and discarded) purely to prove its
    // *typecheck+eval* succeeded alongside the dependency's internal call.
    assert_eq!(
        doc.pages.len(),
        1,
        "one A4 page from the V0_0 page-break call"
    );
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the line-break'd body must have been placed on the page make-doc built"
    );
}

// ============================================================================
// X2b (design-cross-version-import.md's "Slice X2" §X2.3/X2.4): the guard's
// TYPE half narrows to the export boundary only — an INTERNAL forked-type
// ascription (a local `let rec .. : ty = ..` nested inside another binding's
// body) no longer trips `collect_free_globals`'s free-type scan.
// ============================================================================

/// The `V0_0` package: same neutral-export shape as
/// `XVER_PAGEBREAK_PKG_SRC` (`make-doc : block-boxes -> document`), but this
/// time `page-break`'s first argument is produced by an INTERNAL, explicitly
/// `page`-typed local `let rec` (`Expr::LetRecIn`'s own `: page` ascription)
/// rather than a bare `A4Paper` literal — the ascription text is exactly the
/// site `walk_rec_binding_body`'s `boundary` flag now suppresses when it is
/// NOT a top-level `TopBinding::LetRec`. Before X2b, `collect_free_globals`
/// walked this ascription unconditionally (`Expr::LetRecIn` shares
/// `walk_rec_binding_body` with `TopBinding::LetRec`), so `free.types`
/// contained `"page"` and the whole dependency was hard-rejected by
/// `v1::xver_adapt::reject_type_names()` even though `get-page`'s `page`-ADT
/// value never escapes `make-doc`'s own `block-boxes -> document` export.
const XVER_INTERNAL_TYPED_PKG_SRC: &str = "\
@stage: persistent

let xver-content pbinfo = (| text-origin = (0pt, 0pt); text-height = 400pt |)
let xver-parts pbinfo =
  (| header-origin = (0pt, 0pt); header-content = block-nil;
     footer-origin = (0pt, 0pt); footer-content = block-nil |)
let make-doc body =
  let-rec get-page : page | () = A4Paper in
  page-break (get-page ()) xver-content xver-parts body
";

/// X2b headline (`xver_internal_forked_type_ascription_renders`): the shape
/// X1/X2a's over-conservative type-half guard rejected (an internal `: page`
/// ascription, textually present but never boundary-observable) now compiles
/// and renders end-to-end. Proves BOTH halves of the narrowing: (a) the
/// pre-typecheck guard no longer rejects at the splice arm (a rejection
/// would surface as `CompileError::CrossVersionUnsupportedName`, not
/// whatever assertion below), and (b) the internal ascription's `: page`
/// text is still MEANINGFUL to the version-scoped typechecker (X2a's
/// `Ast::VersionScope(V0_0, _)` swaps in `base_type_env_with_version
/// (V0_0)` for `make-doc`'s whole body, so `get-page`'s ascribed `page`
/// type and `A4Paper`'s ctor both resolve under V0_0 — if the ascription
/// were silently ignored rather than genuinely accepted-and-typechecked,
/// a real ascription/value mismatch elsewhere would go undetected; this test
/// only pins the success path, `xver_boundary_forked_type_page_rejected`
/// below continues to pin that a TOP-LEVEL `page`-typed export still
/// rejects).
#[test]
fn xver_internal_forked_type_ascription_renders() {
    let dir = TempDir::new("internal-typed-ascription");
    dir.write(
        "dist/packages/xver-internal-typed.satyg",
        XVER_INTERNAL_TYPED_PKG_SRC,
    );
    dir.write("xver-helper.satyh", XVER_HELPER_SRC);
    let entry_src = "\
@require: xver-internal-typed
@import: xver-helper

let ctx = get-initial-context 440pt (command \\XverHelper.math) in
let body = line-break true true ctx (read-inline ctx {Hello, internal-typed world!}) in
make-doc body
";
    dir.write("entry.saty", entry_src);

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts).unwrap_or_else(|e| {
        panic!("loading the X2b internal-typed-ascription xver fixture should succeed: {e}")
    });
    assert!(
        program.files.iter().any(|f| matches!(f.cst, rustyfi_loader::LoadedCst::V0_0(_))),
        "xver-internal-typed.satyg should have been detected as V0_0 (it's a @require: corpus target)"
    );

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v1_with_trials(&program.files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a V0_0 dependency using a `page`-typed local `let rec` PURELY \
                 INTERNALLY (behind a neutral `block-boxes -> document` export) should \
                 now compile+render under X2b: {e}"
            )
        });

    // `A4Paper` (V0_0's page ADT, produced by the internal `get-page`) —
    // one page, and the `line-break`d body was actually placed onto it.
    assert_eq!(
        doc.pages.len(),
        1,
        "one A4 page from the internally-produced page-break call"
    );
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the line-break'd body must have been placed on the page make-doc built"
    );
}

// ============================================================================
// X2a negative (repurposed from X1): the TYPE half of the forked-name guard
// stays conservative — a V0_0 dependency that textually names a forked
// TYPE is still rejected, even internally (not narrowed to "on the export
// boundary only" — that's X2b/X3's job, design doc §X2.3/X2.4).
// ============================================================================

/// X1's original negative test pinned `page-break` (a forked VALUE) here;
/// X2a repurposed it to a forked TYPE reference (`math`) — X3a now RELABELS
/// `math` and accepts it (see `xver_export_math_relabels_and_renders`
/// below), so this test is repointed to `page`: X3.1's note explains `page`
/// never shows up in `typecheck::forked_type_names()`'s automatic diff (its
/// bare NAME lowers identically under both versions) but its VALUE
/// representation still forks (0.0.6: a 9-ctor ADT, `Value::Ctor`; 0.1: a
/// `length*length` tuple, `Value::Product`) — `v1::xver_adapt::
/// reject_type_names()` adds it explicitly, so a `V0_0` dependency naming
/// it (even in a plain, unused `type` synonym) must still be rejected, now
/// with `slice: "X3"` (the type-half check moved to `v1::xver_adapt`'s
/// classification) — *before* any elaboration/typecheck/eval runs on the
/// spliced dependency.
#[test]
fn negative_case_forked_type_on_boundary_is_still_rejected() {
    let dir = TempDir::new("negative-type");
    dir.write(
        "dist/packages/xver-forked-type.satyg",
        "@stage: persistent\n\ntype xver-page-alias = page\n",
    );
    dir.write("entry.saty", "@require: xver-forked-type\n\n0\n");

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the negative xver fixture should succeed: {e}"));
    assert!(
        matches!(
            program.files[0].cst,
            rustyfi_loader::LoadedCst::V0_0(_)
        ),
        "xver-forked-type.satyg should have been detected as V0_0 (it's a @require: corpus target)"
    );

    let mono = Mono;
    let err = rustyfi_lang::compile_document_v1(&program.files, &mono)
        .expect_err("a dependency referencing a version-forked TYPE must still be rejected");
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "page");
            assert_eq!(slice, "X3");
        }
        other => panic!("expected CrossVersionUnsupportedName, got: {other}"),
    }
}

/// The SAME forked type, spelled as a top-level value ascription rather than
/// a `type` declaration. Both are export-position text a consumer can see, so
/// both must reject; the walk used to visit `LetRec`'s ascription and not
/// plain `Let`'s, so `let x : page = ..` crossed the boundary silently while
/// `type a = page` was refused -- a guard whose whole purpose is preventing a
/// silent mis-render, decided by which way the package happened to spell it.
///
/// `page` is the sharpest case: 0.0.6 represents it as a 9-ctor ADT
/// (`Value::Ctor`), 0.1 as a `length * length` tuple (`Value::Product`), so a
/// value that slips through has no shared runtime representation at all.
#[test]
fn negative_case_forked_type_in_a_value_ascription_is_rejected() {
    let dir = TempDir::new("negative-ascription");
    dir.write(
        "dist/packages/xver-forked-ann.satyg",
        "@stage: persistent

let xver-page : page = A4Paper
",
    );
    dir.write("entry.saty", "@require: xver-forked-ann

0
");

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the fixture should succeed: {e}"));

    let mono = Mono;
    let err = rustyfi_lang::compile_document_v1(&program.files, &mono)
        .expect_err("a value ascribed with a version-forked type must be rejected");
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "page");
            assert_eq!(slice, "X3");
        }
        other => panic!("expected CrossVersionUnsupportedName, got: {other}"),
    }
}

/// The other half of that fix: walking ascriptions must not start rejecting
/// the version-NEUTRAL ones. A `let x : int = ..` names nothing forked and has
/// to keep crossing, or tightening the guard would break every 0.0.6 package
/// that annotates an ordinary export.
#[test]
fn a_version_neutral_value_ascription_still_crosses() {
    let dir = TempDir::new("neutral-ascription");
    dir.write(
        "dist/packages/xver-neutral-ann.satyg",
        "@stage: persistent

let xver-count : int = 42
",
    );
    dir.write("entry.saty", "@require: xver-neutral-ann

0
");

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the fixture should succeed: {e}"));

    // The entry is a bare `0`, so the compile still fails -- but on being a
    // non-document, which is only reachable AFTER the boundary check has
    // passed the dependency. What matters is which error comes back.
    let mono = Mono;
    match rustyfi_lang::compile_document_v1(&program.files, &mono) {
        Err(CompileError::CrossVersionUnsupportedName { name, .. }) => {
            panic!("a version-neutral ascription must not be rejected, but `{name}` was")
        }
        _ => {}
    }
}

// ============================================================================
// X3a ("Slice X3 — forked-type export adapter", X3.6's test plan): `math` is
// the sole (a)-class forked type — representationally identical to `V0_1`'s
// `math-text` (same shared `Value::MathText`/`Value::Math`, `types.rs`'s
// `BaseType::MathText`) — so a `V0_0` export naming it now RELABELS instead
// of rejecting. Everything else forked (`page`, and every opaque nominal —
// `deco` included, X3a defers a real value adapter to X3b) still
// hard-rejects.
// ============================================================================

/// The `V0_0` package: a `type .. = .. of math` variant (the ONE surface
/// site this port's typechecker actually consults for type text — see
/// `v1::xver_adapt`'s module doc comment) wrapping/unwrapping a REAL math
/// value built via `${1}` (embedded math syntax, `BaseType::MathText`).
/// Without X3a's relabel, `XverMathWrap`'s ctor payload would register as
/// the nominal `Variant("math",[])` under the merged program's `V0_1`
/// whole-program `Checker` (`typecheck.rs`'s `name_to_mono("math", V0_1)`
/// falls through to the unbound-nominal default — 0.1 has no `math` type at
/// all), so `XverMathWrap(${1})` — constructed inside `xver-get-math`'s
/// `Ast::VersionScope(V0_0, _)`-wrapped body — would fail to unify against
/// the REAL `Base(MathText)` `${1}` produces: a genuine `TypeError`, not a
/// silent mis-render. This is what proves the relabel is load-bearing, not
/// cosmetic.
const XVER_MATH_EXPORT_PKG_SRC: &str = "\
@stage: persistent

type xver-math-wrap = XverMathWrap of math

let xver-get-math () = XverMathWrap(${1})

let xver-unwrap-math w =
  match w with
  | XverMathWrap(m) -> m
";

/// X3.6's `xver_export_math_relabels_and_renders` (POSITIVE): the `V0_1`
/// entry `@require:`s the package above, unwraps its `math`-typed export,
/// and feeds the crossed value straight into `V0_1`'s own `read-math`/
/// `embed-math` primitives (whose signatures expect `math-text` —
/// `prim_types.rs`'s `t_math_text()`, the SAME `Base(MathText)` the crossed
/// value already carries) — no further adaptation needed, proving the
/// relabel is transparent: the whole-program merge type-checks AND the
/// resulting inline boxes actually get placed onto a real page.
#[test]
fn xver_export_math_relabels_and_renders() {
    let dir = TempDir::new("math-export-positive");
    dir.write(
        "dist/packages/xver-math-export.satyg",
        XVER_MATH_EXPORT_PKG_SRC,
    );
    dir.write("xver-helper.satyh", XVER_HELPER_SRC);
    let entry_src = "\
@require: xver-math-export
@import: xver-helper

let wrapped = xver-get-math () in
let m = xver-unwrap-math wrapped in
let ctx = get-initial-context 440pt (command \\XverHelper.math) in
let body = line-break true true ctx (embed-math ctx (read-math ctx m)) in
let content pbinfo = (| text-origin = (72pt, 100pt), text-height = 640pt |) in
let parts pbinfo =
  (| header-origin = (72pt, 72pt),  header-content = block-nil,
     footer-origin = (72pt, 800pt), footer-content = block-nil |)
in
page-break (210mm, 297mm) content parts body
";
    dir.write("entry.saty", entry_src);

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the math-export xver fixture should succeed: {e}"));
    assert!(
        program.files.iter().any(|f| matches!(f.cst, rustyfi_loader::LoadedCst::V0_0(_))),
        "xver-math-export.satyg should have been detected as V0_0 (it's a @require: corpus target)"
    );

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v1_with_trials(&program.files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a V0_0 dependency exporting a `math`-typed value (via a `type .. of \
                 math` ctor) should relabel to `math-text` and compile+render under X3a: {e}"
            )
        });

    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the embedded math body (crossed from the V0_0 dependency) must have been placed"
    );
}

/// X3.6's `xver_boundary_forked_type_page_rejected` (NEGATIVE): a `V0_0`
/// dependency actually EXPORTING (not just naming, unused) a `page`-typed
/// value still fails loudly at compile time, never reaching eval — `page`'s
/// runtime representation (a 9-ctor ADT, `Value::Ctor`) has no `V0_1`
/// counterpart (a `length*length` tuple, `Value::Product`), so relabeling
/// would be unsound (X3.8/S1).
#[test]
fn xver_boundary_forked_type_page_rejected() {
    let dir = TempDir::new("page-export-negative");
    dir.write(
        "dist/packages/xver-page-export.satyg",
        // `let-rec name : ty | patbot* = value` — `RecBinding`'s `COLON ty
        // BAR` ascription form (`cst.rs`'s `RecAscription` doc comment); the
        // only surface syntax this grammar has for putting an explicit type
        // next to a genuine VALUE export (`xver-get-page : unit -> page`,
        // `A4Paper` a real 0.0.6 page-ADT ctor).
        "@stage: persistent\n\nlet-rec xver-get-page : page | () = A4Paper\n",
    );
    dir.write("entry.saty", "@require: xver-page-export\n\n0\n");

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the page-export negative fixture should succeed: {e}"));

    let mono = Mono;
    let err = rustyfi_lang::compile_document_v1(&program.files, &mono)
        .expect_err("a dependency exporting a `page`-typed value must still be rejected");
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "page");
            assert_eq!(slice, "X3");
        }
        other => panic!("expected CrossVersionUnsupportedName, got: {other}"),
    }
}

/// X3.6's `xver_boundary_deco_export_coerces_and_renders` (POSITIVE, X3b —
/// repurposed from the X3a-era `xver_boundary_deco_export_rejected`): the
/// `V0_0` package below has TWO independent `deco`-touching bindings:
///
/// - `xver-deco-alias`, a bare `type .. = deco` synonym — the ORIGINAL X3a
///   fixture, which needs no value-level coercion at all (X3b's module doc
///   comment: `deco`'s bare NAME already means the right thing under `V0_1`
///   once accepted — no relabel, unlike `math`);
/// - `xver-frame-deco`, a REAL top-level `let-rec .. : deco | (x, y) w h d =
///   ..` export whose body returns a `graphics list` (0.0.6 semantics) — the
///   genuinely value-coerced case: X3b splices a second, un-scoped binding of
///   the SAME name that unites that list into a single `graphics` via the
///   real `V0_1` `unite-graphics` primitive.
///
/// The `V0_1` entry `@require:`s the package and applies `xver-frame-deco`
/// through `inline-frame-outer` (a REAL consumer — `primitives::apply_deco`,
/// fired by `lib.rs`'s `fire_inline_frame` at render time, not a stand-in),
/// so the coercion is exercised end-to-end: if the wrap were missing (or
/// wrong-shaped), `apply_deco`'s `coerce_graphics_result` would see the raw
/// `Value::List` under the AMBIENT `V0_1` `interp.version` and `as_graphics`
/// would fail with an `EvalError`, not silently mis-render. A successful
/// render whose fired page graphics contain a `GraphicsElem::Group` (the
/// `unite-graphics` wrapper, `primitives.rs`'s `prim_unite_graphics`) is the
/// coercion proof.
#[test]
fn xver_boundary_deco_export_coerces_and_renders() {
    let dir = TempDir::new("deco-export-positive");
    dir.write(
        "dist/packages/xver-deco-export.satyg",
        "@stage: persistent\n\n\
         type xver-deco-alias = deco\n\n\
         let-rec xver-frame-deco : deco | (x, y) w h d =\n\
         \x20 [\n\
         \x20   fill (Gray(0.0))\n\
         \x20     (close-with-line\n\
         \x20        (line-to (x +' w, y +' h)\n\
         \x20           (line-to (x +' w, y)\n\
         \x20              (start-path (x, y)))))\n\
         \x20 ]\n",
    );
    dir.write("xver-helper.satyh", XVER_HELPER_SRC);
    let entry_src = "\
@require: xver-deco-export
@import: xver-helper

let ctx = get-initial-context 440pt (command \\XverHelper.math) in
let framed =
  inline-frame-outer (2pt, 2pt, 2pt, 2pt) xver-frame-deco
    (read-inline ctx {Hello, cross-version deco world!})
in
let body = line-break true true ctx framed in
let content pbinfo = (| text-origin = (72pt, 100pt), text-height = 640pt |) in
let parts pbinfo =
  (| header-origin = (72pt, 72pt),  header-content = block-nil,
     footer-origin = (72pt, 800pt), footer-content = block-nil |)
in
page-break (210mm, 297mm) content parts body
";
    dir.write("entry.saty", entry_src);

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the deco-export positive fixture should succeed: {e}"));
    assert!(
        program.files.iter().any(|f| matches!(f.cst, rustyfi_loader::LoadedCst::V0_0(_))),
        "xver-deco-export.satyg should have been detected as V0_0 (it's a @require: corpus target)"
    );

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v1_with_trials(&program.files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a V0_0 dependency exporting a bare `: deco` value should be value-\
                 coerced (graphics list -> graphics via unite-graphics) and compile+render \
                 under X3b: {e}"
            )
        });

    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the framed inline content must have been placed"
    );
    assert!(
        doc.extras.page_graphics[0]
            .iter()
            .any(|g| matches!(g, GraphicsElem::Group(_))),
        "the fired deco's graphics must include a unite-graphics Group (the X3b wrap), got: {:?}",
        doc.extras.page_graphics[0]
    );
}

/// X3b, extended: the shape the REAL 0.0.6 corpus uses — a `deco` export that
/// is both ARROW-TAILED (`length -> deco`) and MODULE-SCOPED (inside `module
/// .. : sig .. end`). `deco.satyh`, which every `std-ja` document reaches, is
/// exactly this (`val simple-frame : length -> color -> color -> deco`), and
/// both properties were originally outside X3b's scope.
///
/// The module case cannot reuse the top-level mechanism: a member is named
/// `XverDeco.frame` and there is no `let XverDeco.frame` to write, so the
/// wrapper is appended INSIDE the module's own decls
/// (`xver_adapt::inject_module_deco_wrappers`) where sequential shadowing
/// applies. As in the bare-export test above, the `GraphicsElem::Group` in the
/// FIRED page graphics is the proof the coercion actually ran: 0.0.6's deco
/// returns `graphics list`, and only `unite-graphics` builds a `Group`.
#[test]
fn xver_boundary_deco_export_module_scoped_curried_coerces_and_renders() {
    let dir = TempDir::new("deco-export-module-curried");
    dir.write(
        "dist/packages/xver-deco-mod.satyg",
        "@stage: persistent\n\n\
         module XverDeco : sig\n\
         \x20 val frame : length -> deco\n\
         end = struct\n\
         \x20 let frame t (x, y) w h d =\n\
         \x20   [\n\
         \x20     fill (Gray(0.0))\n\
         \x20       (close-with-line\n\
         \x20          (line-to (x +\' w, y +\' h)\n\
         \x20             (line-to (x +\' w, y)\n\
         \x20                (start-path (x, y)))))\n\
         \x20   ]\n\
         end\n",
    );
    dir.write("xver-helper.satyh", XVER_HELPER_SRC);
    dir.write(
        "entry.saty",
        "\
@require: xver-deco-mod
@import: xver-helper

let ctx = get-initial-context 440pt (command \\XverHelper.math) in
let framed =
  inline-frame-outer (2pt, 2pt, 2pt, 2pt) (XverDeco.frame 1pt)
    (read-inline ctx {Hello, module-scoped cross-version deco!})
in
let body = line-break true true ctx framed in
let content pbinfo = (| text-origin = (72pt, 100pt), text-height = 640pt |) in
let parts pbinfo =
  (| header-origin = (72pt, 72pt),  header-content = block-nil,
     footer-origin = (72pt, 800pt), footer-content = block-nil |)
in
page-break (210mm, 297mm) content parts body
",
    );

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the module-deco fixture should succeed: {e}"));

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v1_with_trials(&program.files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a module-scoped, arrow-tailed V0_0 `deco` export should be value-coerced \
                 (graphics list -> graphics) and compile+render: {e}"
            )
        });

    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        doc.extras.page_graphics[0]
            .iter()
            .any(|g| matches!(g, GraphicsElem::Group(_))),
        "the fired deco's graphics must include a unite-graphics Group (proof the \
         in-module wrapper ran), got: {:?}",
        doc.extras.page_graphics[0]
    );
}

/// X3.6's `xver_boundary_mathboxes_not_aliased` (NEGATIVE/soundness, S2): a
/// `V0_0` export whose crossed value is `math` (relabeled to
/// `math-text`) must NOT satisfy a `V0_1` `math-boxes` site — `math-boxes`
/// is `V0_1`-only (the EVALUATED math tree, `BaseType::MathBoxes`,
/// value.rs:57, deliberately distinct from `BaseType::MathText`) and no
/// `V0_0` value is ever shaped that way. Passing the crossed (still
/// `math-text`) value directly to `embed-math`, which expects `math-boxes`
/// under `V0_1`, must be a compile-time `TypeError`.
#[test]
fn xver_boundary_mathboxes_not_aliased() {
    let dir = TempDir::new("mathboxes-not-aliased");
    dir.write(
        "dist/packages/xver-math-export.satyg",
        XVER_MATH_EXPORT_PKG_SRC,
    );
    let entry_src = "\
@require: xver-math-export

let wrapped = xver-get-math () in
let m = xver-unwrap-math wrapped in
let ctx = get-initial-context 440pt (command \\XverHelperUnused.math) in
% `embed-math` expects `math-boxes`, not the crossed `math-text` value `m`
% directly — this must be a TypeError, proving `math` never aliases
% `math-boxes`.
let bad = embed-math ctx m in
0
";
    dir.write("entry.saty", entry_src);

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    // The entry never resolves `\XverHelperUnused.math` (no such import) —
    // that's fine, loading only needs to reach the point where
    // `compile_document_v1` typechecks the merged program and rejects the
    // `math-text` vs `math-boxes` mismatch; an unrelated unbound-module
    // error would also demonstrate nothing, so this fixture keeps the
    // `embed-math ctx m` misuse as the FIRST thing that could fail. If the
    // module reference itself turns out to error first, the assertion below
    // still holds the real invariant: `compile_document_v1` must reject
    // this program somehow, and if it's a type error, the mismatch below is
    // the exact one X3a's soundness guarantees.
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts).unwrap_or_else(|e| {
        panic!("loading the mathboxes-not-aliased fixture should succeed: {e}")
    });

    let mono = Mono;
    let err = rustyfi_lang::compile_document_v1(&program.files, &mono)
        .expect_err("`math` (relabeled to `math-text`) must not satisfy a `math-boxes` site");
    // Any rejection here is the soundness proof (a `math-text`-vs-
    // `math-boxes` mismatch surfaces as `CompileError::Type`, an HM
    // unification failure — NOT `CrossVersionUnsupportedName`, since the
    // crossing itself was accepted).
    assert!(
        matches!(err, CompileError::Type(_) | CompileError::Elaborate(_)),
        "expected a real type/elaboration error from the math-text/math-boxes mismatch, got: {err}"
    );
}

/// X3b positive (`deco-set`, the tuple-of-4 sibling of
/// `xver_boundary_deco_export_coerces_and_renders`): a `V0_0` package
/// exports a bare `: deco-set` value via the mandatory `| ()` idiom
/// (`elaborate.rs` requires a `let-rec`'s RHS to be a function, so a plain
/// tuple value can ONLY be written this way — the same idiom
/// `xver_boundary_forked_type_page_rejected`'s `xver-get-page : page | () =
/// A4Paper` fixture uses; see `xver_adapt.rs`'s `classify_rec_binding_deco`
/// doc comment) whose FIRST component (`decoS`, the only one
/// `inline-frame-breakable` actually fires for an unbroken frame — see
/// `primitives.rs`'s `prim_inline_frame_breakable` doc comment) returns a
/// real `graphics list`. The `V0_1` entry applies it through
/// `inline-frame-breakable` (a REAL consumer, same firing path as
/// `inline-frame-outer` — `lib.rs`'s `fire_inline_frame`/`apply_deco`), so
/// a wrong/missing coercion on ANY of the four wrapped components would
/// surface as an `EvalError`, not a silent mis-render.
#[test]
fn xver_boundary_decoset_export_coerces_and_renders() {
    let dir = TempDir::new("decoset-export-positive");
    dir.write(
        "dist/packages/xver-decoset-export.satyg",
        "@stage: persistent\n\n\
         let-rec xver-my-decoset : deco-set | () =\n\
         \x20 (\n\
         \x20   (fun (x, y) w h d ->\n\
         \x20      [\n\
         \x20        fill (Gray(0.0))\n\
         \x20          (close-with-line\n\
         \x20             (line-to (x +' w, y +' h)\n\
         \x20                (line-to (x +' w, y)\n\
         \x20                   (start-path (x, y)))))\n\
         \x20      ]),\n\
         \x20   (fun (x, y) w h d -> []),\n\
         \x20   (fun (x, y) w h d -> []),\n\
         \x20   (fun (x, y) w h d -> [])\n\
         \x20 )\n",
    );
    dir.write("xver-helper.satyh", XVER_HELPER_SRC);
    let entry_src = "\
@require: xver-decoset-export
@import: xver-helper

let ctx = get-initial-context 440pt (command \\XverHelper.math) in
let framed =
  inline-frame-breakable (2pt, 2pt, 2pt, 2pt) xver-my-decoset
    (read-inline ctx {Hello, cross-version deco-set world!})
in
let body = line-break true true ctx framed in
let content pbinfo = (| text-origin = (72pt, 100pt), text-height = 640pt |) in
let parts pbinfo =
  (| header-origin = (72pt, 72pt),  header-content = block-nil,
     footer-origin = (72pt, 800pt), footer-content = block-nil |)
in
page-break (210mm, 297mm) content parts body
";
    dir.write("entry.saty", entry_src);

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts).unwrap_or_else(|e| {
        panic!("loading the deco-set export positive fixture should succeed: {e}")
    });
    assert!(
        program.files.iter().any(|f| matches!(f.cst, rustyfi_loader::LoadedCst::V0_0(_))),
        "xver-decoset-export.satyg should have been detected as V0_0 (it's a @require: corpus target)"
    );

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v1_with_trials(&program.files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a V0_0 dependency exporting a bare `: deco-set` value should be value-\
                 coerced (each component's graphics list -> graphics via unite-graphics) and \
                 compile+render under X3b: {e}"
            )
        });

    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the framed inline content must have been placed"
    );
    assert!(
        doc.extras.page_graphics[0]
            .iter()
            .any(|g| matches!(g, GraphicsElem::Group(_))),
        "the fired decoS's graphics must include a unite-graphics Group (the X3b wrap), got: {:?}",
        doc.extras.page_graphics[0]
    );
}

/// X3b, nested-module increment: a `deco` export that sits one module
/// DEEPER than `xver_boundary_deco_export_module_scoped_curried_coerces_and_
/// renders`'s — `module XverOuter = struct module XverInner : sig val frame
/// : length -> deco end = struct .. end end`, reached as
/// `XverOuter.XverInner.frame`.
///
/// This used to be a hard rejection: `classify_deco_exports`'s module arm
/// walked its own `decls` with a reject-only helper
/// (`reject_if_nested_value_mentions_deco`), so a `deco` one level down
/// failed the whole dependency. It now RECURSES — the classifier is the same
/// one the top level uses, just under a longer `DecoExport::module_path`,
/// which `inject_module_deco_wrappers` already knew how to match. The
/// `GraphicsElem::Group` in the FIRED page graphics is the coercion proof, as
/// in the shallower fixtures: 0.0.6's deco returns `graphics list`, and only
/// `unite-graphics` builds a `Group`.
#[test]
fn xver_boundary_deco_export_nested_module_coerces_and_renders() {
    let dir = TempDir::new("deco-export-nested-module");
    dir.write(
        "dist/packages/xver-deco-nested.satyg",
        "@stage: persistent\n\n\
         module XverOuter = struct\n\
         \x20 module XverInner : sig\n\
         \x20   val frame : length -> deco\n\
         \x20 end = struct\n\
         \x20   let frame t (x, y) w h d =\n\
         \x20     [\n\
         \x20       fill (Gray(0.0))\n\
         \x20         (close-with-line\n\
         \x20            (line-to (x +\' w, y +\' h)\n\
         \x20               (line-to (x +\' w, y)\n\
         \x20                  (start-path (x, y)))))\n\
         \x20     ]\n\
         \x20 end\n\
         end\n",
    );
    dir.write("xver-helper.satyh", XVER_HELPER_SRC);
    dir.write(
        "entry.saty",
        "\
@require: xver-deco-nested
@import: xver-helper

let ctx = get-initial-context 440pt (command \\XverHelper.math) in
let framed =
  inline-frame-outer (2pt, 2pt, 2pt, 2pt) (XverOuter.XverInner.frame 1pt)
    (read-inline ctx {Hello, nested-module cross-version deco!})
in
let body = line-break true true ctx framed in
let content pbinfo = (| text-origin = (72pt, 100pt), text-height = 640pt |) in
let parts pbinfo =
  (| header-origin = (72pt, 72pt),  header-content = block-nil,
     footer-origin = (72pt, 800pt), footer-content = block-nil |)
in
page-break (210mm, 297mm) content parts body
",
    );

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the nested-module deco fixture should succeed: {e}"));

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v1_with_trials(&program.files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a deco export nested TWO modules deep should be classified and value-\
                 coerced, not rejected: {e}"
            )
        });

    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        doc.extras.page_graphics[0]
            .iter()
            .any(|g| matches!(g, GraphicsElem::Group(_))),
        "the fired deco's graphics must include a unite-graphics Group (proof the \
         nested in-module wrapper ran), got: {:?}",
        doc.extras.page_graphics[0]
    );
}

/// X3b, the other half of the nested-module increment: a `deco` export
/// carried by a struct member's OWN `let-rec .. : deco` ascription inside a
/// module with NO `sig` at all. `classify_deco_exports` used to see this
/// only through the reject-only `decls` walk; it now classifies it exactly
/// as a sig-declared member, under the enclosing module's path.
#[test]
fn xver_boundary_deco_export_sigless_module_ascription_coerces_and_renders() {
    let dir = TempDir::new("deco-export-sigless-module");
    dir.write(
        "dist/packages/xver-deco-sigless.satyg",
        "@stage: persistent\n\n\
         module XverSigless = struct\n\
         \x20 let-rec frame : length -> deco | t (x, y) w h d =\n\
         \x20   [\n\
         \x20     fill (Gray(0.0))\n\
         \x20       (close-with-line\n\
         \x20          (line-to (x +\' w, y +\' h)\n\
         \x20             (line-to (x +\' w, y)\n\
         \x20                (start-path (x, y)))))\n\
         \x20   ]\n\
         end\n",
    );
    dir.write("xver-helper.satyh", XVER_HELPER_SRC);
    dir.write(
        "entry.saty",
        "\
@require: xver-deco-sigless
@import: xver-helper

let ctx = get-initial-context 440pt (command \\XverHelper.math) in
let framed =
  inline-frame-outer (2pt, 2pt, 2pt, 2pt) (XverSigless.frame 1pt)
    (read-inline ctx {Hello, sig-less module cross-version deco!})
in
let body = line-break true true ctx framed in
let content pbinfo = (| text-origin = (72pt, 100pt), text-height = 640pt |) in
let parts pbinfo =
  (| header-origin = (72pt, 72pt),  header-content = block-nil,
     footer-origin = (72pt, 800pt), footer-content = block-nil |)
in
page-break (210mm, 297mm) content parts body
",
    );

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the sig-less module deco fixture should succeed: {e}"));

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v1_with_trials(&program.files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a `let-rec .. : deco` inside a sig-less module should be classified and \
                 value-coerced, not rejected: {e}"
            )
        });

    assert!(
        doc.extras.page_graphics[0]
            .iter()
            .any(|g| matches!(g, GraphicsElem::Group(_))),
        "the fired deco's graphics must include a unite-graphics Group, got: {:?}",
        doc.extras.page_graphics[0]
    );
}
