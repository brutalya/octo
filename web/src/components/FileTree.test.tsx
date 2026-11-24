import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { FileTree } from './FileTree';
import { useFilesStore } from '../stores/files';

const renameEverywhere = vi.fn(async () => {});
const removeEverywhere = vi.fn(async () => {});
const getHere = vi.fn(async () => 1);
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
    others: { private: [], public: [] },
    renameEverywhere, removeEverywhere, getHere, mkdir, open,
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
    expect(removeEverywhere).toHaveBeenCalledWith('public', expect.objectContaining({ path: 'top.txt', here: true, on: [] }));
  });

  it('context menu renames inline', async () => {
    render(<FileTree space="public" />);
    fireEvent.contextMenu(screen.getByText('top.txt'));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Rename' }));
    const input = screen.getByRole('textbox');
    await userEvent.clear(input);
    await userEvent.type(input, 'next.txt{Enter}');
    expect(renameEverywhere).toHaveBeenCalledWith('public', expect.objectContaining({ path: 'top.txt' }), 'next.txt');
  });

  it('renaming a nested file keeps its directory', async () => {
    render(<FileTree space="public" />);
    await userEvent.click(screen.getByText('docs'));
    fireEvent.contextMenu(screen.getByText('a.md'));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Rename' }));
    const input = screen.getByRole('textbox');
    await userEvent.clear(input);
    await userEvent.type(input, 'b.md{Enter}');
    expect(renameEverywhere).toHaveBeenCalledWith('public', expect.objectContaining({ path: 'docs/a.md' }), 'docs/b.md');
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

  it('NameInput reports done once even when Enter is followed by blur', async () => {
    render(<FileTree space="public" newRoot onNewRootDone={() => {}} />);
    const input = screen.getByRole('textbox');
    await userEvent.type(input, 'x{Enter}');
    fireEvent.blur(input);
    expect(mkdir).toHaveBeenCalledTimes(1);
  });

  it('Actions button opens the menu without toggling; Enter toggles a folder', async () => {
    render(<FileTree space="public" />);
    await userEvent.click(screen.getAllByRole('button', { name: 'Actions' })[0]);
    expect(screen.getByRole('menuitem', { name: 'Rename' })).toBeInTheDocument();
    expect(screen.queryByText('a.md')).toBeNull();
    await userEvent.keyboard('{Escape}');
    expect(screen.queryByRole('menu')).toBeNull();
    const dir = screen.getByRole('button', { name: /docs/, expanded: false });
    dir.focus();
    await userEvent.keyboard('{Enter}');
    expect(screen.getByText('a.md')).toBeInTheDocument();
  });

  it('row actions button shows on hover on desktop, not only on phones', () => {
    render(<FileTree space="public" />);
    const btn = screen.getAllByRole('button', { name: 'Actions' })[0];
    expect(btn.className).toMatch(/group-hover:flex/);
  });

  describe('across devices', () => {
    const fromLinux = () =>
      useFilesStore.setState({
        others: {
          private: [],
          public: [{ id: 'lin', name: 'linux', entries: [
            { path: 'docs/a.md', kind: 'file', size: 3, modified: 1 },
            { path: 'report.pdf', kind: 'file', size: 2048, modified: 1 },
          ] }],
        },
      });

    it('a file only on another device is marked with that device and offers Get here', async () => {
      console.log('[Step 1/3] pc is open; linux also has report.pdf');
      fromLinux();
      render(<FileTree space="public" device="pc" />);
      console.log('[Step 2/3] the row names linux; clicking it opens its menu instead of a preview');
      const row = screen.getByRole('button', { name: /report\.pdf/ });
      expect(row).toHaveTextContent('linux');
      expect(screen.getAllByRole('img', { name: 'Only on other devices' })).toHaveLength(1);
      await userEvent.click(screen.getByText('report.pdf'));
      expect(open).not.toHaveBeenCalled();
      expect(screen.getByRole('menu')).toHaveTextContent('not on pc');
      console.log('[Step 3/3] Get on pc copies it here');
      await userEvent.click(screen.getByRole('menuitem', { name: 'Get on pc' }));
      expect(getHere).toHaveBeenCalledWith('public', [expect.objectContaining({ path: 'report.pdf', on: [{ id: 'lin', name: 'linux' }] })]);
    });

    it('a file on several devices is deleted on all of them', async () => {
      fromLinux();
      render(<FileTree space="public" device="pc" />);
      await userEvent.click(screen.getByText('docs'));
      fireEvent.contextMenu(screen.getByText('a.md'));
      await userEvent.click(screen.getByRole('menuitem', { name: 'Delete on all devices' }));
      expect(removeEverywhere).toHaveBeenCalledWith('public', expect.objectContaining({ path: 'docs/a.md', here: true, on: [{ id: 'lin', name: 'linux' }] }));
    });

    it('the space menu gets everything missing here', async () => {
      fromLinux();
      render(<FileTree space="public" device="pc" />);
      await userEvent.click(screen.getByRole('button', { name: 'Public actions' }));
      expect(screen.getByRole('menu')).toHaveTextContent('3 files · 1 not on pc');
      await userEvent.click(screen.getByRole('menuitem', { name: 'Get all on pc' }));
      expect(getHere).toHaveBeenCalledWith('public', [expect.objectContaining({ path: 'report.pdf' })]);
    });
  });

  describe('picking', () => {
    it('a folder checkbox picks the files inside; the root picks all', async () => {
      const onSelect = vi.fn();
      render(<FileTree space="public" selected={new Set(['docs/a.md'])} onSelect={onSelect} />);
      expect(screen.getByRole('checkbox', { name: 'Select docs' })).toBeChecked();
      const all = screen.getByRole('checkbox', { name: 'Select all in Public' }) as HTMLInputElement;
      expect(all.indeterminate).toBe(true);
      await userEvent.click(all);
      expect(onSelect).toHaveBeenCalledWith(['docs/a.md', 'top.txt'], true);
      await userEvent.click(screen.getByRole('checkbox', { name: 'Select docs' }));
      expect(onSelect).toHaveBeenLastCalledWith(['docs/a.md'], false);
    });

    it('Attach to chat is offered when a chat can take the file', async () => {
      const onAttach = vi.fn();
      render(<FileTree space="public" onAttach={onAttach} />);
      fireEvent.contextMenu(screen.getByText('top.txt'));
      await userEvent.click(screen.getByRole('menuitem', { name: 'Attach to chat' }));
      expect(onAttach).toHaveBeenCalledWith(expect.objectContaining({ path: 'top.txt' }));
    });

    it('Upload here uploads into that folder', async () => {
      const upload = vi.fn(async () => []);
      useFilesStore.setState({ upload });
      render(<FileTree space="public" />);
      fireEvent.contextMenu(screen.getByText('docs'));
      await userEvent.click(screen.getByRole('menuitem', { name: 'Upload here' }));
      const f = new File(['x'], 'x.txt');
      await userEvent.upload(screen.getByLabelText('Upload to folder'), f);
      expect(upload).toHaveBeenCalledWith('public', 'docs', [f]);
    });
  });
});
