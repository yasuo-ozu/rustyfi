//! A type-synonym cycle routed through a 0.1 command slot's `?(l : τ)`
//! optional-label map.
//!
//! # What was wrong
//!
//! `typecheck::expand_synonyms` inlines synonyms transparently, and its own
//! doc comment states that it terminates *only* because
//! `check_synonym_cycles` (run unconditionally at `Checker::new`) already
//! rejected every cycle. That check is built from `synonym_refs`, and
//! `synonym_refs`' command-type arm read
//!
//! ```text
//!     cs.iter().for_each(|c| synonym_refs(&c.ty, synonyms, out));
//! ```
//!
//! — recursing into each slot's mandatory `ty` and **not** into that slot's
//! `opt_labels`, the second `MonoType`-bearing field of `CmdArgType`.
//! `expand_synonyms_cmd_args` had no such blind spot and expanded both. So a
//! cycle that goes through an optional-label slot was invisible to the guard
//! and reachable by the expander: `type t = inline [?(l : t) string]`, once
//! referenced, drove `expand_synonyms` into unbounded recursion and **aborted
//! the process with a stack overflow** rather than reporting the cycle.
//!
//! The fix is not a patched arm: `synonym_refs` is now derived from the type
//! definitions (`rustyfi_lang::visit`), so it cannot omit a field.
//!
//! The mandatory-slot control below took the clean error all along; it is
//! here to show the two paths now agree, and to catch a regression that
//! breaks cycle detection wholesale rather than just at this one field.

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::CompileError;
use rustyfi_loader::{LoadedCst, LoadedFile};
use rustyfi_syntax::parse_file_v1;
use rustyfi_syntax::RustyfiVersion;

/// Never exercised — every fixture here fails type-checking long before
/// glyph metrics matter. Same stub shape as `v01_opt_cmd_rows.rs`'s `Mono`.
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

fn run(lib_src: &str) -> Result<(), CompileError> {
    let files = vec![
        LoadedFile {
            path: std::path::PathBuf::from("lib.satyh"),
            cst: LoadedCst::V0_1(
                parse_file_v1(lib_src).unwrap_or_else(|e| panic!("lib parse failed: {e}")),
            ),
            origin: Default::default(),
            version: RustyfiVersion::V0_1,
        },
        LoadedFile {
            path: std::path::PathBuf::from("doc.saty"),
            cst: LoadedCst::V0_1(parse_file_v1("1").expect("doc parse")),
            origin: Default::default(),
            version: RustyfiVersion::V0_1,
        },
    ];
    let mono = Mono;
    rustyfi_lang::compile_document_v1(&files, &mono).map(|_| ())
}

fn assert_reports_cycle(lib_src: &str, what: &str) {
    match run(lib_src) {
        Err(CompileError::Type(e)) => {
            let msg = e.to_string();
            assert!(
                msg.contains("cyclic type synonym"),
                "{what}: expected a `cyclic type synonym` error, got: {msg}"
            );
        }
        Err(other) => panic!("{what}: expected a `cyclic type synonym` type error, got: {other}"),
        Ok(()) => panic!("{what}: expected rejection, but compilation succeeded"),
    }
}

/// THE regression. Before the fix this aborted the test process with
/// `thread ... has overflowed its stack / fatal runtime error: stack
/// overflow` — note that a `#[should_panic]` test could not have caught it,
/// because a stack overflow is a `SIGABRT`, not an unwind.
#[test]
fn cycle_through_an_optional_label_slot_is_reported_not_overflowed() {
    assert_reports_cycle(
        "module M = struct\n\
         type t = inline [?(l : t) string]\n\
         type u = | U of t\n\
         val x = 1\n\
         end",
        "optional-label slot",
    );
}

/// The control: the same cycle through the slot's MANDATORY type. This arm
/// was always caught — `synonym_refs` did recurse into `c.ty`.
#[test]
fn cycle_through_a_mandatory_slot_is_reported() {
    assert_reports_cycle(
        "module M = struct\n\
         type t = inline [t]\n\
         type u = | U of t\n\
         val x = 1\n\
         end",
        "mandatory slot",
    );
}

/// `check_synonym_cycles` runs over every registered synonym at
/// `Checker::new`, so the cycle is reported even when nothing references the
/// synonym — which is what makes this a guard rather than a lucky ordering.
/// Before the fix this case passed *vacuously*: the cycle went unreported and
/// nothing expanded it, so the file compiled.
#[test]
fn an_unreferenced_optional_label_cycle_is_still_reported() {
    assert_reports_cycle(
        "module M = struct\n\
         type t = inline [?(l : t) string]\n\
         val x = 1\n\
         end",
        "unreferenced optional-label slot",
    );
}

/// Two-step cycle (`t -> u -> t`) where only the second hop goes through an
/// optional-label slot — the shape a single-hop self-reference check would
/// still miss.
#[test]
fn a_two_step_cycle_via_an_optional_label_slot_is_reported() {
    assert_reports_cycle(
        "module M = struct\n\
         type t = u list\n\
         type u = inline [?(l : t) string]\n\
         val x = 1\n\
         end",
        "two-step cycle",
    );
}

/// The negative control: an optional-label slot that mentions a synonym
/// **acyclically** must still compile. A fix that simply rejected every
/// `opt_labels` mention would pass every test above and fail this one.
#[test]
fn an_acyclic_optional_label_mention_still_compiles() {
    match run("module M = struct\n\
         type s = string\n\
         type t = inline [?(l : s) string]\n\
         type u = | U of t\n\
         val x = 1\n\
         end")
    {
        Ok(()) | Err(CompileError::NotADocument(_)) => {}
        Err(other) => panic!("an acyclic `?(l : s)` mention must compile, got: {other}"),
    }
}
