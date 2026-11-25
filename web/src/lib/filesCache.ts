import type { FileEntry } from '../api/types';

const KEY = 'octo.files.cache';

/** Last known Public entries per device id. Private is never stored (see docs/stage1.md). */
export type FilesCache = Record<string, FileEntry[]>;

export function readCache(): FilesCache {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '{}');
    return typeof v === 'object' && v !== null && !Array.isArray(v) ? (v as FilesCache) : {};
  } catch {
    return {};
  }
}

export function writeCache(cache: FilesCache): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(cache));
  } catch {
    // storage unavailable or full: the cache is only a convenience
  }
}

/** Replaces one device's entries. */
export function putCache(id: string, entries: FileEntry[]): void {
  writeCache({ ...readCache(), [id]: entries });
}

/** Drops every device not in `keep`. */
export function pruneCache(keep: string[]): void {
  const cache = readCache();
  const next = Object.fromEntries(Object.entries(cache).filter(([id]) => keep.includes(id)));
  if (Object.keys(next).length !== Object.keys(cache).length) writeCache(next);
}
