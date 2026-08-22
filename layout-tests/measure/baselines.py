#!/usr/bin/env python3
"""Exact text BASELINES straight out of a PDF's content stream.

`pdftotext -bbox` reports a word's GLYPH BOX, whose top/bottom come from the
font descriptor — and the two engines emit different descriptors for the same
latin glyphs, so those boxes are not comparable across engines. The
text-positioning operators are: `BT ... Tm/Td ... Tj` places a run at an exact
baseline in PDF user space, identically for both writers.

This walks each page's (possibly Flate-compressed) content stream with a tiny
tokenizer, tracks the text matrix AND the graphics-state CTM, and prints one row
per distinct baseline:

    y_from_top   dy_from_previous   x_of_first_run   fonts   text-ish

`y_from_top` = page_height - baseline_y, so it reads the same way as
`pdftotext -bbox` output (increasing downward).

The CTM is not optional: the two writers position text through DIFFERENT
operators. Upstream SATySFi emits `q 1 0 0 1 0 0 cm BT <Tm> Ts TJ ET Q` per run
— an identity `cm`, everything in `Tm` — while this port emits `BT <Td> Tj ET`
and wraps table cells and inline graphics in a translating `q .. cm .. Q`. Read
the text matrix alone and every run inside such a wrapper lands at the page
origin: easytable's port render reports 513 baselines that way against its real
565, and upstream (which never translates) reports 565 either way. That is a
52-baseline phantom deficit created purely by reading the stream wrong.

Usage: baselines.py FILE.pdf [--page N]... [--tol 0.05]

`page_baselines()` is the reusable entry point (used by
`layout-tests/fidelity.py`'s `lines` metric).
"""

from __future__ import annotations

import argparse
import re
import sys
import zlib
from pathlib import Path


def parse_objects(data: bytes) -> dict[int, bytes]:
    """Map object number -> raw object body (between `obj` and `endobj`)."""
    objs: dict[int, bytes] = {}
    for m in re.finditer(rb"(\d+)\s+(\d+)\s+obj\b", data):
        num = int(m.group(1))
        start = m.end()
        end = data.find(b"endobj", start)
        if end < 0:
            continue
        objs[num] = data[start:end]
    return objs


def stream_bytes(body: bytes) -> bytes | None:
    i = body.find(b"stream")
    if i < 0:
        return None
    j = i + len(b"stream")
    if body[j : j + 2] == b"\r\n":
        j += 2
    elif body[j : j + 1] in (b"\n", b"\r"):
        j += 1
    k = body.rfind(b"endstream")
    raw = body[j:k] if k > 0 else body[j:]
    if b"FlateDecode" in body[:i]:
        try:
            return zlib.decompress(raw)
        except Exception:
            try:
                return zlib.decompressobj().decompress(raw)
            except Exception:
                return None
    return raw


def page_order(objs: dict[int, bytes]) -> list[int]:
    """Page object numbers in DOCUMENT order, by walking /Type /Pages /Kids.
    Object-number order is NOT document order (upstream SATySFi emits them
    out of order), and reading the wrong page is a silent, total wrong answer."""
    kids: dict[int, list[int]] = {}
    roots: list[int] = []
    is_page: set[int] = set()
    for num, body in objs.items():
        if re.search(rb"/Type\s*/Pages\b", body):
            m = re.search(rb"/Kids\s*\[(.*?)\]", body, re.S)
            kids[num] = [int(x) for x in re.findall(rb"(\d+)\s+\d+\s+R", m.group(1))] if m else []
            roots.append(num)
        elif re.search(rb"/Type\s*/Page\b(?!s)", body):
            is_page.add(num)
    if not kids:
        return sorted(is_page)
    # The root Pages node is the one nobody lists as a kid.
    child_of_someone = {c for v in kids.values() for c in v}
    tops = [n for n in roots if n not in child_of_someone] or roots[:1]
    out: list[int] = []
    seen: set[int] = set()

    def walk(n: int) -> None:
        if n in seen:
            return
        seen.add(n)
        if n in kids:
            for c in kids[n]:
                walk(c)
        elif n in is_page:
            out.append(n)

    for t in tops:
        walk(t)
    for n in sorted(is_page):
        if n not in out:
            out.append(n)
    return out


def page_objects(data: bytes, objs: dict[int, bytes]) -> list[tuple[float, list[int]]]:
    """[(page_height, [content-stream obj nums])] in document order."""
    out = []
    for num in page_order(objs):
        body = objs[num]
        mb = re.search(rb"/MediaBox\s*\[\s*([\d.\-]+)\s+([\d.\-]+)\s+([\d.\-]+)\s+([\d.\-]+)", body)
        h = float(mb.group(4)) - float(mb.group(2)) if mb else 841.89
        cm = re.search(rb"/Contents\s+(\d+)\s+\d+\s+R", body)
        cs: list[int] = []
        if cm:
            cs = [int(cm.group(1))]
        else:
            cm2 = re.search(rb"/Contents\s*\[([^\]]*)\]", body)
            if cm2:
                cs = [int(x) for x in re.findall(rb"(\d+)\s+\d+\s+R", cm2.group(1))]
        out.append((h, cs))
    return out


def page_resources(objs: dict[int, bytes]) -> list[bytes | None]:
    """Each page's raw /Resources value (a `<< .. >>` blob or an `N 0 R`), in
    document order — what `runs_of` needs to follow a `Do` into a Form XObject."""
    return [dict_value(objs[num], b"Resources") for num in page_order(objs)]


def deref(objs: dict[int, bytes], blob: bytes | None) -> bytes:
    """Follow an `N 0 R` indirect reference; anything else is already the value."""
    if blob is None:
        return b""
    m = re.fullmatch(rb"\s*(\d+)\s+\d+\s+R\s*", blob)
    return objs.get(int(m.group(1)), b"") if m else blob


def dict_value(body: bytes, key: bytes) -> bytes | None:
    """The raw value of `/key` in a dictionary blob: a balanced `<< .. >>`, an
    `N 0 R`, an array, a name, or a number. Enough of a parser to walk
    /Resources -> /XObject -> a form's stream; not a PDF library."""
    m = re.search(rb"/" + key + rb"\b", body)
    if not m:
        return None
    i = m.end()
    while i < len(body) and body[i : i + 1].isspace():
        i += 1
    if body[i : i + 2] == b"<<":
        depth, j = 0, i
        while j < len(body):
            if body[j : j + 2] == b"<<":
                depth += 1
                j += 2
            elif body[j : j + 2] == b">>":
                depth -= 1
                j += 2
                if depth == 0:
                    return body[i:j]
            else:
                j += 1
        return body[i:]
    m2 = re.match(rb"\d+\s+\d+\s+R|/[^\s/\[\]<>()]+|\[[^\]]*\]|[-+]?[\d.]+", body[i:])
    return m2.group(0) if m2 else None


def xobjects(objs: dict[int, bytes], resources: bytes | None) -> dict[str, int]:
    """{name -> object number} for the /XObject entries a stream can `Do`."""
    xo = dict_value(deref(objs, resources), b"XObject")
    if xo is None:
        return {}
    return {
        name.decode("latin1"): int(num)
        for name, num in re.findall(rb"/([^\s/\[\]<>()]+)\s+(\d+)\s+\d+\s+R", deref(objs, xo))
    }


# A literal string must be consumed AS ONE TOKEN even when it contains escaped
# or balanced parentheses, or the rest of its bytes leak out as operators. The
# port really does emit them: `\(` / `\)` appear 166 times in latexcmds' stream,
# and the naive `\([^)]*\)` ended a token at the first escaped `)`, leaving a
# stray `'` (the show-next-line operator) to be read as a text-showing op —
# which is why that render reported 5195 runs against its 5193 `BT`s.
TOKEN = re.compile(
    rb"(\((?:\\.|[^\\()]|\((?:\\.|[^\\()])*\))*\)"
    rb"|<[0-9A-Fa-f\s]*>|/[^\s/\[\]<>()]+|\[|\]|[-+]?[\d.]+|[A-Za-z'\"*]+)",
    re.S,
)

IDENTITY = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)


def matmul(a: list[float], b: list[float]) -> list[float]:
    """`a` then `b` — the 3x2 affine product PDF's `cm`/`Tm` compose with."""
    return [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]


def runs_of(
    content: bytes,
    objs: dict[int, bytes] | None = None,
    resources: bytes | None = None,
    base_ctm: list[float] | None = None,
    _seen: frozenset[int] = frozenset(),
) -> list[tuple[float, float, str]]:
    """[(x, y, font)] for every text-showing operator, in stream order, in
    DEVICE space — i.e. Tm composed with the graphics-state CTM, so a run
    inside a translating `q .. cm .. Q` reports where it actually lands.

    Pass `objs` and the stream's `resources` to also FOLLOW `Do` into Form
    XObjects. Without that, text inside a form is silently invisible — and
    forms are where a good deal of it lives: figbox's embedded example pages
    hold text in 23 of them in each engine, and gakushin's whole 学振 form
    template is nothing but nested forms (its page-level text is 18 baselines
    against 66 once the forms are walked). This is the walker-skips-a-nested-
    container bug shape, in a measuring tool: the missing content just looks
    like whitespace.
    """
    toks = [m.group(1) for m in TOKEN.finditer(content)]
    out: list[tuple[float, float, str]] = []
    stack: list[bytes] = []
    tm = list(IDENTITY)
    tlm = list(tm)
    ctm = list(base_ctm) if base_ctm else list(IDENTITY)
    gstack: list[list[float]] = []
    leading = 0.0
    font = "?"
    forms = xobjects(objs, resources) if objs is not None else {}

    def num(b: bytes) -> float:
        try:
            return float(b)
        except ValueError:
            return 0.0

    def newline(dx: float, dy: float) -> list[float]:
        return matmul([1.0, 0.0, 0.0, 1.0, dx, dy], tlm)

    for t in toks:
        if t in (b"Tj", b"TJ"):
            m = matmul(tm, ctm)
            out.append((m[4], m[5], font))
            stack.clear()
            continue
        if t in (b"'", b'"'):
            # Both move to the next line FIRST, then show.
            tlm = newline(0.0, -leading)
            tm = list(tlm)
            m = matmul(tm, ctm)
            out.append((m[4], m[5], font))
            stack.clear()
            continue
        if t == b"Tf":
            if len(stack) >= 2 and stack[-2].startswith(b"/"):
                font = stack[-2].decode("latin1")
            stack.clear()
            continue
        if t == b"TL":
            if stack:
                leading = num(stack[-1])
            stack.clear()
            continue
        if t == b"Tm":
            if len(stack) >= 6:
                tm = [num(x) for x in stack[-6:]]
                tlm = list(tm)
            stack.clear()
            continue
        if t in (b"Td", b"TD"):
            if len(stack) >= 2:
                dx, dy = num(stack[-2]), num(stack[-1])
                if t == b"TD":
                    leading = -dy
                tlm = newline(dx, dy)
                tm = list(tlm)
            stack.clear()
            continue
        if t == b"T*":
            tlm = newline(0.0, -leading)
            tm = list(tlm)
            stack.clear()
            continue
        if t == b"cm":
            if len(stack) >= 6:
                ctm = matmul([num(x) for x in stack[-6:]], ctm)
            stack.clear()
            continue
        if t == b"q":
            gstack.append(list(ctm))
            stack.clear()
            continue
        if t == b"Q":
            if gstack:
                ctm = gstack.pop()
            stack.clear()
            continue
        if t == b"Do":
            name = stack[-1][1:].decode("latin1") if stack and stack[-1].startswith(b"/") else ""
            objnum = forms.get(name)
            if objs is not None and objnum is not None and objnum not in _seen and len(_seen) < 8:
                body = objs[objnum]
                if re.search(rb"/Subtype\s*/Form\b", body):
                    mat = dict_value(body, b"Matrix")
                    nums = [float(x) for x in re.findall(rb"[-+]?[\d.]+", mat or b"")]
                    form_ctm = matmul(nums, ctm) if len(nums) == 6 else list(ctm)
                    out.extend(
                        runs_of(
                            stream_bytes(body) or b"",
                            objs,
                            dict_value(body, b"Resources"),
                            form_ctm,
                            _seen | {objnum},
                        )
                    )
            stack.clear()
            continue
        if t in (b"BT",):
            tm = list(IDENTITY)
            tlm = list(tm)
            stack.clear()
            continue
        stack.append(t)
        if len(stack) > 8:
            del stack[0]
    return out


def page_baselines(pdf: Path, tol: float = 0.05) -> list[list[float]]:
    """Per page, the sorted distinct text BASELINES as y-from-top.

    This is the reusable form of what the CLI prints. Runs whose baselines
    agree to within `tol` are one row: within a single writer, every run of one
    typeset line is emitted at the identical `y` (to the 3 decimals PDF numbers
    carry), so `tol` only has to absorb that rounding — see
    `layout-tests/fidelity.py`'s `line_count` for the measured evidence that
    the port-vs-upstream delta is flat from 0.02 all the way to 0.5.

    `tol=0.0` returns the RAW distinct baselines (exact duplicates collapsed
    only), which a caller can then cluster at any tolerance it likes without
    re-reading the file — what `fidelity.py --tol-sweep` does.
    """
    data = pdf.read_bytes()
    objs = parse_objects(data)
    res = page_resources(objs)
    out: list[list[float]] = []
    for pi, (h, cs) in enumerate(page_objects(data, objs)):
        content = b"".join(stream_bytes(objs.get(n, b"")) or b"" for n in cs)
        rows: list[float] = []
        runs = runs_of(content, objs, res[pi] if pi < len(res) else None)
        for _, y, _ in sorted(runs, key=lambda r: h - r[1]):
            yt = h - y
            if rows and yt - rows[-1] <= tol:
                continue
            rows.append(yt)
        out.append(rows)
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("pdf", type=Path)
    ap.add_argument("--page", type=int, action="append", default=[])
    ap.add_argument("--tol", type=float, default=0.05, help="baselines within this are one row")
    args = ap.parse_args()

    data = args.pdf.read_bytes()
    objs = parse_objects(data)
    pages = page_objects(data, objs)
    res = page_resources(objs)
    want = args.page or list(range(len(pages)))
    for pi in want:
        if pi >= len(pages):
            continue
        h, cs = pages[pi]
        content = b"".join(stream_bytes(objs.get(n, b"")) or b"" for n in cs)
        rs = runs_of(content, objs, res[pi] if pi < len(res) else None)
        # Cluster in BASELINE order, not stream order: with the CTM followed and
        # forms walked, the stream visits a page's lines out of order often
        # enough that clustering as they arrive would split one line in two.
        rows: list[tuple[float, float, list[str]]] = []
        for x, y, f in sorted(rs, key=lambda r: h - r[1]):
            yt = h - y
            if rows and yt - rows[-1][0] <= args.tol:
                if f not in rows[-1][2]:
                    rows[-1][2].append(f)
                continue
            rows.append((yt, x, [f]))
        print(f"=== page {pi} (h={h}) — {len(rows)} baselines ===")
        prev = None
        for yt, x, fs in rows:
            d = "" if prev is None else f"{yt - prev:+8.3f}"
            print(f"  y={yt:9.3f} d={d:>9} x={x:8.2f}  {','.join(fs)}")
            prev = yt
    return 0


if __name__ == "__main__":
    sys.exit(main())
