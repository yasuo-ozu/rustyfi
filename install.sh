#!/usr/bin/env bash
#
# Install rustyfi. Run `./install.sh --help` for options and the layout.
#
# Two modes. LOCAL, when the script sits in a checkout: install from
# target/release plus the repo's lib-rustyfi/, config.toml, README, LICENSE.
# The release workflow's `Stage archive` step uses this with the staging
# directory as the prefix, so a release archive and a from-source install are
# the same tree built by the same code.
#
# REMOTE, when there is no checkout around the script — the
# `curl -fsSL .../install.sh | bash` case: download the release archive for
# this platform, verify its checksum, and unpack the already-staged tree.

set -Eeuo pipefail

REPO_DEFAULT=yasuo-ozu/rustyfi

repo=${RUSTYFI_REPO:-$REPO_DEFAULT}
version=${RUSTYFI_VERSION:-}
prefix=${PREFIX:-}
bin=${RUSTYFI_BIN:-}
mode=

die() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

have() { command -v "$1" >/dev/null 2>&1; }

usage() {
    cat <<'EOF'
Usage: ./install.sh [--prefix DIR] [--bin PATH] [--version TAG] [--local|--remote]
       curl -fsSL https://raw.githubusercontent.com/yasuo-ozu/rustyfi/main/install.sh | bash

Install rustyfi, either from the checkout the script sits in or from a
published release archive.

Options:
  --prefix DIR   Install under DIR. Also settable as $PREFIX.
                 Precedence: --prefix, then $PREFIX, then the default below.
                 The default depends on privilege:
                   unprivileged (euid != 0)  ->  $HOME/.local
                   root / sudo  (euid == 0)  ->  /usr
  --bin PATH     Local mode only: the rustyfi executable to install. Also
                 settable as $RUSTYFI_BIN. Defaults to
                 target/release/rustyfi[.exe] in the repository this script
                 lives in; CI passes target/<triple>/release/rustyfi[.exe].
  --version TAG  Remote mode only: install release TAG instead of the latest.
                 Also settable as $RUSTYFI_VERSION.
  --local        Force local mode; fails if the script is not in a checkout.
  --remote       Force remote mode even inside a checkout.
  -h, --help     Show this help.

Mode is chosen by whether the script's own directory looks like the rustyfi
repository (Cargo.toml + lib-rustyfi/ + config.toml). Piped into bash there is
no such directory, so remote mode is used. The chosen mode is printed.

Environment:
  RUSTYFI_REPO            owner/name to download from (default yasuo-ozu/rustyfi)
  RUSTYFI_DOWNLOAD_BASE   asset base URL; assets are fetched from <base>/<tag>/
  RUSTYFI_LATEST_URL      JSON endpoint whose "tag_name" is the latest release

Resulting layout, relative to the prefix:

  bin/rustyfi[.exe]
  lib/rustyfi/dist/packages/       0.0 packages
  lib/rustyfi/dist/fonts/          the faces + their licences
  lib/rustyfi/dist/hash/           font name -> file
  lib/rustyfi/dist-v01/packages/   0.1 packages
  share/rustyfi/config.toml        shipped defaults, read from <exe>/../share
  share/man/man1/rustyfi.1
  share/doc/rustyfi/README.md, LICENSE

The binary finds lib/ and share/ relative to itself, so the tree is
self-contained: copy or unpack it anywhere and nothing needs exporting.

Local mode fetches the ~175 MB of bundled faces via ./download-fonts.sh when
lib-rustyfi/dist/fonts is empty (they are gitignored, so a fresh checkout has
nothing there). A release archive already contains them, so remote mode never
downloads fonts separately.

Examples:
  ./install.sh --prefix ~/.local
  sudo ./install.sh                       # installs into /usr
  PREFIX=/opt/rustyfi ./install.sh
  ./install.sh --remote --version v0.1.0
  ./install.sh --prefix /tmp/stage --bin target/x86_64-unknown-linux-gnu/release/rustyfi
EOF
}

while [ $# -gt 0 ]; do
    case $1 in
        --prefix)
            [ $# -ge 2 ] || die "--prefix needs a directory"
            prefix=$2
            shift 2
            ;;
        --prefix=*)
            prefix=${1#--prefix=}
            shift
            ;;
        --bin)
            [ $# -ge 2 ] || die "--bin needs a path"
            bin=$2
            shift 2
            ;;
        --bin=*)
            bin=${1#--bin=}
            shift
            ;;
        --version)
            [ $# -ge 2 ] || die "--version needs a release tag"
            version=$2
            shift 2
            ;;
        --version=*)
            version=${1#--version=}
            shift
            ;;
        --local)
            mode=local
            shift
            ;;
        --remote)
            mode=remote
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        --)
            shift
            break
            ;;
        *)
            die "unknown argument: $1 (try --help)"
            ;;
    esac
done

[ $# -eq 0 ] || die "unexpected argument: $1 (try --help)"

# Piped into bash there is no script file, so BASH_SOURCE may be unset or name
# something that is not this script. macOS has no `readlink -f`, hence cd/pwd.
script=${BASH_SOURCE[0]:-}
if [ -n "$script" ] && [ -f "$script" ]; then
    REPO_ROOT=$(CDPATH='' cd -- "$(dirname -- "$script")" && pwd -P)
else
    REPO_ROOT=
fi

is_checkout() {
    [ -n "$1" ] && [ -f "$1/Cargo.toml" ] && [ -d "$1/lib-rustyfi" ] && [ -f "$1/config.toml" ]
}

if [ -z "$mode" ]; then
    if is_checkout "$REPO_ROOT"; then mode=local; else mode=remote; fi
elif [ "$mode" = local ] && ! is_checkout "$REPO_ROOT"; then
    die "--local needs the script to sit in a rustyfi checkout, but ${REPO_ROOT:-that directory} is not one (no Cargo.toml + lib-rustyfi/ + config.toml)"
fi

# Root installs into /usr because that is where a system package manager would
# put it; everyone else gets ~/.local, which needs no privileges. Both are
# searched by the binary's own `roots::prefix_roots()`.
if [ -z "$prefix" ]; then
    if [ "$(id -u)" -eq 0 ]; then
        prefix=/usr
    else
        [ -n "${HOME:-}" ] || die "\$HOME is not set; pass --prefix DIR"
        prefix=$HOME/.local
    fi
fi

# Check the nearest existing ancestor: the prefix itself usually does not exist
# yet, and CI's `stage/$DIR` has no existing parent either.
check_writable() {
    d=$1
    while [ ! -e "$d" ]; do
        parent=$(dirname -- "$d")
        [ "$parent" != "$d" ] || break
        d=$parent
    done
    [ -w "$d" ] || die "cannot write to $d (installing into $prefix). Pass --prefix DIR for somewhere writable, or re-run as root."
}

printf 'install.sh: %s mode, prefix %s\n' "$mode" "$prefix"
check_writable "$prefix"

install_local() {
    # A plain `cargo build --release` lands in target/release; CI builds with
    # `--target <triple>` and passes that path explicitly.
    if [ -z "$bin" ]; then
        for candidate in "$REPO_ROOT/target/release/rustyfi" \
                         "$REPO_ROOT/target/release/rustyfi.exe"; do
            if [ -f "$candidate" ]; then
                bin=$candidate
                break
            fi
        done
    fi

    if [ -z "$bin" ]; then
        die "no rustyfi executable found at $REPO_ROOT/target/release/rustyfi[.exe]

Build one first:

    cargo build --release --locked --bin rustyfi

or point at an existing one with --bin PATH (or RUSTYFI_BIN=PATH)."
    fi

    [ -f "$bin" ] || die "not a file: $bin (--bin/RUSTYFI_BIN)"

    # Always install as `rustyfi`, keeping only the Windows `.exe` suffix.
    case $bin in
        *.exe) exe_name=rustyfi.exe ;;
        *) exe_name=rustyfi ;;
    esac

    if [ -z "$(ls -A "$REPO_ROOT/lib-rustyfi/dist/fonts" 2>/dev/null | grep -v '^\.gitignore$')" ]; then
        if [ -x "$REPO_ROOT/download-fonts.sh" ]; then
            printf 'no bundled faces in lib-rustyfi/dist/fonts — fetching them\n'
            sh "$REPO_ROOT/download-fonts.sh"
        else
            printf 'warning: lib-rustyfi/dist/fonts is empty and %s is missing;\n' \
                   "$REPO_ROOT/download-fonts.sh" >&2
            printf '         the install will have no bundled faces\n' >&2
        fi
    fi

    printf 'installing %s -> %s\n' "$bin" "$prefix"

    mkdir -p "$prefix/bin" \
             "$prefix/lib/rustyfi" \
             "$prefix/share/rustyfi" \
             "$prefix/share/doc/rustyfi" \
             "$prefix/share/man/man1"

    cp "$bin" "$prefix/bin/$exe_name"
    cp -R "$REPO_ROOT/lib-rustyfi/dist" "$REPO_ROOT/lib-rustyfi/dist-v01" "$prefix/lib/rustyfi/"
    cp "$REPO_ROOT/config.toml" "$prefix/share/rustyfi/"
    cp "$REPO_ROOT/README.md" "$REPO_ROOT/LICENSE" "$prefix/share/doc/rustyfi/"

    # The man page is rendered from the same clap command tree as `--help`, so
    # generating it here keeps it from drifting out of sync with the CLI.
    # `test -s` catches a binary that exits 0 while writing nothing.
    "$prefix/bin/$exe_name" man > "$prefix/share/man/man1/rustyfi.1"
    test -s "$prefix/share/man/man1/rustyfi.1"

    # `dist/{fonts,hash}` carry a .gitignore that has no meaning in an install.
    # Scoped to lib/rustyfi, not the whole prefix as the workflow's inline
    # version did: a real prefix may hold other software's .gitignore files.
    find "$prefix/lib/rustyfi" -name .gitignore -delete

    printf 'installed rustyfi to %s/bin/%s\n' "$prefix" "$exe_name"
}

# --- remote ------------------------------------------------------------------

download() {
    if have curl; then
        curl -fsSL -o "$2" "$1"
    elif have wget; then
        wget -qO "$2" "$1"
    else
        die "need curl or wget to download $1"
    fi
}

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | cut -d' ' -f1
    elif have shasum; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        die "need sha256sum or 'shasum -a 256' to verify the download"
    fi
}

# The triples are exactly release.yml's build matrix; anything else has no
# published asset and must be built from source.
detect_target() {
    local os arch
    case $(uname -s) in
        Linux) os=linux ;;
        Darwin) os=darwin ;;
        MINGW* | MSYS* | CYGWIN* | Windows_NT) os=windows ;;
        *) os=$(uname -s) ;;
    esac
    case $(uname -m) in
        x86_64 | amd64) arch=x86_64 ;;
        arm64 | aarch64) arch=aarch64 ;;
        *) arch=$(uname -m) ;;
    esac
    case "$os/$arch" in
        linux/x86_64) target=x86_64-unknown-linux-gnu; ext=tar.gz ;;
        linux/aarch64) target=aarch64-unknown-linux-gnu; ext=tar.gz ;;
        darwin/x86_64) target=x86_64-apple-darwin; ext=tar.gz ;;
        darwin/aarch64) target=aarch64-apple-darwin; ext=tar.gz ;;
        windows/x86_64) target=x86_64-pc-windows-msvc; ext=zip ;;
        *)
            die "no prebuilt binary for $os/$arch. Build from source with:

    git clone https://github.com/$repo && cd rustyfi
    cargo build --release --locked --bin rustyfi
    ./install.sh"
            ;;
    esac
}

install_remote() {
    local base latest tag asset url tmp expected actual root
    detect_target

    base=${RUSTYFI_DOWNLOAD_BASE:-https://github.com/$repo/releases/download}
    latest=${RUSTYFI_LATEST_URL:-https://api.github.com/repos/$repo/releases/latest}

    tmp=$(mktemp -d)
    # shellcheck disable=SC2064  # expand $tmp now: it is gone by trap time otherwise
    trap "rm -rf '$tmp'" EXIT

    tag=$version
    if [ -z "$tag" ]; then
        printf 'resolving the latest release of %s\n' "$repo"
        download "$latest" "$tmp/latest.json"
        # `|| true`: a repo with no releases answers {"message":"Not Found"},
        # and under pipefail the unmatched grep would kill the script before
        # the diagnostic below could run.
        tag=$(grep -o '"tag_name"[^,]*' "$tmp/latest.json" | head -1 | cut -d'"' -f4 || true)
        [ -n "$tag" ] || die "could not read a release tag from $latest — has $repo published a release yet? Pin one with --version TAG."
    fi

    asset="rustyfi-$tag-$target.$ext"
    url="$base/$tag/$asset"
    printf 'downloading %s\n' "$url"
    download "$url" "$tmp/$asset" ||
        die "could not download $url (no such release or asset?)"
    download "$url.sha256" "$tmp/$asset.sha256" ||
        die "could not download $url.sha256"

    # release.yml writes "<hash>  <filename>", from `shasum -a 256` on unix and
    # Get-FileHash|ToLower on Windows. Compare hashes rather than running
    # `sha256sum -c`, whose embedded filename will not match our temp path.
    expected=$(cut -d' ' -f1 < "$tmp/$asset.sha256" | tr '[:upper:]' '[:lower:]')
    actual=$(sha256_of "$tmp/$asset" | tr '[:upper:]' '[:lower:]')
    [ -n "$expected" ] || die "empty checksum file for $asset"
    if [ "$expected" != "$actual" ]; then
        die "checksum mismatch for $asset — refusing to install
  expected $expected
  actual   $actual"
    fi
    printf 'checksum ok (%s)\n' "$actual"

    mkdir -p "$tmp/x"
    case $ext in
        tar.gz)
            have tar || die "need tar to unpack $asset"
            tar -xzf "$tmp/$asset" -C "$tmp/x"
            ;;
        zip)
            have unzip || die "need unzip to unpack $asset"
            unzip -q "$tmp/$asset" -d "$tmp/x"
            ;;
    esac

    # The archive is `tar -C stage -czf … "$DIR"`, so it holds one top-level
    # directory named after the asset. Its contents are the staged tree — man
    # page included, which is why remote mode never runs the binary.
    root="$tmp/x/rustyfi-$tag-$target"
    [ -d "$root" ] || die "unexpected archive layout: no rustyfi-$tag-$target/ inside $asset"

    printf 'installing %s -> %s\n' "$asset" "$prefix"
    mkdir -p "$prefix"
    cp -R "$root/." "$prefix/"

    printf 'installed rustyfi %s to %s/bin\n' "$tag" "$prefix"
}

case $mode in
    local) install_local ;;
    remote) install_remote ;;
esac
