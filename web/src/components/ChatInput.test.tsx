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
