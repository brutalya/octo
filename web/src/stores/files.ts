import { create } from 'zustand';
import { api, ApiError } from '../api/client';
import { errMsg } from '../lib/format';
import { uniqueName } from '../lib/files';
import { putCache, pruneCache, readCache } from '../lib/filesCache';
import type { Holder, Reach, SharedEntry } from '../lib/shared';
import type { FileEntry, Space } from '../api/types';

const PREVIEW_MAX = 200_000;

interface Preview {
  space: Space;
  path: string;
  text: string;
  /** Object URL of an image preview (revoked when the preview goes away). */
  imageUrl?: string;
}

const IMAGE_RE = /\.(png|jpe?g|webp|gif)$/i;

/** Misses in a row (each one a retry) before a device counts as offline instead of connecting. */
const RETRIES = 3;

const UNREACHABLE = 'Device unreachable';
const unreachable = (e: unknown) =>
  (e instanceof ApiError && e.status === 502) || /node unreachable/i.test(errMsg(e));

/** How another device is doing, keyed by node id. */
export interface DeviceLink {
  name: string;
  state: Reach;
}

interface FilesState {
  /** This device's own files (always asked here, whichever device is open for chats). */
  entries: Record<Space, FileEntry[]>;
  preview: Preview | null;
  /** A failed action or load; a device that merely does not answer shows in `links`, never here. */
  error: string | null;
  links: Record<string, DeviceLink>;
  /** Drops cached entries of every device not in `keep` (unpaired or gone). */
  prune: (keep: string[]) => void;
  load: (space: Space) => Promise<void>;
  /** Returns the entries the server stored (fewer than `files` if some failed). `nodeId` = another device (a chat's attachments). */
  upload: (space: Space, dir: string, files: File[], nodeId?: string | null) => Promise<FileEntry[]>;
  mkdir: (space: Space, path: string) => Promise<void>;
  rename: (space: Space, path: string, newPath: string) => Promise<void>;
  remove: (space: Space, path: string) => Promise<void>;
  open: (space: Space, path: string) => Promise<void>;
  closePreview: () => void;
  /** The same space on the other devices (this device is `entries`). */
  others: Record<Space, Holder[]>;
  /** `online: false` = not asked now (its last list is kept). */
  loadOthers: (space: Space, devices: { id: string; name: string; online?: boolean }[]) => Promise<void>;
  /** Copies files that are only on other devices to the same folders here; returns how many. */
  getHere: (space: Space, items: SharedEntry[]) => Promise<number>;
  removeEverywhere: (space: Space, e: SharedEntry) => Promise<void>;
  renameEverywhere: (space: Space, e: SharedEntry, newPath: string) => Promise<void>;
  /** Copies a Public file into Private (same folder), then removes every Public copy. */
  moveToPrivate: (e: SharedEntry) => Promise<void>;
}

const parentOf = (path: string) => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '');
const nameOf = (path: string) => path.slice(path.lastIndexOf('/') + 1);

export const useFilesStore = create<FilesState>((set, get) => {
  const misses = new Map<string, number>();
  /** The last load failed for a real reason (not just no answer); a later good load clears it. */
  let loadFailed = false;

  /** Counts a miss for `key`; returns the resulting state. */
  const miss = (key: string): Reach => {
    const n = (misses.get(key) ?? 0) + 1;
    misses.set(key, n);
    return n >= RETRIES ? 'offline' : 'connecting';
  };

  const fail = (e: unknown) => set({ error: unreachable(e) ? UNREACHABLE : errMsg(e) });

  /** Runs `f`; failures land in `error`. */
  const run = async (f: () => Promise<void>) => {
    set({ error: null });
    try {
      await f();
    } catch (e) {
      fail(e);
    }
  };

  const load = async (space: Space) => {
    try {
      const list = await api.filesOn(null, space);
      set((s) => ({ entries: { ...s.entries, [space]: list } }));
      if (loadFailed) set({ error: null });
      loadFailed = false;
    } catch (e) {
      loadFailed = true;
      fail(e);
    }
  };

  /** Replaces the preview and releases the previous object URL. */
  const setPreview = (next: Preview | null) => {
    const old = get().preview?.imageUrl;
    if (old) URL.revokeObjectURL(old);
    set({ preview: next });
  };

  const under = (p: string, path: string) => p === path || p.startsWith(path + '/');
  /** Applies a change made on the other devices to their last fetched lists. */
  const editOthers = (space: Space, f: (e: FileEntry) => FileEntry | null) =>
    set((s) => ({
      others: {
        ...s.others,
        [space]: s.others[space].map((h) => ({
          ...h,
          entries: h.entries.map(f).filter((e): e is FileEntry => e !== null),
        })),
      },
    }));
  const dropOthers = (space: Space, path: string) => editOthers(space, (e) => (under(e.path, path) ? null : e));
  const renameOthers = (space: Space, from: string, to: string) =>
    editOthers(space, (e) => (under(e.path, from) ? { ...e, path: to + e.path.slice(from.length) } : e));

  return {
    entries: { private: [], public: [] },
    preview: null,
    error: null,
    links: {},
    prune: pruneCache,
    load,
    upload: async (space, dir, files, nodeId = null) => {
      const stored: FileEntry[] = [];
      await run(async () => {
        try {
          for (const f of files) stored.push(...(await api.uploadOn(nodeId, space, dir, f)));
        } finally {
          if (nodeId === null) await load(space);
        }
      });
      return stored;
    },
    mkdir: async (space, path) => {
      await run(() => api.mkdirOn(null, space, path));
      await load(space);
    },
    rename: async (space, path, newPath) => {
      await run(() => api.renameFileOn(null, space, path, newPath));
      await load(space);
    },
    remove: async (space, path) => {
      await run(async () => {
        await api.deleteFileOn(null, space, path);
        const p = get().preview;
        if (p && p.space === space && (p.path === path || p.path.startsWith(path + '/'))) setPreview(null);
      });
      await load(space);
    },
    open: (space, path) =>
      run(async () => {
        if (IMAGE_RE.test(path)) {
          const blob = await api.fileBlobOn(null, space, path);
          setPreview({ space, path, text: '', imageUrl: URL.createObjectURL(blob) });
          return;
        }
        let text = await api.fileTextOn(null, space, path);
        if (text.length > PREVIEW_MAX) text = text.slice(0, PREVIEW_MAX) + '…';
        setPreview({ space, path, text });
      }),
    closePreview: () => setPreview(null),
    others: { private: [], public: [] },
    loadOthers: async (space, devices) => {
      // Only Public is ever cached; Private lives in memory for the session.
      const cache = space === 'public' ? readCache() : {};
      // Right away: what is known (shown or cached) while the devices are asked.
      set((s) => {
        const before = new Map(s.others[space].map((h) => [h.id, h]));
        const next = devices
          .map((d): Holder | null => {
            const kept = before.get(d.id);
            const cached = cache[d.id];
            return kept ?? (cached ? { id: d.id, name: d.name, entries: cached, stale: 'connecting' } : null);
          })
          .filter((h): h is Holder => h !== null);
        const links = { ...s.links };
        for (const d of devices) links[d.id] ??= { name: d.name, state: d.online === false ? 'offline' : 'connecting' };
        return { others: { ...s.others, [space]: next }, links };
      });
      const got = await Promise.all(
        devices.map(async (d) => {
          if (d.online === false) return undefined; // not asked
          try {
            return await api.filesOn(d.id, space);
          } catch {
            return null; // no answer this time
          }
        }),
      );
      const fresh: Record<string, FileEntry[]> = {};
      set((s) => {
        const before = new Map(s.others[space].map((h) => [h.id, h]));
        const links: Record<string, DeviceLink> = {};
        const next: Holder[] = [];
        devices.forEach((d, i) => {
          const entries = got[i];
          let state: Reach;
          if (entries) {
            misses.delete(d.id);
            state = 'online';
            next.push({ id: d.id, name: d.name, entries });
            fresh[d.id] = entries;
          } else {
            state = entries === undefined ? 'offline' : miss(d.id);
            // A device that did not answer keeps its last list, so the tree does not flicker.
            const prev = before.get(d.id);
            if (prev) next.push({ ...prev, name: d.name, stale: state as 'connecting' | 'offline' });
          }
          links[d.id] = { name: d.name, state };
        });
        for (const k of [...misses.keys()]) if (!devices.some((d) => d.id === k)) misses.delete(k);
        return { others: { ...s.others, [space]: next }, links };
      });
      if (space === 'public') {
        pruneCache(devices.map((d) => d.id));
        for (const [id, entries] of Object.entries(fresh)) putCache(id, entries);
      }
    },
    getHere: async (space, items) => {
      let done = 0;
      await run(async () => {
        try {
          for (const e of items) {
            // A holder that has not answered yet cannot serve the file.
            const from = e.on.find((h) => !h.stale);
            if (e.here || e.kind !== 'file' || !from) continue;
            const blob = await api.fileBlobOn(from.id, space, e.path);
            await api.uploadOn(null, space, parentOf(e.path), new File([await blob.arrayBuffer()], nameOf(e.path)));
            done++;
          }
        } finally {
          await load(space);
        }
      });
      return done;
    },
    removeEverywhere: async (space, e) => {
      await run(async () => {
        if (e.here) await api.deleteFileOn(null, space, e.path);
        for (const h of e.on) await api.deleteFileOn(h.id, space, e.path);
        const p = get().preview;
        if (p && p.space === space && (p.path === e.path || p.path.startsWith(e.path + '/'))) setPreview(null);
      });
      dropOthers(space, e.path);
      await load(space);
    },
    renameEverywhere: async (space, e, newPath) => {
      await run(async () => {
        if (e.here) await api.renameFileOn(null, space, e.path, newPath);
        for (const h of e.on) await api.renameFileOn(h.id, space, e.path, newPath);
      });
      renameOthers(space, e.path, newPath);
      await load(space);
    },
    moveToPrivate: async (e) => {
      const from = e.here ? null : e.on.find((h) => !h.stale);
      if (e.kind !== 'file' || (!e.here && !from)) return;
      let moved = false;
      await run(async () => {
        const blob = await api.fileBlobOn(from ? from.id : null, 'public', e.path);
        const dir = parentOf(e.path);
        const inDir = (await api.filesOn(null, 'private'))
          .filter((x) => parentOf(x.path) === dir)
          .map((x) => nameOf(x.path));
        await api.uploadOn(null, 'private', dir, new File([await blob.arrayBuffer()], uniqueName(nameOf(e.path), inDir)));
        // Only once the private copy is stored.
        if (e.here) await api.deleteFileOn(null, 'public', e.path);
        for (const h of e.on) await api.deleteFileOn(h.id, 'public', e.path);
        moved = true;
      });
      if (moved) dropOthers('public', e.path);
      await Promise.all([load('public'), load('private')]);
    },
  };
});
