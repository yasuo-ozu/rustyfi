# rustyfi @TAG@ — @TARGET@

A native Rust port of [SATySFi](https://github.com/gfngfn/SATySFi). `rustyfi`
compiles a `.saty` document to PDF (also HTML, via `--format`).

```console
$ ./rustyfi doc.saty -o doc.pdf
```

## What is in this archive

```
rustyfi                        the binary
lib-rustyfi/dist/packages/     the bundled SATySFi standard library
README.md                      the project README
```

The binary finds `lib-rustyfi/` by searching upward from the document, so
running it from beside this directory just works. Point `--lib-root` at it
explicitly if you move either one.

## Fonts are NOT in this archive

Without them the port falls back to the PDF base-14 fonts: Latin text still
sets, but the metrics are not SATySFi's and **CJK will not render at all**.

The faces (IPAex, Junicode, Latin Modern, DejaVu Math) are fetched from their
upstream sources, sha1-pinned, by a script in the repository:

```console
$ sh scripts/download-fonts.sh          # writes lib-rustyfi/dist/{fonts,hash}/
```

They are not redistributed here because one of them is located through the
host's fontconfig rather than downloaded, so a bundled set would differ by
platform — an archive that is honestly incomplete beats one that is silently
different. For a one-off, `--font <FILE>` takes a TTF/OTF directly and needs no
lib-root at all.

## The binary is a multicall

It dispatches on `argv[0]`:

| invoked as | behaves as |
|---|---|
| `rustyfi` | the compiler, plus the `satyrographos` and `multicall` subcommands |
| `satyrographos` | the package manager only |

`rustyfi multicall install --dir DIR` writes the aliases for you.

## Verifying this download

```console
$ shasum -a 256 -c rustyfi-@TAG@-@TARGET@.tar.gz.sha256
```

Licensed LGPL-3.0. Sources and issue tracker:
<https://github.com/yasuo-ozu/rustyfi>
