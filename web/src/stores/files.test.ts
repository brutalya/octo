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

beforeEach(() => {
  localStorage.clear();
  useFilesStore.setState({
    entries: { private: [], public: [] },
    others: { private: [], public: [] },
    links: {},
    error: null,
    preview: null,
  });
});
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

    it('loadOthers lists each device; one never reached is left out', async () => {
      console.log('[Step 1/2] pc answers, linux is unreachable');
      useFilesStore.setState({ others: { private: [], public: [] } });
      const f = vi.fn(async (url: string) =>
        url.startsWith('/api/nodes/pc/') ? json(entries) : new Response('{"error":"node unreachable"}', { status: 502 }),
      );
      vi.stubGlobal('fetch', f);
      await useFilesStore.getState().loadOthers('public', [pc, lin]);
      console.log('[Step 2/2] only pc is listed');
      expect(f.mock.calls.map((c) => c[0])).toEqual(['/api/nodes/pc/files/public', '/api/nodes/lin/files/public']);
      expect(useFilesStore.getState().others.public).toEqual([{ id: 'pc', name: 'pc', entries }]);
    });

    it('a device that misses one answer, or is briefly offline, keeps its files on screen', async () => {
      console.log('[Step 1/3] both answered before');
      const linFiles = [{ path: 'r.pdf', kind: 'file' as const, size: 1, modified: 1 }];
      useFilesStore.setState({
        others: { private: [], public: [{ ...pc, entries }, { ...lin, entries: linFiles }] },
      });
      console.log('[Step 2/3] now pc times out and linux is reported offline');
      const f = vi.fn(async () => new Response('{"error":"node unreachable"}', { status: 502 }));
      vi.stubGlobal('fetch', f);
      await useFilesStore.getState().loadOthers('public', [pc, { ...lin, online: false }]);
      console.log('[Step 3/3] both keep their last lists; the offline one is not even asked');
      expect(f.mock.calls.map((c) => (c as unknown[])[0])).toEqual(['/api/nodes/pc/files/public']);
      expect(useFilesStore.getState().others.public).toEqual([
        { ...pc, entries, stale: 'connecting' },
        { ...lin, entries: linFiles, stale: 'offline' },
      ]);
    });

    it('a device that left the network drops out', async () => {
      useFilesStore.setState({ others: { private: [], public: [{ ...pc, entries }, { ...lin, entries }] } });
      vi.stubGlobal('fetch', vi.fn(async () => json(entries)));
      await useFilesStore.getState().loadOthers('public', [pc]);
      expect(useFilesStore.getState().others.public.map((h) => h.id)).toEqual(['pc']);
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

    it('moveToPrivate copies into Private (never over a file there), then removes every Public copy', async () => {
      console.log('[Step 1/2] a.md is on pc and linux; Private already has an a.md in docs');
      const f = vi.fn(async (url: string, init?: RequestInit) => {
        if (url === '/api/files/private') return json([{ path: 'docs/a.md', kind: 'file', size: 1, modified: 1 }]);
        if (url === '/api/files/public/content?path=docs%2Fa.md') return new Response('aaa');
        if (init?.method === 'POST') return json([{ path: 'docs/a (1).md', kind: 'file', size: 3, modified: 1 }]);
        if (init?.method === 'DELETE') return new Response(null, { status: 200 });
        return json([]);
      });
      vi.stubGlobal('fetch', f);
      await useFilesStore.getState().moveToPrivate({
        path: 'docs/a.md', kind: 'file', size: 3, modified: 1, here: true, on: [lin],
      });
      console.log('[Step 2/2] stored as docs/a (1).md in Private; Public copies gone');
      const post = f.mock.calls.find((c) => ((c as unknown[])[1] as RequestInit | undefined)?.method === 'POST') as unknown as [string, RequestInit];
      expect(post[0]).toBe('/api/files/private?path=docs');
      expect(((post[1].body as FormData).get('file') as File).name).toBe('a (1).md');
      const deletes = f.mock.calls.filter((c) => ((c as unknown[])[1] as RequestInit | undefined)?.method === 'DELETE').map((c) => c[0]);
      expect(deletes).toEqual(['/api/files/public?path=docs%2Fa.md', '/api/nodes/lin/files/public?path=docs%2Fa.md']);
    });

    it('moveToPrivate keeps the Public copies when the copy failed', async () => {
      const f = vi.fn(async (url: string, init?: RequestInit) => {
        if (init?.method === 'POST') return new Response('{"error":"too large"}', { status: 413 });
        if (url.includes('/content')) return new Response('aaa');
        return json([]);
      });
      vi.stubGlobal('fetch', f);
      await useFilesStore.getState().moveToPrivate({ path: 'a.md', kind: 'file', size: 3, modified: 1, here: true, on: [] });
      expect(f.mock.calls.some((c) => ((c as unknown[])[1] as RequestInit | undefined)?.method === 'DELETE')).toBe(false);
      expect(useFilesStore.getState().error).toBe('too large');
    });
  });

  describe('connecting and the cache', () => {
    const CACHE = 'octo.files.cache';
    const pc = { id: 'pc', name: 'pc' };
    const lin = { id: 'lin', name: 'linux' };
    const pcFiles: FileEntry[] = [
      { path: 'design', kind: 'dir', size: 0, modified: 1 },
      { path: 'design/a.png', kind: 'file', size: 9, modified: 1 },
      { path: 'old.txt', kind: 'file', size: 1, modified: 1 },
    ];
    const down = () => new Response('{"error":"node unreachable"}', { status: 502 });
    const state = () => useFilesStore.getState();

    it('a node that has not answered yet is connecting, not an error', async () => {
      console.log('[Step 1/2] linux does not answer');
      vi.stubGlobal('fetch', vi.fn(async () => down()));
      await state().loadOthers('public', [lin]);
      console.log('[Step 2/2] state says connecting, no error text');
      expect(state().links.lin).toEqual({ name: 'linux', state: 'connecting' });
      expect(state().error).toBeNull();
    });

    it('after repeated misses the device is offline; one answer brings it back', async () => {
      let up = false;
      vi.stubGlobal('fetch', vi.fn(async () => (up ? json(pcFiles) : down())));
      for (let i = 0; i < 3; i++) await state().loadOthers('public', [pc]);
      expect(state().links.pc.state).toBe('offline');
      up = true;
      await state().loadOthers('public', [pc]);
      expect(state().links.pc.state).toBe('online');
      expect(state().others.public).toEqual([{ ...pc, entries: pcFiles }]);
    });

    it('a failed load shows its error; a good load clears it', async () => {
      vi.stubGlobal('fetch', vi.fn(async () => new Response('{"error":"disk failure"}', { status: 500 })));
      await state().load('public');
      expect(state().error).toBe('disk failure');
      vi.stubGlobal('fetch', vi.fn(async () => json(entries)));
      await state().load('public');
      expect(state().error).toBeNull();
    });

    it('this device\'s files are loaded and changed here whichever device is open for chats', async () => {
      console.log('[Step 1/3] pc is open for chats');
      const { setNodeBase } = await import('../api/client');
      setNodeBase('lin');
      const f = vi.fn(async (_url: string, init?: RequestInit) => (init?.method ? json([]) : json(entries)));
      vi.stubGlobal('fetch', f);
      console.log('[Step 2/3] list, preview, upload, mkdir, rename, delete');
      await state().load('public');
      await state().load('private');
      await state().upload('public', '', [new File(['x'], 'a')]);
      await state().mkdir('public', 'd');
      await state().rename('public', 'a', 'b');
      await state().remove('public', 'b');
      setNodeBase(null);
      console.log('[Step 3/3] nothing went through the open device, nothing cached for it');
      expect(f.mock.calls.map((c) => c[0] as string).filter((u) => u.includes('/nodes/'))).toEqual([]);
      expect(state().entries.public).toEqual(entries);
      expect(localStorage.length).toBe(0);
    });

    it('an upload meant for a chat goes to the chat device and leaves the local list alone', async () => {
      const f = vi.fn(async () => json([{ path: 'x/a', kind: 'file', size: 1, modified: 1 }]));
      vi.stubGlobal('fetch', f);
      const got = await state().upload('public', 'x', [new File(['x'], 'a')], 'lin');
      expect(got).toHaveLength(1);
      expect(f.mock.calls.map((c) => (c as unknown[])[0])).toEqual(['/api/nodes/lin/files/public?path=x']);
    });

    it('an action that hits an unreachable device never shows the raw message', async () => {
      vi.stubGlobal('fetch', vi.fn(async () => down()));
      await state().removeEverywhere('public', {
        path: 'r.pdf', kind: 'file', size: 1, modified: 1, here: false, on: [{ id: 'lin', name: 'linux' }],
      });
      expect(state().error).not.toMatch(/node unreachable/);
      expect(state().error).toBe('Device unreachable');
    });

    it('fresh entries are cached; a re-created store shows them stale before any answer', async () => {
      console.log('[Step 1/3] pc answers once');
      vi.stubGlobal('fetch', vi.fn(async () => json(pcFiles)));
      await state().loadOthers('public', [pc]);
      expect(JSON.parse(localStorage.getItem(CACHE) ?? '{}')).toEqual({ pc: pcFiles });
      console.log('[Step 2/3] app restarts: new store, pc never answers');
      vi.resetModules();
      const fresh = (await import('./files')).useFilesStore;
      vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>(() => {})));
      void fresh.getState().loadOthers('public', [pc]);
      console.log('[Step 3/3] last known entries are on screen, marked connecting');
      expect(fresh.getState().others.public).toEqual([{ ...pc, entries: pcFiles, stale: 'connecting' }]);
      expect(fresh.getState().links.pc.state).toBe('connecting');
    });

    it('a refresh replaces the cached entries, deletions included', async () => {
      vi.stubGlobal('fetch', vi.fn(async () => json(pcFiles)));
      await state().loadOthers('public', [pc]);
      const left = pcFiles.slice(0, 2);
      vi.stubGlobal('fetch', vi.fn(async () => json(left)));
      await state().loadOthers('public', [pc]);
      expect(state().others.public).toEqual([{ ...pc, entries: left }]);
      expect(JSON.parse(localStorage.getItem(CACHE) ?? '{}')).toEqual({ pc: left });
    });

    it('a failed refresh keeps the cached entries and does not overwrite the cache', async () => {
      vi.stubGlobal('fetch', vi.fn(async () => json(pcFiles)));
      await state().loadOthers('public', [pc]);
      vi.stubGlobal('fetch', vi.fn(async () => down()));
      await state().loadOthers('public', [pc]);
      expect(state().others.public[0]).toEqual({ ...pc, entries: pcFiles, stale: 'connecting' });
      expect(JSON.parse(localStorage.getItem(CACHE) ?? '{}')).toEqual({ pc: pcFiles });
    });

    it('a device that is no longer in the node list loses its entries and cache', async () => {
      vi.stubGlobal('fetch', vi.fn(async () => json(pcFiles)));
      await state().loadOthers('public', [pc, lin]);
      expect(Object.keys(JSON.parse(localStorage.getItem(CACHE) ?? '{}')).sort()).toEqual(['lin', 'pc']);
      await state().loadOthers('public', [pc]);
      expect(state().others.public.map((h) => h.id)).toEqual(['pc']);
      expect(Object.keys(JSON.parse(localStorage.getItem(CACHE) ?? '{}'))).toEqual(['pc']);
      expect(state().links.lin).toBeUndefined();
      state().prune(['me']);
      expect(localStorage.getItem(CACHE)).toBe('{}');
    });

    it('Private is never written to storage', async () => {
      vi.stubGlobal('fetch', vi.fn(async () => json(pcFiles)));
      await state().loadOthers('private', [pc]);
      expect(state().others.private[0].entries).toEqual(pcFiles);
      expect(localStorage.getItem(CACHE)).toBeNull();
      await state().load('private');
      expect(localStorage.length).toBe(0);
    });

    it('getHere takes the file from a device that answered, skipping a cached-only holder', async () => {
      const f = vi.fn(async (url: string, init?: RequestInit) => {
        if (url === '/api/nodes/pc/files/public/content?path=old.txt') return new Response('x');
        if (init?.method === 'POST') return json([]);
        return json([]);
      });
      vi.stubGlobal('fetch', f);
      const got = await state().getHere('public', [
        { path: 'old.txt', kind: 'file', size: 1, modified: 1, here: false, on: [{ ...lin, stale: 'offline' }, pc] },
      ]);
      expect(got).toBe(1);
      expect(f.mock.calls.map((c) => c[0])).toContain('/api/nodes/pc/files/public/content?path=old.txt');
      expect(f.mock.calls.some((c) => String(c[0]).includes('/nodes/lin/'))).toBe(false);
    });
  });
});
