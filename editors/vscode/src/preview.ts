import * as vscode from 'vscode';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as crypto from 'crypto';
import {
  buildPreviewArgs,
  humanizeDiagnostic,
  outputExtension,
  type MathMode,
  type PreviewFormat,
} from './core/previewArgs';
import { renderMarkdown } from './core/markdown';
import { findBinary } from './binary';
import { run, type RunHandle } from './run';

function nonce(): string {
  return crypto.randomBytes(16).toString('base64');
}

/**
 * One preview panel, pinned to the document it was opened from.
 *
 * Two design points worth stating, because both are failure modes the user
 * feels immediately:
 *
 * 1. THE LAST GOOD RENDER IS NEVER THROWN AWAY.  A document under the cursor
 *    is syntactically broken most of the time, so a preview that blanks on a
 *    failed compile is blank most of the time.  A failure updates an error
 *    banner and nothing else; the body keeps the last successful render.
 *
 * 2. THE BODY IS PATCHED, NOT RELOADED.  The webview's HTML shell is set once
 *    and re-renders arrive as postMessage payloads, so the scroll position
 *    survives (reassigning `webview.html` would reload the document and jump
 *    to the top on every keystroke).
 */
export class Preview {
  private static readonly VIEW_TYPE = 'rustyfi.preview';
  private static open = new Map<string, Preview>();

  private panel: vscode.WebviewPanel;
  private disposables: vscode.Disposable[] = [];
  private inflight: RunHandle | undefined;
  private timer: NodeJS.Timeout | undefined;
  private tempSource: string | undefined;
  private tempDir: string;
  private disposed = false;
  private generation = 0;

  static show(
    doc: vscode.TextDocument,
    out: vscode.OutputChannel,
    mediaRoot: vscode.Uri,
  ): Preview {
    const key = doc.uri.toString();
    const existing = Preview.open.get(key);
    if (existing) {
      existing.panel.reveal(vscode.ViewColumn.Beside, true);
      return existing;
    }
    const p = new Preview(doc, out, mediaRoot);
    Preview.open.set(key, p);
    return p;
  }

  static forDocument(doc: vscode.TextDocument): Preview | undefined {
    return Preview.open.get(doc.uri.toString());
  }

  static disposeAll(): void {
    for (const p of [...Preview.open.values()]) p.dispose();
  }

  private constructor(
    private readonly doc: vscode.TextDocument,
    private readonly out: vscode.OutputChannel,
    private readonly mediaRoot: vscode.Uri,
  ) {
    this.tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'rustyfi-preview-'));

    this.panel = vscode.window.createWebviewPanel(
      Preview.VIEW_TYPE,
      `Preview: ${path.basename(doc.uri.fsPath || doc.uri.path)}`,
      { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true },
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        // `media/` only: the pdf.js library and its worker. Still no remote
        // origin anywhere, and nothing else on disk is reachable.
        localResourceRoots: [mediaRoot],
      },
    );

    this.panel.webview.html = this.shell();
    this.panel.onDidDispose(() => this.dispose(), null, this.disposables);

    this.disposables.push(
      vscode.workspace.onDidChangeTextDocument((e) => {
        if (e.document.uri.toString() !== this.doc.uri.toString()) return;
        if (!vscode.workspace.getConfiguration('rustyfi').get<boolean>('preview.enable', true)) return;
        this.schedule();
      }),
    );

    this.disposables.push(
      vscode.workspace.onDidCloseTextDocument((d) => {
        if (d.uri.toString() === this.doc.uri.toString()) this.dispose();
      }),
    );

    this.render(); // first render is immediate, not debounced
  }

  /** Debounced by `rustyfi.preview.debounce` (default 300 ms). */
  private schedule(): void {
    const ms = vscode.workspace
      .getConfiguration('rustyfi')
      .get<number>('preview.debounce', 300);
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => this.render(), Math.max(0, ms));
  }

  refresh(): void { this.render(); }

  private post(msg: unknown): void {
    if (!this.disposed) void this.panel.webview.postMessage(msg);
  }

  private async render(): Promise<void> {
    if (this.disposed) return;

    const bin = findBinary(this.out);
    if (!bin) { this.post({ type: 'error', message: 'rustyfi binary not found.' }); return; }

    // Supersede any compile still running: the newest keystroke wins and the
    // older process is killed rather than left to finish into the void.
    if (this.inflight) { this.inflight.cancel(); this.inflight = undefined; }

    const gen = ++this.generation;
    this.post({ type: 'busy' });

    const text = this.doc.getText();
    let inputPath: string;
    let cwd: string | undefined;

    if (this.doc.uri.scheme === 'file') {
      // The temp source is a SIBLING of the real document.  That is the whole
      // trick for `@require:`/`@import:`: relative resolution is done from the
      // importing file's own directory, so a temp file in os.tmpdir() would
      // fail to resolve `@import: ../src/foo` while a sibling resolves it
      // exactly as the real file does.  The leading dot keeps it out of the
      // way, and it is removed on dispose.
      const dir = path.dirname(this.doc.uri.fsPath);
      const ext = path.extname(this.doc.uri.fsPath) || '.saty';
      const stamp = crypto.createHash('sha1').update(this.doc.uri.fsPath).digest('hex').slice(0, 8);
      inputPath = path.join(dir, `.rustyfi-preview-${stamp}${ext}`);
      cwd = dir;
    } else {
      // An untitled buffer has no directory, so relative imports cannot work.
      inputPath = path.join(this.tempDir, 'untitled.saty');
      cwd = this.tempDir;
      if (/^\s*@(require|import)\s*:/m.test(text)) {
        this.post({
          type: 'note',
          message:
            'This buffer is untitled, so it has no directory: relative @import: paths will not resolve until you save it.',
        });
      }
    }

    try {
      fs.writeFileSync(inputPath, text, 'utf8');
      this.tempSource = inputPath;
    } catch (e) {
      this.post({ type: 'error', message: `Could not write the preview temp file: ${e}` });
      return;
    }

    const cfg = vscode.workspace.getConfiguration('rustyfi');
    const format = cfg.get<PreviewFormat>('preview.format', 'pdf');
    const outPath = path.join(this.tempDir, `preview${outputExtension(format)}`);
    const args = buildPreviewArgs({
      inputPath,
      format,
      outputPath: outPath,
      // Keep the cross-reference aux file in the temp dir so the preview never
      // drops a `.satysfi-aux` beside the user's document, nor clobbers the
      // one a real build wrote there.
      auxPath: path.join(this.tempDir, 'preview.satysfi-aux'),
      mathMode: cfg.get<MathMode>('preview.mathMode', 'unicode-math'),
      libRoot: cfg.get<string>('libRoot', ''),
    });

    const started = Date.now();
    const handle = run(bin.path, args, {
      cwd,
      timeoutMs: cfg.get<number>('preview.timeout', 30000),
    });
    this.inflight = handle;

    let res;
    try {
      res = await handle.result;
    } catch (e) {
      if (gen === this.generation) this.post({ type: 'error', message: `Could not run rustyfi: ${e}` });
      return;
    } finally {
      if (this.inflight === handle) this.inflight = undefined;
    }

    // A newer render started (or the panel closed) while this one was
    // running: discard the result rather than letting a slow compile
    // overwrite a fresher one.
    if (gen !== this.generation || this.disposed) return;

    const ms = Date.now() - started;

    if (res.code !== 0) {
      const realName = path.basename(this.doc.uri.fsPath || this.doc.uri.path);
      const detail = humanizeDiagnostic(
        (res.stderr || res.stdout).trim().split('\n').slice(-8).join('\n'),
        inputPath,
        realName,
      );
      this.out.appendLine(`[rustyfi] preview compile exited ${res.code} in ${ms}ms`);
      // Body untouched -- the last good render stays on screen.
      this.post({ type: 'error', message: detail || `rustyfi exited with code ${res.code}` });
      return;
    }

    if (format === 'pdf') {
      let bytes: Buffer;
      try {
        bytes = fs.readFileSync(outPath);
      } catch (e) {
        this.post({ type: 'error', message: `Compile succeeded but no PDF was produced: ${e}` });
        return;
      }
      // Base64 over postMessage rather than a file URI: the PDF lives in a
      // temp dir outside `localResourceRoots`, and widening those to a
      // mutable temp directory to save one copy is a worse trade than the
      // copy. Structured clone would take a Uint8Array, but the webview's
      // message channel serialises to JSON, so base64 it is.
      this.out.appendLine(`[rustyfi] preview compiled in ${ms}ms (${bytes.length} bytes of PDF)`);
      this.post({ type: 'pdf', data: bytes.toString('base64'), ms });
      return;
    }

    let md: string;
    try {
      md = fs.readFileSync(outPath, 'utf8');
    } catch (e) {
      this.post({ type: 'error', message: `Compile succeeded but no output was produced: ${e}` });
      return;
    }

    this.out.appendLine(`[rustyfi] preview compiled in ${ms}ms (${md.length} bytes of Markdown)`);
    this.post({ type: 'render', html: renderMarkdown(md), ms });
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    Preview.open.delete(this.doc.uri.toString());

    if (this.timer) clearTimeout(this.timer);
    if (this.inflight) this.inflight.cancel();
    this.inflight = undefined;

    for (const d of this.disposables) { try { d.dispose(); } catch { /* ignore */ } }
    this.disposables = [];

    if (this.tempSource) { try { fs.unlinkSync(this.tempSource); } catch { /* ignore */ } }
    try { fs.rmSync(this.tempDir, { recursive: true, force: true }); } catch { /* ignore */ }

    try { this.panel.dispose(); } catch { /* ignore */ }
  }

  /**
   * The static shell.  Set once; every re-render is a postMessage that patches
   * `#content`, which is what preserves the scroll position.
   *
   * CSP: `default-src 'none'` and no remote origin anywhere -- nothing is
   * fetched from a CDN, the styles are inline and the only script is the one
   * carrying this nonce.
   */
  private shell(): string {
    const n = nonce();
    const w = this.panel.webview;
    const pdfLib = w.asWebviewUri(vscode.Uri.joinPath(this.mediaRoot, 'pdf.min.mjs'));
    const pdfWorker = w.asWebviewUri(vscode.Uri.joinPath(this.mediaRoot, 'pdf.worker.min.mjs'));
    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta http-equiv="Content-Security-Policy"
      content="default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-${n}' ${w.cspSource}; worker-src ${w.cspSource} blob:; img-src data: blob:;"/>
<style>
  :root { color-scheme: light dark; }
  body {
    font-family: var(--vscode-editor-font-family, ui-serif, Georgia, serif);
    font-size: 15px; line-height: 1.7;
    color: var(--vscode-editor-foreground, #ddd);
    background: var(--vscode-editor-background, #1e1e1e);
    margin: 0; padding: 0 2rem 4rem;
  }
  #content { max-width: 46rem; margin: 0 auto; padding-top: 2.5rem; }
  h1,h2,h3,h4,h5,h6 { line-height: 1.3; margin: 1.8em 0 .6em; font-weight: 600; }
  h1 { font-size: 1.8em; } h2 { font-size: 1.45em; } h3 { font-size: 1.2em; }
  p { margin: 0 0 1em; }
  code, pre { font-family: var(--vscode-editor-font-family, ui-monospace, monospace); }
  code { font-size: .9em; background: rgba(127,127,127,.18); padding: .1em .3em; border-radius: 3px; }
  pre { background: rgba(127,127,127,.13); padding: .8em 1em; border-radius: 5px; overflow-x: auto; }
  pre code { background: none; padding: 0; }
  blockquote { margin: 1em 0; padding: .2em 1em; border-left: 3px solid rgba(127,127,127,.5); opacity: .9; }
  hr { border: none; border-top: 1px solid rgba(127,127,127,.4); margin: 2em 0; }
  a { color: var(--vscode-textLink-foreground, #4daafc); }
  ul, ol { padding-left: 1.5em; }
  figure.rustyfi-figure { margin: 1.2em 0; text-align: center; overflow-x: auto; }
  figure.rustyfi-figure svg { max-width: 100%; height: auto; }
  pre.rustyfi-unsafe { outline: 1px dashed rgba(220,120,60,.7); }
  #banner {
    position: sticky; top: 0; z-index: 10;
    display: none; white-space: pre-wrap;
    font-family: var(--vscode-editor-font-family, ui-monospace, monospace);
    font-size: 12px; line-height: 1.5;
    padding: .55em .9em;
    background: var(--vscode-inputValidation-errorBackground, #5a1d1d);
    color: var(--vscode-inputValidation-errorForeground, #f4d5d5);
    border-bottom: 1px solid var(--vscode-inputValidation-errorBorder, #be1100);
  }
  #banner.note { background: var(--vscode-inputValidation-warningBackground, #4d3800);
                 color: var(--vscode-inputValidation-warningForeground, #f2e0b0);
                 border-bottom-color: var(--vscode-inputValidation-warningBorder, #b89500); }
  #banner .tag { font-weight: 700; margin-right: .5em; text-transform: uppercase; letter-spacing: .04em; }
  #spinner {
    position: fixed; top: .6rem; right: .9rem; z-index: 20;
    width: .55rem; height: .55rem; border-radius: 50%;
    background: var(--vscode-progressBar-background, #4daafc);
    opacity: 0; transition: opacity .15s;
  }
  #spinner.on { opacity: .85; }
  #placeholder { opacity: .55; font-style: italic; padding-top: 3rem; text-align: center; }
  /* PDF mode: pages stacked, each a canvas scaled to the panel width. The
     page keeps a light ground in BOTH themes -- a PDF is ink on paper and
     inverting it would misrepresent what the build produces. */
  body.pdf #content { max-width: none; padding-top: 1rem; }
  .pdf-page {
    display: block; margin: 0 auto 1rem; max-width: 100%;
    background: #fff; box-shadow: 0 1px 6px rgba(0,0,0,.45);
  }
</style>
</head>
<body>
<div id="banner"></div>
<div id="spinner"></div>
<div id="content"><div id="placeholder">Compiling preview…</div></div>
<script type="module" nonce="${n}">
(function () {
  const vscode = acquireVsCodeApi();
  const PDF_LIB = "${pdfLib}";
  const PDF_WORKER = "${pdfWorker}";
  const content = document.getElementById('content');
  const banner  = document.getElementById('banner');
  const spinner = document.getElementById('spinner');

  // Restore the scroll offset across a webview reload (VS Code may recreate
  // the view when it is hidden and shown again).
  const prev = vscode.getState();
  if (prev && prev.html) {
    content.innerHTML = prev.html;
    if (typeof prev.scroll === 'number') window.scrollTo(0, prev.scroll);
  }

  let saveTimer = null;
  window.addEventListener('scroll', function () {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(function () {
      const s = vscode.getState() || {};
      s.scroll = window.scrollY;
      vscode.setState(s);
    }, 120);
  }, { passive: true });

  function showBanner(kind, text) {
    banner.className = kind === 'note' ? 'note' : '';
    banner.innerHTML = '';
    const tag = document.createElement('span');
    tag.className = 'tag';
    tag.textContent = kind === 'note' ? 'note' : 'error';
    banner.appendChild(tag);
    banner.appendChild(document.createTextNode(text));
    banner.style.display = 'block';
  }

  window.addEventListener('message', function (event) {
    const m = event.data;
    if (!m) return;

    if (m.type === 'busy') { spinner.classList.add('on'); return; }

    if (m.type === 'error' || m.type === 'note') {
      spinner.classList.remove('on');
      // The body is deliberately left alone: the last good render stays up.
      showBanner(m.type, m.message);
      return;
    }

    if (m.type === 'render') {
      spinner.classList.remove('on');
      banner.style.display = 'none';
      document.body.classList.remove('pdf');
      // Preserve the scroll offset across the patch.
      const y = window.scrollY;
      content.innerHTML = m.html;
      window.scrollTo(0, y);
      const s = vscode.getState() || {};
      s.html = m.html; s.scroll = y;
      vscode.setState(s);
      return;
    }

    if (m.type === 'pdf') {
      renderPdf(m.data);
      return;
    }
  });

  // ---- PDF mode ---------------------------------------------------------
  //
  // pdf.js is loaded ONCE, lazily, and only if a PDF actually arrives: it is
  // 1.7 MB and a Markdown-mode user should never pay for it.
  let pdfjs = null;
  let pdfGeneration = 0;

  async function library() {
    if (!pdfjs) {
      pdfjs = await import(PDF_LIB);
      pdfjs.GlobalWorkerOptions.workerSrc = PDF_WORKER;
    }
    return pdfjs;
  }

  function bytesOf(b64) {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  async function renderPdf(b64) {
    // Every arrival supersedes the one before it; a slow render of an older
    // document must not paint over a newer one. Same rule the extension side
    // applies to compiles, enforced again here because rendering is async
    // too and the two races are independent.
    const gen = ++pdfGeneration;
    try {
      const lib = await library();
      const doc = await lib.getDocument({ data: bytesOf(b64) }).promise;
      if (gen !== pdfGeneration) return;

      // Render into a detached fragment and swap it in at the end, so the
      // pane never shows a half-drawn document.
      const frag = document.createDocumentFragment();
      const width = Math.max(320, content.clientWidth || window.innerWidth - 32);
      const dpr = window.devicePixelRatio || 1;

      for (let i = 1; i <= doc.numPages; i++) {
        const page = await doc.getPage(i);
        if (gen !== pdfGeneration) return;
        const unscaled = page.getViewport({ scale: 1 });
        const viewport = page.getViewport({ scale: width / unscaled.width });
        const canvas = document.createElement('canvas');
        canvas.className = 'pdf-page';
        canvas.width = Math.floor(viewport.width * dpr);
        canvas.height = Math.floor(viewport.height * dpr);
        canvas.style.width = viewport.width + 'px';
        canvas.style.height = viewport.height + 'px';
        const ctx = canvas.getContext('2d');
        ctx.scale(dpr, dpr);
        await page.render({ canvasContext: ctx, viewport: viewport }).promise;
        if (gen !== pdfGeneration) return;
        frag.appendChild(canvas);
      }

      if (gen !== pdfGeneration) return;
      spinner.classList.remove('on');
      banner.style.display = 'none';
      document.body.classList.add('pdf');
      const y = window.scrollY;
      content.replaceChildren(frag);
      window.scrollTo(0, y);
      // Canvases cannot go through setState, so PDF mode saves only the
      // offset. A hidden-then-shown panel re-renders from the next compile
      // rather than restoring pixels.
      const st = vscode.getState() || {};
      st.html = ''; st.scroll = y;
      vscode.setState(st);
    } catch (e) {
      if (gen !== pdfGeneration) return;
      spinner.classList.remove('on');
      showBanner('error', 'Could not render the PDF: ' + (e && e.message ? e.message : e));
    }
  }
}());
</script>
</body>
</html>`;
  }
}
