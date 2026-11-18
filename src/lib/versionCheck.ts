/**
 * Version checker — queries latest release and compares with current version.
 *
 * Configuration (override via window.__GUANJI_CONFIG__ in index.html, or env vars):
 *   - RELEASE_API_URL : GitHub API endpoint for latest release
 *                      default: https://api.github.com/repos/daoyking/guanji/releases/latest
 *   - CURRENT_VERSION : hard-coded fallback if package.json is not accessible
 *
 * Usage:
 *   const { hasUpdate, version, downloadUrl, note } = await checkForUpdate();
 */

const DEFAULT_RELEASE_URL =
  'https://api.github.com/repos/daoyking/guanji/releases/latest';
const CURRENT_VERSION = __APP_VERSION__; // injected by vite.config.ts at build time

export interface UpdateInfo {
  /** Whether a newer version is available */
  hasUpdate: boolean;
  /** Latest version string, e.g. "v0.2.0" */
  version?: string;
  /** Direct URL to the release page */
  downloadUrl?: string;
  /** Release body / changelog (first 200 chars) */
  note?: string;
  /** Error message if the check failed */
  error?: string;
}

let cached: UpdateInfo | null = null;
let cacheTime = 0;
const CACHE_TTL_MS = 1000 * 60 * 30; // 30 minutes

function parseVersion(v: string): [number, number, number] {
  const m = v.replace(/^v/, '').match(/(\d+)\.(\d+)\.(\d+)/);
  return m ? [+m[1], +m[2], +m[3]] : [0, 0, 0];
}

function isNewer(a: string, b: string): boolean {
  const [ma, mb] = [parseVersion(a), parseVersion(b)];
  for (let i = 0; i < 3; i++) {
    if (ma[i] > mb[i]) return true;
    if (ma[i] < mb[i]) return false;
  }
  return false;
}

export async function checkForUpdate(force = false): Promise<UpdateInfo> {
  const now = Date.now();
  if (!force && cached && now - cacheTime < CACHE_TTL_MS) return cached;

  const url = import.meta.env.VITE_RELEASE_URL || DEFAULT_RELEASE_URL;

  try {
    const res = await fetch(url, {
      headers: { Accept: 'application/json' },
      // Tauri webview may block cross-origin; catch and return early
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json() as Record<string, unknown>;

    const version = String(data.tag_name ?? '');
    const body = String(data.body ?? '');
    const htmlUrl = String(data.html_url ?? '');
    const note = body.slice(0, 200).replace(/\n/g, ' ').trim();

    const info: UpdateInfo = {
      hasUpdate: version ? isNewer(version, CURRENT_VERSION) : false,
      version: version || undefined,
      downloadUrl: htmlUrl || undefined,
      note: note || undefined,
    };
    cached = info;
    cacheTime = now;
    return info;
  } catch (e) {
    // Return silent failure — don't block the app
    const info: UpdateInfo = {
      hasUpdate: false,
      error: (e as Error).message,
    };
    cached = info;
    cacheTime = now;
    return info;
  }
}

// Expose a getter so the UI can reactively poll
let _subscribe: ((info: UpdateInfo) => void) | null = null;
export function subscribeToUpdate(fn: (info: UpdateInfo) => void) {
  _subscribe = fn;
}

export function fireUpdate(info: UpdateInfo) {
  _subscribe?.(info);
}
