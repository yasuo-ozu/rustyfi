#!/bin/sh
# Fetch the real CJK+Latin faces stdja/mdja actually name (item #1), and
# write this port's
# `dist/hash/{fonts,default-font}.rustyfi-hash` (plain-JSON schema —
# rustyfi-pdf's `fonts.rs` module doc — NOT upstream's Yojson variant
# syntax). Mirrors upstream SATySFi's own `download-fonts.sh` (cache dir +
# sha1-pinned downloads), targeting this repo's `lib-rustyfi/dist/fonts/`.
#
# Fetches:
#   - IPAex (ipaexm.ttf, ipaexg.ttf) — IPA Font License Agreement v1.0,
#     redistributable; the license text is copied alongside the fonts.
#     Real TrueType (`glyf`) outlines: embeddable by this port's CID writer.
#   - Junicode (Junicode.ttf / -Bold.ttf / -Italic.ttf) — SIL OFL 1.1.
#     Real TrueType outlines: embeddable.
#   - Latin Modern Math (latinmodern-math.otf) — GUST e-foundry's own math
#     companion to the Latin Modern family, the REAL upstream face (same
#     build CTAN's `lm-math` package ships), under the GUST Font License
#     (GFL — see `LICENSE-GUST-FontLicense.txt`, reused from the lmsans/lmmono
#     block below, same license). Pinned-zip download (hermetic, like
#     IPAex/Junicode/lmsans/lmmono — no fontconfig lookup needed). CFF-outline
#     (`OTTO`) OpenType with a real `MATH` table — embeddable via this port's
#     `CIDFontType0`/`FontFile3` (CFF) path (`cid.rs`'s `write_font_cff`,
#     commit `526e1f3`, subsetting added in `962addc`) AND readable by
#     `ttf.rs`'s MATH parser. Registered as the `"lmmath"` abbrev and wired as
#     `default-font.rustyfi-hash`'s `"math"` default — this is upstream
#     SATySFi's own default math font, so this is the upstream-correct
#     choice (Slice B originally wired DejaVu Math TeX Gyre here as a
#     `glyf`-outline stand-in, from before the
#     CFF embedding path existed; now that CFF embedding + subsetting both
#     land, LM Math replaces it as the default).
#
#   - DejaVu Math TeX Gyre (DejaVuMathTeXGyre.ttf) — Bitstream Vera-style
#     license (DejaVu terms; see `LICENSE-DejaVu.txt`, written alongside the
#     font below). `glyf`-outline TrueType with a real OpenType `MATH` table
#     — embeddable by this port's CID writer AND readable by `ttf.rs`'s MATH
#     parser. Still registered as the `"dejavu-math"` abbrev (kept as a
#     fallback/option), but no longer the `"math"` default now that real LM
#     Math (above) is bundled. No pinned-zip download exists for this one
#     (unlike IPAex/Junicode/LM Math) — it is located via `fc-match`/
#     fontconfig on the machine running this script (present via the
#     `dejavu-fonts` package on this repo's dev/CI images) and copied
#     byte-for-byte into `$FONTS_DIR`; if fontconfig can't find it, this step
#     is skipped gracefully (the `dejavu-math` abbrev is simply omitted)
#     rather than failing the whole script.
#
#   - Latin Modern Sans / Latin Modern Mono (lmsans10-regular.otf,
#     lmmono10-regular.otf) — the REAL upstream faces, GUST e-foundry's own
#     "flat" OpenType release (the same build CTAN's `lm` package ships),
#     under the GUST Font License (GFL — an LPPL-1.3c-equivalent, freely
#     redistributable license; the full text is copied alongside the fonts,
#     see `LICENSE-GUST-FontLicense.txt`, written below). These are
#     CFF-outline (`OTTO`) OpenType fonts, embeddable via this port's
#     `CIDFontType0`/`FontFile3` (CFF) path added in commit `526e1f3`
#     (`cid.rs`'s `write_font_cff`, "S1" — whole-OTF embed, no subsetting
#     yet). Registered as the `lmsans`/`lmmono` abbrevs (Slice 1/3),
#     replacing the earlier Noto Sans/Noto Sans Mono `glyf` stand-in
#     (commit `0ef39ef`) that was needed only because `FontFile2`/glyf-only
#     embedding couldn't carry a CFF face at the time.
#
# Still NOT fetched: `lmroman`/`lmroman-b`/`lmroman-it` (Latin Modern Roman)
# — nothing in stdja's own `set-font` calls names them (`Junicode` is the
# port's Latin default instead — see the written `default-font.rustyfi-hash`
# below); add a block mirroring the lmsans/lmmono one below if a document
# ever needs them. `lmodern` is registered as an alias for the bundled LM
# Math font (see below) so `set-math-font \`lmodern\`` resolves — it now
# points at the real `lmmath` font (previously aliased to the DejaVu Math
# stand-in, before LM Math itself was bundled).
#
# Never commits font binaries: `lib-rustyfi/dist/fonts/*.ttf` is
# `.gitignore`d (see that directory's `.gitignore`); this script (and the
# hash files it writes under `lib-rustyfi/dist/hash/`) is the only checked-in
# artifact. Nor does it write anything else into the working tree — the
# download cache is under `$TMPDIR` (see `CACHE` below).

set -ue

MESSAGE_PREFIX="[download-fonts.sh]"
cd "$(dirname "$0")/.."   # repo root
# The archive cache lives in the system temp dir, not in the working tree: it
# is ~175 MB of pinned upstream zips, none of it source, and a copy sitting in
# `scripts/` shows up in every `du`, backup and grep for the rest of time.
#
# A STABLE path, deliberately, not `mktemp -d`: the whole point of the cache is
# that a re-run verifies sha1s instead of re-downloading ~150 MB, and a fresh
# directory per run would throw that away. The cost is that a machine which
# clears its temp dir on reboot re-downloads once; the archives are pinned, so
# that is slow rather than risky. `RUSTYFI_FONTCACHE` overrides it — set that to
# a persistent path on a metered connection.
CACHE="${RUSTYFI_FONTCACHE:-${TMPDIR:-/tmp}/rustyfi-fontcache}"
FONTS_DIR="lib-rustyfi/dist/fonts"
HASH_DIR="lib-rustyfi/dist/hash"
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
cp "$CACHE/junicode/Junicode-BoldItalic.ttf" "$FONTS_DIR/Junicode-BoldItalic.ttf"
show_message "installed Junicode.ttf / -Bold.ttf / -Italic.ttf / -BoldItalic.ttf (SIL OFL 1.1)"

# Junicode ships no licence file of its own (the 1.002 zip is fonts, docs and a
# ChangeLog), so the OFL text is written here: OFL 1.1 §2 requires it to travel
# with any redistribution, and the release archives redistribute these faces.
# The copyright line is the font's own `name` table entry, not a guess.
cat > "$FONTS_DIR/LICENSE-Junicode-OFL.txt" <<'OFL_LICENSE_EOF'
Copyright (c) 1998-2018 by Peter S. Baker.

This Font Software is licensed under the SIL Open Font License, Version 1.1.
This license is copied below, and is also available with a FAQ at:
https://openfontlicense.org

-----------------------------------------------------------
SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007
-----------------------------------------------------------

PREAMBLE
The goals of the Open Font License (OFL) are to stimulate worldwide
development of collaborative font projects, to support the font creation
efforts of academic and linguistic communities, and to provide a free and
open framework in which fonts may be shared and improved in partnership
with others.

The OFL allows the licensed fonts to be used, studied, modified and
redistributed freely as long as they are not sold by themselves. The
fonts, including any derivative works, can be bundled, embedded,
redistributed and/or sold with any software provided that any reserved
names are not used by derivative works. The fonts and derivatives,
however, cannot be released under any other type of license. The
requirement for fonts to remain under this license does not apply
to any document created using the fonts or their derivatives.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright
Holder(s) under this license and clearly marked as such. This may
include source files, build scripts and documentation.

"Reserved Font Name" refers to any names specified as such after the
copyright statement(s).

"Original Version" refers to the collection of Font Software components as
distributed by the Copyright Holder(s).

"Modified Version" refers to any derivative made by adding to, deleting,
or substituting -- in part or in whole -- any of the components of the
Original Version, by changing formats or by porting the Font Software to a
new environment.

"Author" refers to any designer, engineer, programmer, technical
writer or other person who contributed to the Font Software.

PERMISSION & CONDITIONS
Permission is hereby granted, free of charge, to any person obtaining
a copy of the Font Software, to use, study, copy, merge, embed, modify,
redistribute, and sell modified and unmodified copies of the Font
Software, subject to the following conditions:

1) Neither the Font Software nor any of its individual components,
in Original or Modified Versions, may be sold by itself.

2) Original or Modified Versions of the Font Software may be bundled,
redistributed and/or sold with any software, provided that each copy
contains the above copyright notice and this license. These can be
included either as stand-alone text files, human-readable headers or
in the appropriate machine-readable metadata fields within text or
binary files as long as those fields can be easily viewed by the user.

3) No Modified Version of the Font Software may use the Reserved Font
Name(s) unless explicit written permission is granted by the corresponding
Copyright Holder. This restriction only applies to the primary font name as
presented to the users.

4) The name(s) of the Copyright Holder(s) or the Author(s) of the Font
Software shall not be used to promote, endorse or advertise any
Modified Version, except to acknowledge the contribution(s) of the
Copyright Holder(s) and the Author(s) or with their explicit written
permission.

5) The Font Software, modified or unmodified, in part or in whole,
must be distributed entirely under this license, and must not be
distributed under any other license. The requirement for fonts to
remain under this license does not apply to any document created
using the Font Software.

TERMINATION
This license becomes null and void if any of the above conditions are
not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT
OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE
COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM
OTHER DEALINGS IN THE FONT SOFTWARE.
OFL_LICENSE_EOF
show_message "installed LICENSE-Junicode-OFL.txt (SIL OFL 1.1)"


# ---- DejaVu Math TeX Gyre (see header comment) ----------------------------
MATH_SRC="$(fc-match --format='%{file}' 'DejaVu Math TeX Gyre' 2>/dev/null || true)"
if [ -n "$MATH_SRC" ] && [ -f "$MATH_SRC" ]; then
  cp "$MATH_SRC" "$FONTS_DIR/DejaVuMathTeXGyre.ttf"
  # DejaVu's own LICENSE (Bitstream Vera terms + the TeX Gyre DJV Math
  # addendum covering the MATH-specific glyphs/AMSFonts Euler Fraktur
  # import) — embedded verbatim rather than fetched at run time, so this
  # step has no network dependency and works offline/in CI.
  cat > "$FONTS_DIR/LICENSE-DejaVu.txt" <<'DEJAVU_LICENSE_EOF'
Fonts are (c) Bitstream (see below). DejaVu changes are in public domain.
Glyphs imported from Arev fonts are (c) Tavmjong Bah (see below)


Bitstream Vera Fonts Copyright
------------------------------

Copyright (c) 2003 by Bitstream, Inc. All Rights Reserved. Bitstream Vera is
a trademark of Bitstream, Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of the fonts accompanying this license ("Fonts") and associated
documentation files (the "Font Software"), to reproduce and distribute the
Font Software, including without limitation the rights to use, copy, merge,
publish, distribute, and/or sell copies of the Font Software, and to permit
persons to whom the Font Software is furnished to do so, subject to the
following conditions:

The above copyright and trademark notices and this permission notice shall
be included in all copies of one or more of the Font Software typefaces.

The Font Software may be modified, altered, or added to, and in particular
the designs of glyphs or characters in the Fonts may be modified and
additional glyphs or characters may be added to the Fonts, only if the fonts
are renamed to names not containing either the words "Bitstream" or the word
"Vera".

This License becomes null and void to the extent applicable to Fonts or Font
Software that has been modified and is distributed under the "Bitstream
Vera" names.

The Font Software may be sold as part of a larger software package but no
copy of one or more of the Font Software typefaces may be sold by itself.

THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF COPYRIGHT, PATENT,
TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL BITSTREAM OR THE GNOME
FOUNDATION BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, INCLUDING
ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL DAMAGES,
WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF
THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM OTHER DEALINGS IN THE
FONT SOFTWARE.

Except as contained in this notice, the names of Gnome, the Gnome
Foundation, and Bitstream Inc., shall not be used in advertising or
otherwise to promote the sale, use or other dealings in this Font Software
without prior written authorization from the Gnome Foundation or Bitstream
Inc., respectively. For further information, contact: fonts at gnome dot
org.

Arev Fonts Copyright
------------------------------

Copyright (c) 2006 by Tavmjong Bah. All Rights Reserved.

Permission is hereby granted, free of charge, to any person obtaining
a copy of the fonts accompanying this license ("Fonts") and
associated documentation files (the "Font Software"), to reproduce
and distribute the modifications to the Bitstream Vera Font Software,
including without limitation the rights to use, copy, merge, publish,
distribute, and/or sell copies of the Font Software, and to permit
persons to whom the Font Software is furnished to do so, subject to
the following conditions:

The above copyright and trademark notices and this permission notice
shall be included in all copies of one or more of the Font Software
typefaces.

The Font Software may be modified, altered, or added to, and in
particular the designs of glyphs or characters in the Fonts may be
modified and additional glyphs or characters may be added to the
Fonts, only if the fonts are renamed to names not containing either
the words "Tavmjong Bah" or the word "Arev".

This License becomes null and void to the extent applicable to Fonts
or Font Software that has been modified and is distributed under the
"Tavmjong Bah Arev" names.

The Font Software may be sold as part of a larger software package but
no copy of one or more of the Font Software typefaces may be sold by
itself.

THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT
OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL
TAVMJONG BAH BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM
OTHER DEALINGS IN THE FONT SOFTWARE.

Except as contained in this notice, the name of Tavmjong Bah shall not
be used in advertising or otherwise to promote the sale, use or other
dealings in this Font Software without prior written authorization
from Tavmjong Bah. For further information, contact: tavmjong @ free
. fr.

TeX Gyre DJV Math
-----------------
Fonts are (c) Bitstream (see below). DejaVu changes are in public domain.

Math extensions done by B. Jackowski, P. Strzelczyk and P. Pianowski
(on behalf of TeX users groups) are in public domain.

Letters imported from Euler Fraktur from AMSfonts are (c) American
Mathematical Society (see below).

AMSFonts (v. 2.2) copyright

The PostScript Type 1 implementation of the AMSFonts produced by and
previously distributed by Blue Sky Research and Y&Y, Inc. are now freely
available for general use. This has been accomplished through the
cooperation of a consortium of scientific publishers with Blue Sky Research
and Y&Y. Members of this consortium include:

Elsevier Science IBM Corporation Society for Industrial and Applied
Mathematics (SIAM) Springer-Verlag American Mathematical Society (AMS)

In order to assure the authenticity of these fonts, copyright will be held
by the American Mathematical Society. This is not meant to restrict in any
way the legitimate use of the fonts, such as (but not limited to)
electronic distribution of documents containing these fonts, inclusion of
these fonts into other public domain or commercial font collections or
computer applications, use of the outline data to create derivative fonts
and/or faces, etc. However, the AMS does require that the AMS copyright
notice be removed from any derivative versions of the fonts which have been
altered in any way. In addition, to ensure the fidelity of TeX documents
using Computer Modern fonts, Professor Donald Knuth, creator of the
Computer Modern faces, has requested that any alterations which yield
different font metrics be given a different name.
DEJAVU_LICENSE_EOF
  show_message "installed DejaVuMathTeXGyre.ttf (DejaVu license, see LICENSE-DejaVu.txt)"
else
  echo "$MESSAGE_PREFIX 'DejaVu Math TeX Gyre' not found via fontconfig — the" \
       "'dejavu-math' abbrev will be omitted (LM Math below is the 'math'" \
       "default regardless; if that is also unavailable, math renders through" \
       "the regular text font, as before Slice B)." >&2
  MATH_SRC=""
fi

# ---- lmsans / lmmono: real Latin Modern Sans / Latin Modern Mono ----------
# GUST e-foundry's pinned "flat" OpenType release zip (see header comment) —
# a single zip holding all 72 Latin Modern OTFs at top level (no subdirs), so
# we extract just the two faces we need. Mirrors the IPAex/Junicode
# `download_file` discipline above — no `fc-match`, hermetic, reproducible on
# any host. Both faces are CFF-outline (`OTTO`) OpenType, verified via the
# sfnt magic (`4f 54 54 4f` = "OTTO") at pin time.
LM_ZIP="Latin_Modern-otf-2_007-31_03_2026.zip"
download_file "$LM_ZIP" \
  "https://www.gust.org.pl/projects/e-foundry/latin-modern/download/$LM_ZIP" \
  "59e1c509c5407c954b76a8aeb68193ef3d3ecf50"
rm -rf "$CACHE/LatinModernOTF"
unzip -o -q "$CACHE/$LM_ZIP" "lmsans10-regular.otf" "lmmono10-regular.otf" -d "$CACHE/LatinModernOTF"
cp "$CACHE/LatinModernOTF/lmsans10-regular.otf" "$FONTS_DIR/lmsans10-regular.otf"
cp "$CACHE/LatinModernOTF/lmmono10-regular.otf" "$FONTS_DIR/lmmono10-regular.otf"
# GUST Font License (GFL) v1.0 — an LPPL-1.3c-equivalent, freely
# redistributable license; embedded verbatim (fetched once at authoring
# time from https://www.gust.org.pl/projects/e-foundry/licenses/GUST-FONT-LICENSE.txt)
# rather than downloaded at run time, so this step has no extra network
# dependency and works offline/in CI, mirroring the DejaVu license below.
cat > "$FONTS_DIR/LICENSE-GUST-FontLicense.txt" <<'GFL_LICENSE_EOF'
% This is version 1.0, dated 22 June 2009, of the GUST Font License.
% (GUST is the Polish TeX Users Group, https://www.gust.org.pl)
%
% For the most recent version of this license see
% https://www.gust.org.pl/fonts/licenses/GUST-FONT-LICENSE.txt
% or
% https://tug.org/fonts/licenses/GUST-FONT-LICENSE.txt
%
% This work may be distributed and/or modified under the conditions
% of the LaTeX Project Public License, either version 1.3c of this
% license or (at your option) any later version.
%
% Please also observe the following clause:
% 1) it is requested, but not legally required, that derived works be
%    distributed only after changing the names of the fonts comprising this
%    work and given in an accompanying "manifest", and that the
%    files comprising the Work, as listed in the manifest, also be given
%    new names. Any exceptions to this request are also given in the
%    manifest.
%
%    We recommend the manifest be given in a separate file named
%    MANIFEST-<fontid>.txt, where <fontid> is some unique identification
%    of the font family. If a separate "readme" file accompanies the Work,
%    we recommend a name of the form README-<fontid>.txt.
%
% The latest version of the LaTeX Project Public License is in
% https://www.latex-project.org/lppl.txt and version 1.3c or later
% is part of all distributions of LaTeX version 2006/05/20 or later.
GFL_LICENSE_EOF
LMSANS_SRC="dist/fonts/lmsans10-regular.otf"
LMMONO_SRC="dist/fonts/lmmono10-regular.otf"
show_message "installed lmsans10-regular.otf / lmmono10-regular.otf (real Latin Modern, GUST Font License)"

# ---- lmmath: real Latin Modern Math (see header comment) ------------------
# GUST e-foundry's own pinned release zip for the LM Math companion face —
# a single face (`otf/latinmodern-math.otf`) under a versioned top-level
# directory, unlike the flat lmsans/lmmono zip above. Same
# `download_file`/hermetic discipline — no `fc-match`, reproducible on any
# host. CFF-outline (`OTTO`) OpenType, verified via the sfnt magic
# (`4f 54 54 4f` = "OTTO") plus a `MATH` table at pin time (see header
# comment). Reuses the GFL license file written just above (same license,
# same upstream GUST e-foundry project) — no separate license file needed.
LM_MATH_ZIP="latinmodern-math-1959.zip"
download_file "$LM_MATH_ZIP" \
  "https://www.gust.org.pl/projects/e-foundry/lm-math/download/$LM_MATH_ZIP" \
  "cb2cf7ef2c366f2db384a77741a85887409ff39e"
rm -rf "$CACHE/LatinModernMathOTF"
unzip -o -q "$CACHE/$LM_MATH_ZIP" "latinmodern-math-1959/otf/latinmodern-math.otf" \
  -d "$CACHE/LatinModernMathOTF"
cp "$CACHE/LatinModernMathOTF/latinmodern-math-1959/otf/latinmodern-math.otf" \
  "$FONTS_DIR/latinmodern-math.otf"
if [ -f "$FONTS_DIR/latinmodern-math.otf" ]; then
  LMMATH_SRC="dist/fonts/latinmodern-math.otf"
  show_message "installed latinmodern-math.otf (real Latin Modern Math, GUST Font License)"
else
  # Defensive fallback only — download_file already hard-fails the whole
  # script on a checksum mismatch or network error, same as IPAex/Junicode/
  # lmsans/lmmono above, so this branch should be unreachable in practice.
  # Kept so the JSON-writing gate below degrades gracefully (`"math"` left
  # unset) rather than referencing a file that doesn't exist, matching the
  # DejaVu-absent case.
  LMMATH_SRC=""
fi

# ---- Write dist/hash/*.rustyfi-hash (this port's plain-JSON schema) ------
{
  printf '{\n'
  printf '  "ipaexm":     { "src": "dist/fonts/ipaexm.ttf" },\n'
  printf '  "ipaexg":     { "src": "dist/fonts/ipaexg.ttf" },\n'
  printf '  "Junicode":   { "src": "dist/fonts/Junicode.ttf" },\n'
  printf '  "Junicode-b": { "src": "dist/fonts/Junicode-Bold.ttf" },\n'
  printf '  "Junicode-it":{ "src": "dist/fonts/Junicode-Italic.ttf" },\n'
  printf '  "Junicode-bi":{ "src": "dist/fonts/Junicode-BoldItalic.ttf" },\n'
  printf '  "lmsans":     { "src": "%s" },\n' "$LMSANS_SRC"
  printf '  "lmmono":     { "src": "%s" }' "$LMMONO_SRC"
  if [ -n "$LMMATH_SRC" ]; then
    printf ',\n  "lmmath":     { "src": "%s" }' "$LMMATH_SRC"
    # `lmodern` is the abbrev stdja/std-ja actually name (`set-math-font
    # \`lmodern\``, `FontLatinModernMath.main`) — alias it to the real LM
    # Math face now that it is bundled (Slice 0); previously aliased to the
    # DejaVu Math stand-in.
    printf ',\n  "lmodern":    { "src": "%s" }' "$LMMATH_SRC"
  fi
  if [ -n "$MATH_SRC" ]; then
    printf ',\n  "dejavu-math":{ "src": "dist/fonts/DejaVuMathTeXGyre.ttf" }'
  fi
  printf '\n}\n'
} > "$HASH_DIR/fonts.rustyfi-hash"

{
  printf '{\n'
  printf '  "regular": "Junicode", "bold": "Junicode-b", "oblique": "Junicode-it",\n'
  # LM Math is the upstream-correct default when present; DejaVu Math TeX
  # Gyre (glyf stand-in) is kept as a fallback only if LM Math is somehow
  # unavailable. If neither is present, "math" stays unset entirely
  # (Context::math_font stays FontKey(0), pre-Slice-B behavior).
  if [ -n "$LMMATH_SRC" ]; then
    printf '  "math": "lmmath",\n'
  elif [ -n "$MATH_SRC" ]; then
    printf '  "math": "dejavu-math",\n'
  fi
  printf '  "scripts": {\n'
  printf '    "han-ideographic": { "font-name": "ipaexm",   "ratio": 0.88, "rising": 0.0 },\n'
  printf '    "kana":            { "font-name": "ipaexm",   "ratio": 0.88, "rising": 0.0 },\n'
  printf '    "latin":           { "font-name": "Junicode", "ratio": 1.0,  "rising": 0.0 },\n'
  printf '    "other-script":    { "font-name": "Junicode", "ratio": 1.0,  "rising": 0.0 }\n'
  printf '  }\n'
  printf '}\n'
} > "$HASH_DIR/default-font.rustyfi-hash"

show_message "wrote $HASH_DIR/fonts.rustyfi-hash and default-font.rustyfi-hash"
show_message "end"
