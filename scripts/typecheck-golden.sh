#!/usr/bin/env bash
# L3 (`…/tmp/l3-typecheck-refactor.md` §8.3): the typecheck.rs differential
# golden harness — the "ordering tripwire" that catches a swapped
# expected/found, a dropped `?`, or an off-by-one arm move during the L3
# refactor (a class of regression the unit-test suite's spot assertions can
# miss, since it changes an error/warning *string*, not a verdict).
#
# What it does: runs `crates/rustyfi-lang/tests/typecheck_golden.rs`'s
# `typecheck_golden` test (an `#[ignore]`d integration test — see that
# file's doc comment) once against `HEAD` (or a caller-supplied ref) and
# once against the current working tree, and diffs the two outputs. Each
# output line is one `OK <tag> (<version>) <n-warnings> [<warnings>]` or
# `ERR <tag> (<version>) <message>` for every `.saty`/`.satyh`/`.satyg`
# fixture under `crates/*/tests/fixtures/` plus every bundled package under
# `lib-rustyfi/dist/packages/` (loader-merged via a synthetic
# `@require: <pkg>` entry, the same way `stdja` loads today).
#
# Usage:
#   scripts/typecheck-golden.sh [baseline-ref]
#
# `baseline-ref` defaults to `HEAD`. Requires a clean working tree for
# `crates/rustyfi-lang/src/typecheck.rs` (the file this compares) — the
# script stashes nothing; it uses `git show` to materialize the baseline
# version into a temp file and restores the working-tree version afterward,
# so uncommitted changes to typecheck.rs are the very thing being tested and
# must already be in place when you run this.
#
# Exit status: 0 iff the two outputs are byte-identical (mod whitespace-only
# `sort`ing, applied to both sides identically so the corpus's own file
# order never causes a false mismatch). On a non-empty diff, prints it and
# exits 1 — that is a behavior regression; do not commit through it.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

baseline_ref="${1:-HEAD}"
typecheck_rs="crates/rustyfi-lang/src/typecheck.rs"

work_dir="$(mktemp -d)"
current_copy="$work_dir/typecheck.rs.current"
baseline_copy="$work_dir/typecheck.rs.baseline"
cp "$typecheck_rs" "$current_copy"
git show "$baseline_ref:$typecheck_rs" > "$baseline_copy"

# Always restore the working tree's own typecheck.rs (the version being
# tested) before exiting, whether we succeed, fail, or are interrupted —
# never leave the checkout pointed at the baseline swap.
restore_working_tree() {
    cp "$current_copy" "$typecheck_rs"
    rm -rf "$work_dir"
}
trap restore_working_tree EXIT

run_golden() {
    local out_file="$1"
    cargo test -p rustyfi-lang --test typecheck_golden -- --ignored --nocapture \
        > "$out_file.raw" 2>&1
    grep -E '^(OK|ERR) ' "$out_file.raw" | sort > "$out_file"
}

echo "== running golden harness against baseline ($baseline_ref) =="
cp "$baseline_copy" "$typecheck_rs"
run_golden "$work_dir/before"

echo "== running golden harness against the working tree =="
cp "$current_copy" "$typecheck_rs"
run_golden "$work_dir/after"

if diff -u "$work_dir/before" "$work_dir/after" > "$work_dir/diff"; then
    echo "GOLDEN DIFF EMPTY — byte-identical across $(wc -l < "$work_dir/before") corpus entries."
    exit 0
else
    echo "GOLDEN DIFF NON-EMPTY — behavior regression, do not commit:"
    cat "$work_dir/diff"
    exit 1
fi
