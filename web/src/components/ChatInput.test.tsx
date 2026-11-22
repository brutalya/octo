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

  it('files dropped from the computer are uploaded and attached', async () => {
    const upload = vi.fn(async (_s: string, _d: string, fs: File[]) =>
      fs.map((f) => ({ path: f.name, kind: 'file' as const, size: f.size, modified: 1 })),
    );
    useFilesStore.setState({ entries: { private: [], public: [] }, upload });
    render(<ChatInput space="public" streaming={false} onSend={vi.fn()} onStop={() => {}} />);
    const file = new File(['%PDF'], 'labs.pdf', { type: 'application/pdf' });
    fireEvent.drop(screen.getByTestId('chat-input'), { dataTransfer: { files: [file], types: ['Files'], getData: () => '' } });
    expect(await screen.findByText('labs.pdf')).toBeInTheDocument();
    expect(upload).toHaveBeenCalledWith('public', '', [file]);
  });

  it('a pasted screenshot is uploaded under a fresh name and attached', async () => {
    const upload = vi.fn(async (_s: string, _d: string, fs: File[]) =>
      fs.map((f) => ({ path: f.name, kind: 'file' as const, size: f.size, modified: 1 })),
    );
    useFilesStore.setState({ entries: { private: [], public: [] }, upload });
    render(<ChatInput space="public" streaming={false} onSend={vi.fn()} onStop={() => {}} />);
    const shot = new File(['png'], 'image.png', { type: 'image/png' });
    fireEvent.paste(screen.getByRole('textbox'), { clipboardData: { files: [shot], getData: () => '' } });
    const chip = await screen.findByText(/^pasted-\d{8}-\d{6}\.png$/);
    expect(chip).toBeInTheDocument();
    const sent = upload.mock.calls[0][2][0];
    expect(sent.name).toMatch(/^pasted-/);
    expect(sent.type).toBe('image/png');
  });

  it('pasting plain text does not upload anything', async () => {
    const upload = vi.fn(async () => []);
    useFilesStore.setState({ upload });
    render(<ChatInput space="public" streaming={false} onSend={vi.fn()} onStop={() => {}} />);
    fireEvent.paste(screen.getByRole('textbox'), { clipboardData: { files: [], getData: () => 'hello' } });
    expect(upload).not.toHaveBeenCalled();
  });

  it('an upload never overwrites a file of the same name', async () => {
    const upload = vi.fn(async (_s: string, _d: string, fs: File[]) =>
      fs.map((f) => ({ path: f.name, kind: 'file' as const, size: f.size, modified: 1 })),
    );
    useFilesStore.setState({ entries: { private: [], public: [{ path: 'a.md', kind: 'file', size: 1, modified: 1 }] }, upload });
    render(<ChatInput space="public" streaming={false} onSend={vi.fn()} onStop={() => {}} />);
    await userEvent.upload(screen.getByLabelText('Upload attachment'), new File(['new'], 'a.md'));
    expect(await screen.findByText('a (1).md')).toBeInTheDocument();
    expect(upload.mock.calls[0][2][0].name).toBe('a (1).md');
  });

  it('shows that an upload is running and when it failed', async () => {
    let finish: (v: never[]) => void = () => {};
    const upload = vi.fn(() => new Promise<never[]>((r) => (finish = r)));
    useFilesStore.setState({ entries: { private: [], public: [] }, upload, error: null });
    render(<ChatInput space="public" streaming={false} onSend={vi.fn()} onStop={() => {}} />);
    await userEvent.upload(screen.getByLabelText('Upload attachment'), new File(['x'], 'big.pdf'));
    expect(await screen.findByRole('status')).toHaveTextContent('Uploading big.pdf');
    useFilesStore.setState({ error: 'file too large' });
    finish([]);
    expect(await screen.findByRole('alert')).toHaveTextContent('big.pdf: file too large');
  });
});

