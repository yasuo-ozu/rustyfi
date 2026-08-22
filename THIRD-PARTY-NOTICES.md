# Third-party notices

rustyfi itself — everything under `crates/` — is MIT, see `LICENSE`. It bundles
and redistributes the following, which are not.

## SATySFi packages (`lib-rustyfi/`) — LGPL-3.0

    Copyright (c) gfngfn and the SATySFi contributors
    https://github.com/gfngfn/SATySFi

Redistributed under the GNU Lesser General Public License v3. Full texts:
`LICENSE.LGPL-3.0` and `LICENSE.GPL-3.0` (LGPL-3.0 incorporates the GPL-3.0 by
reference, so both are required). See `lib-rustyfi/LICENSE` for which files are
verbatim and which are modified — `dist-v01/packages/` are **modified**
versions of the `saphe-split` branch, marked as LGPL-3.0 section 4 requires.

These are SATySFi source files that the engine interprets at run time. They are
not linked into the binary, and the MIT licence of the engine is unaffected.

## Bundled fonts (`lib-rustyfi/dist/fonts/`, fetched by `download-fonts.sh`)

Not committed to this repository; fetched from pinned upstream archives and
verified by SHA-1. Each face is installed with its licence text beside it.

| Face | Licence |
|---|---|
| IPAexMincho, IPAexGothic | IPA Font License v1.0 (extracted from the upstream archive) |
| Junicode | SIL Open Font License 1.1 |
| Latin Modern, Latin Modern Math | GUST Font License (LPPL-like) |
| DejaVu Math TeX Gyre | Bitstream Vera terms + the TeX Gyre DJV Math addendum |

## Vendored test corpus (`layout-tests/corpus/`)

Present in this repository only; **not** shipped in release archives.

| Project | Origin | Licence |
|---|---|---|
| easytable, enumitem, figbox, railway, slydifi | monaqa | MIT |
| satysfi-base | nyuichi/satysfi-base | MIT |
| latexcmds, xpath | yasuo-ozu | LGPL-3.0 |
| gakushin | yasuo-ozu | this project's own |
| fss | na4zagin3/satysfi-fss | **no licence declared upstream** — see below |

`fss` carries no licence file, which means no rights are granted by default. It
is retained pending clarification from its author; if none is forthcoming it
should be removed from the corpus.

## Hyphenation data

`en-gb.standard.bincode` is derived from the `hyphenation` crate's TeX pattern
data (Apache-2.0 / MIT). Provenance is documented at
`crates/rustyfi-lang/src/hyphenation.rs`.

## Font metrics

`crates/rustyfi-pdf/src/base14.rs` transcribes Adobe's freely redistributable
Core-14 AFM metrics.

    Copyright (c) 1985-1997 Adobe Systems Incorporated. All rights reserved.

## Rust dependencies

Overwhelmingly `MIT OR Apache-2.0`, with some `MIT` and `Unicode-3.0`. No
copyleft dependency is used: the only crate offering a copyleft option is
`r-efi` (`MIT OR Apache-2.0 OR LGPL-2.1-or-later`), a disjunction, and MIT is
taken. Regenerate the full per-crate list with `cargo about` or
`cargo license`.
