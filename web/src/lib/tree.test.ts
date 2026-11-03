import { describe, expect, it } from 'vitest';
import { buildTree } from './tree';
import type { FileEntry } from '../api/types';

const f = (path: string, kind: 'file' | 'dir' = 'file'): FileEntry => ({ path, kind, size: 1, modified: 1 });

describe('buildTree', () => {
  it('nests children under dirs, dirs first then by name', () => {
    const t = buildTree([f('b.txt'), f('d', 'dir'), f('d/z.txt'), f('d/a.txt'), f('a', 'dir')]);
    expect(t.map((n) => n.name)).toEqual(['a', 'd', 'b.txt']);
    expect(t[1].children.map((n) => n.name)).toEqual(['a.txt', 'z.txt']);
  });
  it('is independent of input order', () => {
    const t = buildTree([f('d/x/y.txt'), f('d/x', 'dir'), f('d', 'dir')]);
    expect(t[0].children[0].children[0].entry.path).toBe('d/x/y.txt');
  });
  it('synthesizes missing parent dirs', () => {
    const t = buildTree([f('p/q/r.txt')]);
    expect(t[0].entry).toMatchObject({ path: 'p', kind: 'dir' });
    expect(t[0].children[0].children[0].name).toBe('r.txt');
  });
  it('empty input', () => {
    expect(buildTree([])).toEqual([]);
  });
});
