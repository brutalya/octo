import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { FileTree } from './FileTree';
import { useFilesStore } from '../stores/files';

const rename = vi.fn(async () => {});
const remove = vi.fn(async () => {});
const mkdir = vi.fn(async () => {});
const open = vi.fn(async () => {});

beforeEach(() => {
  vi.clearAllMocks();
  useFilesStore.setState({
    entries: {
      private: [],
      public: [
        { path: 'docs', kind: 'dir', size: 0, modified: 1 },
        { path: 'docs/a.md', kind: 'file', size: 3, modified: 1 },
        { path: 'top.txt', kind: 'file', size: 1, modified: 1 },
      ],
    },
    rename, remove, mkdir, open,
  });
});

describe('FileTree', () => {
  it('folders start collapsed and toggle on click', async () => {
    render(<FileTree space="public" />);
    expect(screen.getByText('top.txt')).toBeInTheDocument();
    expect(screen.queryByText('a.md')).toBeNull();
    await userEvent.click(screen.getByText('docs'));
    expect(screen.getByText('a.md')).toBeInTheDocument();
    await userEvent.click(screen.getByText('docs'));
    expect(screen.queryByText('a.md')).toBeNull();
  });

  it('click on a file opens preview', async () => {
    render(<FileTree space="public" />);
    await userEvent.click(screen.getByText('top.txt'));
    expect(open).toHaveBeenCalledWith('public', 'top.txt');
  });

  it('context menu deletes', async () => {
    render(<FileTree space="public" />);
    fireEvent.contextMenu(screen.getByText('top.txt'));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    expect(remove).toHaveBeenCalledWith('public', 'top.txt');
  });

  it('context menu renames inline', async () => {
    render(<FileTree space="public" />);
    fireEvent.contextMenu(screen.getByText('top.txt'));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Rename' }));
    const input = screen.getByRole('textbox');
    await userEvent.clear(input);
    await userEvent.type(input, 'next.txt{Enter}');
    expect(rename).toHaveBeenCalledWith('public', 'top.txt', 'next.txt');
  });

  it('renaming a nested file keeps its directory', async () => {
    render(<FileTree space="public" />);
    await userEvent.click(screen.getByText('docs'));
    fireEvent.contextMenu(screen.getByText('a.md'));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Rename' }));
    const input = screen.getByRole('textbox');
    await userEvent.clear(input);
    await userEvent.type(input, 'b.md{Enter}');
    expect(rename).toHaveBeenCalledWith('public', 'docs/a.md', 'docs/b.md');
  });

  it('new folder inside a directory', async () => {
    render(<FileTree space="public" />);
    fireEvent.contextMenu(screen.getByText('docs'));
    await userEvent.click(screen.getByRole('menuitem', { name: 'New folder' }));
    await userEvent.type(screen.getByRole('textbox'), 'sub{Enter}');
    expect(mkdir).toHaveBeenCalledWith('public', 'docs/sub');
  });

  it('files are draggable carrying space and path', () => {
    render(<FileTree space="public" />);
    const set = vi.fn();
    fireEvent.dragStart(screen.getByText('top.txt'), { dataTransfer: { setData: set } });
    expect(set).toHaveBeenCalledWith('application/x-octo-file', JSON.stringify({ space: 'public', path: 'top.txt' }));
  });
});
