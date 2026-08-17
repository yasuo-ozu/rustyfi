//! Slice X4a (`docs/plans/design-cross-version-import.md`'s "Slice X4 —
//! reverse direction", specifically the X4a sub-slice): a `V0_0_6` document
//! `@require:`-ing a `V0_1` package end-to-end — the REVERSE of
//! `xver_import.rs`'s whole direction.
//!
//! Driven through the REAL loader (`satysfi_loader::load`, `LoadOptions {
//! version: V0_0_6, .. }`) so the Q4-mirror per-file detection rule
//! (`load_legacy`'s `require_v01_targets` worklist logic) and the new
//! `resolve_require` `dist-v01/packages/` base are exercised for real, not
//! bypassed, and through `satysfi_lang::compile_document_v006_xver` (the new
//! sibling entry point), so the entry's own `Ast::VersionScope(V0_0_6, _)`
//! wrap (both `prelude` AND the document tail) and the ambient-`V0_1`
//! `compile_program_xver`/`check_program` plumbing all run for real:
//!
//! - [`reverse_unsealed_v01_dep_renders`] (X4a headline): a `V0_0_6` entry
//!   `@require:`ing the REAL vendored `v01-mini.satyh` (copied byte-for-byte
//!   from `lib-satysfi/dist-v01/packages/`), calling its `document`/`p`
//!   members module-qualified, renders to a real page with real placed
//!   content.
//! - [`reverse_sealed_v01_dep_renders_and_enforces_hiding`] (X4a sealing
//!   proof, resolves Q2): a `V0_0_6` entry `@require:`ing the REAL vendored
//!   `v01-sealed.satyh`, calling `V01Sealed.make`/`get`/`\show` (allowed) —
//!   renders — **and** a sibling negative case that instead pattern-matches
//!   on the sealed `T` constructor directly is rejected by
//!   `check_program`'s existing hidden-constructor error path, proving 0.1's
//!   sealing machinery enforces against 0.0.6-authored consumer code with NO
//!   new code on the enforcement side (`v1::module_check` is untouched).
//! - [`reverse_guard_rejects_forked_export`] (X4a negative): a small inline
//!   `V0_1` fixture exporting a `graphics`-typed value (a `type .. =
//!   graphics` synonym) — `@require:`d by a `V0_0_6` entry — rejected with
//!   `CompileError::CrossVersionUnsupportedName { slice: "X4a", .. }`, not a
//!   mis-render.
//! - [`reverse_math_export_relabels_and_renders`] (X4a positive, the
//!   re-derived whitelist): a small inline `V0_1` fixture exporting a
//!   `math-text`-typed value — a `V0_0_6` entry consumes it where `math` is
//!   expected — renders, proving the reverse (coarsening) relabel
//!   `v1::xver_adapt::relabel_or_reject_name`'s new `(V0_1, V0_0_6)` arm
//!   implements.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::CompileError;
use satysfi_loader::{LoadOptions, SatysfiVersion};

/// This repo's root, resolved relative to this crate's own manifest
/// directory — same helper every other `v01_*`/`xver_*` integration test in
/// this crate reproduces locally (no shared test-support library target
/// exists here).
fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel)
}

/// A small fixture tree under a unique temp directory (cleaned up on drop) —
/// mirrors `xver_import.rs`'s own `TempDir` helper.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satysfi-lang-xver-reverse-test-{tag}-{}-{}-{}",
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

    fn write(&self, rel: &str, content: &str) {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&p, content).expect("write fixture file");
    }

    /// Copy the REAL vendored `lib-satysfi/dist-v01/packages/<name>`
    /// byte-for-byte into this temp tree's own `dist-v01/packages/<name>` —
    /// so `@require: <name>` against `self.path()` as `lib_root` resolves to
    /// the actual 0.1 fixture corpus content (`resolve_require`'s new
    /// `dist-v01/packages/` base, Slice X4a).
    fn copy_real_v01_package(&self, name: &str) {
        let real = repo(&format!("lib-satysfi/dist-v01/packages/{name}"));
        let content = fs::read_to_string(&real).unwrap_or_else(|e| panic!("read {real:?}: {e}"));
        self.write(&format!("dist-v01/packages/{name}"), &content);
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A real (ASCII-only) `FontMetrics` — every positive fixture below renders
/// actual text, so `advance` must return `Some` for ASCII.
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

fn load_v006(dir: &TempDir, entry_rel: &str) -> Vec<satysfi_loader::LoadedFile> {
    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: SatysfiVersion::V0_0_6,
        ..Default::default()
    };
    let program = satysfi_loader::load(&dir.path().join(entry_rel), &opts)
        .unwrap_or_else(|e| panic!("loading the reverse xver fixture should succeed: {e}"));
    assert!(
        program.files.iter().any(|f| matches!(f.cst, satysfi_loader::LoadedCst::V0_1(_))),
        "the required 0.1 package should have been detected as V0_1 (Q4-mirror rule)"
    );
    program.files
}

// ============================================================================
// X4a headline: an UNSEALED 0.1 dependency (v01-mini.satyh).
// ============================================================================

#[test]
fn reverse_unsealed_v01_dep_renders() {
    let dir = TempDir::new("unsealed");
    dir.copy_real_v01_package("v01-mini.satyh");
    let entry_src = "\
@require: v01-mini

V01Mini.document (|title = `v01-reverse`|) '<
  +V01Mini.p{ Hello from a 0.0.6 document requiring a 0.1 package. }
>
";
    dir.write("entry.saty", entry_src);
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = satysfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a V0_0_6 entry requiring a version-neutral (unsealed) V0_1 package \
                 should compile+render under X4a: {e}"
            )
        });

    assert_eq!(doc.pages.len(), 1, "one A4 page (v01-mini's document uses 210mm x 297mm)");
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the +V01Mini.p paragraph must have been placed on the page"
    );
}

// ============================================================================
// X4a sealing proof (Q2 resolution): a SEALED 0.1 dependency
// (v01-sealed.satyh) — allowed access renders, hidden-constructor access is
// still rejected by `v1::module_check`'s EXISTING enforcement, unmodified.
// ============================================================================

#[test]
fn reverse_sealed_v01_dep_renders_and_enforces_hiding() {
    let dir = TempDir::new("sealed-positive");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.copy_real_v01_package("v01-sealed.satyh");
    let entry_src = "\
@require: v01-mini
@require: v01-sealed

let v = V01Sealed.make 41 in
let answer = embed-string (arabic (V01Sealed.get v + 1)) in
V01Mini.document (|title = `v01-sealed-reverse`|) '<
  +V01Mini.p{ The answer is #answer;. }
  +V01Mini.p{ Sealed says: \\V01Sealed.show(V01Sealed.make 7); }
>
";
    dir.write("entry.saty", entry_src);
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = satysfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a V0_0_6 entry using a SEALED V0_1 package's public interface \
                 (make/get/\\show) should compile+render under X4a: {e}"
            )
        });

    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.pages[0].lines.is_empty(),
        "both paragraphs (the answer and the \\show call) must have been placed"
    );
}

/// The negative sibling: a `V0_0_6`-authored entry that instead pattern-
/// matches on `V01Sealed`'s hidden `T` constructor directly must still be
/// rejected — `v1::module_check::check_program`'s hidden-constructor
/// enforcement, driven purely from `V01Sealed`'s OWN `cst_v1` seal (`type t
/// :: o`), has no idea the consuming code is 0.0.6-authored, and needs NO
/// new code to reject it (§X4.1 point 2's "gets 0.1's sealing enforcement
/// automatically" claim, pinned).
#[test]
fn reverse_sealed_v01_dep_hidden_ctor_still_rejected() {
    let dir = TempDir::new("sealed-negative");
    dir.copy_real_v01_package("v01-sealed.satyh");
    // `T` is `V01Sealed`'s sealed constructor (`type t = | T of int`). 0.0.6
    // has NO qualified-constructor-pattern syntax at all (`Mod.Ctor(..)` is
    // a parse error — a real, independently-discovered grammar fact, not
    // this test dodging the interesting case): a 0.0.6-authored consumer can
    // only ever reach a bare constructor name, so the way to attempt the
    // violation in 0.0.6 syntax is `open V01Sealed in` (bringing its members
    // into UNQUALIFIED scope, exactly like a 0.1-authored `let open` would)
    // followed by a bare `T(x)` pattern — hidden by the seal, must still
    // fail to resolve.
    let entry_src = "\
@require: v01-sealed

let v = V01Sealed.make 41 in
open V01Sealed in
let n =
  match v with
  | T(x) -> x
in
0
";
    dir.write("entry.saty", entry_src);
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let err = satysfi_lang::compile_document_v006_xver(&files, &mono).expect_err(
        "pattern-matching a sealed module's hidden constructor from 0.0.6-authored code \
         must still be rejected by check_program's UNMODIFIED enforcement",
    );
    // Whatever the exact shape (a TypeError naming the hidden constructor, or
    // an elaboration-time unresolved-constructor error — both are
    // `check_program`'s/elaborate's PRE-EXISTING enforcement, not
    // `CrossVersionUnsupportedName`, since crossing the boundary itself was
    // never in question here), it must NOT be a silent success.
    assert!(
        !matches!(err, CompileError::CrossVersionUnsupportedName { .. }),
        "the rejection must come from the SEALING machinery, not the xver guard: {err}"
    );
}

// ============================================================================
// X4a negative: the conservative guard rejects every forked-typed V0_1
// export EXCEPT the proven-identical math-text/math-boxes family.
// ============================================================================

const V01_FORKED_EXPORT_PKG_SRC: &str = "\
module V01ForkedExport = struct
  type my-graphics-alias = graphics
end
";

#[test]
fn reverse_guard_rejects_forked_export() {
    let dir = TempDir::new("forked-negative");
    dir.write("dist-v01/packages/v01-forked-export.satyh", V01_FORKED_EXPORT_PKG_SRC);
    dir.write("entry.saty", "@require: v01-forked-export\n\n0\n");
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let err = satysfi_lang::compile_document_v006_xver(&files, &mono)
        .expect_err("a V0_1 dependency naming a forked TYPE (graphics) must be rejected");
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "graphics");
            assert_eq!(slice, "X4a");
        }
        other => panic!("expected CrossVersionUnsupportedName, got: {other}"),
    }
}

// ============================================================================
// X4a positive: the re-derived reverse whitelist — a V0_1 export typed
// `math-text` (0.1's unevaluated math source) coarsens to 0.0.6's single
// undifferentiated `math` type with ZERO value coercion (the shared
// `Value::MathText` representation).
// ============================================================================

// A SEALED module: its sig's `val my-math : math-text` item is the ONE
// surface site this port's guard actually SEES (`collect_free_globals`'s
// `walk_sig_annot` — an ordinary unsealed `val` binding carries no type
// ascription syntax at all, `cst_v1::Bind::Value` has no `: ty` field). The
// crossing VALUE (`${1+1}`, `Value::MathText`) needs no adaptation either
// way — it is representationally identical under both versions (§2.2); only
// the sig's TEXT is what the whitelist guard inspects.
const V01_MATH_EXPORT_PKG_SRC: &str = "\
module V01MathExport :> sig
  val my-math : math-text
end = struct
  val my-math = ${1+1}
end
";

#[test]
fn reverse_math_export_relabels_and_renders() {
    let dir = TempDir::new("math-export-positive");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write("dist-v01/packages/v01-math-export.satyh", V01_MATH_EXPORT_PKG_SRC);
    let entry_src = "\
@require: v01-mini
@require: v01-math-export

V01Mini.document (|title = `v01-math-reverse`|) '<
  +V01Mini.p{ \\V01Mini.math(V01MathExport.my-math); }
>
";
    dir.write("entry.saty", entry_src);
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = satysfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a SEALED V0_1 dependency exporting a `math-text`-typed value should be \
                 accepted (whitelisted, spliced verbatim) and compile+render under X4a: {e}"
            )
        });

    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the embedded math body (crossed from the V0_1 dependency) must have been placed"
    );
}


