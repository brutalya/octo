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

    it('asks the other online devices for their public files', () => {
      useFilesStore.setState({ loadOthers });
      render(<FilesPanel />);
      expect(loadOthers).toHaveBeenCalledWith('public', [{ id: null, name: 'phone' }]);
    });

    it('picked files go to the chat; ones only elsewhere are brought here first', async () => {
      console.log('[Step 1/3] pc is open; b.md is only on the phone');
      const getHere = vi.fn(async () => {
        useFilesStore.setState({ entries: { private: [], public: [f('a.md'), f('b.md')] } });
        return 1;
      });
      useFilesStore.setState({
        loadOthers,
        getHere,
        entries: { private: [], public: [f('a.md')] },
        others: { private: [], public: [{ id: null, name: 'phone', entries: [f('b.md')] }] },
      });
      render(<FilesPanel />);
      console.log('[Step 2/3] pick both and attach');
      await userEvent.click(screen.getByRole('checkbox', { name: 'Select all in Public' }));
      expect(screen.getByText('2 selected')).toBeInTheDocument();
      await userEvent.click(screen.getByRole('button', { name: 'Attach to chat' }));
      console.log('[Step 3/3] b.md was copied to pc, both are attached, the bar is gone');
      expect(getHere).toHaveBeenCalledWith('public', [expect.objectContaining({ path: 'b.md', here: false })]);
      expect(useAttachStore.getState().fromFiles).toEqual([
        { space: 'public', path: 'a.md' },
        { space: 'public', path: 'b.md' },
      ]);
      expect(screen.queryByText(/selected/)).toBeNull();
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
});
