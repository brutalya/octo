import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { FilesPanel } from './FilesPanel';
import { useFilesStore } from '../stores/files';
import { useThreadsStore } from '../stores/threads';
import { useNodesStore } from '../stores/nodes';
import { useAttachStore } from '../stores/attach';
import type { FileEntry, Thread } from '../api/types';

const th = (id: string, space: Thread['space']): Thread => ({
  id, title: id, provider: 'ollama', space, approval: 'confirm', model: 'm', cwd: null, origin: 'octo', created_at: 1, updated_at: 1,
});
const load = vi.fn(async () => {});
const upload = vi.fn(async () => [] as FileEntry[]);

beforeEach(() => {
  vi.clearAllMocks();
  useFilesStore.setState({ entries: { private: [], public: [] }, preview: null, load, upload });
  useThreadsStore.setState({ threads: [th('a', 'public'), th('b', 'private')], activeId: 'a' });
});

describe('FilesPanel', () => {
  it('tab follows the active thread space', () => {
    render(<FilesPanel />);
    expect(screen.getByRole('tab', { name: 'Public' })).toHaveAttribute('aria-selected', 'true');
    act(() => useThreadsStore.setState({ activeId: 'b' }));
    expect(screen.getByRole('tab', { name: 'Private' })).toHaveAttribute('aria-selected', 'true');
  });

  it('user can pick the other tab and it loads that space', async () => {
    render(<FilesPanel />);
    await userEvent.click(screen.getByRole('tab', { name: 'Private' }));
    expect(screen.getByRole('tab', { name: 'Private' })).toHaveAttribute('aria-selected', 'true');
    expect(load).toHaveBeenCalledWith('private');
  });

  it('dropping files uploads them to the active space root', () => {
    render(<FilesPanel />);
    const file = new File(['x'], 'x.txt');
    fireEvent.drop(screen.getByTestId('files-drop'), { dataTransfer: { files: [file], types: ['Files'] } });
    expect(upload).toHaveBeenCalledWith('public', '', [file]);
  });

  it('collapses', async () => {
    render(<FilesPanel />);
    await userEvent.click(screen.getByRole('button', { name: 'Collapse files' }));
    expect(screen.queryByRole('tab', { name: 'Public' })).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Expand files' }));
    expect(screen.getByRole('tab', { name: 'Public' })).toBeInTheDocument();
  });

  it('shows a text preview', () => {
    useFilesStore.setState({ preview: { space: 'public', path: 'a.md', text: 'hello body' } });
    render(<FilesPanel />);
    expect(screen.getByText('hello body')).toBeInTheDocument();
  });

  it('shows an image preview without a text block', () => {
    useFilesStore.setState({ preview: { space: 'private', path: 'x.png', text: '', imageUrl: 'blob:abc' } });
    render(<FilesPanel />);
    expect(screen.getByRole('img', { name: 'x.png' })).toHaveAttribute('src', 'blob:abc');
  });

  it('has an upload button on desktop too', async () => {
    render(<FilesPanel />);
    const file = new File(['x'], 'x.txt');
    await userEvent.upload(screen.getByLabelText('Upload file'), file);
    expect(upload).toHaveBeenCalledWith('public', '', [file]);
    expect(screen.getByRole('button', { name: 'Upload' })).toBeInTheDocument();
  });

  it('opens the preview in a dialog that Escape closes', async () => {
    const closePreview = vi.fn();
    useFilesStore.setState({ preview: { space: 'public', path: 'docs/a.md', text: 'hello body' }, closePreview });
    render(<FilesPanel />);
    const dialog = screen.getByRole('dialog', { name: 'docs/a.md' });
    expect(dialog).toHaveTextContent('hello body');
    await userEvent.keyboard('{Escape}');
    expect(closePreview).toHaveBeenCalled();
  });

  it('a click outside the preview closes it', async () => {
    const closePreview = vi.fn();
    useFilesStore.setState({ preview: { space: 'public', path: 'a.md', text: 'x' }, closePreview });
    render(<FilesPanel />);
    await userEvent.click(screen.getByTestId('preview-backdrop'));
    expect(closePreview).toHaveBeenCalled();
  });

  describe('picking files for the chat', () => {
    const f = (path: string): FileEntry => ({ path, kind: 'file', size: 1, modified: 1 });
    const loadOthers = vi.fn(async () => {});

    beforeEach(() => {
      useAttachStore.setState({ fromFiles: [] });
      useNodesStore.setState({
        nodes: [
          { node_id: 'me', name: 'phone', is_self: true, online: true, trust: null, agents: null },
          { node_id: 'pc', name: 'pc', is_self: false, online: true, trust: 'full', agents: true },
          { node_id: 'off', name: 'laptop', is_self: false, online: false, trust: 'full', agents: true },
        ],
        selectedId: 'pc',
      });
    });

    it('passes every other device (never this one, whichever is open for chats) with whether it is online (an offline one keeps its last list)', () => {
      useFilesStore.setState({ loadOthers });
      render(<FilesPanel />);
      expect(loadOthers).toHaveBeenCalledWith('public', [
        { id: 'pc', name: 'pc', online: true },
        { id: 'off', name: 'laptop', online: false },
      ]);
    });

    it('picked files go to the chat; ones only elsewhere are brought here first', async () => {
      console.log('[Step 1/3] this device runs the chat; b.md is only on pc');
      useNodesStore.setState({ selectedId: null });
      const getHere = vi.fn(async () => {
        useFilesStore.setState({ entries: { private: [], public: [f('a.md'), f('b.md')] } });
        return 1;
      });
      useFilesStore.setState({
        loadOthers,
        getHere,
        entries: { private: [], public: [f('a.md')] },
        others: { private: [], public: [{ id: 'pc', name: 'pc', entries: [f('b.md')] }] },
      });
      render(<FilesPanel />);
      console.log('[Step 2/3] pick both and attach');
      await userEvent.click(screen.getByRole('checkbox', { name: 'Select all in Public' }));
      expect(screen.getByText('2 selected')).toBeInTheDocument();
      await userEvent.click(screen.getByRole('button', { name: 'Attach to chat' }));
      console.log('[Step 3/3] b.md was copied here, both are attached, the bar is gone');
      expect(getHere).toHaveBeenCalledWith('public', [expect.objectContaining({ path: 'b.md', here: false })]);
      expect(useAttachStore.getState().fromFiles).toEqual([
        { space: 'public', path: 'a.md' },
        { space: 'public', path: 'b.md' },
      ]);
      expect(screen.queryByText(/selected/)).toBeNull();
    });

    it('a chat on another device only takes files that device has, nothing is copied', async () => {
      console.log('[Step 1/2] the chat runs on pc; a.md is on this phone only, b.md is on pc');
      const getHere = vi.fn(async () => 0);
      useFilesStore.setState({
        loadOthers,
        getHere,
        entries: { private: [], public: [f('a.md')] },
        others: { private: [], public: [{ id: 'pc', name: 'pc', entries: [f('b.md')] }] },
      });
      render(<FilesPanel />);
      await userEvent.click(screen.getByRole('checkbox', { name: 'Select all in Public' }));
      await userEvent.click(screen.getByRole('button', { name: 'Attach to chat' }));
      console.log('[Step 2/2] only b.md is attached');
      expect(getHere).not.toHaveBeenCalled();
      expect(useAttachStore.getState().fromFiles).toEqual([{ space: 'public', path: 'b.md' }]);
    });

    it('Private files of this device cannot go to a chat on another device', async () => {
      useThreadsStore.setState({ activeId: 'b' });
      useFilesStore.setState({ loadOthers, entries: { private: [f('s.md')], public: [] } });
      render(<FilesPanel />);
      await userEvent.click(screen.getByRole('checkbox', { name: 'Select s.md' }));
      expect(screen.queryByRole('button', { name: 'Attach to chat' })).toBeNull();
    });

    it('files of the other space cannot go to this chat', async () => {
      useFilesStore.setState({ loadOthers, entries: { private: [f('s.md')], public: [] } });
      render(<FilesPanel />);
      await userEvent.click(screen.getByRole('tab', { name: 'Private' }));
      await userEvent.click(screen.getByRole('checkbox', { name: 'Select s.md' }));
      expect(screen.getByText('Open a private chat to attach')).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Attach to chat' })).toBeNull();
    });
  });

  it('this device offers its Public auto-sync switch', async () => {
    console.log('[Step 1/2] pc is open for chats; this device setting says off');
    useNodesStore.setState({
      nodes: [
        { node_id: 'me', name: 'phone', is_self: true, online: true, trust: null, agents: null },
        { node_id: 'pc', name: 'pc', is_self: false, online: true, trust: 'full', agents: true },
      ],
      selectedId: 'pc',
    });
    const f = vi.fn(async (_url: string, init?: RequestInit) =>
      new Response(JSON.stringify({ public_sync: init?.method === 'PATCH' })),
    );
    vi.stubGlobal('fetch', f);
    render(<FilesPanel />);
    expect(await screen.findByText('Auto-sync off')).toBeInTheDocument();
    console.log('[Step 2/2] switching it on saves the setting');
    await userEvent.click(screen.getByRole('button', { name: 'Public actions' }));
    await userEvent.click(screen.getByRole('menuitemcheckbox', { name: 'Auto-sync Public' }));
    expect(await screen.findByText('Auto-sync on')).toBeInTheDocument();
    const patch = f.mock.calls.find((c) => ((c as unknown[])[1] as RequestInit | undefined)?.method === 'PATCH') as unknown as [string, RequestInit];
    expect(patch[0]).toBe('/api/settings');
    expect(JSON.parse(patch[1].body as string)).toEqual({ public_sync: true });
    vi.unstubAllGlobals();
  });

  describe('Public is one space', () => {
    const f = (path: string): FileEntry => ({ path, kind: 'file', size: 1, modified: 1 });
    const nodes = [
      { node_id: 'me', name: 'phone', is_self: true, online: true, trust: null, agents: null },
      { node_id: 'pc', name: 'pc', is_self: false, online: false, trust: 'full' as const, agents: true },
    ];

    it.each([null, 'pc'])('shows the same tree and header whichever device is open (%s)', (open) => {
      console.log('[Step 1/2] pc is offline but was indexed; open device = ' + open);
      useNodesStore.setState({ nodes, selectedId: open });
      useFilesStore.setState({
        loadOthers: vi.fn(async () => {}),
        entries: { private: [], public: [f('here.md')] },
        others: { private: [], public: [{ id: 'pc', name: 'pc', entries: [f('pc.md')], stale: 'offline' }] },
        links: { pc: { name: 'pc', state: 'offline' } },
      });
      render(<FilesPanel />);
      console.log('[Step 2/2] header is Public only, both files are listed, no spinner');
      expect(screen.getByRole('button', { name: 'Public' })).toBeInTheDocument();
      expect(screen.queryByText(/on phone|on pc/)).toBeNull();
      expect(screen.queryByRole('img', { name: 'Connecting' })).toBeNull();
      expect(screen.getByText('here.md')).toBeInTheDocument();
      expect(screen.getByText('pc.md')).toBeInTheDocument();
    });

    it('clicking a file of the offline device says it is offline', async () => {
      useNodesStore.setState({ nodes, selectedId: 'pc' });
      useFilesStore.setState({
        loadOthers: vi.fn(async () => {}),
        entries: { private: [], public: [] },
        others: { private: [], public: [{ id: 'pc', name: 'pc', entries: [f('pc.md')], stale: 'offline' }] },
      });
      render(<FilesPanel />);
      await userEvent.click(screen.getByText('pc.md'));
      expect(screen.getByRole('status')).toHaveTextContent('pc is offline');
    });
  });

  describe('connecting', () => {
    const loadOthers = vi.fn(async () => {});
    beforeEach(() => {
      useNodesStore.setState({
        nodes: [
          { node_id: 'me', name: 'phone', is_self: true, online: true, trust: null, agents: null },
          { node_id: 'lin', name: 'linux', is_self: false, online: true, trust: 'full', agents: true },
        ],
        selectedId: null,
      });
      useFilesStore.setState({ loadOthers, error: null });
    });

    it('shows no error while a device is connecting', () => {
      useFilesStore.setState({ links: { lin: { name: 'linux', state: 'connecting' } } });
      render(<FilesPanel />);
      expect(screen.queryByText(/offline|unreachable/)).toBeNull();
    });

    it('names an offline device and retries on request, asking even a device marked offline', async () => {
      useFilesStore.setState({ links: { lin: { name: 'linux', state: 'offline' } } });
      render(<FilesPanel />);
      expect(screen.getByText('linux is offline')).toBeInTheDocument();
      loadOthers.mockClear();
      await userEvent.click(screen.getByRole('button', { name: 'Retry' }));
      expect(loadOthers).toHaveBeenCalledWith('public', [{ id: 'lin', name: 'linux', online: true }]);
    });

    it('the line is gone once the device answers', () => {
      useFilesStore.setState({ links: { lin: { name: 'linux', state: 'offline' } } });
      render(<FilesPanel />);
      act(() => useFilesStore.setState({ links: { lin: { name: 'linux', state: 'online' } } }));
      expect(screen.queryByText(/offline/)).toBeNull();
    });
  });
});

