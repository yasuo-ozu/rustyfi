import { test } from 'node:test';
import assert from 'node:assert/strict';
import { renderMarkdown, renderInline, sanitizeSvg, escapeHtml } from '../core/markdown';

test('headings, paragraphs and lists render', () => {
  const h = renderMarkdown('# Title\n\ntext here\n\n- a\n- b\n');
  assert.match(h, /<h1>Title<\/h1>/);
  assert.match(h, /<p>text here<\/p>/);
  assert.match(h, /<ul>\n<li>a<\/li>\n<li>b<\/li>\n<\/ul>/);
});

test('ordered lists render as <ol>', () => {
  assert.match(renderMarkdown('1. one\n2. two\n'), /<ol>[\s\S]*<li>one<\/li>[\s\S]*<\/ol>/);
});

test('fenced code is escaped and not interpreted as markup', () => {
  const h = renderMarkdown('```rust\nlet x = <b>&1;\n```\n');
  assert.match(h, /<pre><code class="language-rust">/);
  assert.match(h, /&lt;b&gt;&amp;1;/);
  assert.ok(!h.includes('<b>'));
});

test('inline code, emphasis, strong and links', () => {
  assert.match(renderInline('`a<b>`'), /<code>a&lt;b&gt;<\/code>/);
  assert.match(renderInline('**bold**'), /<strong>bold<\/strong>/);
  assert.match(renderInline('*it*'), /<em>it<\/em>/);
  assert.match(renderInline('[t](https://e.org)'), /<a href="https:\/\/e\.org">t<\/a>/);
});

// --- escaping / injection --------------------------------------------------

test('raw HTML in document text is escaped, not emitted', () => {
  const h = renderMarkdown('<img src=x onerror=alert(1)>\n');
  assert.ok(!/<img/i.test(h), 'an img tag must not survive');
  assert.match(h, /&lt;img/);
});

test('a javascript: link is dropped, keeping only its label', () => {
  const h = renderInline('[click](javascript:alert(1))');
  assert.ok(!/href/.test(h));
  assert.match(h, /click/);
});

test('escapeHtml covers the five significant characters', () => {
  assert.equal(escapeHtml(`<>&"'`), '&lt;&gt;&amp;&quot;&#39;');
});

// --- SVG sanitizer ---------------------------------------------------------

test('a plain figure SVG survives sanitizing', () => {
  const svg = '<svg viewBox="0 0 10 10"><g><path d="M0 0 L1 1" fill="#333"/></g></svg>';
  const out = sanitizeSvg(svg);
  assert.ok(out !== null);
  assert.match(out!, /<svg viewbox="0 0 10 10">/);
  assert.match(out!, /<path d="M0 0 L1 1" fill="#333"\/>/);
});

test('a <script> inside an SVG is removed with its contents', () => {
  const out = sanitizeSvg('<svg><script>alert(1)</script><path d="M0 0"/></svg>');
  assert.ok(out !== null);
  assert.ok(!/script/i.test(out!), 'script tag must be gone');
  assert.ok(!/alert/.test(out!), 'script body must be gone too');
  assert.match(out!, /<path/);
});

test('event-handler attributes are stripped', () => {
  const out = sanitizeSvg('<svg><path d="M0 0" onload="alert(1)" onclick="x()"/></svg>');
  assert.ok(out !== null);
  assert.ok(!/onload|onclick|alert/i.test(out!));
  assert.match(out!, /d="M0 0"/);
});

test('foreignObject and image are dropped (they can load or script)', () => {
  const a = sanitizeSvg('<svg><foreignObject><b>hi</b></foreignObject><path d="M1"/></svg>');
  assert.ok(a !== null && !/foreignObject|<b>/i.test(a));
  const b = sanitizeSvg('<svg><image href="https://evil/x.png"/><path d="M1"/></svg>');
  assert.ok(b !== null && !/<image|evil/i.test(b!));
});

test('clip-path may reference a local fragment but not a remote url', () => {
  const ok = sanitizeSvg('<svg><path d="M1" clip-path="url(#c1)"/></svg>');
  assert.match(ok!, /clip-path="url\(#c1\)"/);
  const bad = sanitizeSvg('<svg><path d="M1" clip-path="url(https://evil/x)"/></svg>');
  assert.ok(!/evil/.test(bad!));
});

test('SVG text content is escaped', () => {
  const out = sanitizeSvg('<svg><text>a &lt; b <c></text></svg>');
  assert.ok(out !== null);
  assert.ok(!/<c>/.test(out!));
});

test('mismatched SVG nesting is refused rather than half-rebuilt', () => {
  assert.equal(sanitizeSvg('<svg><g><path d="M1"/></svg>'), null);
});

test('an SVG block in a markdown document becomes a figure', () => {
  const h = renderMarkdown('text\n\n<svg viewBox="0 0 4 4"><path d="M0 0"/></svg>\n\nmore\n');
  assert.match(h, /<figure class="rustyfi-figure"><svg/);
  assert.match(h, /<p>text<\/p>/);
  assert.match(h, /<p>more<\/p>/);
});

test('an unsanitizable SVG block degrades to escaped source, not to injection', () => {
  const h = renderMarkdown('<svg><g><path d="M1"/></svg>\n');
  assert.match(h, /rustyfi-unsafe/);
  assert.match(h, /&lt;svg/);
});

test('a multi-line SVG figure is consumed whole', () => {
  const h = renderMarkdown('<svg viewBox="0 0 4 4">\n<path d="M0 0"/>\n</svg>\n\nafter\n');
  assert.match(h, /<figure/);
  assert.match(h, /<p>after<\/p>/);
  assert.ok(!/&lt;\/svg&gt;/.test(h));
});
