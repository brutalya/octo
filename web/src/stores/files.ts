import { create } from 'zustand';
import { api, ApiError, currentNodeId, nodeGeneration } from '../api/client';
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

/** How a device is doing; `open` is the device whose files are `entries`. Others are keyed by node id. */
export interface DeviceLink {
  name: string;
  state: Reach;
}

interface FilesState {
  entries: Record<Space, FileEntry[]>;
  preview: Preview | null;
  /** A failed action or load; a device that merely does not answer shows in `links`, never here. */
  error: string | null;
  links: Record<string, DeviceLink>;
  /** Back to the cached view of a newly opened device. */
  reset: () => void;
  /** Drops cached entries of every device not in `keep` (unpaired or gone). */
  prune: (keep: string[]) => void;
  load: (space: Space) => Promise<void>;
  /** Returns the entries the server stored (fewer than `files` if some failed). */
  upload: (space: Space, dir: string, files: File[]) => Promise<FileEntry[]>;
  mkdir: (space: Space, path: string) => Promise<void>;
  rename: (space: Space, path: string, newPath: string) => Promise<void>;
  remove: (space: Space, path: string) => Promise<void>;
  open: (space: Space, path: string) => Promise<void>;
  closePreview: () => void;
  /** The same space on the other reachable devices (the open device is `entries`). */
  others: Record<Space, Holder[]>;
  /** `online: false` = not asked now (its last list is kept). */
  loadOthers: (space: Space, devices: { id: string | null; name: string; online?: boolean }[]) => Promise<void>;
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

  const setLink = (key: string, name: string, state: Reach) =>
    set((s) => ({ links: { ...s.links, [key]: { name, state } } }));
  /** Counts a miss for `key`; returns the resulting state. */
  const miss = (key: string): Reach => {
    const n = (misses.get(key) ?? 0) + 1;
    misses.set(key, n);
    return n >= RETRIES ? 'offline' : 'connecting';
  };

  const fail = (gen: number, e: unknown) => {
    if (gen !== nodeGeneration()) return;
    if (unreachable(e)) {
      if (currentNodeId()) setLink('open', '', miss('open'));
      else set({ error: UNREACHABLE });
    } else set({ error: errMsg(e) });
  };

  /** Runs `f`; failures land in `error` unless the node changed meanwhile. */
  const run = async (f: () => Promise<void>) => {
    const gen = nodeGeneration();
    set({ error: null });
    try {
      await f();
    } catch (e) {
      fail(gen, e);
    }
  };

  const load = async (space: Space) => {
    const gen = nodeGeneration();
    try {
      const list = await api.files(space);
      if (gen !== nodeGeneration()) return;
      set((s) => ({ entries: { ...s.entries, [space]: list } }));
      const id = currentNodeId();
      if (id) {
        misses.delete('open');
        setLink('open', '', 'online');
        if (space === 'public') putCache(id, list);
      }
      if (loadFailed) set({ error: null });
      loadFailed = false;
    } catch (e) {
      loadFailed = gen === nodeGeneration() && !unreachable(e);
      fail(gen, e);
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
    reset: () => {
      misses.clear();
      loadFailed = false;
      const id = currentNodeId();
      set({
        entries: { private: [], public: id ? (readCache()[id] ?? []) : [] },
        others: { private: [], public: [] },
        links: id ? { open: { name: '', state: 'connecting' } } : {},
        error: null,
      });
    },
    prune: pruneCache,
    load,
    upload: async (space, dir, files) => {
      const stored: FileEntry[] = [];
      await run(async () => {
        try {
          for (const f of files) stored.push(...(await api.upload(space, dir, f)));
        } finally {
          await load(space);
        }
      });
      return stored;
    },
    mkdir: async (space, path) => {
      await run(() => api.mkdir(space, path));
      await load(space);
    },
    rename: async (space, path, newPath) => {
      await run(() => api.renameFile(space, path, newPath));
      await load(space);
    },
    remove: async (space, path) => {
      await run(async () => {
        await api.deleteFile(space, path);
        const p = get().preview;
        if (p && p.space === space && (p.path === path || p.path.startsWith(path + '/'))) setPreview(null);
      });
      await load(space);
    },
    open: (space, path) =>
      run(async () => {
        const gen = nodeGeneration();
        if (IMAGE_RE.test(path)) {
          const blob = await api.fileBlob(space, path);
          if (gen === nodeGeneration()) setPreview({ space, path, text: '', imageUrl: URL.createObjectURL(blob) });
          return;
        }
        let text = await api.fileText(space, path);
        if (text.length > PREVIEW_MAX) text = text.slice(0, PREVIEW_MAX) + '…';
        if (gen === nodeGeneration()) setPreview({ space, path, text });
      }),
    closePreview: () => setPreview(null),
    others: { private: [], public: [] },
    loadOthers: async (space, devices) => {
      const gen = nodeGeneration();
      const key = (d: { id: string | null }) => d.id ?? 'self';
      // Only Public is ever cached; Private lives in memory for the session.
      const cache = space === 'public' ? readCache() : {};
      // Right away: what is known (shown or cached) while the devices are asked.
      set((s) => {
        const before = new Map(s.others[space].map((h) => [h.id, h]));
        const next = devices
          .map((d): Holder | null => {
            const kept = before.get(d.id);
            const cached = d.id ? cache[d.id] : undefined;
            return kept ?? (cached ? { id: d.id, name: d.name, entries: cached, stale: 'connecting' } : null);
          })
          .filter((h): h is Holder => h !== null);
        const links = { ...s.links };
        for (const d of devices) links[key(d)] ??= { name: d.name, state: d.online === false ? 'offline' : 'connecting' };
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
      if (gen !== nodeGeneration()) return;
      const fresh: Record<string, FileEntry[]> = {};
      set((s) => {
        const before = new Map(s.others[space].map((h) => [h.id, h]));
        const links: Record<string, DeviceLink> = s.links.open ? { open: s.links.open } : {};
        const next: Holder[] = [];
        devices.forEach((d, i) => {
          const entries = got[i];
          let state: Reach;
          if (entries) {
            misses.delete(key(d));
            state = 'online';
            next.push({ id: d.id, name: d.name, entries });
            if (d.id) fresh[d.id] = entries;
          } else {
            state = entries === undefined ? 'offline' : miss(key(d));
            // A device that did not answer keeps its last list, so the tree does not flicker.
            const prev = before.get(d.id);
            if (prev) next.push({ ...prev, name: d.name, stale: state as 'connecting' | 'offline' });
          }
          links[key(d)] = { name: d.name, state };
        });
        for (const k of [...misses.keys()]) if (k !== 'open' && !devices.some((d) => key(d) === k)) misses.delete(k);
        return { others: { ...s.others, [space]: next }, links };
      });
      if (space === 'public') {
        const cur = currentNodeId();
        const keep = [...devices.flatMap((d) => (d.id ? [d.id] : [])), ...(cur ? [cur] : [])];
        pruneCache(keep);
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
            await api.upload(space, parentOf(e.path), new File([await blob.arrayBuffer()], nameOf(e.path)));
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
        if (e.here) await api.deleteFile(space, e.path);
        for (const h of e.on) await api.deleteFileOn(h.id, space, e.path);
        const p = get().preview;
        if (p && p.space === space && (p.path === e.path || p.path.startsWith(e.path + '/'))) setPreview(null);
      });
      dropOthers(space, e.path);
      await load(space);
    },
    renameEverywhere: async (space, e, newPath) => {
      await run(async () => {
        if (e.here) await api.renameFile(space, e.path, newPath);
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
        const blob = from ? await api.fileBlobOn(from.id, 'public', e.path) : await api.fileBlob('public', e.path);
        const dir = parentOf(e.path);
        const inDir = (await api.files('private'))
          .filter((x) => parentOf(x.path) === dir)
          .map((x) => nameOf(x.path));
        await api.upload('private', dir, new File([await blob.arrayBuffer()], uniqueName(nameOf(e.path), inDir)));
        // Only once the private copy is stored.
        if (e.here) await api.deleteFile('public', e.path);
        for (const h of e.on) await api.deleteFileOn(h.id, 'public', e.path);
        moved = true;
      });
      if (moved) dropOthers('public', e.path);
      await Promise.all([load('public'), load('private')]);
    },
  };
});
