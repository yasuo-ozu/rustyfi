#!/usr/bin/env python3
"""Vertical-divergence scanner: port PDF vs upstream SATySFi PDF.

Aligns the two documents' word sequences (reading order, whole document) with
difflib, then reports where the port's vertical position drifts from upstream's.

MEASUREMENT TRAP (see CLAUDE.md / the task notes): the two PDF writers emit
DIFFERENT font descriptors for the same LATIN glyphs, so a latin word's `yMin`
can differ by several points with identical baselines. CJK descriptors agree
(19.36 vs 19.34), so `--cjk-only` restricts every dy to CJK-bearing words,
which is the trustworthy signal.

Usage:
  dyscan.py PORT.pdf REF.pdf [--cjk-only] [--per-page] [--first-diverge]
"""

from __future__ import annotations

import argparse
import html
import re
import statistics
import subprocess
import sys
from dataclasses import dataclass, field
from difflib import SequenceMatcher
from pathlib import Path

PAGE_RE = re.compile(r'<page width="([\d.]+)" height="([\d.]+)"')
WORD_RE = re.compile(
    r'<word xMin="([\d.]+)" yMin="([\d.]+)" xMax="([\d.]+)" yMax="([\d.]+)">(.*?)</word>'
)


def is_cjk(s: str) -> bool:
    return any(
        "぀" <= c <= "ヿ" or "一" <= c <= "鿿" or "＀" <= c <= "￯"
        for c in s
    )


@dataclass
class Word:
    text: str
    x0: float
    y0: float
    x1: float
    y1: float
    page: int


def extract(pdf: Path) -> list[Word]:
    proc = subprocess.run(
        ["pdftotext", "-bbox", str(pdf), "-"], capture_output=True, timeout=180, text=True
    )
    if proc.returncode != 0:
        raise SystemExit(f"pdftotext failed on {pdf}: {proc.stderr[-400:]}")
    out: list[Word] = []
    page = -1
    for line in proc.stdout.splitlines():
        if PAGE_RE.search(line):
            page += 1
            continue
        wm = WORD_RE.search(line)
        if wm:
            x0, y0, x1, y1 = (float(wm.group(i)) for i in range(1, 5))
            text = html.unescape(wm.group(5)).strip()
            if text:
                out.append(Word(text, x0, y0, x1, y1, page))
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("port", type=Path)
    ap.add_argument("ref", type=Path)
    ap.add_argument("--cjk-only", action="store_true")
    ap.add_argument("--per-page", action="store_true")
    ap.add_argument("--pairs", type=int, default=0, help="dump the first N aligned pairs")
    ap.add_argument("--page", type=int, default=None, help="dump every aligned pair on this port page")
    ap.add_argument("--gaps", type=float, default=None, help="report local advance deltas >= this many pt")
    ap.add_argument("--cumulative", type=int, default=0, help="print ~N samples of the running global dy")
    ap.add_argument("--page-height", type=float, default=842.0)
    args = ap.parse_args()

    pw = extract(args.port)
    rw = extract(args.ref)
    print(f"port words={len(pw)}  ref words={len(rw)}")

    sm = SequenceMatcher(None, [w.text for w in rw], [w.text for w in pw], autojunk=False)
    pairs: list[tuple[Word, Word]] = []
    for a, b, size in sm.get_matching_blocks():
        for k in range(size):
            pairs.append((pw[b + k], rw[a + k]))
    if args.cjk_only:
        pairs = [(p, r) for p, r in pairs if is_cjk(p.text)]
    print(f"aligned pairs={len(pairs)}")

    dys = [p.y0 - r.y0 for p, r in pairs if p.page == r.page]
    same_page = len(dys)
    print(f"same-page pairs={same_page}  median dy={statistics.median(dys):.2f}pt" if dys else "no same-page pairs")

    if args.per_page:
        by_page: dict[int, list[float]] = {}
        cross: dict[int, int] = {}
        for p, r in pairs:
            if p.page == r.page:
                by_page.setdefault(p.page, []).append(p.y0 - r.y0)
            else:
                cross[p.page] = cross.get(p.page, 0) + 1
        print("\npage  n   med_dy   p10     p90    cross-page")
        for pg in sorted(set(list(by_page) + list(cross))):
            v = by_page.get(pg, [])
            if v:
                v2 = sorted(v)
                p10 = v2[int(0.1 * (len(v2) - 1))]
                p90 = v2[int(0.9 * (len(v2) - 1))]
                print(f"{pg:>4} {len(v):>4} {statistics.median(v):>7.2f} {p10:>7.2f} {p90:>7.2f}   {cross.get(pg,0)}")
            else:
                print(f"{pg:>4}    0       -       -       -   {cross.get(pg,0)}")

    if args.page is not None:
        print(f"\n--- aligned pairs on port page {args.page} ---")
        print("  port_y   ref_y      dy   page(p/r)  text")
        for p, r in pairs:
            if p.page == args.page:
                print(f"{p.y0:>8.2f} {r.y0:>8.2f} {p.y0-r.y0:>7.2f}   {p.page}/{r.page}   {p.text[:40]}")

    if args.pairs:
        print("\n--- first pairs ---")
        for p, r in pairs[: args.pairs]:
            print(f"p{p.page} y={p.y0:8.2f}  r{r.page} y={r.y0:8.2f}  dy={p.y0-r.y0:7.2f}  {p.text[:40]}")

    if args.cumulative:
        # Global reading-order position: page_index * page_height + y0. This
        # folds pagination INTO the vertical coordinate, so the running
        # difference is the port's accumulated space surplus (>0) or deficit
        # (<0) relative to upstream over the whole document.
        H = args.page_height
        print(f"\n--- cumulative global dy (page_height={H}) ---")
        print("  i  pg(p/r)   port_glob    ref_glob    cum_dy   text")
        step = max(1, len(pairs) // args.cumulative)
        for i, (p, r) in enumerate(pairs):
            pg = p.page * H + p.y0
            rg = r.page * H + r.y0
            if i % step == 0 or i == len(pairs) - 1:
                print(f"{i:>4} {p.page:>3}/{r.page:<3} {pg:>10.1f} {rg:>10.1f} {pg-rg:>9.1f}   {p.text[:30]}")

    if args.gaps is not None:
        # Consecutive aligned pairs that stay on ONE page in BOTH engines: the
        # local vertical advance between them is directly comparable, and the
        # per-font yMin offset cancels in the subtraction.
        print(f"\n--- local advance deltas |d| >= {args.gaps}pt (port advance - upstream advance) ---")
        print("   pg    port_adv   up_adv       d   from -> to")
        tot = 0.0
        rows = []
        for i in range(len(pairs) - 1):
            p1, r1 = pairs[i]
            p2, r2 = pairs[i + 1]
            if p1.page != p2.page or r1.page != r2.page or p1.page != r1.page:
                continue
            pa = p2.y0 - p1.y0
            ra = r2.y0 - r1.y0
            if pa < -1.0 or ra < -1.0:
                continue  # went backwards (column/float); not a simple advance
            d = pa - ra
            tot += d
            if abs(d) >= args.gaps:
                rows.append((p1.page, pa, ra, d, p1.text, p2.text))
        for pg, pa, ra, d, t1, t2 in rows:
            print(f"{pg:>5} {pa:>10.2f} {ra:>8.2f} {d:>8.2f}   {t1[:18]} -> {t2[:18]}")
        print(f"sum of all in-page advance deltas = {tot:.2f}pt over {len(rows)} flagged")
    return 0


if __name__ == "__main__":
    sys.exit(main())
