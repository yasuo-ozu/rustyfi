/**
 * A small Markdown -> HTML renderer for the preview panel.
 *
 * Why not a dependency: the input is not arbitrary Markdown off the internet,
 * it is the output of `rustyfi --format markdown`, whose constructs are a
 * known and small set (headings, paragraphs, fenced code, bullet and ordered
 * lists, blockquotes, rules, links, emphasis, inline code) plus RAW INLINE SVG
 * for figures.  That last part is the reason a stock renderer would not have
 * been a drop-in anyway: it has to be sanitized, not passed through, because
 * the glyphs and `<text>` labels inside it come from the user's document.
 * Pulling in a parser and then still hand-writing the sanitizer is more
 * supply chain for less control, so this renders the subset directly.
 *
 * Everything that is not explicitly recognised is HTML-escaped.  Combined
 * with the webview's CSP (no inline script beyond one nonce, no remote
 * origins at all) that is the whole security story.
 */

const ESC: Record<string, string> = {
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
};

export function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ESC[c]);
}

// ---------------------------------------------------------------------------
// SVG sanitizer
// ---------------------------------------------------------------------------

const SVG_TAGS = new Set([
  'svg', 'g', 'path', 'text', 'tspan', 'rect', 'line', 'circle', 'ellipse',
  'polyline', 'polygon', 'defs', 'clippath', 'title', 'desc', 'marker',
]);

const SVG_ATTRS = new Set([
  'd', 'x', 'y', 'dx', 'dy', 'x1', 'y1', 'x2', 'y2', 'cx', 'cy', 'r', 'rx', 'ry',
  'width', 'height', 'viewbox', 'transform', 'fill', 'stroke', 'stroke-width',
  'stroke-linecap', 'stroke-linejoin', 'stroke-dasharray', 'stroke-opacity',
  'opacity', 'fill-opacity', 'fill-rule', 'clip-rule', 'font-family',
  'font-size', 'font-weight', 'font-style', 'text-anchor', 'dominant-baseline',
  'points', 'class', 'id', 'xmlns', 'preserveaspectratio', 'clip-path',
  'letter-spacing', 'word-spacing', 'writing-mode', 'baseline-shift',
]);

/** `url(#local)` is needed for clip-path; anything else that can fetch is not. */
function safeAttrValue(name: string, value: string): boolean {
  const v = value.trim().toLowerCase();
  if (name.startsWith('on')) return false;
  if (v.includes('javascript:') || v.includes('data:text/html')) return false;
  if (v.includes('url(')) return /^url\(\s*['"]?#[A-Za-z0-9_.:-]+['"]?\s*\)$/.test(v);
  if (v.includes('<')) return false;
  return true;
}

/**
 * Rebuild an SVG fragment from an allowlist.  Anything unrecognised is
 * dropped (tags) or omitted (attributes); text nodes are escaped.  Returns
 * null when the fragment is too malformed to trust, in which case the caller
 * shows it as escaped source instead of guessing.
 */
export function sanitizeSvg(fragment: string): string | null {
  let out = '';
  let i = 0;
  const stack: string[] = [];
  const tagRe = /<\/?([A-Za-z][A-Za-z0-9:-]*)((?:[^>"']|"[^"]*"|'[^']*')*?)(\/?)>/g;
  tagRe.lastIndex = 0;
  let m: RegExpExecArray | null;

  while ((m = tagRe.exec(fragment)) !== null) {
    // text between the previous tag and this one
    out += escapeHtml(fragment.slice(i, m.index));
    i = m.index + m[0].length;

    const raw = m[0];
    const name = m[1].toLowerCase();
    const isClose = raw.startsWith('</');
    const selfClose = m[3] === '/';

    if (!SVG_TAGS.has(name)) {
      // Unknown element: drop the tag, keep nothing of it.  `script`,
      // `foreignObject`, `image`, `style` all land here.
      if (!isClose && !selfClose) {
        // skip its whole subtree so its text does not leak out as content
        const close = new RegExp(`</\\s*${name}\\s*>`, 'i');
        const rest = fragment.slice(i);
        const cm = close.exec(rest);
        if (cm) i += cm.index + cm[0].length;
      }
      continue;
    }

    if (isClose) {
      if (stack.length === 0 || stack[stack.length - 1] !== name) return null;
      stack.pop();
      out += `</${name}>`;
      continue;
    }

    const attrs: string[] = [];
    const attrRe = /([A-Za-z_:][-A-Za-z0-9_:.]*)\s*=\s*("([^"]*)"|'([^']*)')/g;
    let a: RegExpExecArray | null;
    while ((a = attrRe.exec(m[2])) !== null) {
      const an = a[1].toLowerCase();
      const av = a[3] !== undefined ? a[3] : a[4] ?? '';
      if (!SVG_ATTRS.has(an)) continue;
      if (!safeAttrValue(an, av)) continue;
      attrs.push(`${an}="${escapeHtml(av)}"`);
    }

    const open = `<${name}${attrs.length ? ' ' + attrs.join(' ') : ''}`;
    if (selfClose) {
      out += open + '/>';
    } else {
      stack.push(name);
      out += open + '>';
    }
  }

  out += escapeHtml(fragment.slice(i));
  if (stack.length !== 0) return null;
  return out;
}

// ---------------------------------------------------------------------------
// Inline markdown
// ---------------------------------------------------------------------------

/** Only these schemes may appear in a rendered link. */
function safeHref(url: string): string | null {
  const u = url.trim();
  if (/^(https?:|mailto:)/i.test(u)) return u;
  if (/^#/.test(u)) return u;
  return null;
}

export function renderInline(src: string): string {
  let out = '';
  let i = 0;
  const n = src.length;

  while (i < n) {
    const c = src[i];

    // inline code — highest precedence, nothing inside it is markup
    if (c === '`') {
      let ticks = 0;
      while (i + ticks < n && src[i + ticks] === '`') ticks++;
      const fence = '`'.repeat(ticks);
      const close = src.indexOf(fence, i + ticks);
      if (close !== -1) {
        out += `<code>${escapeHtml(src.slice(i + ticks, close))}</code>`;
        i = close + ticks;
        continue;
      }
    }

    // link  [text](href)
    if (c === '[') {
      const m = /^\[([^\]]*)\]\(([^)\s]*)\)/.exec(src.slice(i));
      if (m) {
        const href = safeHref(m[2]);
        const label = renderInline(m[1]);
        out += href ? `<a href="${escapeHtml(href)}">${label}</a>` : label;
        i += m[0].length;
        continue;
      }
    }

    // strong then emphasis
    if (c === '*' || c === '_') {
      const two = src.slice(i, i + 2);
      if ((two === '**' || two === '__')) {
        const close = src.indexOf(two, i + 2);
        if (close !== -1) {
          out += `<strong>${renderInline(src.slice(i + 2, close))}</strong>`;
          i = close + 2;
          continue;
        }
      }
      const close = src.indexOf(c, i + 1);
      if (close !== -1 && close > i + 1) {
        out += `<em>${renderInline(src.slice(i + 1, close))}</em>`;
        i = close + 1;
        continue;
      }
    }

    out += ESC[c] ?? c;
    i++;
  }
  return out;
}

// ---------------------------------------------------------------------------
// Block markdown
// ---------------------------------------------------------------------------

interface ListState { tag: 'ul' | 'ol'; }

export function renderMarkdown(src: string): string {
  const lines = src.split(/\r?\n/);
  const out: string[] = [];
  const lists: ListState[] = [];
  let para: string[] = [];

  const closeLists = () => { while (lists.length) out.push(`</${lists.pop()!.tag}>`); };
  const flushPara = () => {
    if (para.length) {
      out.push(`<p>${renderInline(para.join(' '))}</p>`);
      para = [];
    }
  };
  const flushAll = () => { flushPara(); closeLists(); };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // ---- raw inline SVG (a figure) ------------------------------------
    if (/^\s*<svg[\s>]/i.test(line)) {
      flushAll();
      let depth = 0;
      const buf: string[] = [];
      let j = i;
      for (; j < lines.length; j++) {
        buf.push(lines[j]);
        depth += (lines[j].match(/<svg[\s>]/gi) ?? []).length;
        depth -= (lines[j].match(/<\/svg\s*>/gi) ?? []).length;
        if (depth <= 0) break;
      }
      i = j;
      const frag = buf.join('\n');
      const clean = sanitizeSvg(frag);
      out.push(
        clean !== null
          ? `<figure class="rustyfi-figure">${clean}</figure>`
          : `<pre class="rustyfi-unsafe"><code>${escapeHtml(frag)}</code></pre>`,
      );
      continue;
    }

    // ---- fenced code ---------------------------------------------------
    const fence = /^\s*(`{3,}|~{3,})\s*([A-Za-z0-9_+-]*)\s*$/.exec(line);
    if (fence) {
      flushAll();
      const marker = fence[1][0].repeat(fence[1].length);
      const lang = fence[2];
      const body: string[] = [];
      let j = i + 1;
      for (; j < lines.length; j++) {
        if (new RegExp(`^\\s*${marker[0]}{${marker.length},}\\s*$`).test(lines[j])) break;
        body.push(lines[j]);
      }
      i = j;
      const cls = lang ? ` class="language-${escapeHtml(lang)}"` : '';
      out.push(`<pre><code${cls}>${escapeHtml(body.join('\n'))}</code></pre>`);
      continue;
    }

    // ---- blank ---------------------------------------------------------
    if (/^\s*$/.test(line)) { flushPara(); continue; }

    // ---- heading -------------------------------------------------------
    const h = /^(#{1,6})\s+(.*)$/.exec(line);
    if (h) {
      flushAll();
      const lvl = h[1].length;
      out.push(`<h${lvl}>${renderInline(h[2].trim())}</h${lvl}>`);
      continue;
    }

    // ---- horizontal rule ------------------------------------------------
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) { flushAll(); out.push('<hr/>'); continue; }

    // ---- blockquote ------------------------------------------------------
    const bq = /^\s*>\s?(.*)$/.exec(line);
    if (bq) {
      flushAll();
      const body: string[] = [bq[1]];
      let j = i + 1;
      for (; j < lines.length; j++) {
        const m2 = /^\s*>\s?(.*)$/.exec(lines[j]);
        if (!m2) break;
        body.push(m2[1]);
      }
      i = j - 1;
      out.push(`<blockquote>${renderMarkdown(body.join('\n'))}</blockquote>`);
      continue;
    }

    // ---- list items -------------------------------------------------------
    const ul = /^(\s*)[-*+]\s+(.*)$/.exec(line);
    const ol = /^(\s*)\d+[.)]\s+(.*)$/.exec(line);
    if (ul || ol) {
      flushPara();
      const want: 'ul' | 'ol' = ul ? 'ul' : 'ol';
      if (lists.length === 0 || lists[lists.length - 1].tag !== want) {
        closeLists();
        lists.push({ tag: want });
        out.push(`<${want}>`);
      }
      out.push(`<li>${renderInline((ul ?? ol)![2])}</li>`);
      continue;
    }

    // ---- paragraph text ----------------------------------------------------
    closeLists();
    para.push(line.trim());
  }

  flushAll();
  return out.join('\n');
}
