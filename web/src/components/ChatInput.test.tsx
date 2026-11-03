import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ChatInput } from './ChatInput';

describe('ChatInput', () => {
  it('Enter sends and clears', async () => {
    const onSend = vi.fn();
    render(<ChatInput streaming={false} onSend={onSend} onStop={() => {}} />);
    const box = screen.getByRole('textbox');
    await userEvent.type(box, 'hello{Enter}');
    expect(onSend).toHaveBeenCalledWith('hello');
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
