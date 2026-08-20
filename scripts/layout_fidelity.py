#!/usr/bin/env python3
"""Layout-fidelity comparison: this Rust port vs. upstream SATySFi.

For each corpus document that ships an upstream-built reference PDF (produced by
the original OCaml SATySFi and vendored under `scripts/layout_fidelity_corpus/`), this
builds the SAME source with the Rust port and compares the two PDFs' *layout* —
not their bytes. Comparison is via poppler's `pdftotext -bbox`, which emits every
word's bounding box; the port bundles the SAME fonts SATySFi uses (IPAex / Latin
Modern / Junicode / DejaVu-Math), so glyph metrics are identical and any layout
divergence reflects the ENGINE (line breaking, inter-box spacing, page breaking,
box placement) — exactly what we want to measure.

Each document is a showcase of one complex typesetting construct, so the set
below covers every complex part of the corpus that the port can currently build:

  latexcmds  math, colored/framed/shadowed boxes, LaTeX-like commands, x-refs
  xpath      vector graphics: paths, bezier curves, intersections, diagrams
  enumitem   nested and heavily-customized itemize / enumerate
  easytable  tables: cells, rules, alignment, multi-row/col
  figbox     figures, floats, image boxes, captions (composes easytable+enumitem)
  slydifi    slides: frames, layers, absolute placement
  gakushin   a real grant-application form (self-snapshot only)

(`gakushin` has no committed upstream PDF to compare against — it is checked in
SELF-SNAPSHOT mode — and upstream cannot build it at all without the
Satyrographos-installed `fonts-junicode` package, which the vendored corpus does
not carry. An earlier version of this note said `slydifi` was excluded for
needing an un-bundled `railway` package and an unimplemented math primitive;
that stopped being true when `railway` was vendored and the primitive landed —
it is in `DOCS`, it builds, and it matches upstream's 30 pages.)

TWO CONFOUNDS THIS HARNESS USED TO HAVE, AND HOW THE METRICS AVOID THEM
----------------------------------------------------------------------

Both are artifacts of `pdftotext`, and each one misdirected investigations
before it was pinned down. Every metric below is chosen so that neither can
move it:

1. **The font-descriptor trap.** A word's `pdftotext -bbox` box has its top and
   bottom from the FONT DESCRIPTOR, and the two writers emit different
   descriptors for identical glyphs. So the same word on the same baseline gets
   a different `yMin` in each engine, and a mixed CJK/latin line clusters as one
   line in one engine and two in the other with the layout IDENTICAL. `lines`
   therefore comes from the PDF CONTENT STREAM (`vspace_probe/baselines.py`),
   never from glyph boxes — see `line_count`.

2. **Word splits are justification-sensitive.** `pdftotext` splits words on
   inter-glyph GAPS, so an identical CJK run tokenizes one way on an unjustified
   last line and another way on a justified one: upstream's stretched CJK glue
   opens gaps at `、`/`。` that poppler splits on and the port's does not. Word
   COUNTS are consequently not comparable, and were reporting content deficits
   of 100 (easytable) and 60 (enumitem) words where the real character deficits
   are 56 and 13. So content is compared as CHARACTERS — see `content_metrics`.

Metrics per document (all font-robust, since fonts are identical):

  GATED — a regression in one of these fails the run:

  lines / lines_dev
                   text BASELINE count, from the content stream, and its
                   absolute deviation from upstream's. The ratchet is on the
                   DEVIATION: it may shrink, never grow.
  chars_missing    characters upstream typeset that the port did not, as a
                   whole-document MULTISET difference. Order- and
                   tokenization-immune: this is the "no content was lost" floor.
  chars_extra      the reverse. A rise means the port emitted text upstream
                   has none of — duplicated runs, stray debug output.
  char_match       difflib ratio over the whole-document CHARACTER stream in
                   reading order. Content AND ordering: unlike `chars_missing`
                   it also falls when content merely MOVES (across a page
                   boundary, say), which is the thing `chars_missing` is
                   deliberately blind to.
  width_p95_pt     95th-percentile |word-width delta| over aligned words, in pt.
                   Should be ~0 because metrics match; a large value means the
                   port set a run in the wrong font / size / with wrong tracking.
  page_gap         |port pages - upstream pages|. The headline PAGINATION
                   signal, ratcheted the same way as `lines_dev`.

  INFORMATIONAL — recorded and reported, NOT gated:

  words / upstream_words
                   poppler word counts. Kept because a lot of prior notes quote
                   them, but confound 2 means a delta here says nothing on its
                   own; `chars_missing` is the number to read instead.
  text_match       the original word-sequence difflib ratio. Same reason: it
                   drops on a pure re-tokenization (easytable sits at 0.874
                   while 99.6% of its characters match exactly), so gating it
                   would gate the tokenizer, not the layout. `char_match` is
                   its trustworthy replacement and IS gated.
  chars / upstream_chars, left_margin

`--tol-sweep` re-measures the claim behind `lines` on demand: it prints the
port-minus-upstream baseline delta across a range of clustering tolerances for
every doc, which is what separates a real gap (tolerance-stable) from a
clustering artifact (collapses as the tolerance grows).

The baseline (`layout_fidelity_baseline.json`) pins each metric at its current
value, so this PASSES today and FAILS on a regression. Re-baseline with
`--update` after an intentional change.

Output: each run leaves the pair it compared beside the package it came from,

  scripts/layout_fidelity_corpus/<doc>/<doc>.port.pdf      the port's render
  scripts/layout_fidelity_corpus/<doc>/<doc>.satysfi.pdf   the reference it was
                                                           compared against

so when a metric moves you can open the two PDFs the number came from instead of
re-running to reproduce them. Both suffixes are gitignored; the vendored
reference itself keeps its own upstream name (`doc.ref`) and stays tracked, and
`<doc>.satysfi.pdf` is a copy of it under the predictable name. `--out-dir`
collects every doc into one directory instead; `--no-persist` writes nothing.

Usage:
  scripts/layout_fidelity.py [--doc NAME]... [--update] [--bin PATH]
      [--keep-going] [--verbose]

Exit status: 0 iff every compared document meets its baseline (or --update).
Self-skips (exit 0 with a SKIP note) if poppler, the port binary, or the
the vendored corpus is missing, so it is safe to invoke unconditionally.
"""

from __future__ import annotations

import argparse
import html
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
from collections import Counter
from dataclasses import dataclass, field
from difflib import SequenceMatcher
from pathlib import Path

# The content-stream baseline reader `lines` is measured with. It already
# existed — it is what settled the math-metrics work — so this imports it rather
# than growing a second copy that can drift out of agreement with the probe
# scripts (`vspace_probe/pagetops.py` reads the same `runs_of`).
sys.path.insert(0, str(Path(__file__).resolve().parent / "vspace_probe"))
from baselines import page_baselines  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
# The corpus is VENDORED next to this script rather than pulled in as git
# submodules: only three things were ever needed from those 11 repos — each
# document's own directory (its `.saty`, the upstream-built reference PDF, and
# any images or relatively-`@import`ed siblings), each package's `src/`, and
# `satysfi-base/src`. That is 9.4 MB against the submodules' 19 MB, needs no
# `git submodule update --init` (nor `submodules: recursive` in CI), and cannot
# drift: the reference PDFs are the fixed point the whole comparison rests on.
CORPUS = Path(__file__).resolve().parent / "layout_fidelity_corpus"
LIB_RUSTYFI = REPO / "lib-rustyfi"
BASELINE_PATH = Path(__file__).resolve().parent / "layout_fidelity_baseline.json"

# Content-stream baselines within this many points of each other are one text
# line. Every run of one typeset line is emitted at the identical `y` by both
# writers, so this only absorbs the rounding of PDF's decimal numbers; see
# `line_count` for the tolerance sweep that establishes it.
BASELINE_TOL = 0.05

# Tolerances `--tol-sweep` reports the port-vs-upstream baseline delta at.
SWEEP_TOLS = (0.02, 0.05, 0.1, 0.5, 1.0, 2.0, 3.0)


@dataclass
class Doc:
    name: str
    # Source .saty, relative to CORPUS; built with CWD = its parent dir so
    # relative `@import:`s and any CWD-relative image loads resolve as upstream.
    src: str
    # Upstream-built reference PDF, relative to CORPUS. EMPTY ("") means no
    # upstream PDF is available to compare against — the doc is then checked in
    # SELF-SNAPSHOT mode: the port's own layout signature (page / word / line
    # counts, left margin) is pinned in the baseline and guarded against
    # regression. This still "covers" the doc's complex parts.
    ref: str
    # One-line description of the complex construct this document showcases.
    covers: str
    # Sibling corpus packages this doc `@require:`s by published name, staged
    # into the lib-root as {published_prefix: source_src_dir_relative_to_CORPUS}.
    # A "" prefix copies the source dir's CONTENTS into the packages root (used
    # for multi-package trees like fss, whose `src/` holds fss/, sss/, ...).
    stage: dict[str, str] = field(default_factory=dict)


DOCS: list[Doc] = [
    Doc(
        "latexcmds",
        "latexcmds/doc/latexcmds-doc.saty",
        "latexcmds/doc/latexcmds-doc.pdf",
        "math, colored/framed/shadowed boxes, LaTeX-like commands, cross-refs",
        {"latexcmds": "latexcmds/src"},
    ),
    Doc(
        "xpath",
        "xpath/doc/xpath-doc.saty",
        "xpath/doc/xpath-doc.pdf",
        "vector graphics: paths, bezier curves, intersections, diagrams",
        {},  # xpath's package + shared util come in via relative @import
    ),
    Doc(
        "enumitem",
        "enumitem/doc/enumitem.saty",
        "enumitem/doc/enumitem.pdf",
        "nested and heavily-customized itemize / enumerate",
        {"enumitem": "enumitem/src"},
    ),
    Doc(
        "easytable",
        "easytable/doc/easytable.saty",
        "easytable/doc/easytable.pdf",
        "tables: cells, rules, alignment, multi-row/col",
        {"easytable": "easytable/src", "enumitem": "enumitem/src"},
    ),
    Doc(
        "figbox",
        "figbox/doc/manual.saty",
        "figbox/doc/manual.pdf",
        "figures, floats, image boxes, captions (composes easytable+enumitem)",
        {
            "figbox": "figbox/src",
            "easytable": "easytable/src",
            "enumitem": "enumitem/src",
        },
    ),
    Doc(
        "slydifi",
        "slydifi/doc/slydifi.saty",
        "slydifi/doc/slydifi.pdf",
        "slides: frames, layout templates, overlays, themed decorations",
        {
            "class-slydifi": "slydifi/src",
            "easytable": "easytable/src",
            "enumitem": "enumitem/src",
            "railway": "railway/src",
        },
    ),
    Doc(
        "gakushin",
        "gakushin/dc.saty",
        "",  # no upstream PDF committed -> self-snapshot mode
        "load-pdf-image embedding + the 学振 multi-column form layout (fss)",
        {"": "fss/src"},
    ),
]


# --------------------------------------------------------------------------
# Prerequisite probing (so the caller can self-skip cleanly).
# --------------------------------------------------------------------------


def find_pdftotext() -> str | None:
    exe = shutil.which("pdftotext")
    if not exe:
        return None
    try:
        subprocess.run([exe, "-v"], capture_output=True, timeout=15)
    except Exception:
        return None
    return exe


def default_bin() -> Path:
    return REPO / "target" / "debug" / "rustyfi"


def prereqs_ok(bin_path: Path) -> list[str]:
    """Return a list of missing-prerequisite messages (empty => all present)."""
    missing = []
    if find_pdftotext() is None:
        missing.append("poppler `pdftotext` not on PATH")
    if not bin_path.exists():
        missing.append(f"port binary not built at {bin_path} (run `cargo build -p rustyfi`)")
    if not CORPUS.exists():
        missing.append(f"{CORPUS} missing (the vendored corpus should be committed alongside this script)")
    if not (LIB_RUSTYFI / "dist" / "packages").exists():
        missing.append(f"{LIB_RUSTYFI}/dist/packages missing")
    return missing


# --------------------------------------------------------------------------
# lib-root assembly: port packages + staged sibling corpus package sources.
# --------------------------------------------------------------------------


def assemble_lib_root(dst: Path, docs: list[Doc]) -> Path:
    """Build a lib-root whose `dist/packages` holds the port's own packages plus
    every corpus package the selected docs `@require:` by name. Fonts are NOT
    copied — callers pass `--font-dir lib-rustyfi` for those."""
    pkg = dst / "dist" / "packages"
    pkg.mkdir(parents=True, exist_ok=True)
    # The port's bundled packages (base, stdjareport, itemize, math, ...).
    src_pkg = LIB_RUSTYFI / "dist" / "packages"
    for entry in src_pkg.iterdir():
        target = pkg / entry.name
        if entry.is_dir():
            shutil.copytree(entry, target, dirs_exist_ok=True)
        else:
            shutil.copy2(entry, target)
    # Overlay the FULL upstream `base` package from the satysfi-base submodule:
    # the port bundles only the subset its own stdlib needs, but enumitem /
    # easytable / figbox `@require:` further base modules (base/length,
    # base/list-ext, base/typeset/base, ...). This mirrors how the corpus is
    # actually built and leaves the base-independent docs (latexcmds/xpath)
    # byte-for-byte unchanged.
    base_src = CORPUS / "satysfi-base" / "src"
    if base_src.exists():
        shutil.copytree(base_src, pkg / "base", dirs_exist_ok=True)
    # Stage each sibling corpus package's sources under its published prefix.
    # An empty prefix copies the source dir's CONTENTS into the packages root
    # (for multi-package trees like fss: src/{fss,sss,...} -> packages/{fss,...}).
    for doc in docs:
        for prefix, srcdir in doc.stage.items():
            srcpath = CORPUS / srcdir
            if not srcpath.exists():
                continue
            dest = pkg if prefix == "" else pkg / prefix
            shutil.copytree(srcpath, dest, dirs_exist_ok=True)
    return dst


# --------------------------------------------------------------------------
# Build a doc with the port, extract per-page word boxes from a PDF.
# --------------------------------------------------------------------------


@dataclass
class Word:
    text: str
    x0: float
    y0: float
    x1: float
    y1: float

    @property
    def width(self) -> float:
        return self.x1 - self.x0


@dataclass
class Page:
    width: float
    height: float
    words: list[Word] = field(default_factory=list)


PAGE_RE = re.compile(r'<page width="([\d.]+)" height="([\d.]+)"')
WORD_RE = re.compile(
    r'<word xMin="([\d.]+)" yMin="([\d.]+)" xMax="([\d.]+)" yMax="([\d.]+)">(.*?)</word>'
)


def build_pdf(doc: Doc, bin_path: Path, lib_root: Path, out_pdf: Path, timeout: int) -> None:
    src = CORPUS / doc.src
    cwd = src.parent
    cmd = [
        str(bin_path),
        # Bypass the content-addressed compile cache: it is keyed on the SOURCE,
        # not the port binary, so a layout-engine change would otherwise be
        # masked by a stale cached render. The test must reflect the current
        # binary's layout.
        "--no-cache",
        # Also ignore any `<doc>.satysfi-aux`: it seeds cross-reference reads,
        # so honouring one would make a measurement depend on whatever a
        # previous run left behind — and would WRITE one back into the vendored
        # corpus, dirtying the working tree on every run. The port resolves
        # cross-references with its own fixpoint regardless.
        "--no-aux",
        "--lib-root",
        str(lib_root),
        "--font-dir",
        str(LIB_RUSTYFI),
        "-o",
        str(out_pdf),
        src.name,
    ]
    proc = subprocess.run(cmd, cwd=cwd, capture_output=True, timeout=timeout, text=True)
    if proc.returncode != 0 or not out_pdf.exists():
        tail = "\n".join((proc.stdout + proc.stderr).splitlines()[-8:])
        raise RuntimeError(f"port failed to build {doc.name}:\n{tail}")


def find_satysfi(explicit: str | None) -> str | None:
    """The ORIGINAL OCaml SATySFi binary, if available (via `--satysfi`, then
    PATH — e.g. inside `nix develop`). Used to GENERATE the reference PDFs the
    baseline is anchored to (see flake.nix)."""
    if explicit:
        return explicit if Path(explicit).exists() else None
    return shutil.which("satysfi")


def assemble_satysfi_lib_root(dst: Path, docs: list[Doc]) -> Path:
    """A `-C` config root for the ORIGINAL SATySFi holding only the NON-stdlib
    corpus packages (base + each doc's sibling packages). SATySFi's own standard
    library (stdjabook, math, itemize, ...) comes from its default config path,
    so — unlike the port's lib-root — we do NOT copy lib-rustyfi here."""
    pkg = dst / "dist" / "packages"
    pkg.mkdir(parents=True, exist_ok=True)
    base_src = CORPUS / "satysfi-base" / "src"
    if base_src.exists():
        shutil.copytree(base_src, pkg / "base", dirs_exist_ok=True)
    for doc in docs:
        for prefix, srcdir in doc.stage.items():
            srcpath = CORPUS / srcdir
            if not srcpath.exists():
                continue
            dest = pkg if prefix == "" else pkg / prefix
            shutil.copytree(srcpath, dest, dirs_exist_ok=True)
    return dst


def build_ref_satysfi(doc: Doc, satysfi: str, lib_root: Path, out_pdf: Path, timeout: int) -> None:
    """Generate `doc`'s reference PDF with the ORIGINAL SATySFi. `-C <lib_root>`
    adds the corpus packages to its config search path (its stdlib stays on the
    default path). Built from the doc's own dir so relative `@import:`s and
    CWD-relative `load-pdf-image` targets resolve, exactly as for the port."""
    src = CORPUS / doc.src
    # `-C` takes the config ROOT; SATySFi searches `<root>/dist/packages/`.
    cmd = [satysfi, src.name, "-o", str(out_pdf), "-C", str(lib_root)]
    proc = subprocess.run(cmd, cwd=src.parent, capture_output=True, timeout=timeout, text=True)
    if proc.returncode != 0 or not out_pdf.exists():
        tail = "\n".join((proc.stdout + proc.stderr).splitlines()[-10:])
        raise RuntimeError(f"original SATySFi failed to build {doc.name}:\n{tail}")


def extract_pages(pdf: Path, pdftotext: str) -> list[Page]:
    proc = subprocess.run(
        [pdftotext, "-bbox", str(pdf), "-"], capture_output=True, timeout=120, text=True
    )
    if proc.returncode != 0:
        raise RuntimeError(f"pdftotext failed on {pdf}: {proc.stderr[-400:]}")
    pages: list[Page] = []
    for line in proc.stdout.splitlines():
        pm = PAGE_RE.search(line)
        if pm:
            pages.append(Page(float(pm.group(1)), float(pm.group(2))))
            continue
        wm = WORD_RE.search(line)
        if wm and pages:
            x0, y0, x1, y1 = (float(wm.group(i)) for i in range(1, 5))
            text = html.unescape(wm.group(5)).strip()
            if text:
                pages[-1].words.append(Word(text, x0, y0, x1, y1))
    return pages


# --------------------------------------------------------------------------
# Metrics.
# --------------------------------------------------------------------------


def all_words(pages: list[Page]) -> list[Word]:
    return [w for p in pages for w in p.words]


def count_baselines(baselines: list[list[float]], tol: float) -> int:
    """Total baselines over all pages, clustering rows within `tol` into one.

    Takes the RAW per-page rows (`page_baselines(pdf, 0.0)`) so one read of the
    PDF can be re-clustered at any tolerance — which is what `--tol-sweep` does.
    A row is compared against the last one KEPT, not the last one seen, so a
    dense ladder of near-baselines cannot chain into a single line.
    """
    total = 0
    for rows in baselines:
        keep = None
        for y in rows:
            if keep is None or y - keep > tol:
                total += 1
                keep = y
    return total


def line_count(baselines: list[list[float]]) -> int:
    """Distinct text BASELINES, summed over pages.

    Counted from the PDF content stream's text-positioning operators (`Tm`/`Td`
    composed with the CTM), NOT from `pdftotext` glyph boxes. That is the whole
    point: a glyph box's top and bottom come from the font descriptor and the
    two writers emit different descriptors for identical glyphs, so a
    box-clustered line count is not comparable across engines at all. `BT ..
    Tm/Td .. Tj` places a run at an exact baseline in PDF user space, identically
    for both.

    The old glyph-box count and this one, measured on the same renders:

        doc         glyph-box (tol 3.0)      baselines (tol 0.05)
                    port / up  delta         port / up  delta
        latexcmds    315 / 319   -4           341 / 343   -2
        xpath        281 / 290   -9           292 / 290   +2
        enumitem     869 / 885  -16           882 / 883   -1
        easytable    555 / 592  -37           565 / 565    0
        figbox       541 / 546   -5           590 / 590    0
        slydifi      336 / 337   -1           392 / 393   -1
        gakushin      66 / --                 156 / --        (self-snapshot)

    easytable's headline "37 lines short" was ENTIRELY the artifact: the two
    engines set the same 565 lines. The old column also moved wildly with the
    clustering tolerance (easytable swung -180 -> -36 -> -10 -> +2 between tol
    1.0 and 8.0) whereas this one is flat from 0.02 to 0.5 on every document,
    because runs of one line share a baseline exactly and consecutive lines are
    ~19pt apart. Re-check with `--tol-sweep` — and note what it shows at tol
    1.0 and above, where slydifi's stable -1 becomes -8 and then +9: that is
    the merging artifact reappearing, and the old metric's 3.0 lived in it.

    The absolute counts are HIGHER than the glyph-box ones on the two documents
    with embedded PDF pages (figbox 541 -> 590, gakushin 66 -> 156) because
    `runs_of` follows `Do` into Form XObjects. Both engines gain equally on
    figbox; gakushin's jump is its 学振 form template, whose fonts poppler
    largely cannot decode to text at all.

    What this DOES count that a "line of text" arguably is not: a math
    sub/superscript sits on its own baseline, as does a run inside a rotated
    graphic. Both engines pay that equally, and the residual deltas above are
    a couple of baselines, so it is left in rather than heuristically filtered.
    """
    return count_baselines(baselines, BASELINE_TOL)


def char_stream(pages: list[Page]) -> str:
    """Every word's text, concatenated in reading order with NO separators.

    Dropping the separators is what makes content comparison immune to
    confound 2: `pdftotext` decides where one word ends and the next begins from
    the size of the GAP between glyphs, so justification alone moves those
    boundaries. Concatenating deletes the boundaries, leaving the character
    sequence the engine actually typeset — which is what we wanted to compare in
    the first place.
    """
    return "".join(w.text for p in pages for w in p.words)


def content_metrics(port: list[Page], ref: list[Page]) -> tuple[int, int, float, int, int]:
    """(chars, upstream_chars, char_match, chars_missing, chars_extra).

    `chars_missing` / `chars_extra` are a whole-document character MULTISET
    difference, so they are blind to ordering and to page placement and answer
    exactly one question: did any content fail to appear. `char_match` is a
    difflib ratio over the same two character streams IN READING ORDER, so it
    additionally falls when content merely moved.

    Both directions are reported because they are not symmetric in meaning.
    `chars_extra` is often the port being BETTER: upstream's math superscripts
    frequently carry no usable `ToUnicode`, so `𝐸=𝑚𝑐²`'s exponent extracts as
    nothing from the reference and as `2` from the port — figbox's entire
    +6-character surplus is six such exponents.
    """
    ps, rs = char_stream(port), char_stream(ref)
    cp, cr = Counter(ps), Counter(rs)
    missing = sum((cr - cp).values())
    extra = sum((cp - cr).values())
    match = SequenceMatcher(None, rs, ps, autojunk=False).ratio()
    return len(ps), len(rs), match, missing, extra


def left_margin(pages: list[Page]) -> float:
    """Median over pages of that page's minimum word xMin — the text block's
    left edge, robust to a stray centered/indented word."""
    mins = [min((w.x0 for w in p.words), default=0.0) for p in pages if p.words]
    return round(statistics.median(mins), 2) if mins else 0.0


@dataclass
class Metrics:
    pages: int
    words: int
    lines: int
    chars: int
    left_margin: float
    # All None in self-snapshot mode (no upstream reference to compare to).
    text_match: float | None  # informational; see the module docstring
    char_match: float | None  # vs reference, GATED
    width_p95_pt: float | None  # vs reference
    chars_missing: int | None = None
    chars_extra: int | None = None
    # The REFERENCE's own counts. The count checks measure the port's DEVIATION
    # FROM UPSTREAM (see `check_against_baseline`), so they have to be recorded.
    ref_pages: int | None = None
    ref_lines: int | None = None
    ref_words: int | None = None
    ref_chars: int | None = None

    def to_json(self) -> dict:
        d = {
            "pages": self.pages,
            "words": self.words,
            "lines": self.lines,
            "chars": self.chars,
            "left_margin": self.left_margin,
        }
        if self.text_match is not None:
            d["text_match"] = round(self.text_match, 4)
        if self.char_match is not None:
            d["char_match"] = round(self.char_match, 4)
        if self.width_p95_pt is not None:
            d["width_p95_pt"] = round(self.width_p95_pt, 3)
        if self.ref_words is not None:
            d["upstream_words"] = self.ref_words
        if self.ref_chars is not None:
            d["upstream_chars"] = self.ref_chars
        if self.chars_missing is not None:
            d["chars_missing"] = self.chars_missing
            d["chars_extra"] = self.chars_extra
        # Deviation from upstream, not an absolute count: what the guard pins.
        if self.ref_lines is not None:
            d["upstream_lines"] = self.ref_lines
            d["lines_dev"] = abs(self.lines - self.ref_lines)
        return d


def compare(
    port: list[Page],
    ref: list[Page] | None,
    port_baselines: list[list[float]],
    ref_baselines: list[list[float]] | None,
) -> Metrics:
    """Port-side layout metrics; against `ref` if given (vs-upstream), else
    just the self-snapshot counts (the comparison metrics left None).

    Two extractions per PDF feed this, deliberately: GEOMETRY comes from the
    content stream (`*_baselines`, immune to the font-descriptor trap) and TEXT
    comes from `pdftotext` (which is what decodes `ToUnicode`), compared as
    characters so its word splitting cannot leak in.
    """
    port_words = all_words(port)
    text_match: float | None = None
    char_match: float | None = None
    width_p95: float | None = None
    chars_missing: int | None = None
    chars_extra: int | None = None
    ref_chars: int | None = None
    chars = len(char_stream(port))

    if ref is not None:
        ref_words = all_words(ref)
        ref_text = [w.text for w in ref_words]
        port_text = [w.text for w in port_words]
        sm = SequenceMatcher(None, ref_text, port_text, autojunk=False)
        text_match = sm.ratio()
        # Word-width deltas over aligned (equal-text) words.
        width_deltas: list[float] = []
        for a, b, size in sm.get_matching_blocks():
            for k in range(size):
                width_deltas.append(abs(ref_words[a + k].width - port_words[b + k].width))
        if width_deltas:
            width_deltas.sort()
            idx = min(len(width_deltas) - 1, int(round(0.95 * (len(width_deltas) - 1))))
            width_p95 = width_deltas[idx]
        else:
            width_p95 = 0.0
        chars, ref_chars, char_match, chars_missing, chars_extra = content_metrics(port, ref)

    return Metrics(
        pages=len(port),
        words=len(port_words),
        lines=line_count(port_baselines),
        chars=chars,
        left_margin=left_margin(port),
        text_match=text_match,
        char_match=char_match,
        width_p95_pt=width_p95,
        chars_missing=chars_missing,
        chars_extra=chars_extra,
        ref_pages=None if ref is None else len(ref),
        ref_lines=None if ref_baselines is None else line_count(ref_baselines),
        ref_words=None if ref is None else len(all_words(ref)),
        ref_chars=ref_chars,
    )


# --------------------------------------------------------------------------
# Baseline comparison.
# --------------------------------------------------------------------------

# A metric may drift by this fraction (for counts) before it is a regression;
# char_match may fall by at most CHAR_MATCH_SLACK below its baseline.
COUNT_SLACK = 0.06          # ±6% on page / line / word / char counts
WIDTH_SLACK_PT = 0.5        # width_p95 may exceed baseline by this many pt
CHAR_MATCH_SLACK = 0.005    # char_match may fall this far below baseline


def check_against_baseline(name: str, m: Metrics, base: dict) -> list[str]:
    """Return a list of regression messages (empty => within tolerance).

    `text_match` and `words` are DELIBERATELY not checked here. Both move on a
    pure re-tokenization (module docstring, confound 2), so gating them gates
    poppler's word splitter: closing the CJK inter-character glue gap would
    RAISE text_match by re-splitting upstream's runs the port's way without one
    glyph moving, and a change that opened gaps the other way would fail this
    gate having improved the layout. `char_match` measures the same
    content-and-ordering property over characters and is gated in their place;
    `chars_missing`/`chars_extra` measure content alone.
    """
    fails = []
    if m.char_match is not None and "char_match" in base:
        if m.char_match + CHAR_MATCH_SLACK < base["char_match"]:
            fails.append(
                f"char_match {m.char_match:.4f} < baseline {base['char_match']:.4f} - {CHAR_MATCH_SLACK}"
            )
    for key in ("chars_missing", "chars_extra"):
        got = getattr(m, key)
        if got is not None and key in base and got > base[key]:
            fails.append(
                f"{key} GREW: {got} > baseline {base[key]} — the port "
                f"{'dropped' if key == 'chars_missing' else 'invented'} content"
            )
    if m.width_p95_pt is not None and "width_p95_pt" in base:
        if m.width_p95_pt > base["width_p95_pt"] + WIDTH_SLACK_PT:
            fails.append(
                f"width_p95_pt {m.width_p95_pt:.3f} > baseline {base['width_p95_pt']:.3f} + {WIDTH_SLACK_PT}"
            )
    # Counts are checked as a DEVIATION FROM UPSTREAM that may shrink but never
    # grow — the same convergence guard `page_gap` already applies to pages.
    #
    # They used to be compared against the PORT'S OWN recorded counts ±6%, which
    # measures drift from our past rather than fidelity, and actively misleads:
    # latexcmds sat at 1096 words against upstream's 1095 — as close as it has
    # ever been — and was still reported as a regression because the pinned
    # baseline said 1029. A document that MOVES TOWARD SATySFi must never fail.
    for key, dev_key in (("lines", "lines_dev"),):
        ref = getattr(m, f"ref_{key}")
        if ref is not None and dev_key in base:
            dev = abs(m.__dict__[key] - ref)
            if dev > base[dev_key]:
                fails.append(
                    f"{key} deviation from SATySFi WIDENED: |port {m.__dict__[key]} - "
                    f"SATySFi {ref}| = {dev} > baseline {base[dev_key]}"
                )
    # Self-snapshot mode (no upstream reference at all): the port's own history
    # is all there is, so those docs keep a ±6% drift guard on every count.
    if m.ref_pages is None:
        for key in ("lines", "words", "chars"):
            if key not in base:
                continue
            b = base[key]
            if not (b * (1 - COUNT_SLACK) <= m.__dict__[key] <= b * (1 + COUNT_SLACK)):
                fails.append(
                    f"{key} {m.__dict__[key]} outside baseline {b} ±{int(COUNT_SLACK * 100)}%"
                )
    # Pages keep the ±6% self-snapshot guard too; against upstream they are
    # governed by the stricter `page_gap` parity check below.
    if m.ref_pages is None and "pages" in base:
        b = base["pages"]
        if not (b * (1 - COUNT_SLACK) <= m.pages <= b * (1 + COUNT_SLACK)):
            fails.append(f"pages {m.pages} outside baseline {b} ±{int(COUNT_SLACK * 100)}%")
    # Page-count PARITY with the original SATySFi (the goal "the corpus test
    # matches in page count"): the port's absolute page-count gap to SATySFi
    # must not GROW beyond its recorded value — it may only shrink toward 0.
    # This is the enforced convergence guard; `--update` re-pins it, so tightening
    # the gap (e.g. a spacing fix) is locked in and can never silently regress.
    if "upstream_pages" in base and "page_gap" in base:
        gap = abs(m.pages - base["upstream_pages"])
        if gap > base["page_gap"]:
            fails.append(
                f"page-count gap to SATySFi WIDENED: |port {m.pages} - SATySFi "
                f"{base['upstream_pages']}| = {gap} > baseline gap {base['page_gap']}"
            )
    return fails


def fmt_report(name: str, covers: str, ref_pages: int | None, m: Metrics) -> str:
    head = f"  {name:<10} [{covers}]\n"
    if m.char_match is not None:
        drift = m.pages - (ref_pages or 0)
        sign = "+" if drift >= 0 else ""
        ldev = m.lines - (m.ref_lines or 0)
        return (
            head
            + f"    char_match={m.char_match:.4f}  chars: port={m.chars} upstream={m.ref_chars} "
            f"(missing={m.chars_missing} extra={m.chars_extra})\n"
            f"    pages: port={m.pages} upstream={ref_pages} ({sign}{drift})   "
            f"lines: port={m.lines} upstream={m.ref_lines} ({ldev:+d})\n"
            f"    width_p95={m.width_p95_pt:.3f}pt  left_margin={m.left_margin}  "
            f"[info: text_match={m.text_match:.4f}  words={m.words}/{m.ref_words}]"
        )
    # Self-snapshot (no upstream reference).
    return (
        head + f"    [self-snapshot — no upstream reference]  left_margin={m.left_margin}\n"
        f"    pages: port={m.pages}   lines: port={m.lines}   chars: port={m.chars}   "
        f"words: port={m.words}"
    )


# --------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--doc", action="append", default=[], help="only this doc (repeatable)")
    ap.add_argument("--update", action="store_true", help="rewrite the baseline from current results")
    ap.add_argument("--bin", type=Path, default=default_bin(), help="path to the rustyfi binary")
    ap.add_argument("--keep-going", action="store_true", help="report all docs even if one fails")
    ap.add_argument("--timeout", type=int, default=600, help="per-doc build timeout (s)")
    ap.add_argument(
        "--gen-refs",
        action="store_true",
        help="generate the reference PDFs with the ORIGINAL SATySFi (needs `satysfi` on "
        "PATH — see flake.nix / `nix develop`) instead of using the committed submodule "
        "PDFs. The two agree on SATySFi 0.0.11, but this makes the reference provenance "
        "explicit and reproducible.",
    )
    ap.add_argument("--satysfi", default=None, help="path to the original SATySFi binary (for --gen-refs)")
    ap.add_argument(
        "--out-dir",
        type=Path,
        default=None,
        help="write every doc's PDFs into this ONE directory instead of each doc's own "
        "corpus directory. By default each pair lands beside the package it came from, "
        "in scripts/layout_fidelity_corpus/<doc>/ — <doc>.port.pdf (the port's render) "
        "and, when a reference exists, <doc>.satysfi.pdf (the reference it was compared "
        "against). Both suffixes are gitignored.",
    )
    ap.add_argument(
        "--no-persist",
        action="store_true",
        help="build in a temp dir and discard, writing no PDFs anywhere.",
    )
    ap.add_argument(
        "--tol-sweep",
        action="store_true",
        help="also print the port-minus-upstream BASELINE-count delta at a range of "
        "clustering tolerances. A real line-count gap is tolerance-stable; a clustering "
        "artifact collapses as the tolerance grows. This is the evidence behind `lines` "
        "being trustworthy, re-measurable on demand rather than quoted from a docstring.",
    )
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    missing = prereqs_ok(args.bin)
    if missing:
        print("SKIP layout-fidelity — prerequisites missing:")
        for msg in missing:
            print(f"  - {msg}")
        return 0

    pdftotext = find_pdftotext()
    docs = [d for d in DOCS if not args.doc or d.name in args.doc]
    if not docs:
        print(f"no matching docs among {[d.name for d in DOCS]}")
        return 2

    satysfi = None
    if args.gen_refs:
        satysfi = find_satysfi(args.satysfi)
        if satysfi is None:
            print("SKIP layout-fidelity — --gen-refs given but the original `satysfi` is not "
                  "available (enter `nix develop`, see flake.nix).")
            return 0

    baseline = {}
    if BASELINE_PATH.exists():
        baseline = json.loads(BASELINE_PATH.read_text())

    results: dict[str, dict] = {}
    all_fails: list[str] = []
    ref_kind = "original SATySFi (freshly generated)" if satysfi else "upstream SATySFi (submodule PDFs)"
    print(f"== layout fidelity: Rust port vs {ref_kind} ({len(docs)} docs) ==")

    with tempfile.TemporaryDirectory(prefix="rustyfi-layout-") as tmp:
        tmpd = Path(tmp)
        lib_root = assemble_lib_root(tmpd / "libroot", docs)
        saty_root = assemble_satysfi_lib_root(tmpd / "satyroot", docs) if satysfi else None
        for doc in docs:
            self_mode = doc.ref == ""
            # Reference PDF source: freshly generated by the original SATySFi
            # (--gen-refs), else the committed submodule PDF. gakushin has no
            # reference either way (its fonts-junicode dep needs Satyrographos),
            # so it stays a self-snapshot.
            ref_pdf = None
            if not self_mode:
                if satysfi:
                    ref_pdf = tmpd / f"ref-{doc.name}.pdf"
                    try:
                        build_ref_satysfi(doc, satysfi, saty_root, ref_pdf, args.timeout)
                    except Exception as e:
                        msg = f"{doc.name}: ERROR generating reference with original SATySFi — {e}"
                        print("  " + msg)
                        all_fails.append(msg)
                        if args.keep_going:
                            continue
                        return 1
                else:
                    ref_pdf = CORPUS / doc.ref
                    if not ref_pdf.exists():
                        print(f"  {doc.name}: SKIP — no upstream reference at {ref_pdf}")
                        continue
            out_pdf = tmpd / f"{doc.name}.pdf"
            try:
                build_pdf(doc, args.bin, lib_root, out_pdf, args.timeout)
                port_pages = extract_pages(out_pdf, pdftotext)
                ref_pages = None if ref_pdf is None else extract_pages(ref_pdf, pdftotext)
                # Geometry, from the content stream — a SECOND read of the same
                # two PDFs, because `lines` must not touch a glyph box. Read RAW
                # (tol=0.0) so `--tol-sweep` can re-cluster without re-reading.
                port_bl = page_baselines(out_pdf, 0.0)
                ref_bl = None if ref_pdf is None else page_baselines(ref_pdf, 0.0)
            except Exception as e:
                msg = f"{doc.name}: ERROR — {e}"
                print("  " + msg)
                all_fails.append(msg)
                if args.keep_going:
                    continue
                return 1

            # Persist both renders next to the package they came from, so the
            # pair a failure refers to is sitting where you would look for it
            # rather than in a temp dir that is already gone. `--out-dir`
            # collects every doc into one directory instead; `--no-persist`
            # keeps the old discard-everything behaviour.
            #
            # Both suffixes are gitignored, and deliberately so: `.satysfi.pdf`
            # is a COPY of the reference (which lives at doc.ref under its own
            # upstream name, e.g. latexcmds/doc/latexcmds-doc.pdf, and stays
            # tracked). Copying it under the predictable name is what makes the
            # pair directly diffable — `<doc>.port.pdf` vs `<doc>.satysfi.pdf`.
            if not args.no_persist:
                dest = args.out_dir if args.out_dir else (CORPUS / doc.name)
                dest.mkdir(parents=True, exist_ok=True)
                shutil.copy2(out_pdf, dest / f"{doc.name}.port.pdf")
                if ref_pdf is not None:
                    shutil.copy2(ref_pdf, dest / f"{doc.name}.satysfi.pdf")

            m = compare(port_pages, ref_pages, port_bl, ref_bl)
            entry = m.to_json() | {"covers": doc.covers}
            if ref_pages is not None:
                entry["upstream_pages"] = len(ref_pages)
                # The port's current page-count gap to SATySFi (0 = exact
                # match). Guarded against widening by check_against_baseline.
                entry["page_gap"] = abs(m.pages - len(ref_pages))
            results[doc.name] = entry
            print(fmt_report(doc.name, doc.covers, len(ref_pages) if ref_pages is not None else None, m))
            if args.tol_sweep and ref_bl is not None:
                print(
                    "    tol-sweep (port-upstream baselines): "
                    + "  ".join(
                        f"{t}:{count_baselines(port_bl, t) - count_baselines(ref_bl, t):+d}"
                        for t in SWEEP_TOLS
                    )
                )

            if not args.update and doc.name in baseline:
                fails = check_against_baseline(doc.name, m, baseline[doc.name])
                for f in fails:
                    line = f"{doc.name}: REGRESSION — {f}"
                    print("    !! " + line)
                    all_fails.append(line)
            elif not args.update:
                print(f"    (no baseline for {doc.name} — run with --update to record one)")

    if args.update:
        BASELINE_PATH.write_text(json.dumps(results, indent=2, ensure_ascii=False) + "\n")
        print(f"\nwrote baseline for {len(results)} docs to {BASELINE_PATH}")
        return 0

    if all_fails:
        print(f"\nFAIL — {len(all_fails)} layout regression(s):")
        for f in all_fails:
            print(f"  - {f}")
        return 1
    print(f"\nOK — all {len(results)} docs within layout-fidelity baseline.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
