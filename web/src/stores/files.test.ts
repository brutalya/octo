import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { useFilesStore } from './files';
import type { FileEntry } from '../api/types';

const entries: FileEntry[] = [
  { path: 'docs', kind: 'dir', size: 0, modified: 1 },
  { path: 'docs/a.md', kind: 'file', size: 3, modified: 1 },
];
const json = (v: unknown) =>
  new Response(JSON.stringify(v), { status: 200, headers: { 'content-type': 'application/json' } });
const call = (f: ReturnType<typeof vi.fn>, i = 0) => f.mock.calls[i] as unknown as [string, RequestInit];

beforeEach(() => useFilesStore.setState({ entries: { private: [], public: [] }, preview: null }));
afterEach(() => vi.unstubAllGlobals());

describe('files store', () => {
  it('load fetches the tree of a space', async () => {
    const f = vi.fn(async () => json(entries));
    vi.stubGlobal('fetch', f);
    await useFilesStore.getState().load('public');
    expect(call(f)[0]).toBe('/api/files/public');
    expect(useFilesStore.getState().entries.public).toEqual(entries);
    expect(useFilesStore.getState().entries.private).toEqual([]);
  });

  it('upload posts multipart to the dir and reloads', async () => {
    const f = vi.fn(async (url: string) => (url.includes('?') ? new Response(null, { status: 200 }) : json(entries)));
    vi.stubGlobal('fetch', f);
    await useFilesStore.getState().upload('private', 'docs/', [new File(['hi'], 'n.txt')]);
    const [url, init] = call(f);
    expect(url).toBe('/api/files/private?path=docs%2F');
    expect(init.method).toBe('POST');
    expect(init.body).toBeInstanceOf(FormData);
    expect(call(f, 1)[0]).toBe('/api/files/private');
    expect(useFilesStore.getState().entries.private).toEqual(entries);
  });

  it('rename PATCHes new_path and reloads', async () => {
    const f = vi.fn(async (url: string) => (url.includes('?') ? new Response(null, { status: 200 }) : json(entries)));
    vi.stubGlobal('fetch', f);
    await useFilesStore.getState().rename('public', 'docs/a.md', 'docs/b.md');
    const [url, init] = call(f);
    expect(url).toBe('/api/files/public?path=docs%2Fa.md');
    expect(init.method).toBe('PATCH');
    expect(JSON.parse(init.body as string)).toEqual({ new_path: 'docs/b.md' });
    expect(useFilesStore.getState().entries.public).toEqual(entries);
  });

  it('remove DELETEs and reloads; closes a preview of the removed file', async () => {
    const f = vi.fn(async (url: string) => (url.includes('?') ? new Response(null, { status: 200 }) : json([])));
    vi.stubGlobal('fetch', f);
    useFilesStore.setState({ preview: { space: 'public', path: 'docs/a.md', text: 'x' } });
    await useFilesStore.getState().remove('public', 'docs/a.md');
    expect(call(f)[1].method).toBe('DELETE');
    expect(useFilesStore.getState().entries.public).toEqual([]);
    expect(useFilesStore.getState().preview).toBeNull();
  });

  it('mkdir POSTs to /dir', async () => {
    const f = vi.fn(async (url: string) => (url.includes('?') ? new Response(null, { status: 200 }) : json(entries)));
    vi.stubGlobal('fetch', f);
    await useFilesStore.getState().mkdir('public', 'new');
    expect(call(f)[0]).toBe('/api/files/public/dir?path=new');
    expect(call(f)[1].method).toBe('POST');
  });

  it('open loads text preview', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response('hello')));
    await useFilesStore.getState().open('public', 'docs/a.md');
    expect(useFilesStore.getState().preview).toEqual({ space: 'public', path: 'docs/a.md', text: 'hello' });
    useFilesStore.getState().closePreview();
    expect(useFilesStore.getState().preview).toBeNull();
  });
});
