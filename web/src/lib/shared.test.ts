import { describe, it, expect } from 'vitest';
import { mergeDevices, missingUnder, type Holder } from './shared';
import type { FileEntry } from '../api/types';

const f = (path: string, size = 1): FileEntry => ({ path, kind: 'file', size, modified: 1 });
const d = (path: string): FileEntry => ({ path, kind: 'dir', size: 0, modified: 1 });

const here: Holder = { id: 'pc', name: 'pc', entries: [d('docs'), f('docs/a.md'), f('notes.md')] };
const phone: Holder = { id: null, name: 'phone', entries: [d('docs'), f('docs/a.md'), f('docs/b.md'), f('report.pdf', 9)] };
const linux: Holder = { id: 'lin', name: 'linux', entries: [d('projects'), f('projects/x.rs'), f('report.pdf', 9)] };

describe('mergeDevices', () => {
  it('lists every path once and says where it is when not here', () => {
    console.log('[Step 1/2] merge the open device (pc) with phone and linux');
    const m = mergeDevices(here, [phone, linux]);
    const by = Object.fromEntries(m.map((e) => [e.path, e]));
    console.log('[Step 2/2] local entries are here; the rest name the devices holding them');
    expect(m.map((e) => e.path).sort()).toEqual(['docs', 'docs/a.md', 'docs/b.md', 'notes.md', 'projects', 'projects/x.rs', 'report.pdf']);
    expect(by['docs/a.md']).toMatchObject({ here: true });
    expect(by['notes.md'].here).toBe(true);
    expect(by['docs/b.md']).toMatchObject({ here: false, on: [{ id: null, name: 'phone' }] });
    expect(by['report.pdf']).toMatchObject({ here: false, size: 9 });
    expect(by['report.pdf'].on.map((h) => h.name)).toEqual(['phone', 'linux']);
    expect(by['projects']).toMatchObject({ kind: 'dir', here: false });
  });

  it('a folder that is here is here even when some files inside are not', () => {
    const m = mergeDevices(here, [phone]);
    expect(m.find((e) => e.path === 'docs')?.here).toBe(true);
  });

  it('with no other devices it is just the local list', () => {
    const m = mergeDevices(here, []);
    expect(m.every((e) => e.here)).toBe(true);
    expect(m).toHaveLength(3);
  });
});

describe('missingUnder', () => {
  it('files not here under a folder, each with a device to take it from', () => {
    const m = mergeDevices(here, [phone, linux]);
    expect(missingUnder(m, 'docs').map((e) => e.path)).toEqual(['docs/b.md']);
    expect(missingUnder(m, '').map((e) => e.path).sort()).toEqual(['docs/b.md', 'projects/x.rs', 'report.pdf']);
  });
});
