import { create } from 'zustand';
import type { AttachmentRef } from '../api/types';

/** Files picked in the Files panel for the next message (shown as one chip in the input). */
interface AttachState {
  fromFiles: AttachmentRef[];
  add: (refs: AttachmentRef[]) => void;
  clear: () => void;
}

const same = (a: AttachmentRef, b: AttachmentRef) => a.space === b.space && a.path === b.path;

export const useAttachStore = create<AttachState>((set) => ({
  fromFiles: [],
  add: (refs) =>
    set((s) => ({ fromFiles: [...s.fromFiles, ...refs.filter((r) => !s.fromFiles.some((x) => same(x, r)))] })),
  clear: () => set({ fromFiles: [] }),
}));
