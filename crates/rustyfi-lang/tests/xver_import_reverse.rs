//! A `V0_0` document `@require:`-ing a `V0_1` package end-to-end —
//! the REVERSE of `xver_import.rs`. Driven through the REAL loader
//! (`LoadOptions { version: V0_0, .. }`), so `load_legacy`'s
//! `require_v01_targets` detection and `resolve_require`'s `dist-v01/packages/`
//! base are exercised rather than bypassed.
//!
//! The reverse `deco`: a `V0_1` dependency's `deco`/`deco-set` sig
//! export crosses, value-coerced by wrapping the single `graphics` a 0.1 deco
//! returns in the SINGLETON LIST a 0.0.6 consumer expects (the inverse of the
//! forward deco coercion's `unite-graphics`). Since `v1::module_check`
//! conformance-checks EVERY `:>` annotation name-keyed, the coercion shadow
//! needs an explicit `check_program_with_xver_shadows` exemption; derivation
//! in `v1::xver_adapt`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::CompileError;
use rustyfi_loader::{LoadOptions, RustyfiVersion};

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

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

    /// Copy the REAL vendored `lib-rustyfi/dist-v01/packages/<name>` in, so
    /// `@require:` hits `resolve_require`'s `dist-v01/packages/` base.
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

// An UNSEALED 0.1 dependency (v01-mini.satyh).

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

// Reverse-import sealing: a SEALED 0.1 dependency — allowed access
// renders, hidden-constructor access is still rejected by `v1::module_check`'s
// EXISTING enforcement, unmodified.

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

/// `check_program`'s hidden-constructor enforcement is driven purely from
/// `V01Sealed`'s OWN `cst_v1` seal (`type t :: o`), so it needs NO new code to
/// reject a 0.0.6-authored consumer.
#[test]
fn reverse_sealed_v01_dep_hidden_ctor_still_rejected() {
    let dir = TempDir::new("sealed-negative");
    dir.copy_real_v01_package("v01-sealed.satyh");
    // 0.0.6 has NO qualified-constructor-pattern syntax (`Mod.Ctor(..)` is a
    // parse error), so the only way to attempt the violation in 0.0.6 syntax is
    // `open V01Sealed in` followed by a bare `T(x)` pattern.
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
    // Either error shape is fine as long as it is NOT
    // `CrossVersionUnsupportedName`: crossing was never in question here.
    assert!(
        !matches!(err, CompileError::CrossVersionUnsupportedName { .. }),
        "the rejection must come from the SEALING machinery, not the xver guard: {err}"
    );
}

// The guard rejects every forked-typed V0_1 export EXCEPT the
// proven-identical math-text/math-boxes family.
//
// `font`, not `graphics`: `graphics` is NOT forked (the same `GraphicsType`
// base type in both generations — `typecheck::name_to_mono`), so it crosses in
// BOTH directions and cannot stand in for a rejected export. `font` does fork:
// 0.1's is saphe-split's opaque `BaseType(FontType)` handle, and 0.0.6
// registers no `font` type at all.
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

// A V0_1 export typed `math-text` coarsens to 0.0.6's single
// `math` with ZERO value coercion (the shared `Value::MathText` rep).
//
// SEALED because a sig's `val .. : ty` item is the ONE surface site this port's
// guard SEES (`collect_free_globals`'s `walk_sig_annot`) — `cst_v1::Bind::Value`
// has no `: ty` field, so an unsealed `val` carries no ascription to inspect.
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

// A `V0_1` `deco`/`deco-set` export crossing into a 0.0.6 document.

/// The body returns a SINGLE `graphics` (0.1 semantics) — what gets downgraded.
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

/// A REAL consumer (`primitives::apply_deco`, fired by `fire_inline_frame` under
/// `interp.version == V0_0`). Without the reverse deco coercion both failure modes are
/// loud, not mis-renders: `inline-frame-outer` inside `VersionScope(V0_0, _)`
/// carries `t_deco(V0_0)` so an unwrapped 0.1 deco fails to unify, and past that
/// `coerce_graphics_result` would `as_list` a bare `Value::Graphics`.
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

/// The generated wrapper eta-expands over the leading argument first, then over
/// `deco`'s own four.
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

/// Each component is downgraded independently. `inline-frame-breakable` fires
/// only the FIRST for a non-broken single-line frame.
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

/// The shadow cannot FORWARD the label, because 0.1's two halves disagree about
/// who owns the `option`: `Ast::LambdaOpt` binds the receiving binder at
/// `length option` while `Ast::ApplyOpt`'s `?(thickness = e)` takes the raw
/// `length` and wraps it in `Some` itself. So it CASE-SPLITS instead — `Some(v)`
/// re-supplies as `?(thickness = v)`, `None` omits the label and lets
/// `push_opt_slots` restore it. Both arms are exercised below, at different
/// insets, so "both fired" also asserts the label reached the 0.1 closure.
#[test]
fn reverse_deco_export_optional_arg_sig_coerces_and_renders() {
    let dir = TempDir::new("deco-export-optional-coerces");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write(
        "dist-v01/packages/v01-deco-opt.satyh",
        "\
module V01DecoOpt :> sig
  val my-deco : ?(thickness : length) length -> deco
end = struct
  val my-deco ?(thickness = topt) t (x, y) w h d =
    let i =
      match topt with
      | None    -> 0pt
      | Some(v) -> v
      end
    in
    fill (Gray(0.0))
      (close-with-line
         (line-to (x +' w, y +' h -' i)
            (line-to (x +' w, y)
               (start-path (x +' i, y)))))
end
",
    );
    dir.write(
        "entry.saty",
        "\
@require: v01-mini
@require: v01-deco-opt

let-inline ctx \\framed it =
  (inline-frame-outer (2pt, 2pt, 2pt, 2pt) (V01DecoOpt.my-deco 1pt)
     (read-inline ctx it))
    ++ (inline-frame-outer (2pt, 2pt, 2pt, 2pt)
          (V01DecoOpt.my-deco ?(thickness = 4pt) 1pt)
          (read-inline ctx it))
in
V01Mini.document (|title = `v01-deco-opt-reverse`|) '<
  +V01Mini.p{ \\framed{Hi} }
>
",
    );
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a LABELLED-optional-argument arrow in a V0_1 `deco` export should cross \
                 under X4b, by case-splitting on the label rather than forwarding it: {e}"
            )
        });
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert_eq!(
        doc.extras.page_graphics[0].len(),
        2,
        "both the label-omitting and the label-supplying call must have fired the \
         coerced deco, got: {:?}",
        doc.extras.page_graphics[0]
    );
}

/// A ROW-VARIABLE tail (`?(l : τ | ?'r)`) leaves the label set OPEN, so there
/// is no finite case split to generate and the export still rejects.
#[test]
fn reverse_deco_export_open_optional_row_still_rejected() {
    let dir = TempDir::new("deco-export-optrow-negative");
    dir.write(
        "dist-v01/packages/v01-deco-optrow.satyh",
        "\
module V01DecoOptRow :> sig
  val my-deco : ?(thickness : length | ?'r) length -> deco
end = struct
  val my-deco ?(thickness = topt) t (x, y) w h d =
    fill (Gray(0.0)) (close-with-line (line-to (x +' w, y +' h) (start-path (x, y))))
end
",
    );
    dir.write("entry.saty", "@require: v01-deco-optrow\n\n0\n");
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let err = rustyfi_lang::compile_document_v006_xver(&files, &mono).expect_err(
        "an OPEN optional row in a `deco` export must reject — the shadow's case split \
         has to enumerate the labels, and a row variable admits any number of them",
    );
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "deco");
            assert_eq!(slice, "X4b");
        }
        other => panic!("expected CrossVersionUnsupportedName, got: {other}"),
    }
}

/// The shadow must name the member under exactly the qualified key
/// `v1::module_check` seals it by: `walk_nested_seals_a` pushes each nested
/// module's name onto its parent's path (the same composition
/// `elaborate::qualify_key` uses), so `classify_deco_exports_v01_sig` recurses
/// into `Decl::Module` under a lengthened `module_path`.
#[test]
fn reverse_deco_export_nested_module_sig_coerces_and_renders() {
    let dir = TempDir::new("deco-export-nested-coerces");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write(
        "dist-v01/packages/v01-deco-nested.satyh",
        "\
module V01DecoNested :> sig
  module Inner : sig val my-deco : deco end
end = struct
  module Inner :> sig val my-deco : deco end = struct
    val my-deco (x, y) w h d =
      fill (Gray(0.0))
        (close-with-line
           (line-to (x +' w, y +' h)
              (line-to (x +' w, y)
                 (start-path (x, y)))))
  end
end
",
    );
    dir.write(
        "entry.saty",
        &deco_consumer_entry("v01-deco-nested", "V01DecoNested.Inner.my-deco"),
    );
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!("a NESTED module's sig `deco` export should cross under X4b: {e}")
        });
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.extras.page_graphics[0].is_empty(),
        "the crossed nested-module deco must have fired"
    );
}

/// Resolved through the very table `v1::module_check::resolve_sig` consults —
/// `surface::find_sig_keyed`, searched OUTWARD from the same `site_path` — so the
/// member still lands under the composed key `V01DecoNamedSig.Inner.my-deco`,
/// which is the `env.seals` key, the `Ast::LetIn` binder name, and the string
/// the consumer writes. The `signature S = ..` decl binds no value, so it
/// neither crosses nor rejects; it registers the definition `module Inner : S`
/// dereferences, which is why `lib.rs`'s reverse arm calls
/// `surface::build_file_surface` BEFORE the classifier.
#[test]
fn reverse_deco_export_nested_named_signature_coerces_and_renders() {
    let dir = TempDir::new("deco-export-named-sig-coerces");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write(
        "dist-v01/packages/v01-deco-named-sig.satyh",
        "\
module V01DecoNamedSig :> sig
  signature S = sig val my-deco : deco end
  module Inner : S
end = struct
  signature S = sig val my-deco : deco end
  module Inner :> S = struct
    val my-deco (x, y) w h d =
      fill (Gray(0.0))
        (close-with-line
           (line-to (x +' w, y +' h)
              (line-to (x +' w, y)
                 (start-path (x, y)))))
  end
end
",
    );
    dir.write(
        "entry.saty",
        &deco_consumer_entry("v01-deco-named-sig", "V01DecoNamedSig.Inner.my-deco"),
    );
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a `deco` behind a nested module decl typed by a NAMED signature should cross \
                 under X4b: {e}"
            )
        });
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.extras.page_graphics[0].is_empty(),
        "the crossed named-signature deco must have fired"
    );
}

/// `include S` splices S's decls into the ENCLOSING signature in place
/// (`module_check::splice_decls`), so the key is `V01DecoInclude.my-deco`, NOT
/// `V01DecoInclude.S.my-deco`.
#[test]
fn reverse_deco_export_through_sig_include_coerces_and_renders() {
    let dir = TempDir::new("deco-export-include-coerces");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write(
        "dist-v01/packages/v01-deco-include.satyh",
        "\
module V01DecoInclude :> sig
  signature S = sig val my-deco : deco end
  include S
end = struct
  signature S = sig val my-deco : deco end
  val my-deco (x, y) w h d =
    fill (Gray(0.0))
      (close-with-line
         (line-to (x +' w, y +' h)
            (line-to (x +' w, y)
               (start-path (x, y)))))
end
",
    );
    dir.write(
        "entry.saty",
        &deco_consumer_entry("v01-deco-include", "V01DecoInclude.my-deco"),
    );
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| panic!("an `include`d `deco` export should cross under X4b: {e}"));
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.extras.page_graphics[0].is_empty(),
        "the crossed included deco must have fired"
    );
}

/// The scan reads a `val`'s SPELLED type, so without expanding the signature's
/// transparent type declarations this tail reads as the bare name `frame`,
/// matches no forked builtin, and the export silently declines to cross —
/// surfacing later as an ordinary `TypeError` rather than a boundary
/// diagnostic. `V01Syns` expands it in place.
#[test]
fn reverse_deco_export_via_type_synonym_coerces_and_renders() {
    let dir = TempDir::new("deco-export-synonym-coerces");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write(
        "dist-v01/packages/v01-deco-synonym.satyh",
        "\
module V01DecoSynonym :> sig
  type frame = deco
  val my-deco : length -> frame
end = struct
  type frame = deco
  val my-deco t (x, y) w h d =
    fill (Gray(0.0)) (close-with-line (line-to (x +' w, y +' h) (start-path (x, y))))
end
",
    );
    dir.write(
        "entry.saty",
        &deco_consumer_entry("v01-deco-synonym", "V01DecoSynonym.my-deco 1pt"),
    );
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!("a `deco` declared at a type SYNONYM should cross under X4b: {e}")
        });
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.extras.page_graphics[0].is_empty(),
        "the crossed synonym-declared deco must have fired"
    );
}

/// The refinement descends into the named member (one chain segment per layer)
/// in BOTH `v1::module_check::resolve_sig` and this scan, so `Inner.t` really is
/// transparent-`deco` here and the export crosses.
#[test]
fn reverse_deco_export_via_with_submodule_type_refinement_coerces_and_renders() {
    let dir = TempDir::new("deco-export-with-submodule-type-coerces");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write(
        "dist-v01/packages/v01-deco-refined.satyh",
        "\
module V01DecoRefined :> sig
  module Inner : sig
    type t :: o
    val my-deco : t
  end
end with Inner type t = deco = struct
  module Inner = struct
    type t = deco
    val my-deco (x, y) w h d =
      fill (Gray(0.0)) (close-with-line (line-to (x +' w, y +' h) (start-path (x, y))))
  end
end
",
    );
    dir.write(
        "entry.saty",
        &deco_consumer_entry("v01-deco-refined", "V01DecoRefined.Inner.my-deco"),
    );
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a `deco` a `with M type` refinement makes transparent should cross under \
                 X4b: {e}"
            )
        });
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.extras.page_graphics[0].is_empty(),
        "the crossed, refinement-declared deco must have fired"
    );
}

/// A functor is not a module — its members exist only at an APPLICATION's own
/// path, computed by `v1::functor` in whatever file writes `module Inst =
/// V01DecoFunctor.Make Arg`, a file this scan cannot see. So the path a shadow
/// would name is not a function of THIS file's signature at all.
#[test]
fn reverse_deco_export_behind_functor_sig_member_still_rejected() {
    let dir = TempDir::new("deco-export-functor-negative");
    dir.write(
        "dist-v01/packages/v01-deco-functor.satyh",
        "\
module V01DecoFunctor :> sig
  module Make : (X : sig val n : int end) -> sig val my-deco : deco end
end = struct
  module Make = fun (X : sig val n : int end) -> struct
    val my-deco (x, y) w h d =
      fill (Gray(0.0)) (close-with-line (line-to (x +' w, y +' h) (start-path (x, y))))
  end
end
",
    );
    dir.write("entry.saty", "@require: v01-deco-functor\n\n0\n");
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let err = rustyfi_lang::compile_document_v006_xver(&files, &mono)
        .expect_err("a `deco` behind a functor signature member must still be rejected");
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "deco");
            assert_eq!(slice, "X4b");
        }
        other => panic!("expected CrossVersionUnsupportedName, got: {other}"),
    }
}

// Which consumers see the coerced view.

/// `V01DecoRelay` wants `V01DecoExport.my-deco` at 0.1's own `deco`, but its
/// `@require:` puts it AFTER the exporting dependency in the merged prelude, so
/// an unconditionally spliced shadow would reach it and fail its `:>`
/// conformance check. Shadows are installed LAZILY instead — at each transition
/// INTO a 0.0.6-authored block, originals restored on the way back.
#[test]
fn reverse_deco_export_consumed_by_a_later_v01_dep_keeps_the_v01_view() {
    let dir = TempDir::new("deco-export-later-v01-consumer");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write(
        "dist-v01/packages/v01-deco-export.satyh",
        V01_DECO_EXPORT_PKG_SRC,
    );
    dir.write(
        "dist-v01/packages/v01-deco-relay.satyh",
        "\
@require: v01-deco-export

module V01DecoRelay :> sig
  val re-deco : deco
end = struct
  val re-deco = V01DecoExport.my-deco
end
",
    );
    dir.write(
        "entry.saty",
        &deco_consumer_entry("v01-deco-relay", "V01DecoRelay.re-deco"),
    );
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "a LATER 0.1 dependency consuming the same `deco` export must still see 0.1's \
                 own single-`graphics` shape, not the 0.0.6 coercion shadow: {e}"
            )
        });
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.extras.page_graphics[0].is_empty(),
        "the relayed deco must have fired, coerced once for the 0.0.6 entry"
    );
}

/// The INTERLEAVED case — the one that needs both halves of the schedule. Load
/// order here is
///
/// ```text
///   v01-mini (0.1) | v01-deco-export (0.1) | v006-deco-user (0.0.6)
///                  | v01-deco-relay (0.1)  | entry (0.0.6)
/// ```
///
/// so the SAME export is read at 0.0.6's `graphics list` shape, then at 0.1's
/// single-`graphics` shape, then at 0.0.6's again: one `Install`, one `Restore`
/// and a second `Install` — the only fixture here where the lazy transitions do
/// not collapse to a single install. Both consumers actually FIRE their deco, so
/// the two page-graphics entries assert each got a value its own generation's
/// `apply_deco` could use, not merely one that typechecked.
#[test]
fn reverse_deco_export_interleaved_v006_and_v01_consumers_each_get_their_own_view() {
    let dir = TempDir::new("deco-export-interleaved");
    dir.copy_real_v01_package("v01-mini.satyh");
    dir.write(
        "dist-v01/packages/v01-deco-export.satyh",
        V01_DECO_EXPORT_PKG_SRC,
    );
    // A NATIVE 0.0.6 co-dependency: spliced inside `Ast::VersionScope(V0_0, _)`,
    // so its `inline-frame-outer` carries `t_deco(V0_0)` and only the coerced
    // (list-shaped) view unifies.
    dir.write(
        "dist/packages/v006-deco-user.satyh",
        "\
@require: v01-deco-export

let-inline ctx \\v006-framed it =
  inline-frame-outer (2pt, 2pt, 2pt, 2pt) V01DecoExport.my-deco (read-inline ctx it)
",
    );
    dir.write(
        "dist-v01/packages/v01-deco-relay.satyh",
        "\
@require: v01-deco-export

module V01DecoRelay :> sig
  val re-deco : deco
end = struct
  val re-deco = V01DecoExport.my-deco
end
",
    );
    dir.write(
        "entry.saty",
        "\
@require: v01-mini
@require: v006-deco-user
@require: v01-deco-relay

let-inline ctx \\framed it =
  inline-frame-outer (2pt, 2pt, 2pt, 2pt) V01DecoRelay.re-deco (read-inline ctx it)
in
V01Mini.document (|title = `v01-deco-interleaved`|) '<
  +V01Mini.p{ \\v006-framed{A} \\framed{B} }
>
",
    );
    let files = load_v006(&dir, "entry.saty");

    let mono = Mono;
    let (doc, _trials) = rustyfi_lang::compile_document_v006_xver_with_trials(&files, &mono)
        .unwrap_or_else(|e| {
            panic!(
                "interleaved 0.0.6 and 0.1 consumers of the SAME crossed `deco` export must \
                 each read it at their own generation's shape: {e}"
            )
        });
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert_eq!(
        doc.extras.page_graphics[0].len(),
        2,
        "both the 0.0.6 dependency's frame and the 0.1 relay's must have fired, got: {:?}",
        doc.extras.page_graphics[0]
    );
}
