import type { FileEntry } from '../api/types';

/** A device and its files in one space; `id` is the API node id (null = this device). */
export interface Holder {
  id: string | null;
  name: string;
  entries: FileEntry[];
}

/** A path of the shared tree: `here` = on the open device; `on` = other devices that have it. */
export interface SharedEntry extends FileEntry {
  here: boolean;
  on: { id: string | null; name: string }[];
}

/** One tree of every device's files, seen from the open device (`here`). */
export function mergeDevices(here: Holder, others: Holder[]): SharedEntry[] {
  const out = new Map<string, SharedEntry>();
  for (const e of here.entries) out.set(e.path, { ...e, here: true, on: [] });
  for (const h of others) {
    for (const e of h.entries) {
      const cur = out.get(e.path);
      if (!cur) out.set(e.path, { ...e, here: false, on: [{ id: h.id, name: h.name }] });
      else if (cur.kind === e.kind) cur.on.push({ id: h.id, name: h.name });
    }
  }
  return [...out.values()];
}

/** Files under `dir` ('' = all) that are not on the open device. */
export function missingUnder(entries: SharedEntry[], dir: string): SharedEntry[] {
  return entries.filter((e) => e.kind === 'file' && !e.here && (dir === '' || e.path.startsWith(dir + '/')));
}
