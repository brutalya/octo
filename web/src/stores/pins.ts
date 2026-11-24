import { create } from 'zustand';

const KEY = 'octo.pins';

/** A pinned chat: its device and thread. */
export const pinKey = (nodeId: string, threadId: string): string => `${nodeId}/${threadId}`;

function stored(): string[] {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '[]');
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

function store(pins: string[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(pins));
  } catch {
    // storage unavailable: pins last for this session
  }
}

interface PinsState {
  pins: string[];
  toggle: (key: string) => void;
}

/** Pinned chats of this device's UI (each device keeps its own). */
export const usePinsStore = create<PinsState>((set, get) => ({
  pins: stored(),
  toggle: (key) => {
    const { pins } = get();
    const next = pins.includes(key) ? pins.filter((p) => p !== key) : [...pins, key];
    store(next);
    set({ pins: next });
  },
}));
