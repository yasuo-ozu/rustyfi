# layout-tests

Everything here answers one question: **does this port lay a document out the
way upstream SATySFi does?** Nothing else in the repo can answer it, because
answering it needs upstream's own output to compare against — which is what
`corpus/` carries.

Three roles live here, and they are easy to confuse because all three involve
`.saty` files and Python.

## The gate

    fidelity.py          the comparison, and the pass/fail
    baseline.json        the recorded per-document numbers
    corpus/              10 vendored third-party projects, and 6 PDFs built by
                         the ORIGINAL OCaml SATySFi

`fidelity.py` rebuilds each corpus document with the port and compares the
result against the committed upstream PDF. Run it directly, or through the
`#[ignore]`d Rust wrapper (`crates/rustyfi/tests/layout_fidelity.rs`) that CI
uses:

    python3 layout-tests/fidelity.py                 # check against the baseline
    python3 layout-tests/fidelity.py --update        # re-record it
    python3 layout-tests/fidelity.py --doc easytable # one document

The reference PDFs are the fixed point the whole comparison rests on. Do not
regenerate them to make a number go green.

## The probes

    probes/*.saty        minimised reproductions of one divergence each

A corpus document tells you *that* something diverges; a probe tells you
*what*. Each one strips a single construct down to the smallest document that
still shows the defect, and its header comment records the measurement that
motivated it — read those before touching one.

Several have graduated from scratch work into regression inputs and are now
named from Rust test doc comments (`pagebreak.rs`, `frame_margins.rs`,
`math_cramped.rs`, `math_table.rs`, `math_fraction_radical.rs`). Renaming or
editing one of those means updating the citation.

The `vspace_*` group and `frame_across_page.saty` came from one vertical-spacing
investigation and were numbered `p01`..`p06`; they were renamed by topic when
they merged in here, because a bare `p04` means nothing next to
`math_script_drop.saty`.

## The measuring instrument

    measure/baselines.py  text baselines from the PDF CONTENT STREAM
    measure/lines.py      line extraction
    measure/pagetops.py   first-baseline-per-page
    measure/dyscan.py     baseline-advance scanning

An importable library, not a test — `fidelity.py` puts `measure/` on
`sys.path` and imports it, and `crates/rustyfi-backend/tests/pagebreak.rs`
cites its measurements.

It exists because of one trap worth internalising: **`pdftotext -bbox` reports
a glyph BOX, whose top and bottom come from the font descriptor, and the two
engines emit different descriptors for identical Latin glyphs.** Comparing a
Latin word's `yMin` across engines therefore measures the descriptor, not the
layout. These modules read positions out of the content stream's
text-positioning operators instead. Any new vertical measurement should go
through them rather than growing a second copy that can drift.

## The diagnostics

    tools/compare.py     render one probe with BOTH engines, lines side by side
    tools/delta.py       per-gap baseline-advance difference, whole document
    tools/probe.py       build one probe with both engines and measure it
    tools/linebreak.py   where the two engines put their line breaks

Human-facing. No baselines, no assertions, nothing depends on them. `--help`
on each. Several want the original `satysfi` on `PATH`, i.e. `nix develop`.

## Not here

`download-fonts.sh` and `benchmark.py` live at the repo root — the first
because installing and CI both need it and it is not a layout test, the second
because it measures speed rather than layout.
