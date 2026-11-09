import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { FilesPanel } from './FilesPanel';
import { useFilesStore } from '../stores/files';
import { useThreadsStore } from '../stores/threads';
import type { Thread } from '../api/types';

const th = (id: string, space: Thread['space']): Thread => ({
  id, title: id, provider: 'ollama', space, approval: 'confirm', model: 'm', cwd: null, origin: 'octo', created_at: 1, updated_at: 1,
});
const load = vi.fn(async () => {});
const upload = vi.fn(async () => {});

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
});
