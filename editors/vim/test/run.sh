#!/usr/bin/env sh
# Headless test suite for editors/vim.
#
#   editors/vim/test/run.sh
#
# Runs test/run.vim (filetype, formatter, preview, job abstraction) in every
# editor it can find, and test/lsp.vim (Neovim only) in both supported
# language-server setup routes.
#
# Environment:
#   RUSTYFI_TEST_BIN   the rustyfi executable (default: ../../../target/release/rustyfi)
#   RUSTYFI_LIB_ROOT   package root for the preview's @require: resolution
#   RUSTYFI_EXTRA_VIM  space-separated extra editor binaries to run run.vim in,
#                      e.g. an older Vim or Neovim pulled from a pinned nixpkgs
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
plugin=$(dirname "$here")
repo=$(dirname "$(dirname "$plugin")")

: "${RUSTYFI_TEST_BIN:=$repo/target/release/rustyfi}"
if ! [ -x "$RUSTYFI_TEST_BIN" ]; then
  RUSTYFI_TEST_BIN=$(command -v rustyfi || true)
fi
if ! [ -x "${RUSTYFI_TEST_BIN:-}" ]; then
  echo "no rustyfi binary: build it with 'cargo build --release -p rustyfi'" >&2
  exit 2
fi
export RUSTYFI_TEST_BIN
: "${RUSTYFI_LIB_ROOT:=$repo/lib-rustyfi}"; export RUSTYFI_LIB_ROOT
: "${RUSTYFI_FONT_DIR:=$repo/lib-rustyfi}"; export RUSTYFI_FONT_DIR

echo "rustyfi:  $RUSTYFI_TEST_BIN"
echo "lib root: $RUSTYFI_LIB_ROOT"

status=0
ran=0

is_nvim() { "$1" --version 2>/dev/null | head -1 | grep -qi nvim; }

run_one() {
  bin=$1
  echo
  echo "=== $("$bin" --version 2>/dev/null | head -1)  [$bin]"
  ran=1
  if is_nvim "$bin"; then
    "$bin" --headless -u NONE -i NONE -S "$here/run.vim" || status=1
    "$bin" --headless -u NONE -i NONE -S "$here/lsp.vim" || status=1
    RUSTYFI_LSP_MODE=native "$bin" --headless -u NONE -i NONE -S "$here/lsp.vim" || status=1
  else
    "$bin" -es -u NONE -i NONE -S "$here/run.vim" </dev/null || status=1
  fi
}

command -v nvim >/dev/null 2>&1 && run_one "$(command -v nvim)"

# `vim` is a symlink to nvim on many systems (it is on the machine this plugin
# was written on); only treat it as Vim when it really is Vim.
if command -v vim >/dev/null 2>&1 && ! is_nvim "$(command -v vim)"; then
  run_one "$(command -v vim)"
else
  echo
  echo "note: no real Vim found on PATH (\`vim\` is absent or is Neovim)." >&2
  echo "      Set RUSTYFI_EXTRA_VIM to a Vim 8.2+ binary to exercise the" >&2
  echo "      job_start() branch of autoload/rustyfi/job.vim." >&2
fi

for extra in ${RUSTYFI_EXTRA_VIM:-}; do
  [ -x "$extra" ] && run_one "$extra"
done

[ "$ran" = 0 ] && { echo "no editor to test with" >&2; exit 2; }
echo
[ "$status" = 0 ] && echo "ALL GREEN" || echo "FAILURES"
exit "$status"
