#!/usr/bin/env python3
"""Performance comparison: this Rust port vs. upstream SATySFi.

The sibling `layout_fidelity.py` asks whether the port produces the same
LAYOUT as upstream. This asks what it costs to produce it — wall clock, CPU and
peak memory — over the same vendored corpus, so a performance claim in a commit
message can be reproduced instead of taken on trust.

    scripts/benchmark.py                       # every doc, 3 runs, all configs
    scripts/benchmark.py --doc xpath --runs 5
    scripts/benchmark.py --json bench.json     # machine-readable alongside the table

Four configurations, per document:

  port-cold        the port with `--no-cache --no-aux`. The honest number: every
                   phase actually runs. This is what a first build costs.
  port-cached      the port with its content-addressed compile cache allowed
                   (still `--no-aux`), after a warm-up run. What an unchanged
                   rebuild costs.
  satysfi          upstream, as `layout_fidelity.py` invokes it for references.
  satysfi-bytecomp upstream with `--bytecomp`, if this build has the flag. This
                   is the fair comparison point for the port's evaluator, and
                   the one the project's past measurements used.

Three things this harness does deliberately, because getting them wrong is how
benchmarks come to lie:

  * RUNS ARE INTERLEAVED. Every configuration of every document is run once,
    then again, then again — not all of one config back to back. A machine that
    heats up, or a competing build that starts halfway through, then hits every
    configuration equally instead of penalising whichever ran last.
  * THE MINIMUM IS THE HEADLINE, not the mean. Timing noise is one-sided: an
    interfering process can only make a run slower, never faster. The median is
    reported beside it so a large min/median gap exposes a noisy machine.
  * NOTHING RUNS IN THE REPOSITORY. Each document's corpus package is copied to
    a scratch workspace and built there, because upstream writes a
    `.satysfi-aux` next to the source it builds and a benchmark that dirties the
    working tree is one nobody will run twice.

Requires `/usr/bin/time` (GNU) for peak RSS; without it, CPU still comes from
`getrusage(RUSAGE_CHILDREN)` and the RSS column reads `-`.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import resource
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path

# The corpus, the lib-root assembly and the document list are `layout_fidelity`'s
# and are imported rather than restated: the two harnesses MUST agree on what
# they are building and against which packages, or their results cannot be read
# together.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import layout_fidelity as lf  # noqa: E402

REPO = lf.REPO
CORPUS = lf.CORPUS
LIB_RUSTYFI = lf.LIB_RUSTYFI


def default_bin() -> Path:
    """The RELEASE binary, unlike `layout_fidelity.py`'s debug default.

    A debug build of this workspace is roughly an order of magnitude slower and
    says nothing about the port's actual performance; comparing it against an
    optimised OCaml binary would not be a measurement, it would be a libel.
    """
    return REPO / "target" / "release" / "rustyfi"


def find_gnu_time() -> str | None:
    """GNU `time`, for peak RSS. The shell builtin is not it."""
    for cand in ("/usr/bin/time", shutil.which("gtime"), shutil.which("time")):
        if not cand:
            continue
        try:
            probe = subprocess.run(
                [cand, "-f", "%e", "true"], capture_output=True, text=True, timeout=10
            )
        except (OSError, subprocess.SubprocessError):
            continue
        if probe.returncode == 0:
            return cand
    return None


def satysfi_supports(satysfi: str, flag: str) -> bool:
    try:
        out = subprocess.run(
            [satysfi, "--help"], capture_output=True, text=True, timeout=30
        )
    except (OSError, subprocess.SubprocessError):
        return False
    return flag in (out.stdout + out.stderr)


# ---------------------------------------------------------------------------
# One measured run.
# ---------------------------------------------------------------------------


@dataclass
class Sample:
    wall: float
    cpu: float
    rss_kb: int | None


@dataclass
class Result:
    samples: list[Sample] = field(default_factory=list)
    pages: int | None = None
    error: str | None = None

    def _pick(self, attr: str, fn) -> float | None:
        vals = [getattr(s, attr) for s in self.samples if getattr(s, attr) is not None]
        return fn(vals) if vals else None

    @property
    def wall_min(self) -> float | None:
        return self._pick("wall", min)

    @property
    def wall_med(self) -> float | None:
        return self._pick("wall", statistics.median)

    @property
    def cpu_min(self) -> float | None:
        return self._pick("cpu", min)

    @property
    def cpu_med(self) -> float | None:
        return self._pick("cpu", statistics.median)

    @property
    def rss_max(self) -> int | None:
        v = self._pick("rss_kb", max)
        return int(v) if v is not None else None


def run_once(argv: list[str], cwd: Path, timeout: int, gnu_time: str | None) -> Sample:
    """Run `argv`, returning wall/CPU/RSS. Raises on a non-zero exit."""
    if gnu_time:
        # `%e %U %S %M` on the LAST line of stderr, so the child's own stderr
        # (SATySFi is chatty) does not have to be parsed around.
        wrapped = [gnu_time, "-f", "%e %U %S %M"] + argv
        t0 = time.perf_counter()
        proc = subprocess.run(
            wrapped, cwd=cwd, capture_output=True, text=True, timeout=timeout
        )
        wall_fallback = time.perf_counter() - t0
        if proc.returncode != 0:
            raise RuntimeError(_tail(proc))
        line = proc.stderr.strip().splitlines()[-1].split()
        try:
            e, u, s, m = float(line[0]), float(line[1]), float(line[2]), int(line[3])
            return Sample(wall=e, cpu=u + s, rss_kb=m)
        except (ValueError, IndexError):
            return Sample(wall=wall_fallback, cpu=float("nan"), rss_kb=None)

    # No GNU time: children's CPU from getrusage deltas. `ru_maxrss` there is a
    # high-water mark across ALL children ever reaped, so a per-run delta would
    # be meaningless — the RSS column is left empty rather than filled with a
    # number that looks right and is not.
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    t0 = time.perf_counter()
    proc = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    wall = time.perf_counter() - t0
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    if proc.returncode != 0:
        raise RuntimeError(_tail(proc))
    cpu = (after.ru_utime - before.ru_utime) + (after.ru_stime - before.ru_stime)
    return Sample(wall=wall, cpu=cpu, rss_kb=None)


def _tail(proc: subprocess.CompletedProcess) -> str:
    return "\n".join((proc.stdout + proc.stderr).splitlines()[-8:]) or "(no output)"


# ---------------------------------------------------------------------------
# Configurations.
# ---------------------------------------------------------------------------


def port_argv(
    bin_path: Path, src_name: str, lib_root: Path, out: Path, cache_dir: Path, *, cache: bool
) -> list[str]:
    """Mirrors `layout_fidelity.build_pdf`'s argv, plus `--cache-dir`.

    `--no-aux` always: an aux file changes the cross-reference fixpoint's trial
    count, so honouring one would make a timing depend on whatever a previous
    run left behind. `--cache-dir` into the scratch workspace is the one
    addition — a benchmark has no business writing into the user's real
    `~/.cache`, and a cache left over from yesterday would make `port-cached`
    measure nothing at all.
    """
    argv = [str(bin_path)]
    if not cache:
        argv.append("--no-cache")
    argv += [
        "--no-aux",
        "--cache-dir", str(cache_dir),
        "--lib-root", str(lib_root),
        "--font-dir", str(LIB_RUSTYFI),
        "-o", str(out),
        src_name,
    ]
    return argv


def satysfi_argv(satysfi: str, src_name: str, lib_root: Path, out: Path, *, bytecomp: bool) -> list[str]:
    """Mirrors `layout_fidelity.build_ref_satysfi`'s argv."""
    argv = [satysfi, src_name, "-o", str(out), "-C", str(lib_root)]
    if bytecomp:
        argv.append("--bytecomp")
    return argv


def clear_aux(doc_dir: Path) -> None:
    """Delete any `.satysfi-aux` before an upstream run.

    Upstream reads one if it finds it, and a seeded cross-reference fixpoint can
    save it a whole typesetting pass. The port is measured with `--no-aux`, so
    leaving upstream's aux in place would time two different amounts of work and
    call the difference a result: run 1 cold, runs 2..n warm, and the median
    quietly reports the warm figure. Both engines are measured cold on
    cross-references. (Upstream WITH a warm aux is a legitimate thing to
    measure; it is simply not this column.)
    """
    for aux in doc_dir.glob("*.satysfi-aux"):
        aux.unlink()


def page_count(pdf: Path) -> int | None:
    if not pdf.exists() or pdf.stat().st_size == 0:
        return None
    with pdf.open("rb") as fh:
        if fh.read(5) != b"%PDF-":
            return None
    exe = shutil.which("pdfinfo")
    if not exe:
        return None
    out = subprocess.run([exe, str(pdf)], capture_output=True, text=True)
    for line in out.stdout.splitlines():
        if line.startswith("Pages:"):
            return int(line.split()[1])
    return None


# ---------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--doc", action="append", default=[], help="only this doc (repeatable)")
    ap.add_argument("--runs", type=int, default=3, help="measured runs per configuration")
    ap.add_argument("--bin", type=Path, default=default_bin(), help="the rustyfi binary")
    ap.add_argument("--satysfi", default=None, help="the original SATySFi binary (default: PATH)")
    ap.add_argument("--no-satysfi", action="store_true", help="port-only; skip upstream entirely")
    ap.add_argument("--timeout", type=int, default=900, help="per-run timeout (s)")
    ap.add_argument("--json", type=Path, default=None, help="also write results here")
    ap.add_argument("--keep-going", action="store_true", help="report all docs even if one fails")
    ap.add_argument("--workspace", type=Path, default=None, help="scratch dir (default: a temp dir)")
    args = ap.parse_args()

    docs = [d for d in lf.DOCS if not args.doc or d.name in args.doc]
    if not docs:
        print(f"no such doc; known: {', '.join(d.name for d in lf.DOCS)}", file=sys.stderr)
        return 2
    if not args.bin.exists():
        print(
            f"port binary not built at {args.bin}\n"
            f"  cargo build --release --bin rustyfi",
            file=sys.stderr,
        )
        return 2

    satysfi = None if args.no_satysfi else lf.find_satysfi(args.satysfi)
    bytecomp = bool(satysfi) and satysfi_supports(satysfi, "--bytecomp")
    gnu_time = find_gnu_time()

    tmp = tempfile.TemporaryDirectory(prefix="rustyfi-bench-")
    root = args.workspace or Path(tmp.name)
    root.mkdir(parents=True, exist_ok=True)

    port_root = lf.assemble_lib_root(root / "libroot-port", docs)
    sfi_root = lf.assemble_satysfi_lib_root(root / "libroot-satysfi", docs) if satysfi else None

    # Each doc is built in a COPY of its corpus package, never in the repo:
    # upstream writes a `.satysfi-aux` beside the source it builds.
    work: dict[str, Path] = {}
    for doc in docs:
        pkg = Path(doc.src).parts[0]
        dst = root / "work" / pkg
        if not dst.exists():
            shutil.copytree(CORPUS / pkg, dst)
        work[doc.name] = dst

    configs: list[tuple[str, str]] = [("port-cold", "port"), ("port-cached", "port")]
    if satysfi:
        configs.append(("satysfi", "sfi"))
        if bytecomp:
            configs.append(("satysfi-bytecomp", "sfi"))

    print(f"# rustyfi vs SATySFi — {time.strftime('%Y-%m-%d %H:%M')}")
    print()
    print(f"- host: {platform.platform()}, {os.cpu_count()} cpus, "
          f"load {', '.join(f'{x:.2f}' for x in os.getloadavg())}")
    print(f"- port: `{args.bin}`")
    if satysfi:
        ver = subprocess.run([satysfi, "--version"], capture_output=True, text=True)
        print(f"- upstream: `{satysfi}` ({ver.stdout.strip() or 'version unknown'})"
              f"{'' if bytecomp else ' — no --bytecomp in this build'}")
    else:
        print("- upstream: NOT MEASURED (no SATySFi binary)")
    print(f"- {args.runs} runs per configuration, interleaved; peak RSS via "
          f"{'GNU time' if gnu_time else 'UNAVAILABLE (no GNU time on PATH)'}")
    print()

    results: dict[tuple[str, str], Result] = {
        (d.name, c): Result() for d in docs for c, _ in configs
    }

    cache_dir = root / "port-cache"

    def argv_for(doc, cfg: str, out: Path) -> list[str]:
        name = Path(doc.src).name
        if cfg == "port-cold":
            return port_argv(args.bin, name, port_root, out, cache_dir, cache=False)
        if cfg == "port-cached":
            return port_argv(args.bin, name, port_root, out, cache_dir, cache=True)
        return satysfi_argv(satysfi, name, sfi_root, out, bytecomp=(cfg == "satysfi-bytecomp"))

    # A warm-up per (doc, config): it fills the port's compile cache — which is
    # the whole point of `port-cached` — and pays every first-touch page-fault
    # and file-cache cost once, for BOTH binaries, rather than charging it to
    # whichever happened to go first.
    failed: list[str] = []
    for doc in docs:
        cwd = work[doc.name] / Path(doc.src).parent.relative_to(Path(doc.src).parts[0])
        for cfg, _ in configs:
            out = root / f"warm-{doc.name}-{cfg}.pdf"
            try:
                if cfg.startswith("satysfi"):
                    clear_aux(cwd)
                run_once(argv_for(doc, cfg, out), cwd, args.timeout, gnu_time)
                results[(doc.name, cfg)].pages = page_count(out)
            except Exception as exc:  # noqa: BLE001 — the message is the result
                results[(doc.name, cfg)].error = str(exc)
                failed.append(f"{doc.name}/{cfg}")

    for _ in range(args.runs):
        for doc in docs:
            cwd = work[doc.name] / Path(doc.src).parent.relative_to(Path(doc.src).parts[0])
            for cfg, _ in configs:
                r = results[(doc.name, cfg)]
                if r.error:
                    continue
                out = root / f"{doc.name}-{cfg}.pdf"
                try:
                    if cfg.startswith("satysfi"):
                        clear_aux(cwd)
                    r.samples.append(run_once(argv_for(doc, cfg, out), cwd, args.timeout, gnu_time))
                except Exception as exc:  # noqa: BLE001
                    r.error = str(exc)
                    failed.append(f"{doc.name}/{cfg}")

    cfg_names = [c for c, _ in configs]
    print("| doc | pages | metric | " + " | ".join(cfg_names) + " | port-cold ÷ upstream |")
    print("|---|---|---|" + "---|" * (len(cfg_names) + 1))
    for doc in docs:
        base = results.get((doc.name, "satysfi-bytecomp")) or results.get((doc.name, "satysfi"))
        pages = next(
            (results[(doc.name, c)].pages for c in cfg_names if results[(doc.name, c)].pages),
            None,
        )
        # Units live on every datum, not once in a column header: these tables
        # get pasted into commit messages and issues a row at a time, and a
        # number that arrives without its unit is a number someone will guess at.
        for metric, get in (
            ("wall min/med", lambda r: _fmt2(r.wall_min, r.wall_med)),
            ("cpu  min/med", lambda r: _fmt2(r.cpu_min, r.cpu_med)),
            ("peak rss", lambda r: _fmt_rss(r.rss_max)),
        ):
            cells = []
            for c in cfg_names:
                r = results[(doc.name, c)]
                cells.append("**FAILED**" if r.error else get(r))
            ratio = ""
            if metric.startswith("cpu"):
                pc = results[(doc.name, "port-cold")]
                if base and base.cpu_min and pc.cpu_min and not (base.error or pc.error):
                    ratio = f"{pc.cpu_min / base.cpu_min:.2f}x CPU"
            first = f"{doc.name}" if metric.startswith("wall") else ""
            pg = str(pages or "-") if metric.startswith("wall") else ""
            print(f"| {first} | {pg} | {metric} | " + " | ".join(cells) + f" | {ratio} |")

    for doc in docs:
        for c in cfg_names:
            r = results[(doc.name, c)]
            if r.error:
                print(f"\n**{doc.name} / {c} failed:**\n```\n{r.error}\n```")

    if args.json:
        args.json.write_text(
            json.dumps(
                {
                    "units": {
                        "wall_min": "s", "wall_med": "s",
                        "cpu_min": "s", "cpu_med": "s",
                        "rss_kb": "KiB", "pages": "count",
                    },
                    "runs": args.runs,
                    "port_bin": str(args.bin),
                    "satysfi": satysfi,
                    "results": {
                        f"{d}/{c}": {
                            "wall_min": results[(d, c)].wall_min,
                            "wall_med": results[(d, c)].wall_med,
                            "cpu_min": results[(d, c)].cpu_min,
                            "cpu_med": results[(d, c)].cpu_med,
                            "rss_kb": results[(d, c)].rss_max,
                            "pages": results[(d, c)].pages,
                            "error": results[(d, c)].error,
                        }
                        for (d, c) in results
                    },
                },
                indent=2,
            )
        )
        print(f"\nwrote {args.json}")

    if failed and not args.keep_going:
        print(f"\nFAILED: {', '.join(sorted(set(failed)))}", file=sys.stderr)
        return 1
    return 0


def _fmt2(a: float | None, b: float | None) -> str:
    if a is None:
        return "-"
    return f"{a:.2f} s / {b:.2f} s"


def _fmt_rss(kb: int | None) -> str:
    return "-" if kb is None else f"{kb / 1024:.0f} MB"


if __name__ == "__main__":
    sys.exit(main())
