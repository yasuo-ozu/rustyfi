//! Vendoring the real upstream `map`/`set` stdlib functor
//! packages, transliterated from `saphe-split@b836d512:lib-rustyfi/
//! packages/stdlib/stdlib.0.0.1/src/{map,set}.satyg`.
//!
//! Does NOT reuse `v01_stdlib.rs`'s `compile_v01_via_loader` helper: that
//! gives each file its own throwaway single-file `SurfaceEnv`, but
//! map.satyg/set.satyg need CROSS-FILE named-signature/functor lookups
//! (`Basic.Ord`, `Map.Make`) that only resolve through a `SurfaceEnv`
//! threaded across the whole dependency load order — so every probe here
//! goes through the real public `compile_document_v1` pipeline instead.
//!
//! Value-bar probes read the ACTUAL evaluated strings back out of the
//! rendered document (`PureHorzBox::InnerString`, as `tests/eval.rs`
//! does), proving the functor operations evaluate, not merely type-check.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_backend::{FontKey, FontMetrics, Length, PureHorzBox};
use rustyfi_lang::value::DocumentValue;
use rustyfi_lang::CompileError;
use rustyfi_loader::{LoadOptions, LoadedCst, LoadedFile};
use rustyfi_syntax::{parse_file_v1, RustyfiVersion};

fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi/dist-v01/packages")
}

struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustyfi-lang-v01-map-set-{tag}-{}-{}.saty",
            std::process::id(),
            n
        ));
        fs::write(&path, src).expect("write temp fixture");
        TempDoc(path)
    }
}

impl Drop for TempDoc {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// A real (ASCII-only) `FontMetrics` — needed since the value-bar probes
/// below render real text through `V01Mini.document`'s `read-inline` pass.
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

/// Loads `requires`' packages through the real loader and drops the
/// throwaway probe entry, returning the dependency `LoadedFile`s (in
/// dependency-first order) ready to extend with hand-parsed files.
fn load_real_deps(tag: &str, requires: &str) -> Vec<LoadedFile> {
    let src = format!("{requires}\n0");
    let doc = TempDoc::new(tag, &src);
    let opts = LoadOptions {
        lib_root: Some(lib_root()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let mut program = rustyfi_loader::load(&doc.0, &opts)
        .unwrap_or_else(|e| panic!("[{tag}] real packages should load: {e}"));
    program
        .files
        .pop()
        .expect("loader always yields at least the throwaway entry");
    program.files
}

fn push_hand_parsed(tag: &str, files: &mut Vec<LoadedFile>, name: &str, src: &str) {
    let cst = parse_file_v1(src).unwrap_or_else(|e| panic!("[{tag}] {name} parse failed: {e}"));
    files.push(LoadedFile {
        path: PathBuf::from(name),
        cst: LoadedCst::V0_1(cst),
        origin: Default::default(),
        version: RustyfiVersion::V0_1,
    });
}

fn compile_with_harness(
    tag: &str,
    real_requires: &str,
    harness_src: &str,
    entry_src: &str,
) -> Result<Rc<DocumentValue>, CompileError> {
    let mut files = load_real_deps(tag, real_requires);
    push_hand_parsed(tag, &mut files, "harness.satyh", harness_src);
    push_hand_parsed(tag, &mut files, "entry.saty", entry_src);
    rustyfi_lang::compile_document_v1(&files, &Mono)
}

fn first_line_words(doc: &DocumentValue) -> Vec<String> {
    doc.pages[0].lines[0]
        .contents
        .iter()
        .filter_map(|(_, b)| match b {
            PureHorzBox::InnerString { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

// `map.satyg` — the real upstream AVL-tree `Map.Make` functor.

/// The qualified-export negative probe, adapted to a MODULE-level member —
/// `Map`'s only export is the nested functor `Make`, so "bare access" here
/// is `module M = Make Arg` instead of a bare `val`
/// reference: after only `@require: map`, referencing `Make` WITHOUT its
/// `Map.` qualifier is a precise, functor-framed `LowerError` ("a functor
/// application whose functor or argument is unknown",
/// `v1/lower.rs:725-730`) — never resolves to the real functor.
#[test]
fn map_bare_make_is_unbound_without_qualification() {
    let harness = "\
module MapBareHarness = struct
  module IntOrd = struct
    type t = int
    val compare x y =
      if x < y then Less else if x == y then Equal else Greater
  end
  module M = Make IntOrd
end
";
    let err = compile_with_harness("map-bare", "@require: map", harness, "1")
        .err()
        .unwrap_or_else(|| panic!("expected bare `Make` (without `Map.`) to fail, it compiled"));
    let msg = err.to_string();
    assert!(
        msg.contains("functor application") && msg.contains("unknown"),
        "expected the functor-framed unknown-application error, got: {msg}"
    );
}

/// The value bar: instantiate `Map.Make` against a concrete `int`-keyed
/// `Ord` module, exercise `empty`/`add`/`find`/`remove` for real, and
/// observe the ACTUAL evaluated strings through the rendered document —
/// proving the functor instantiates AND its operations evaluate (not
/// merely type-check).
#[test]
fn map_make_int_ord_instantiates_and_operations_evaluate() {
    let harness = "\
module MapHarness = struct
  module IntOrd = struct
    type t = int
    val compare x y =
      if x < y then Less else if x == y then Equal else Greater
  end
  module IM = Map.Make IntOrd
end
";
    let entry = "\
let m0 = MapHarness.IM.empty in
let m1 = MapHarness.IM.add 1 `one` m0 in
let m2 = MapHarness.IM.add 2 `two` m1 in
let m3 = MapHarness.IM.add 3 `three` m2 in
let found =
  match MapHarness.IM.find 2 m3 with
  | Some(s) -> s
  | None    -> `MISSING`
  end
in
let removed = MapHarness.IM.remove 1 m3 in
let after-remove =
  match MapHarness.IM.find 1 removed with
  | Some(_) -> `STILL-THERE`
  | None    -> `GONE`
  end
in
let ib1 = embed-string found in
let ib2 = embed-string after-remove in
let open V01Mini in
document (| title = `map-harness` |) '<
  +p { #ib1; #ib2; }
>
";
    let doc = compile_with_harness(
        "map-value",
        "@require: v01-mini\n@require: map",
        harness,
        entry,
    )
    .expect("map.satyg's Map.Make IntOrd should instantiate, compile, and evaluate");
    let words = first_line_words(&doc);
    assert_eq!(
        words,
        vec!["two".to_string(), "GONE".to_string()],
        "expected `find 2` to return the value added at key 2, and `find 1` to \
         miss after `remove 1` — proving `add`/`find`/`remove` really evaluated"
    );
}

/// `Map.Make` applied to a module missing `compare` is rejected, but NOT
/// via `v01_functors.rs`'s "does not match functor" wording: those
/// synthetic fixtures' functor bodies never reference `Key.compare`, so
/// the width/arity check (`check_functor_applications`, `lib.rs:222-223`)
/// fires first. The REAL `map.satyg`'s `Make` body DOES use `Key.compare`
/// throughout, so once `BadOrd` is substituted in, plain elaboration hits
/// `BadOrd.compare` as an unbound-variable error BEFORE module_check ever
/// runs — a different enforcement layer, but an equally precise,
/// member-naming rejection.
#[test]
fn map_make_rejects_an_argument_missing_compare() {
    let harness = "\
module MapBadHarness = struct
  module BadOrd = struct
    type t = int
  end
  module M = Map.Make BadOrd
end
";
    let err = compile_with_harness("map-bad-ord", "@require: map", harness, "1")
        .err()
        .unwrap_or_else(|| panic!("expected `Map.Make BadOrd` (missing `compare`) to be rejected"));
    let msg = err.to_string();
    assert!(
        msg.contains("compare"),
        "expected the rejection to name the missing `compare` member, got: {msg}"
    );
}

// `set.satyg` — `Set.Make`, built ON TOP of `map.satyg` (`module Impl =
// Map.Make Elem` inside `Set.Make`'s OWN body — the real-package instance
// of 2f-2a's cross-package functor-application-in-functor-body shape).

#[test]
fn set_bare_make_is_unbound_without_qualification() {
    let harness = "\
module SetBareHarness = struct
  module IntOrd = struct
    type t = int
    val compare x y =
      if x < y then Less else if x == y then Equal else Greater
  end
  module M = Make IntOrd
end
";
    let err = compile_with_harness("set-bare", "@require: set", harness, "1")
        .err()
        .unwrap_or_else(|| panic!("expected bare `Make` (without `Set.`) to fail, it compiled"));
    let msg = err.to_string();
    assert!(
        msg.contains("functor application") && msg.contains("unknown"),
        "expected the functor-framed unknown-application error, got: {msg}"
    );
}

/// The value bar: instantiate `Set.Make` against a concrete `int`-keyed
/// `Ord` module, exercise `empty`/`add`/`is-member`/`remove` for real
/// (transitively driving `map.satyg`'s `Map.Make Elem` underneath), and
/// observe the ACTUAL evaluated booleans (rendered as `yes`/`no` words)
/// through the document.
#[test]
fn set_make_int_ord_instantiates_and_operations_evaluate() {
    let harness = "\
module SetHarness = struct
  module IntOrd = struct
    type t = int
    val compare x y =
      if x < y then Less else if x == y then Equal else Greater
  end
  module IS = Set.Make IntOrd
end
";
    let entry = "\
let s0 = SetHarness.IS.empty in
let s1 = SetHarness.IS.add 1 s0 in
let s2 = SetHarness.IS.add 2 s1 in
let s3 = SetHarness.IS.add 3 s2 in
let member2 = SetHarness.IS.is-member 2 s3 in
let member9 = SetHarness.IS.is-member 9 s3 in
let s4 = SetHarness.IS.remove 2 s3 in
let member2-after-remove = SetHarness.IS.is-member 2 s4 in
let word1 = (if member2 then `yes` else `no`) in
let word2 = (if member9 then `yes` else `no`) in
let word3 = (if member2-after-remove then `yes` else `no`) in
let ib1 = embed-string word1 in
let ib2 = embed-string word2 in
let ib3 = embed-string word3 in
let open V01Mini in
document (| title = `set-harness` |) '<
  +p { #ib1; #ib2; #ib3; }
>
";
    let doc = compile_with_harness(
        "set-value",
        "@require: v01-mini\n@require: set",
        harness,
        entry,
    )
    .expect("set.satyg's Set.Make IntOrd should instantiate, compile, and evaluate");
    let words = first_line_words(&doc);
    assert_eq!(
        words,
        vec!["yes".to_string(), "no".to_string(), "no".to_string()],
        "expected `is-member 2` to be true, `is-member 9` false, and `is-member 2` \
         false again after `remove 2` — proving Set's ops (and Map.Make underneath) \
         really evaluated"
    );
}
