import { create } from 'zustand';
import { api, nodeGeneration } from '../api/client';
import { errMsg } from '../lib/format';
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
}

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
  };
});
