import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ChatInput } from './ChatInput';
import { useFilesStore } from '../stores/files';

describe('ChatInput', () => {
  it('Enter sends and clears', async () => {
    const onSend = vi.fn();
    render(<ChatInput streaming={false} onSend={onSend} onStop={() => {}} />);
    const box = screen.getByRole('textbox');
    await userEvent.type(box, 'hello{Enter}');
    expect(onSend).toHaveBeenCalledWith('hello', []);
    expect(box).toHaveValue('');
  });

  it('Shift+Enter inserts a newline', async () => {
    const onSend = vi.fn();
    render(<ChatInput streaming={false} onSend={onSend} onStop={() => {}} />);
    const box = screen.getByRole('textbox');
    await userEvent.type(box, 'a{Shift>}{Enter}{/Shift}b');
    expect(onSend).not.toHaveBeenCalled();
    expect(box).toHaveValue('a\nb');
  });

  it('does not send blank input', async () => {
    const onSend = vi.fn();
    render(<ChatInput streaming={false} onSend={onSend} onStop={() => {}} />);
    await userEvent.type(screen.getByRole('textbox'), '   {Enter}');
    expect(onSend).not.toHaveBeenCalled();
  });

  it('shows stop instead of send while streaming and blocks Enter', async () => {
    const onSend = vi.fn();
    const onStop = vi.fn();
    render(<ChatInput streaming onSend={onSend} onStop={onStop} />);
    expect(screen.queryByRole('button', { name: 'Send' })).toBeNull();
    await userEvent.type(screen.getByRole('textbox'), 'x{Enter}');
    expect(onSend).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(onStop).toHaveBeenCalled();
  });
});

describe('ChatInput attachments', () => {
  const files = [
    { path: 'a.md', kind: 'file' as const, size: 1, modified: 1 },
    { path: 'dir', kind: 'dir' as const, size: 0, modified: 1 },
  ];
  beforeEach(() => {
    useFilesStore.setState({ entries: { private: [], public: files }, load: vi.fn(async () => {}) });
  });

  it('paperclip picks a file from the current space and shows a chip', async () => {
    const onSend = vi.fn();
    render(<ChatInput space="public" streaming={false} onSend={onSend} onStop={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Attach' }));
    expect(screen.queryByRole('menuitem', { name: 'dir' })).toBeNull();
    await userEvent.click(screen.getByRole('menuitem', { name: 'a.md' }));
    expect(screen.getByText('a.md')).toBeInTheDocument();
    await userEvent.type(screen.getByRole('textbox'), 'hi{Enter}');
    expect(onSend).toHaveBeenCalledWith('hi', [{ space: 'public', path: 'a.md' }]);
    expect(screen.queryByText('a.md')).toBeNull();
  });

  it('chip can be removed', async () => {
    const onSend = vi.fn();
    render(<ChatInput space="public" streaming={false} onSend={onSend} onStop={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Attach' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'a.md' }));
    await userEvent.click(screen.getByRole('button', { name: 'Remove a.md' }));
    await userEvent.type(screen.getByRole('textbox'), 'hi{Enter}');
    expect(onSend).toHaveBeenCalledWith('hi', []);
  });

  it('dragging a tree file onto the input attaches it', () => {
    render(<ChatInput space="public" streaming={false} onSend={() => {}} onStop={() => {}} />);
    const data = JSON.stringify({ space: 'public', path: 'a.md' });
    fireEvent.drop(screen.getByTestId('chat-input'), {
      dataTransfer: { getData: (t: string) => (t === 'application/x-octo-file' ? data : ''), types: ['application/x-octo-file'] },
    });
    expect(screen.getByText('a.md')).toBeInTheDocument();
  });

  it('ignores files dragged from the other space', () => {
    render(<ChatInput space="public" streaming={false} onSend={() => {}} onStop={() => {}} />);
    const data = JSON.stringify({ space: 'private', path: 'secret.md' });
    fireEvent.drop(screen.getByTestId('chat-input'), {
      dataTransfer: { getData: () => data, types: ['application/x-octo-file'] },
    });
    expect(screen.queryByText('secret.md')).toBeNull();
  });
});

describe('ChatInput disabled', () => {
  it('disables send and does not submit', async () => {
    const onSend = vi.fn();
    render(<ChatInput streaming={false} disabled onSend={onSend} onStop={() => {}} />);
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
    await userEvent.type(screen.getByRole('textbox'), 'hi{Enter}');
    expect(onSend).not.toHaveBeenCalled();
  });
});

describe('ChatInput restore', () => {
  it('restores the text when onSend resolves false', async () => {
    const onSend = vi.fn(async () => false);
    render(<ChatInput streaming={false} onSend={onSend} onStop={() => {}} />);
    const box = screen.getByRole('textbox');
    await userEvent.type(box, 'hello{Enter}');
    await vi.waitFor(() => expect(box).toHaveValue('hello'));
  });

  it('attachment chip shows the basename with the full path as title', async () => {
    useFilesStore.setState({ entries: { private: [], public: [{ path: 'docs/a.md', kind: 'file', size: 1, modified: 1 }] } });
    render(<ChatInput space="public" streaming={false} onSend={() => {}} onStop={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Attach' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'docs/a.md' }));
    expect(screen.getByTitle('docs/a.md')).toHaveTextContent('a.md');
    expect(screen.getByTitle('docs/a.md')).not.toHaveTextContent('docs');
  });

  it('paperclip uploads a file from the device and attaches it', async () => {
    const stored = [{ path: 'photo.png', kind: 'file' as const, size: 3, modified: 1 }];
    const upload = vi.fn(async () => stored);
    useFilesStore.setState({ entries: { private: [], public: [] }, upload });
    const onSend = vi.fn();
    render(<ChatInput space="public" streaming={false} onSend={onSend} onStop={() => {}} />);
    // [1/3] the menu offers an upload even when the space has no files yet
    await userEvent.click(screen.getByRole('button', { name: 'Attach' }));
    expect(screen.getByRole('menuitem', { name: 'Upload file' })).toBeInTheDocument();
    // [2/3] picking a file uploads it to the thread's space
    const file = new File(['png'], 'photo.png', { type: 'image/png' });
    await userEvent.upload(screen.getByLabelText('Upload attachment'), file);
    expect(upload).toHaveBeenCalledWith('public', '', [file]);
    // [3/3] and attaches what the server stored
    expect(await screen.findByText('photo.png')).toBeInTheDocument();
    await userEvent.type(screen.getByRole('textbox'), 'look{Enter}');
    expect(onSend).toHaveBeenCalledWith('look', [{ space: 'public', path: 'photo.png' }]);
  });
});

