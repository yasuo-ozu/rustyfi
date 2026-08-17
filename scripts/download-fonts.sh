#!/bin/sh
# Fetch the real CJK+Latin faces stdja/mdja actually name (item #1,
# docs/plans/text-rendering.md §1c), and write this port's
# `dist/hash/{fonts,default-font}.satysfi-hash` (plain-JSON schema —
# satysfi-pdf's `fonts.rs` module doc — NOT upstream's Yojson variant
# syntax). Mirrors upstream SATySFi's own `download-fonts.sh` (cache dir +
# sha1-pinned downloads), targeting this repo's `lib-satysfi/dist/fonts/`.
#
# Fetches:
#   - IPAex (ipaexm.ttf, ipaexg.ttf) — IPA Font License Agreement v1.0,
#     redistributable; the license text is copied alongside the fonts.
#     Real TrueType (`glyf`) outlines: embeddable by this port's CID writer.
#   - Junicode (Junicode.ttf / -Bold.ttf / -Italic.ttf) — SIL OFL 1.1.
#     Real TrueType outlines: embeddable.
#
# Deliberately NOT fetched (upstream's lm2.004otf / latinmodern-math OTFs):
# both are CFF-outline OpenType, which this port's CID writer cannot embed
# as `FontFile2` (glyf-only, see `cid.rs`'s module doc) — `CIDFontType0`/
# `FontFile3` support is a separate, unowned work item. Instead, `lmsans`/
# `lmmono` abbrevs are pointed at whatever TrueType `DejaVu Sans`/`DejaVu
# Sans Mono` fontconfig resolves on THIS machine (a stand-in, not upstream's
# actual Latin Modern faces) — documented here and in
# docs/plans/text-rendering.md's Slice-2 status. `lmroman`/`lmroman-b`/
# `lmroman-it` are left unconfigured for the same CFF reason (nothing in
# stdja's own `set-font` calls names them; `Junicode` is the port's Latin
# default instead — see the written `default-font.satysfi-hash` below).
#
# Never commits font binaries: `lib-satysfi/dist/fonts/*.ttf` is
# `.gitignore`d (see that directory's `.gitignore`); this script (and the
# hash files it writes under `lib-satysfi/dist/hash/`) is the only checked-in
# artifact.

set -ue

MESSAGE_PREFIX="[download-fonts.sh]"
cd "$(dirname "$0")/.."   # repo root
CACHE="scripts/.fontcache"
FONTS_DIR="lib-satysfi/dist/fonts"
HASH_DIR="lib-satysfi/dist/hash"
mkdir -p "$CACHE" "$FONTS_DIR" "$HASH_DIR"

show_message () {
  echo "$MESSAGE_PREFIX $1."
}

if command -v shasum >/dev/null 2>&1; then
  SHA1SUM="shasum"
elif command -v sha1sum >/dev/null 2>&1; then
  SHA1SUM="sha1sum"
else
  echo "$MESSAGE_PREFIX No SHA1 checksum tool found (need shasum or sha1sum)." >&2
  exit 1
fi

# `NAME.sha1` files below are the pinned checksums this script trusts —
# recorded against the exact upstream release zips at authoring time (same
# URLs upstream's own `download-fonts.sh` uses). A checksum mismatch is a
# hard error: never silently proceed with an unexpected file.
validate_file () {
  NAME="$1"
  SHA1="$2"
  echo "$SHA1  $CACHE/$NAME" | $SHA1SUM --check --status -
}

download_file () {
  NAME="$1"
  URL="$2"
  SHA1="$3"
  if [ -f "$CACHE/$NAME" ] && validate_file "$NAME" "$SHA1"; then
    show_message "'$NAME' already cached and verified in '$CACHE/'"
  else
    show_message "downloading '$NAME'"
    curl -fsSL -o "$CACHE/$NAME" "$URL"
    if ! validate_file "$NAME" "$SHA1"; then
      echo "$MESSAGE_PREFIX SHA1 mismatch for '$NAME' — refusing to use it." >&2
      exit 1
    fi
    show_message "downloaded and verified '$NAME'"
  fi
}

# ---- IPAex (ipaexm.ttf / ipaexg.ttf) --------------------------------------
IPAEX_ZIP="IPAexfont00401.zip"
download_file "$IPAEX_ZIP" \
  "https://moji.or.jp/wp-content/ipafont/IPAexfont/IPAexfont00401.zip" \
  "57583c2be5dbfa06648ab0ae4937d7903b32595c"
rm -rf "$CACHE/IPAexfont00401"
unzip -o -q "$CACHE/$IPAEX_ZIP" -d "$CACHE"
cp "$CACHE/IPAexfont00401/ipaexm.ttf" "$FONTS_DIR/ipaexm.ttf"
cp "$CACHE/IPAexfont00401/ipaexg.ttf" "$FONTS_DIR/ipaexg.ttf"
cp "$CACHE/IPAexfont00401/IPA_Font_License_Agreement_v1.0.txt" \
  "$FONTS_DIR/IPA_Font_License_Agreement_v1.0.txt"
show_message "installed ipaexm.ttf / ipaexg.ttf (IPA Font License v1.0)"

# ---- Junicode (Junicode.ttf / -Bold.ttf / -Italic.ttf) --------------------
JUNICODE_ZIP="junicode-1.002.zip"
download_file "$JUNICODE_ZIP" \
  "https://downloads.sourceforge.net/project/junicode/junicode/junicode-1.002/junicode-1.002.zip" \
  "3ae070e6d9368665f5410d2cbd849fd97c18d877"
rm -rf "$CACHE/junicode"
unzip -o -q "$JUNICODE_ZIP" "*.ttf" -d "$CACHE/junicode" 2>/dev/null \
  || (cd "$CACHE" && unzip -o -q "$JUNICODE_ZIP" "*.ttf" -d junicode)
cp "$CACHE/junicode/Junicode.ttf" "$FONTS_DIR/Junicode.ttf"
cp "$CACHE/junicode/Junicode-Bold.ttf" "$FONTS_DIR/Junicode-Bold.ttf"
cp "$CACHE/junicode/Junicode-Italic.ttf" "$FONTS_DIR/Junicode-Italic.ttf"
show_message "installed Junicode.ttf / -Bold.ttf / -Italic.ttf (SIL OFL 1.1)"

# ---- lmsans / lmmono: TrueType stand-ins (see header comment) -------------
LMSANS_SRC="$(fc-match --format='%{file}' 'DejaVu Sans' 2>/dev/null || true)"
LMMONO_SRC="$(fc-match --format='%{file}' 'DejaVu Sans Mono' 2>/dev/null || true)"
if [ -z "$LMSANS_SRC" ] || [ ! -f "$LMSANS_SRC" ]; then
  echo "$MESSAGE_PREFIX 'DejaVu Sans' not found via fontconfig — lmsans will be" \
       "omitted from fonts.satysfi-hash (code.satyh/emph fall back gracefully)." >&2
  LMSANS_SRC=""
fi
if [ -z "$LMMONO_SRC" ] || [ ! -f "$LMMONO_SRC" ]; then
  echo "$MESSAGE_PREFIX 'DejaVu Sans Mono' not found via fontconfig — lmmono will" \
       "be omitted from fonts.satysfi-hash." >&2
  LMMONO_SRC=""
fi

# ---- Write dist/hash/*.satysfi-hash (this port's plain-JSON schema) ------
{
  printf '{\n'
  printf '  "ipaexm":     { "src": "dist/fonts/ipaexm.ttf" },\n'
  printf '  "ipaexg":     { "src": "dist/fonts/ipaexg.ttf" },\n'
  printf '  "Junicode":   { "src": "dist/fonts/Junicode.ttf" },\n'
  printf '  "Junicode-b": { "src": "dist/fonts/Junicode-Bold.ttf" },\n'
  printf '  "Junicode-it":{ "src": "dist/fonts/Junicode-Italic.ttf" }'
  if [ -n "$LMSANS_SRC" ]; then
    printf ',\n  "lmsans":     { "src": "%s" }' "$LMSANS_SRC"
  fi
  if [ -n "$LMMONO_SRC" ]; then
    printf ',\n  "lmmono":     { "src": "%s" }' "$LMMONO_SRC"
  fi
  printf '\n}\n'
} > "$HASH_DIR/fonts.satysfi-hash"

cat > "$HASH_DIR/default-font.satysfi-hash" <<'EOF'
{
  "regular": "Junicode", "bold": "Junicode-b", "oblique": "Junicode-it",
  "scripts": {
    "han-ideographic": { "font-name": "ipaexm",   "ratio": 0.88, "rising": 0.0 },
    "kana":            { "font-name": "ipaexm",   "ratio": 0.88, "rising": 0.0 },
    "latin":           { "font-name": "Junicode", "ratio": 1.0,  "rising": 0.0 },
    "other-script":    { "font-name": "Junicode", "ratio": 1.0,  "rising": 0.0 }
  }
}
EOF

show_message "wrote $HASH_DIR/fonts.satysfi-hash and default-font.satysfi-hash"
show_message "end"
