import { create } from 'zustand';
import { api, nodeGeneration } from '../api/client';
import { errMsg } from '../lib/format';
import type { Holder, SharedEntry } from '../lib/shared';
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

interface FilesState {
  entries: Record<Space, FileEntry[]>;
  preview: Preview | null;
  error: string | null;
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
  loadOthers: (space: Space, devices: { id: string | null; name: string }[]) => Promise<void>;
  /** Copies files that are only on other devices to the same folders here; returns how many. */
  getHere: (space: Space, items: SharedEntry[]) => Promise<number>;
  removeEverywhere: (space: Space, e: SharedEntry) => Promise<void>;
  renameEverywhere: (space: Space, e: SharedEntry, newPath: string) => Promise<void>;
}

const parentOf = (path: string) => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '');
const nameOf = (path: string) => path.slice(path.lastIndexOf('/') + 1);

export const useFilesStore = create<FilesState>((set, get) => {
  const fail = (gen: number, e: unknown) => {
    if (gen === nodeGeneration()) set({ error: errMsg(e) });
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
      if (gen === nodeGeneration()) set((s) => ({ entries: { ...s.entries, [space]: list } }));
    } catch (e) {
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
      const got = await Promise.all(
        devices.map(async (d) => {
          try {
            return { ...d, entries: await api.filesOn(d.id, space) };
          } catch {
            return null; // offline or refused: its files are simply not shown
          }
        }),
      );
      if (gen !== nodeGeneration()) return;
      set((s) => ({ others: { ...s.others, [space]: got.filter((h): h is Holder => h !== null) } }));
    },
    getHere: async (space, items) => {
      let done = 0;
      await run(async () => {
        try {
          for (const e of items) {
            const from = e.on[0];
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
  };
});
