/**
 * Locating the `rustyfi` binary.
 *
 * Order:  the `rustyfi.serverPath` setting  ->  PATH  ->  a
 * `target/{release,debug}/rustyfi` at or above the DOCUMENT, then under one
 * of the open workspace folders (so that working ON the compiler and
 * previewing WITH it needs no configuration).
 *
 * The document walk exists because a workspace folder is not guaranteed:
 * `code path/to/file.saty` opens with NO folder at all, and then a
 * folder-only search finds nothing and the extension reports the binary
 * missing while it sits three directories up. Reported exactly that way.
 *
 * The filesystem probe is injected rather than imported so the ordering can
 * be tested without a filesystem.
 */

export interface DiscoveryInput {
  /** `rustyfi.serverPath`, possibly empty. */
  configured?: string | null;
  /** Absolute paths of open workspace folders. May be empty -- see above. */
  workspaceFolders: string[];
  /** Directory of the document being acted on, when there is one. */
  documentDir?: string | null;
  /** Platform-appropriate parent-of, for the upward walk. */
  dirname?: (p: string) => string;
  /** Value of $PATH, split by the caller's platform delimiter. */
  pathEntries: string[];
  /** Platform-appropriate executable name, e.g. `rustyfi` or `rustyfi.exe`. */
  exeName: string;
  /** Returns true when the path exists and is executable. */
  isExecutable: (p: string) => boolean;
  /** Platform-appropriate path join. */
  join: (...parts: string[]) => string;
}

export interface Discovered {
  path: string;
  source: 'setting' | 'path' | 'document' | 'workspace';
}

/** Walk up from `dir`, looking for a built binary at each level. */
function fromCheckout(i: DiscoveryInput, dir: string): string | null {
  const dirname = i.dirname;
  if (!dirname) return null;
  const seen = new Set<string>();
  let cur = dir;
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    for (const profile of ['release', 'debug']) {
      const cand = i.join(cur, 'target', profile, i.exeName);
      if (i.isExecutable(cand)) return cand;
    }
    const up = dirname(cur);
    if (up === cur) break;
    cur = up;
  }
  return null;
}

export function discoverBinary(i: DiscoveryInput): Discovered | null {
  const configured = (i.configured ?? '').trim();
  if (configured) {
    // An explicitly configured path that does not work is an error the user
    // needs to see, not something to silently fall back from -- otherwise
    // they get the wrong binary and no clue why.
    return i.isExecutable(configured) ? { path: configured, source: 'setting' } : null;
  }

  for (const dir of i.pathEntries) {
    if (!dir) continue;
    const cand = i.join(dir, i.exeName);
    if (i.isExecutable(cand)) return { path: cand, source: 'path' };
  }

  if (i.documentDir) {
    const found = fromCheckout(i, i.documentDir);
    if (found) return { path: found, source: 'document' };
  }

  for (const folder of i.workspaceFolders) {
    const found = fromCheckout(i, folder);
    if (found) return { path: found, source: 'workspace' };
    // Kept for the case where `dirname` was not supplied: the original
    // single-level probe, so a caller that cannot walk still resolves the
    // ordinary layout.
    const cand = i.join(folder, 'target', 'release', i.exeName);
    if (i.isExecutable(cand)) return { path: cand, source: 'workspace' };
  }

  return null;
}

/** True when the configured setting was non-empty but did not resolve — the
 *  case that deserves a different message from "nothing found anywhere". */
export function configuredButMissing(i: DiscoveryInput): boolean {
  const configured = (i.configured ?? '').trim();
  return configured.length > 0 && !i.isExecutable(configured);
}
