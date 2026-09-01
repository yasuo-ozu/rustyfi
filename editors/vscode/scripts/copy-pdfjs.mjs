// Copy the two pdf.js files the preview needs out of node_modules and into
// `media/`, which is the only directory the webview is allowed to load from.
//
// A copy rather than a `localResourceRoots` entry pointing at node_modules:
// the packaged extension excludes node_modules (see .vscodeignore), so the
// dependency has to be materialised at build time or the published .vsix
// would work on the developer's machine and nowhere else.
//
// THE LEGACY BUILD, and this is not a stylistic preference. The modern build
// of pdf.js 6.x calls `Map.prototype.getOrInsertComputed`, a V8 builtin newer
// than the Chromium in VS Code 1.111 -- the preview failed at runtime with
// "this[#Yr].getOrInsertComputed is not a function". 6.x's *legacy* build
// calls it too, so the fix was pinning pdfjs-dist to 4.x AND taking legacy;
// neither alone is enough. A webview is not "a current browser": it is
// whatever Chromium the user's VS Code was built against.
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const from = join(here, '..', 'node_modules', 'pdfjs-dist', 'legacy', 'build');
const to = join(here, '..', 'media');
mkdirSync(to, { recursive: true });
for (const f of ['pdf.min.mjs', 'pdf.worker.min.mjs']) {
  copyFileSync(join(from, f), join(to, f));
  console.log(`copied ${f}`);
}
