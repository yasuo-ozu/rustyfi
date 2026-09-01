/**
 * Building the argument vector for a preview compile.
 *
 * A note on why there is an output FILE here rather than a pipe: `rustyfi`
 * has no stdout mode for a compile.  `-o -` was tried and it does not mean
 * "write to stdout" -- it creates a file literally named `-` in the working
 * directory.  So the preview writes to a temp path and reads it back.  If a
 * `-o -` (or `--emit stdout`, as `fmt` already has) ever lands on the
 * compiler, `OUTPUT_IS_A_FILE` below is the single place that has to change.
 */

export const OUTPUT_IS_A_FILE = true;

export type MathMode =
  | 'unicode-math' | 'svg-math' | 'svg-outline-math' | 'katex' | 'mathml';

/**
 * What the preview renders.
 *
 * `pdf` is the real thing -- the same page the build produces, laid out by
 * the same engine, so what you see is what ships.  `markdown` is the
 * reflowed, semantic reading of the document: no page breaks, no columns,
 * but it wraps to the panel and stays legible at any width.
 */
export type PreviewFormat = 'pdf' | 'markdown';

const MATH_FLAG: Record<MathMode, string> = {
  'unicode-math': '--unicode-math',
  'svg-math': '--svg-math',
  'svg-outline-math': '--svg-outline-math',
  'katex': '--katex',
  'mathml': '--mathml',
};

export interface PreviewArgOptions {
  /** Path the compiler should read.  For an unsaved buffer this is the
   *  sibling temp file, not the document's own path. */
  inputPath: string;
  /** What to render. */
  format: PreviewFormat;
  /** Where the output should land -- a `.pdf` or a `.md` to match. */
  outputPath: string;
  /** Kept out of the document's directory so a preview never litters the
   *  user's tree with a `.satysfi-aux`, while still seeding the
   *  cross-reference fixpoint so a forward reference resolves in one trial. */
  auxPath: string;
  mathMode: MathMode;
  libRoot?: string | null;
}

export function buildPreviewArgs(o: PreviewArgOptions): string[] {
  const args = [o.inputPath, '--format', o.format];
  // The math flags are a MARKDOWN/HTML concern: they choose how an equation
  // is re-expressed in a format that has no maths of its own.  A PDF is
  // typeset by the same engine that lays out the document, so passing one
  // here would be meaningless at best -- `--unicode-math` is documented
  // "Markdown only" -- and a rejected argument at worst.
  if (o.format === 'markdown') {
    const mode: MathMode = MATH_FLAG[o.mathMode] ? o.mathMode : 'unicode-math';
    args.push(MATH_FLAG[mode]);
  }
  args.push('-o', o.outputPath, '--aux-file', o.auxPath);
  if (o.libRoot && o.libRoot.trim()) args.push('--lib-root', o.libRoot.trim());
  return args;
}

/** The extension the compiler will write for a format. */
export function outputExtension(format: PreviewFormat): string {
  return format === 'pdf' ? '.pdf' : '.md';
}

/**
 * Rewrite the preview temp file's path out of a compiler diagnostic.
 *
 * The compiler names the file it was given, which for a preview is the
 * sibling temp file (`.rustyfi-preview-a1b2c3.saty`).  Showing that to the
 * user is confusing -- it is a filename they never created and cannot find in
 * their editor -- so both the full path and the bare basename are folded back
 * to the real document's name before the message reaches the banner.
 */
export function humanizeDiagnostic(
  message: string,
  tempPath: string,
  realName: string,
): string {
  if (!tempPath) return message;
  const base = tempPath.replace(/^.*[\\/]/, '');
  let out = splitAll(message, tempPath, realName);
  out = splitAll(out, base, realName);
  return out;
}

/** Literal (non-regex) replace-all, so a path with regex metacharacters is safe. */
function splitAll(haystack: string, needle: string, replacement: string): string {
  return needle ? haystack.split(needle).join(replacement) : haystack;
}
