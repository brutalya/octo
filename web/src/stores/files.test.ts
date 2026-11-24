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

  it('upload posts multipart to the dir, reloads and returns the stored entries', async () => {
    // The server answers an upload with the entries it stored.
    const saved = [{ path: 'docs/n.txt', kind: 'file', size: 2, modified: 1 }];
    const f = vi.fn(async (url: string) => (url.includes('?') ? json(saved) : json(entries)));
    vi.stubGlobal('fetch', f);
    const got = await useFilesStore.getState().upload('private', 'docs/', [new File(['hi'], 'n.txt')]);
    expect(got).toEqual(saved);
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

  it('upload reloads even when a file fails', async () => {
    const f = vi.fn(async (_url: string, init?: RequestInit) =>
      init?.method === 'POST' ? new Response('no', { status: 500 }) : json(entries),
    );
    vi.stubGlobal('fetch', f);
    await useFilesStore.getState().upload('public', '', [new File(['x'], 'a')]);
    expect(useFilesStore.getState().entries.public).toEqual(entries);
    expect(useFilesStore.getState().error).toBe('no');
  });

  it('preview is capped with a single ellipsis', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response('a'.repeat(300_000), { status: 200 })));
    await useFilesStore.getState().open('public', 'big.txt');
    const text = useFilesStore.getState().preview?.text ?? '';
    expect(text.length).toBe(200_000 + 1);
    expect(text.endsWith('a…')).toBe(true);
  });

  it('open on an image fetches a blob and previews it via an object URL', async () => {
    const f = vi.fn(async () => new Response(new Uint8Array([1, 2, 3]), { headers: { 'content-type': 'image/png' } }));
    vi.stubGlobal('fetch', f);
    const create = vi.fn(() => 'blob:abc');
    const revoke = vi.fn();
    vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: create, revokeObjectURL: revoke }));
    await useFilesStore.getState().open('private', 'photos/x.PNG');
    expect(call(f)[0]).toBe('/api/files/private/content?path=photos%2Fx.PNG');
    expect(useFilesStore.getState().error).toBeNull();
    expect(create).toHaveBeenCalledTimes(1);
    expect(useFilesStore.getState().preview).toEqual({
      space: 'private',
      path: 'photos/x.PNG',
      text: '',
      imageUrl: 'blob:abc',
    });
    useFilesStore.getState().closePreview();
    expect(revoke).toHaveBeenCalledWith('blob:abc');
    expect(useFilesStore.getState().preview).toBeNull();
  });

  it('replacing an image preview revokes the old object URL', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(new Uint8Array([1]))));
    const revoke = vi.fn();
    vi.stubGlobal('URL', Object.assign(URL, { createObjectURL: () => 'blob:1', revokeObjectURL: revoke }));
    await useFilesStore.getState().open('public', 'a.jpg');
    await useFilesStore.getState().open('public', 'b.jpeg');
    expect(revoke).toHaveBeenCalledWith('blob:1');
  });

  describe('other devices', () => {
    const pc = { id: 'pc', name: 'pc' };
    const lin = { id: 'lin', name: 'linux' };

    it('loadOthers lists each device; one that fails is left out', async () => {
      console.log('[Step 1/2] pc answers, linux is unreachable');
      const f = vi.fn(async (url: string) =>
        url.startsWith('/api/nodes/pc/') ? json(entries) : new Response('{"error":"node unreachable"}', { status: 502 }),
      );
      vi.stubGlobal('fetch', f);
      await useFilesStore.getState().loadOthers('public', [pc, lin]);
      console.log('[Step 2/2] only pc is listed');
      expect(f.mock.calls.map((c) => c[0])).toEqual(['/api/nodes/pc/files/public', '/api/nodes/lin/files/public']);
      expect(useFilesStore.getState().others.public).toEqual([{ id: 'pc', name: 'pc', entries }]);
    });

    it('getHere copies each file from a device that has it into the same folder here', async () => {
      const f = vi.fn(async (url: string, init?: RequestInit) => {
        if (url === '/api/nodes/lin/files/public/content?path=docs%2Fb.md') return new Response('bee');
        if (init?.method === 'POST') return json([{ path: 'docs/b.md', kind: 'file', size: 3, modified: 1 }]);
        return json(entries);
      });
      vi.stubGlobal('fetch', f);
      const got = await useFilesStore.getState().getHere('public', [
        { path: 'docs/b.md', kind: 'file', size: 3, modified: 1, here: false, on: [lin, pc] },
      ]);
      expect(got).toBe(1);
      const post = f.mock.calls.find((c) => (c[1] as RequestInit | undefined)?.method === 'POST') as unknown as [string, RequestInit];
      expect(post[0]).toBe('/api/files/public?path=docs');
      const sent = (post[1].body as FormData).get('file') as File;
      expect(sent.name).toBe('b.md');
      expect(await sent.text()).toBe('bee');
    });

    it('removeEverywhere deletes here and on every device that has it', async () => {
      const f = vi.fn(async (url: string) => (url.includes('?') ? new Response(null, { status: 200 }) : json([])));
      vi.stubGlobal('fetch', f);
      await useFilesStore.getState().removeEverywhere('public', {
        path: 'r.pdf', kind: 'file', size: 1, modified: 1, here: true, on: [lin],
      });
      const deletes = f.mock.calls.filter((c) => ((c as unknown[])[1] as RequestInit | undefined)?.method === 'DELETE').map((c) => c[0]);
      expect(deletes).toEqual(['/api/files/public?path=r.pdf', '/api/nodes/lin/files/public?path=r.pdf']);
    });

    it('renameEverywhere renames on every device that has it', async () => {
      const f = vi.fn(async (url: string) => (url.includes('?') ? new Response(null, { status: 200 }) : json([])));
      vi.stubGlobal('fetch', f);
      await useFilesStore.getState().renameEverywhere('public', {
        path: 'r.pdf', kind: 'file', size: 1, modified: 1, here: false, on: [lin, pc],
      }, 's.pdf');
      const patches = f.mock.calls.filter((c) => ((c as unknown[])[1] as RequestInit | undefined)?.method === 'PATCH').map((c) => c[0]);
      expect(patches).toEqual(['/api/nodes/lin/files/public?path=r.pdf', '/api/nodes/pc/files/public?path=r.pdf']);
    });
  });
});
