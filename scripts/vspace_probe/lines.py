#!/usr/bin/env python3
"""Dump a PDF page's text LINES with their yMin/xMin, one line per baseline
cluster. Used to read off where a page's content actually starts and how far
apart consecutive lines sit, for one engine at a time (so no cross-engine font
descriptor comparison is involved)."""

from __future__ import annotations

import argparse
import html
import re
import subprocess
import sys
from pathlib import Path

PAGE_RE = re.compile(r'<page width="([\d.]+)" height="([\d.]+)"')
WORD_RE = re.compile(
    r'<word xMin="([\d.]+)" yMin="([\d.]+)" xMax="([\d.]+)" yMax="([\d.]+)">(.*?)</word>'
)
TOL = 3.0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("pdf", type=Path)
    ap.add_argument("--page", type=int, action="append", default=[], help="0-based page (repeatable)")
    ap.add_argument("--max-lines", type=int, default=200)
    args = ap.parse_args()

    proc = subprocess.run(
        ["pdftotext", "-bbox", str(args.pdf), "-"], capture_output=True, timeout=180, text=True
    )
    pages: list[list[tuple[float, float, float, str]]] = []
    for line in proc.stdout.splitlines():
        if PAGE_RE.search(line):
            pages.append([])
            continue
        wm = WORD_RE.search(line)
        if wm and pages:
            x0, y0, x1, y1 = (float(wm.group(i)) for i in range(1, 5))
            t = html.unescape(wm.group(5)).strip()
            if t:
                pages[-1].append((y0, x0, y1, t))

    want = args.page or list(range(len(pages)))
    for pg in want:
        if pg >= len(pages):
            continue
        ws = sorted(pages[pg], key=lambda w: (round(w[0] / TOL), w[1]))
        print(f"=== page {pg} ({len(ws)} words) ===")
        cur_y = None
        buf: list[str] = []
        x_min = 0.0
        y_max = 0.0
        prev_y = None
        out = 0
        for y0, x0, y1, t in ws:
            if cur_y is None or abs(y0 - cur_y) > TOL:
                if cur_y is not None:
                    d = "" if prev_y is None else f"{cur_y - prev_y:+7.2f}"
                    print(f"  y={cur_y:7.2f} yMax={y_max:7.2f} d={d:>8} x={x_min:6.1f}  {''.join(buf)[:70]}")
                    out += 1
                    if out >= args.max_lines:
                        break
                    prev_y = cur_y
                cur_y = y0
                buf = []
                x_min = x0
                y_max = y1
            buf.append(t)
            y_max = max(y_max, y1)
        if cur_y is not None and out < args.max_lines:
            d = "" if prev_y is None else f"{cur_y - prev_y:+7.2f}"
            print(f"  y={cur_y:7.2f} yMax={y_max:7.2f} d={d:>8} x={x_min:6.1f}  {''.join(buf)[:70]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
