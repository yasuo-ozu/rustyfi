//! Slice X4a ("Slice X4 — reverse direction", specifically the X4a
//! sub-slice): a `V0_0` document `@require:`-ing a `V0_1` package
//! end-to-end — the REVERSE of `xver_import.rs`'s whole direction.
//!
//! Driven through the REAL loader (`rustyfi_loader::load`, `LoadOptions {
//! version: V0_0, .. }`) so the Q4-mirror per-file detection rule
//! (`load_legacy`'s `require_v01_targets` worklist logic) and the new
//! `resolve_require` `dist-v01/packages/` base are exercised for real, not
//! bypassed, and through `rustyfi_lang::compile_document_v006_xver` (the new
//! sibling entry point), so the entry's own `Ast::VersionScope(V0_0, _)`
//! wrap (both `prelude` AND the document tail) and the ambient-`V0_1`
//! `compile_program_xver`/`check_program` plumbing all run for real:
//!
//! - [`reverse_unsealed_v01_dep_renders`] (X4a headline): a `V0_0` entry
//!   `@require:`ing the REAL vendored `v01-mini.satyh` (copied byte-for-byte
//!   from `lib-rustyfi/dist-v01/packages/`), calling its `document`/`p`
//!   members module-qualified, renders to a real page with real placed
//!   content.
//! - [`reverse_sealed_v01_dep_renders_and_enforces_hiding`] (X4a sealing
//!   proof, resolves Q2): a `V0_0` entry `@require:`ing the REAL vendored
//!   `v01-sealed.satyh`, calling `V01Sealed.make`/`get`/`\show` (allowed) —
//!   renders — **and** a sibling negative case that instead pattern-matches
//!   on the sealed `T` constructor directly is rejected by
//!   `check_program`'s existing hidden-constructor error path, proving 0.1's
//!   sealing machinery enforces against 0.0.6-authored consumer code with NO
//!   new code on the enforcement side (`v1::module_check` is untouched).
//! - [`reverse_guard_rejects_forked_export`] (X4a negative): a small inline
//!   `V0_1` fixture exporting a `graphics`-typed value (a `type .. =
//!   graphics` synonym) — `@require:`d by a `V0_0` entry — rejected with
//!   `CompileError::CrossVersionUnsupportedName { slice: "X4a", .. }`, not a
//!   mis-render.
//! - [`reverse_math_export_relabels_and_renders`] (X4a positive, the
//!   re-derived whitelist): a small inline `V0_1` fixture exporting a
//!   `math-text`-typed value — a `V0_0` entry consumes it where `math` is
//!   expected — renders, proving the reverse (coarsening) relabel
//!   `v1::xver_adapt::relabel_or_reject_name`'s new `(V0_1, V0_0)` arm
//!   implements.
//!
//! Slice X4b (`docs/plans/design-cross-version-import.md` §X4.5, extended
//! beyond its own math-only sketch) — the reverse `deco`: a `V0_1`
//! dependency's `deco`/`deco-set` VALUE export (via a module `sig`, the ONE
//! textual site 0.1's grammar can express such an ascription at all — an
//! ordinary unsealed `val` has no ascription syntax whatsoever) now CROSSES,
//! value-coerced by wrapping the single `graphics` a 0.1 deco returns in the
//! SINGLETON LIST a 0.0.6 consumer expects — the literal inverse of X3b's
//! `unite-graphics` wrap. The one obstacle (every 0.1 module signature
//! annotation is the `:>` form, and `v1::module_check`'s phase-D spine walk
//! conformance-checks EVERY such annotation name-keyed, so a coercion shadow
//! trips it a second time) is resolved by an EXPLICIT, caller-supplied
//! exemption — `check_program_with_xver_shadows`, which exempts only the
//! SECOND-and-later binding of a name `lib.rs` has itself just rebound,
//! leaving the exporting module's own conformance check fully in force. See
//! `v1::xver_adapt`'s own "X4b" doc comment for the full derivation.
//!
//! - [`reverse_deco_export_via_sig_coerces_and_renders`] (X4b headline): a
//!   module `V01DecoExport :> sig val my-deco : deco end = struct .. end`
//!   `@require:`d by a `V0_0` entry, whose 0.0.6-authored `let-inline` hands
//!   it to `inline-frame-outer` — renders, with the deco actually fired.
//! - [`reverse_deco_export_curried_sig_coerces_and_renders`] — the same for
//!   an arrow-tailed export (`length -> deco`).
//! - [`reverse_decoset_export_via_sig_coerces_and_renders`] — the 4-tuple
//!   sibling, through `inline-frame-breakable`.
//! - [`reverse_deco_export_optional_arg_sig_still_rejected`] and
//!   [`reverse_deco_export_nested_module_sig_still_rejected`] — the two
//!   DELIBERATE rejections that survive: an optional-argument arrow (no
//!   positional spelling for the generated wrapper to forward) and a `deco`
//!   behind a nested `module` sig decl (whose seal key composition this
//!   direction stays conservative about).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::CompileError;
use rustyfi_loader::{LoadOptions, RustyfiVersion};

/// This repo's root, resolved relative to this crate's own manifest
/// directory — same helper every other `v01_*`/`xver_*` integration test in
/// this crate reproduces locally (no shared test-support library target
/// exists here).
fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// A small fixture tree under a unique temp directory (cleaned up on drop) —
/// mirrors `xver_import.rs`'s own `TempDir` helper.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustyfi-lang-xver-reverse-test-{tag}-{}-{}-{}",
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

    /// Copy the REAL vendored `lib-rustyfi/dist-v01/packages/<name>`
    /// byte-for-byte into this temp tree's own `dist-v01/packages/<name>` —
    /// so `@require: <name>` against `self.path()` as `lib_root` resolves to
    /// the actual 0.1 fixture corpus content (`resolve_require`'s new
    /// `dist-v01/packages/` base, Slice X4a).
    fn copy_real_v01_package(&self, name: &str) {
        let real = repo(&format!("lib-rustyfi/dist-v01/packages/{name}"));
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

fn load_v006(dir: &TempDir, entry_rel: &str) -> Vec<rustyfi_loader::LoadedFile> {
    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: RustyfiVersion::V0_0,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&dir.path().join(entry_rel), &opts)
        .unwrap_or_else(|e| panic!("loading the reverse xver fixture should succeed: {e}"));
    assert!(
        program
            .files
            .iter()
            .any(|f| matches!(f.cst, rustyfi_loader::LoadedCst::V0_1(_))),
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
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a V0_0 entry requiring a version-neutral (unsealed) V0_1 package \
                 should compile+render under X4a: {e}"
            )
        });

    assert_eq!(
        doc.pages.len(),
        1,
        "one A4 page (v01-mini's document uses 210mm x 297mm)"
    );
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
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a V0_0 entry using a SEALED V0_1 package's public interface \
                 (make/get/\\show) should compile+render under X4a: {e}"
            )
        });

    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.pages[0].lines.is_empty(),
        "both paragraphs (the answer and the \\show call) must have been placed"
    );
}

/// The negative sibling: a `V0_0`-authored entry that instead pattern-
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
    let err = rustyfi_lang::compile_document_v006_xver(&files, &mono).expect_err(
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

// `font`, not `graphics`: `graphics` turned out not to be forked at all
// (upstream registers the same `GraphicsType` base type in both generations —
// see `typecheck::name_to_mono`), so it now crosses in BOTH directions and can
// no longer stand in for a rejected export here. `font` still forks: 0.0.6's is
// an opaque nominal, 0.1's a `string` stand-in.
const V01_FORKED_EXPORT_PKG_SRC: &str = "\
module V01ForkedExport = struct
  type my-font-alias = font
end
";

#[test]
fn reverse_guard_rejects_forked_export() {
    let dir = TempDir::new("forked-negative");
    dir.write(
        "dist-v01/packages/v01-forked-export.satyh",
        V01_FORKED_EXPORT_PKG_SRC,
    );
    dir.write("entry.saty", "@require: v01-forked-export\n\n0\n");
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let err = rustyfi_lang::compile_document_v006_xver(&files, &mono)
        .expect_err("a V0_1 dependency naming a forked TYPE (font) must be rejected");
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "font");
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
    dir.write(
        "dist-v01/packages/v01-math-export.satyh",
        V01_MATH_EXPORT_PKG_SRC,
    );
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
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
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

// ============================================================================
// X4b: a `V0_1` dependency's `deco`/`deco-set` export (module-SIG-typed —
// the only textual site 0.1's grammar can express such an ascription at
// all) CROSSES into a 0.0.6 document, value-coerced by wrapping the single
// `graphics` a 0.1 deco returns in the SINGLETON LIST a 0.0.6 consumer
// expects — the exact inverse of X3b's `unite-graphics` wrap. See
// `v1::xver_adapt`'s own "X4b" section for the derivation, including why the
// coercion shadow needs an explicit `check_program_with_xver_shadows`
// exemption from a SECOND `:>` seal check.
// ============================================================================

/// A module typed via its own `sig`: its `val my-deco : deco` item is the
/// ONE surface site 0.1's grammar can express a bare `deco` ascription at
/// all (an ordinary unsealed `val` binding carries no ascription syntax at
/// all — `cst_v1::Bind::Value`'s own doc comment). Its body returns a SINGLE
/// `graphics` (0.1 semantics), which is exactly what must be downgraded.
const V01_DECO_EXPORT_PKG_SRC: &str = "\
module V01DecoExport :> sig
  val my-deco : deco
end = struct
  val my-deco (x, y) w h d =
    fill (Gray(0.0))
      (close-with-line
         (line-to (x +' w, y +' h)
            (line-to (x +' w, y)
               (start-path (x, y)))))
end
";

/// The 0.0.6-authored consumer: a `let-inline` command that hands the
/// crossed deco to `inline-frame-outer` — a REAL consumer
/// (`primitives::apply_deco`, fired by `lib.rs`'s `fire_inline_frame` at
/// render time under `interp.version == V0_0`, not a stand-in). Two things
/// would break without the X4b coercion, and both are compile/eval failures
/// rather than silent mis-renders: `inline-frame-outer` inside the entry's
/// `Ast::VersionScope(V0_0, _)` carries `t_deco(V0_0)` (a `graphics list`
/// result), so an unwrapped 0.1 deco fails to unify; and even past that,
/// `apply_deco`'s `coerce_graphics_result` would `as_list` a bare
/// `Value::Graphics`.
fn deco_consumer_entry(pkg: &str, deco_expr: &str) -> String {
    format!(
        "\
@require: v01-mini
@require: {pkg}

let-inline ctx \\framed it =
  inline-frame-outer (2pt, 2pt, 2pt, 2pt) ({deco_expr}) (read-inline ctx it)
in
V01Mini.document (|title = `v01-deco-reverse`|) '<
  +V01Mini.p{{ \\framed{{Hello, reverse cross-version deco}} }}
>
"
    )
}

#[test]
fn reverse_deco_export_via_sig_coerces_and_renders() {
    let dir = TempDir::new("deco-export-coerces");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write(
        "dist-v01/packages/v01-deco-export.satyh",
        V01_DECO_EXPORT_PKG_SRC,
    );
    dir.write(
        "entry.saty",
        &deco_consumer_entry("v01-deco-export", "V01DecoExport.my-deco"),
    );
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a V0_1 dependency exporting a module-sig-typed `: deco` value should be \
                 value-coerced (single graphics -> graphics list) and compile+render \
                 under X4b: {e}"
            )
        });

    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the framed inline content must have been placed"
    );
    assert!(
        !doc.extras.page_graphics[0].is_empty(),
        "the crossed deco must have FIRED — reaching this assertion at all already \
         proves the downgrade ran, since `coerce_graphics_result` under V0_0 would \
         have failed on an unwrapped single `graphics`"
    );
}

/// The shape a real 0.1 package would use: an ARROW-TAILED `deco` export
/// (`length -> deco`), applied by the consumer before it reaches
/// `inline-frame-outer`. The generated wrapper eta-expands over the leading
/// argument first, then over `deco`'s own four.
#[test]
fn reverse_deco_export_curried_sig_coerces_and_renders() {
    let dir = TempDir::new("deco-export-curried-coerces");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write(
        "dist-v01/packages/v01-deco-curried.satyh",
        "\
module V01DecoCurried :> sig
  val my-deco : length -> deco
end = struct
  val my-deco t (x, y) w h d =
    fill (Gray(0.0)) (close-with-line (line-to (x +' w, y +' h) (start-path (x, y))))
end
",
    );
    dir.write(
        "entry.saty",
        &deco_consumer_entry("v01-deco-curried", "V01DecoCurried.my-deco 1pt"),
    );
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!("an arrow-tailed V0_1 `deco` export should cross under X4b: {e}")
        });
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.extras.page_graphics[0].is_empty(),
        "the crossed, curried deco must have fired"
    );
}

/// The 4-tuple sibling: a `deco-set` export, consumed through
/// `inline-frame-breakable` (whose FIRST component is the only one a
/// non-broken single-line frame fires). Each component is downgraded
/// independently.
#[test]
fn reverse_decoset_export_via_sig_coerces_and_renders() {
    let dir = TempDir::new("decoset-export-coerces");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write(
        "dist-v01/packages/v01-decoset-export.satyh",
        "\
module V01DecoSetExport :> sig
  val my-decoset : deco-set
end = struct
  val one (x, y) w h d =
    fill (Gray(0.0))
      (close-with-line
         (line-to (x +' w, y +' h)
            (line-to (x +' w, y)
               (start-path (x, y)))))
  val my-decoset = (one, one, one, one)
end
",
    );
    dir.write(
        "entry.saty",
        "\
@require: v01-mini
@require: v01-decoset-export

let-inline ctx \\framed it =
  inline-frame-breakable (2pt, 2pt, 2pt, 2pt) V01DecoSetExport.my-decoset
    (read-inline ctx it)
in
V01Mini.document (|title = `v01-decoset-reverse`|) '<
  +V01Mini.p{ \\framed{Hello, reverse cross-version deco-set} }
>
",
    );
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| panic!("a V0_1 `deco-set` export should cross under X4b: {e}"));
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.extras.page_graphics[0].is_empty(),
        "the crossed deco-set's fired component must have produced graphics"
    );
}

/// X4b's DELIBERATE rejection, preserved: an OPTIONAL-argument arrow in the
/// export's type. The generated wrapper forwards its parameters
/// POSITIONALLY, and an optional argument has no positional spelling — so
/// eta-expanding one would silently drop it. Rejected loudly instead.
#[test]
fn reverse_deco_export_optional_arg_sig_still_rejected() {
    let dir = TempDir::new("deco-export-optional-negative");
    dir.write(
        "dist-v01/packages/v01-deco-opt.satyh",
        "\
module V01DecoOpt :> sig
  val my-deco : ?(thickness : length) length -> deco
end = struct
  val my-deco t (x, y) w h d =
    fill (Gray(0.0)) (close-with-line (line-to (x +' w, y +' h) (start-path (x, y))))
end
",
    );
    dir.write("entry.saty", "@require: v01-deco-opt\n\n0\n");
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let err = rustyfi_lang::compile_document_v006_xver(&files, &mono).expect_err(
        "an optional-argument arrow in a `deco` export must reject — the generated \
         wrapper forwards positionally and an optional argument has no positional spelling",
    );
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "deco");
            assert_eq!(slice, "X4b");
        }
        other => panic!("expected CrossVersionUnsupportedName, got: {other}"),
    }
}

/// X4b's other deliberate narrowing: a `deco` reached through a NESTED
/// `module` decl in the signature. The shadow has to name the member under
/// exactly the qualified key `v1::module_check` seals it by, and a nested
/// member's seal goes through `walk_nested_seals_a`'s own path composition —
/// so the reverse direction classifies only the TOP-LEVEL sig's own `val`
/// items and rejects the rest.
#[test]
fn reverse_deco_export_nested_module_sig_still_rejected() {
    let dir = TempDir::new("deco-export-nested-negative");
    dir.write(
        "dist-v01/packages/v01-deco-nested.satyh",
        "\
module V01DecoNested :> sig
  module Inner : sig val my-deco : deco end
end = struct
  module Inner :> sig val my-deco : deco end = struct
    val my-deco (x, y) w h d =
      fill (Gray(0.0)) (close-with-line (line-to (x +' w, y +' h) (start-path (x, y))))
  end
end
",
    );
    dir.write("entry.saty", "@require: v01-deco-nested\n\n0\n");
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let err = rustyfi_lang::compile_document_v006_xver(&files, &mono)
        .expect_err("a NESTED module's sig `deco` export must still be rejected");
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "deco");
            assert_eq!(slice, "X4b");
        }
        other => panic!("expected CrossVersionUnsupportedName, got: {other}"),
    }
}
