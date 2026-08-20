# Build the port's own documents WITH the port.
#
#   make            manual.pdf, logo.pdf, logo.png
#   make manual     just the manual
#   make logo       just the logo
#   make fonts      (re-)run the font download
#   make clean      remove the scratch lib-root under ../target/ (not the PDFs)
#
# The port is invoked through `cargo run`, so there is no prebuilt binary to
# remember: cargo rebuilds it when the crates change and does nothing when they
# have not. `--release` because a debug build of this workspace is roughly an
# order of magnitude slower, and these documents are rebuilt often enough for
# that to be felt; override with `make CARGO_FLAGS=`.
#
# THE FONTS ARE NOT IN THE REPOSITORY. `lib-rustyfi/dist/fonts/` holds nothing
# but a `.gitignore`; the real faces — IPAex, Junicode, Latin Modern, the math
# fonts — are fetched by `scripts/download-fonts.sh`, sha1-pinned, into a cache.
# A build without them does not fail: it quietly falls back to the base-14
# fonts, and the logo's calligraphic letters (which only DejaVu Math TeX Gyre
# supplies) vanish entirely, leaving an empty disc. So the download is a real
# dependency here, expressed as one — the hash file the script writes last.
#
# Both documents are dogfood, and they need different amounts of scaffolding:
# the manual builds against the port's own bundled packages and nothing else,
# while the logo `@require:`s `xpath/xpath`, which is not bundled. So the logo
# gets a scratch lib-root with `xpath` INSTALLED into it by the port's own
# Satyrographos — deliberately, rather than reaching into the corpus tree with a
# relative `@import:`. That install is what `Satyristes` in the corpus package
# is for.

REPO        := $(CURDIR)/..
CARGO       ?= cargo
CARGO_FLAGS ?= --release
RUN         := $(CARGO) run $(CARGO_FLAGS) --manifest-path=$(REPO)/Cargo.toml \
                   --bin rustyfi --
LIB       := $(REPO)/lib-rustyfi
XPATH_SRC := $(REPO)/scripts/layout_fidelity_corpus/xpath
FONTS     := $(LIB)/dist/hash/fonts.satysfi-hash
# Scratch lives under cargo's `target/`, which is already ignored and already
# the place a reader expects to find generated state — one build directory for
# the repository, not one per tool. The rendered PDFs and PNG are NOT here: they
# are committed artifacts and belong beside their source.
BUILD     := $(REPO)/target/manual
LIBROOT   := $(BUILD)/libroot

# `--no-cache` because these are the reference artifacts: the compile cache is
# keyed on the SOURCE, so a change to the engine would otherwise be masked by a
# stale render. `--no-aux` so a build never depends on, or writes, an aux file
# left behind by a previous one.
RUSTYFI = $(RUN) --no-cache --no-aux

.PHONY: all manual logo png fonts clean
.DEFAULT_GOAL := all

all: manual logo png

# `scripts/download-fonts.sh` is cached and sha1-verified, so re-running it is
# cheap; it is still gated on its own output so a normal build does not shell
# out to curl. `make fonts` forces it.
fonts:
	rm -f $(FONTS)
	$(MAKE) $(FONTS)

# Run through `sh`, not `./`: the script is checked in without the executable
# bit, so `./scripts/download-fonts.sh` is a permission error on a fresh clone.
$(FONTS):
	cd $(REPO) && sh scripts/download-fonts.sh

manual: manual.pdf
manual.pdf: manual.saty $(FONTS)
	$(RUSTYFI) --lib-root $(LIB) --font-dir $(LIB) -o $@ $<

# The stamp, not the directory, is the target: a directory's mtime changes
# whenever anything inside it does, so `make` would reinstall on every run.
# Depends on the fonts because it COPIES `dist/` — copying before the download
# would bake a font-less lib-root into the scratch tree and the logo would
# render blank through it.
$(LIBROOT).stamp: $(XPATH_SRC)/Satyristes $(FONTS)
	rm -rf $(LIBROOT)
	mkdir -p $(LIBROOT)
	cp -r $(LIB)/dist $(LIBROOT)/
	$(RUN) satyrographos install $(XPATH_SRC) --lib-root $(LIBROOT)
	touch $@

logo: logo.pdf
logo.pdf: logo.saty $(LIBROOT).stamp
	$(RUSTYFI) --lib-root $(LIBROOT) --font-dir $(LIBROOT) -o $@ $<

# The raster is DERIVED, never authored. `-transp` keeps the page background out
# of it so the mark drops onto any colour; skipped with a warning rather than
# failing the build if poppler is not installed.
png: logo.png
logo.png: logo.pdf
	@if command -v pdftocairo >/dev/null; then \
	    pdftocairo -png -r 300 -transp -singlefile $< $(basename $@); \
	else \
	    echo "pdftocairo not found — skipping $@" >&2; \
	fi

clean:
	rm -rf $(BUILD)
