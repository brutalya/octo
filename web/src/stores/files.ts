import { create } from 'zustand';
import { api } from '../api/client';
import type { FileEntry, Space } from '../api/types';

interface Preview {
  space: Space;
  path: string;
  text: string;
}

interface FilesState {
  entries: Record<Space, FileEntry[]>;
  preview: Preview | null;
  load: (space: Space) => Promise<void>;
  upload: (space: Space, dir: string, files: File[]) => Promise<void>;
  mkdir: (space: Space, path: string) => Promise<void>;
  rename: (space: Space, path: string, newPath: string) => Promise<void>;
  remove: (space: Space, path: string) => Promise<void>;
  open: (space: Space, path: string) => Promise<void>;
  closePreview: () => void;
}

export const useFilesStore = create<FilesState>((set, get) => ({
  entries: { private: [], public: [] },
  preview: null,
  load: async (space) => {
    const list = await api.files(space);
    set((s) => ({ entries: { ...s.entries, [space]: list } }));
  },
  upload: async (space, dir, files) => {
    for (const f of files) await api.upload(space, dir, f);
    await get().load(space);
  },
  mkdir: async (space, path) => {
    await api.mkdir(space, path);
    await get().load(space);
  },
  rename: async (space, path, newPath) => {
    await api.renameFile(space, path, newPath);
    await get().load(space);
  },
  remove: async (space, path) => {
    await api.deleteFile(space, path);
    const p = get().preview;
    if (p && p.space === space && (p.path === path || p.path.startsWith(path + '/'))) set({ preview: null });
    await get().load(space);
  },
  open: async (space, path) => {
    const text = await api.fileText(space, path);
    set({ preview: { space, path, text } });
  },
  closePreview: () => set({ preview: null }),
}));
