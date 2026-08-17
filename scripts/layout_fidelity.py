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

Metrics per document (all font-robust, since fonts are identical):

  text_match       reading-order word-sequence similarity, port vs ref
                   (difflib ratio). ~1.0 means the same text in the same order;
                   a drop flags missing / extra / garbled / reordered content.
  width_p95_pt     95th-percentile |word-width delta| over aligned words, in pt.
                   Should be ~0 because metrics match; a large value means the
                   port set a run in the wrong font / size / with wrong tracking.
  left_margin_*    the text block's left edge (median of per-page min xMin).
  pages_*          page count. The headline PAGINATION-divergence signal.
  lines_*          total text-line count (words grouped by baseline).

The baseline (`layout_fidelity_baseline.json`) pins each metric at its current
value with headroom, so this PASSES today and FAILS on a regression. `text_match`
and `width_p95_pt` are the strong fidelity floors we assert hard; pagination and
line counts are the known-divergent metrics we merely guard against getting worse
(and report loudly). Re-baseline with `--update` after an intentional change.

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
from dataclasses import dataclass, field
from difflib import SequenceMatcher
from pathlib import Path

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

# Words shorter than a baseline are grouped into one text line when their yMin
# is within this many points of each other (baseline clustering tolerance).
LINE_Y_TOL = 3.0


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
        missing.append(f"port binary not built at {bin_path} (run `cargo build -p rustyfi-cli`)")
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


def line_count(pages: list[Page]) -> int:
    total = 0
    for p in pages:
        ys = sorted(w.y0 for w in p.words)
        prev = None
        for y in ys:
            if prev is None or (y - prev) > LINE_Y_TOL:
                total += 1
            prev = y
    return total


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
    left_margin: float
    # All None in self-snapshot mode (no upstream reference to compare to).
    text_match: float | None  # vs reference; 1.0 for the reference against itself
    width_p95_pt: float | None  # vs reference
    # The REFERENCE's own counts. The count checks measure the port's DEVIATION
    # FROM UPSTREAM (see `check_against_baseline`), so they have to be recorded.
    ref_pages: int | None = None
    ref_lines: int | None = None
    ref_words: int | None = None

    def to_json(self) -> dict:
        d = {
            "pages": self.pages,
            "words": self.words,
            "lines": self.lines,
            "left_margin": self.left_margin,
        }
        if self.text_match is not None:
            d["text_match"] = round(self.text_match, 4)
        if self.width_p95_pt is not None:
            d["width_p95_pt"] = round(self.width_p95_pt, 3)
        # Deviation from upstream, not an absolute count: what the guard pins.
        if self.ref_words is not None:
            d["upstream_words"] = self.ref_words
            d["words_dev"] = abs(self.words - self.ref_words)
        if self.ref_lines is not None:
            d["upstream_lines"] = self.ref_lines
            d["lines_dev"] = abs(self.lines - self.ref_lines)
        return d


def compare(port: list[Page], ref: list[Page] | None) -> Metrics:
    """Port-side layout metrics; against `ref` if given (vs-upstream), else
    just the self-snapshot counts (text_match / width_p95 left None)."""
    port_words = all_words(port)
    text_match: float | None = None
    width_p95: float | None = None

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

    return Metrics(
        pages=len(port),
        words=len(port_words),
        lines=line_count(port),
        left_margin=left_margin(port),
        text_match=text_match,
        width_p95_pt=width_p95,
        ref_pages=None if ref is None else len(ref),
        ref_lines=None if ref is None else line_count(ref),
        ref_words=None if ref is None else len(all_words(ref)),
    )


# --------------------------------------------------------------------------
# Baseline comparison.
# --------------------------------------------------------------------------

# A metric may drift by this fraction (for counts) before it is a regression;
# text_match may fall by at most TEXT_MATCH_SLACK below its baseline.
COUNT_SLACK = 0.06          # ±6% on page / line / word counts
WIDTH_SLACK_PT = 0.5        # width_p95 may exceed baseline by this many pt
TEXT_MATCH_SLACK = 0.02     # text_match may fall this far below baseline


def check_against_baseline(name: str, m: Metrics, base: dict) -> list[str]:
    """Return a list of regression messages (empty => within tolerance)."""
    fails = []
    if m.text_match is not None and "text_match" in base:
        if m.text_match + TEXT_MATCH_SLACK < base["text_match"]:
            fails.append(
                f"text_match {m.text_match:.4f} < baseline {base['text_match']:.4f} - {TEXT_MATCH_SLACK}"
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
    for key, dev_key, up_key in (
        ("lines", "lines_dev", "upstream_lines"),
        ("words", "words_dev", "upstream_words"),
    ):
        ref = getattr(m, f"ref_{key}")
        if ref is not None and dev_key in base:
            dev = abs(m.__dict__[key] - ref)
            if dev > base[dev_key]:
                fails.append(
                    f"{key} deviation from SATySFi WIDENED: |port {m.__dict__[key]} - "
                    f"SATySFi {ref}| = {dev} > baseline {base[dev_key]}"
                )
        elif ref is None and key in base:
            # Self-snapshot (no upstream reference): the port's own history is
            # all there is, so keep the ±6% drift guard for it.
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
    if m.text_match is not None:
        drift = m.pages - (ref_pages or 0)
        sign = "+" if drift >= 0 else ""
        return (
            head
            + f"    text_match={m.text_match:.4f}  width_p95={m.width_p95_pt:.3f}pt  "
            f"left_margin={m.left_margin}\n"
            f"    pages: port={m.pages} upstream={ref_pages} ({sign}{drift})   "
            f"lines: port={m.lines}   words: port={m.words}"
        )
    # Self-snapshot (no upstream reference).
    return (
        head + f"    [self-snapshot — no upstream reference]  left_margin={m.left_margin}\n"
        f"    pages: port={m.pages}   lines: port={m.lines}   words: port={m.words}"
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

            m = compare(port_pages, ref_pages)
            entry = m.to_json() | {"covers": doc.covers}
            if ref_pages is not None:
                entry["upstream_pages"] = len(ref_pages)
                # The port's current page-count gap to SATySFi (0 = exact
                # match). Guarded against widening by check_against_baseline.
                entry["page_gap"] = abs(m.pages - len(ref_pages))
            results[doc.name] = entry
            print(fmt_report(doc.name, doc.covers, len(ref_pages) if ref_pages is not None else None, m))

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
