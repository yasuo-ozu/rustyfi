#!/usr/bin/env python3
"""Line-BREAK and intra-line SPACING agreement between the port and upstream.

`layout_fidelity.py` is the regression gate; two of its numbers are the wrong
instrument for a line-breaker change:

- `text_match` compares the flat WORD sequence, which is insensitive to WHERE
  lines break — the only thing a breaker change moves;
- `lines` is largely a `pdftotext` clustering artifact (see that script's own
  `line_count` docstring: the port-minus-upstream delta for easytable swings
  from -180 to +2 as the tolerance goes 1.0 -> 8.0).

This measures the two quantities a line-breaker or inter-character-spacing
change actually moves, both against the committed reference PDFs:

  match     difflib ratio over the sequence of per-baseline LINE TEXTS. 1.0
            means every break landed where upstream put it. Reported at two
            clustering tolerances, because a figure that only holds at one
            tolerance is an artifact, not a layout fact.
  dx_mean   mean |x offset delta| over every character of every line whose TEXT
            is identical in both engines. Offsets are measured from each line's
            OWN first character, so this is engine-relative: no cross-engine
            font-descriptor comparison is involved (the measurement trap in
            `layout_probe.py`'s docstring) and the page margin drops out. This
            is the number inter-CJK glue moves.

Run `layout_fidelity.py` first — it leaves `<doc>.port.pdf` and
`<doc>.satysfi.pdf` side by side in each corpus directory, which is what this
reads. To compare two builds, give `layout_fidelity.py --out-dir DIR` for the
first one and pass DIR here as `--port-dir`.

    layout-tests/fidelity.py --bin target/debug/rustyfi
    layout-tests/tools/linebreak.py
    layout-tests/tools/linebreak.py --doc figbox --worst 10
"""

from __future__ import annotations

import argparse
import difflib
import html
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CORPUS = REPO / "layout-tests" / "corpus"
DOCS = ["latexcmds", "xpath", "enumitem", "easytable", "figbox", "slydifi"]

PAGE_RE = re.compile(r'<page width="([\d.]+)" height="([\d.]+)"')
WORD_RE = re.compile(
    r'<word xMin="([\d.]+)" yMin="([\d.]+)" xMax="([\d.]+)" yMax="([\d.]+)">(.*?)</word>'
)


def raw_words(pdf: Path) -> list[list[tuple[float, float, float, str]]]:
    """Per page, `(yMin, xMin, xMax, text)` for every word box."""
    proc = subprocess.run(
        ["pdftotext", "-bbox", str(pdf), "-"], capture_output=True, timeout=900, text=True
    )
    pages: list[list[tuple[float, float, float, str]]] = []
    for line in proc.stdout.splitlines():
        if PAGE_RE.search(line):
            pages.append([])
            continue
        m = WORD_RE.search(line)
        if m and pages:
            x0, y0, x1, _y1 = (float(m.group(i)) for i in range(1, 5))
            pages[-1].append((y0, x0, x1, html.unescape(m.group(5))))
    return pages


def cluster(pages, tol: float):
    """Group each page's words into baselines, ascending. Mirrors
    `layout_fidelity.line_count`'s clustering so the counts here agree with the
    gate's."""
    for words in pages:
        cur: list[tuple[float, float, str]] = []
        prev_y = None
        for y0, x0, x1, txt in sorted(words, key=lambda w: (w[0], w[1])):
            if prev_y is not None and (y0 - prev_y) > tol:
                yield sorted(cur)
                cur = []
            cur.append((x0, x1, txt))
            prev_y = y0
        if cur:
            yield sorted(cur)


def line_texts(pages, tol: float) -> list[str]:
    return ["".join(t for _, _, t in ln) for ln in cluster(pages, tol)]


def line_chars(pages, tol: float) -> list[tuple[str, list[float]]]:
    """`(text, x of each character)`, the word advance shared evenly over its
    characters — enough resolution to see a half-em kern or a stretched glue."""
    out = []
    for ln in cluster(pages, tol):
        text: list[str] = []
        xs: list[float] = []
        for x0, x1, txt in ln:
            if not txt:
                continue
            step = (x1 - x0) / len(txt)
            for k, ch in enumerate(txt):
                text.append(ch)
                xs.append(x0 + k * step)
        if text:
            out.append(("".join(text), xs))
    return out


def dx_stats(port, ref, tol: float):
    """Mean/p95 |dx| over characters of text-identical lines, plus the per-line
    means so `--worst` can name the offenders."""
    p = line_chars(port, tol)
    r = line_chars(ref, tol)
    sm = difflib.SequenceMatcher(a=[t for t, _ in p], b=[t for t, _ in r], autojunk=False)
    deltas: list[float] = []
    per_line: list[tuple[float, str]] = []
    for blk in sm.get_matching_blocks():
        for k in range(blk.size):
            txt, pxs = p[blk.a + k]
            _, rxs = r[blk.b + k]
            if len(pxs) != len(rxs) or len(pxs) < 2:
                continue
            p0, r0 = pxs[0], rxs[0]
            ds = [abs((a - p0) - (b - r0)) for a, b in zip(pxs[1:], rxs[1:])]
            deltas.extend(ds)
            per_line.append((sum(ds) / len(ds), txt))
    deltas.sort()
    if not deltas:
        return 0.0, 0.0, 0, per_line
    p95 = deltas[min(len(deltas) - 1, int(len(deltas) * 0.95))]
    return sum(deltas) / len(deltas), p95, len(deltas), per_line


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--doc", action="append", default=[], help="only this doc (repeatable)")
    ap.add_argument(
        "--port-dir",
        type=Path,
        default=None,
        help="read <doc>.port.pdf from this ONE directory (layout_fidelity.py "
        "--out-dir) instead of each doc's own corpus directory",
    )
    ap.add_argument("--tol", type=float, action="append", default=[])
    ap.add_argument("--worst", type=int, default=0, help="name the N worst-spaced lines")
    args = ap.parse_args()

    if not shutil.which("pdftotext"):
        print("SKIP linebreak_probe — pdftotext (poppler) not found")
        return 0
    tols = args.tol or [3.0, 6.0]
    docs = [d for d in DOCS if not args.doc or d in args.doc]

    for doc in docs:
        port = (args.port_dir / f"{doc}.port.pdf") if args.port_dir else (
            CORPUS / doc / f"{doc}.port.pdf"
        )
        ref = CORPUS / doc / f"{doc}.satysfi.pdf"
        if not port.exists() or not ref.exists():
            print(f"{doc:<10} SKIP — run layout_fidelity.py first (missing {port.name})")
            continue
        pw, rw = raw_words(port), raw_words(ref)
        bits = []
        for tol in tols:
            pt, rt = line_texts(pw, tol), line_texts(rw, tol)
            sm = difflib.SequenceMatcher(a=pt, b=rt, autojunk=False)
            exact = sum(b.size for b in sm.get_matching_blocks())
            bits.append(
                f"tol{tol:g}: match={sm.ratio():.4f} exact={exact} "
                f"port={len(pt)} ref={len(rt)}"
            )
        mean, p95, n, per_line = dx_stats(pw, rw, tols[-1])
        print(
            f"{doc:<10} {'  |  '.join(bits)}  |  dx_mean={mean:.3f}pt "
            f"dx_p95={p95:.3f}pt over {n} chars"
        )
        for d, txt in sorted(per_line, reverse=True)[: args.worst]:
            print(f"             {d:7.3f}pt  {txt[:78]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
