//! Slice 1 / Tier 0 stdlib port proof (`docs/plans/stdlib-port.md` §Slice 1):
//! `@require: list` (hence, transitively, `@require: option`) — and
//! `@require: option` alone — must PARSE, ELABORATE, TYPECHECK, and EVALUATE
//! through the real multi-file loader with this repo's `lib-satysfi/` as
//! `lib_root`. This mirrors `satysfi-cli`'s own production pipeline
//! (`main.rs`'s `cmd_compile`: `satysfi_loader::load` -> merge preludes ->
//! `compile_document_cst`) rather than a bespoke single-file harness, so it
//! genuinely exercises `@require:` resolution (including the NESTED
//! `list.satyg -> @require: option` edge) through the production loader
//! crate — not just a hand-rolled shortcut.
//!
//! `option.satyg`/`list.satyg` under `lib-satysfi/dist/packages/` are copied
//! byte-for-byte from upstream (the plan's "copy-verbatim" policy) — this
//! test is the proof the compiler now *accepts* them (the Slice-1
//! acceptance bar is "compiles", i.e. evaluates to a value, not merely
//! "parses" — see the plan's Verification table).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck};
use satysfi_loader::{LoadOptions, LoadedProgram};

/// This repo's `lib-satysfi/` (the real Tier-0 packages' home), resolved
/// relative to this crate's own manifest directory — the same convention
/// `compile.rs`'s private `prepare_document` test helper already uses for
/// `stdja-mini.satyh`.
fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-satysfi")
}

/// A uniquely-named temp `.saty` file, cleaned up on drop — scaled down from
/// `satysfi-loader/tests/loader.rs`'s `TempDir` fixture pattern to the one
/// entry file each test here needs (the packages themselves already live on
/// disk under `lib_root()`, so there is no fixture tree to build).
struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satysfi-lang-stdlib-tier0-{tag}-{}-{}.saty",
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

/// Merge a loader-resolved program's preludes into one synthetic
/// `cst::File`, exactly like `satysfi-cli`'s private `merge_program`
/// (`main.rs`): the loader guarantees dependency-first order with the entry
/// document last, so every library's prelude is spliced ahead of the
/// entry's own, in that order (the v0.0.6 analog typechecks each library
/// into a shared environment in dependency order; untyped elaboration gets
/// the same scoping by prelude concatenation).
fn merge_program(program: LoadedProgram) -> satysfi_syntax::cst::File {
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(lib.cst.prelude);
    }
    prelude.extend(entry.cst.prelude);
    satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry.cst.in_kw,
        body: entry.cst.body,
        eoi: entry.cst.eoi,
    }
}

/// `FontMetrics` stub: Tier-0 packages (`option`/`list`) are pure computation
/// over int/list/option values — never text/box primitives — so this is
/// never actually consulted. It exists only because `eval::Interp::new`
/// requires a `&dyn FontMetrics`.
struct NoFonts;

impl FontMetrics for NoFonts {
    fn advance(&self, _f: FontKey, _c: char, _size: Length) -> Option<Length> {
        None
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size
    }
    fn descender(&self, _f: FontKey, _size: Length) -> Length {
        Length::pt(0.0)
    }
}

/// Load `src` (a document `@require:`ing packages resolved against
/// `lib_root()`) through the real loader, merge, elaborate, typecheck, and
/// evaluate — returning the final `Value`. This is the full Slice-1
/// "compiles" bar (`docs/plans/stdlib-port.md`'s Verification table:
/// `Parses` / `Typechecks` / `Compiles`), not merely a parse or a typecheck.
fn compile_via_loader(tag: &str, src: &str) -> Result<Value, String> {
    let doc = TempDoc::new(tag, src);
    let opts = LoadOptions {
        lib_root: Some(lib_root()),
        ..Default::default()
    };
    let program = satysfi_loader::load(&doc.0, &opts).map_err(|e| format!("load: {e}"))?;
    let file = merge_program(program);

    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck(&elaborated).map_err(|e| format!("typecheck: {e}"))?;
    let mono = NoFonts;
    let mut interp = eval::Interp::new(&mono);
    interp
        .eval(&env, &elaborated.body)
        .map_err(|e| format!("eval: {e}"))
}

fn as_int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        other => panic!("expected an int, got {other:?}"),
    }
}

fn as_int_list(v: Value) -> Vec<i64> {
    match v {
        Value::List(items) => items.into_iter().map(as_int).collect(),
        other => panic!("expected a list, got {other:?}"),
    }
}

// ============================================================================
// `@require: list` (transitively pulls in `option`, per `list.satyg`'s own
// `@require: option` header).
// ============================================================================

#[test]
fn require_list_reverse_and_map_compiles_and_evaluates() {
    let src = "@require: list
in
List.reverse (List.map (fun x -> x + 1) [1; 2; 3])";
    let v = compile_via_loader("list-reverse-map", src).expect("list.satyg should compile");
    assert_eq!(as_int_list(v), vec![4, 3, 2]);
}

#[test]
fn require_list_length_compiles_and_evaluates() {
    let src = "@require: list
in
List.length (List.map (fun x -> x + 1) [1; 2; 3])";
    let v = compile_via_loader("list-length", src).expect("list.satyg should compile");
    assert_eq!(as_int(v), 3);
}

#[test]
fn require_list_transitively_compiles_option() {
    // `list.satyg` itself `@require:`s `option` (`List.map-with-ends` calls
    // `Option.is-none`) — proving a *nested* `@require:` resolves and the
    // resulting merged program still compiles, not just `list.satyg` in
    // isolation.
    let src = "@require: list
in
Option.from 0 (Some 5)";
    let v = compile_via_loader("list-nested-option", src).expect("should compile");
    assert_eq!(as_int(v), 5);
}

#[test]
fn require_list_uses_pipe_internally() {
    // `list.satyg`'s `reverse`/`map-adjacent`/`map-with-ends` all use `|>`
    // internally (Blocker B) — `fold-left-adjacent` is the most direct way
    // to exercise that path from outside the module.
    let src = "@require: list
in
List.map-adjacent (fun x left right -> x) [1; 2; 3]";
    let v = compile_via_loader("list-pipe-internal", src).expect("should compile");
    assert_eq!(as_int_list(v), vec![1, 2, 3]);
}

#[test]
fn require_list_mapi_adjacent_uses_a_tuple_pattern_lambda_correctly() {
    // `List.mapi-adjacent`'s OWN definition is `fun (i, acc) x leftopt
    // rightopt -> ..` — a tuple-DESTRUCTURING first parameter. This was the
    // one real grammar gap Slice 1 hit porting `list.satyg` VERBATIM
    // (`Expr::Fun`'s parameters were `Vec<VarTok>`, i.e. plain variables
    // only; `cst.rs`/`elaborate.rs` now accept a full `patbot`, reusing the
    // same pattern-currying `let-rec` already used — see `cst::ast::Expr::
    // Fun`'s doc comment). Asserting on the resulting VALUES (not just that
    // this parses) proves the destructuring actually binds `i`/`acc`
    // correctly through every fold step, not merely that parsing recovered.
    let src = "@require: list
in
List.mapi-adjacent (fun i x leftopt rightopt -> i) [10; 20; 30]";
    let v =
        compile_via_loader("list-mapi-adjacent", src).expect("list.satyg should compile");
    assert_eq!(as_int_list(v), vec![0, 1, 2]);
}

#[test]
fn require_list_map_with_ends_calls_option_is_none_across_module_boundary() {
    // `List.map-with-ends` (also `|>`-internal) calls `Option.is-none`
    // *from list.satyg's own body* — proving the nested `@require: option`
    // dependency is not just loaded but actually callable from within the
    // dependent package, not merely from the entry document.
    let src = "@require: list
in
List.map-with-ends
  (fun is-first is-last x -> if is-first then 100 else if is-last then 200 else x)
  [1; 2; 3]";
    let v = compile_via_loader("list-map-with-ends", src).expect("list.satyg should compile");
    assert_eq!(as_int_list(v), vec![100, 2, 200]);
}

// ============================================================================
// `@require: option` alone (the second, minimal Slice-1 case).
// ============================================================================

#[test]
fn require_option_alone_compiles_and_evaluates() {
    let src = "@require: option
in
Option.from 0 (Some 5)";
    let v = compile_via_loader("option-alone", src).expect("option.satyg should compile");
    assert_eq!(as_int(v), 5);
}

#[test]
fn require_option_map_and_is_none() {
    let src = "@require: option
in
if Option.is-none None
then Option.from 0 (Option.map (fun x -> x + 1) (Some 41))
else -1";
    let v = compile_via_loader("option-map", src).expect("option.satyg should compile");
    assert_eq!(as_int(v), 42);
}

#[test]
fn require_option_bind() {
    let src = "@require: option
in
Option.from 0 (Option.bind (Some 10) (fun x -> Some (x * 2)))";
    let v = compile_via_loader("option-bind", src).expect("option.satyg should compile");
    assert_eq!(as_int(v), 20);
}

// ============================================================================
// `color` built-in variant (frontend-completion.md §Slice1-B), through the
// SAME loader pipeline — no `@require:` needed (it's a built-in, seeded by
// `prim_types::builtin_variants` before any package loads), but routing it
// through `compile_via_loader` proves a document using it compiles
// end-to-end via the production load path too, not merely in isolation.
// ============================================================================

#[test]
fn a_color_value_compiles_via_the_loader() {
    // No `@require:` at all: `color` is a true built-in (seeded before any
    // package loads), so a document naming it needs no package dependency —
    // this also proves the loader/merge path tolerates a zero-dependency
    // entry document just as well as a multi-file one.
    let src = "let c = RGB (0.2, 0.4, 0.6)
in
match c with
| Gray(_)          -> 1
| RGB(r, g, b)     -> 2
| CMYK(_, _, _, _) -> 3";
    let v = compile_via_loader("color-via-loader", src).expect("should compile");
    assert_eq!(as_int(v), 2);
}

#[test]
fn a_color_value_compiles_alongside_require_list() {
    // Combined with a real `@require:` too, so the built-in variant and a
    // loaded package coexist in the same merged program.
    let src = "@require: list
in
let cs = [RGB (1.0, 0.0, 0.0); Gray 0.5; CMYK (0.0, 0.0, 0.0, 1.0)] in
List.length cs";
    let v = compile_via_loader("color-and-list", src).expect("should compile");
    assert_eq!(as_int(v), 3);
}
