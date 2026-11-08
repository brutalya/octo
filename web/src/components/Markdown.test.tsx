import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { Markdown } from './Markdown';

describe('Markdown', () => {
  it('opens links externally', () => {
    render(<Markdown>{'[x](https://e.com)'}</Markdown>);
    const a = screen.getByRole('link', { name: 'x' });
    expect(a).toHaveAttribute('target', '_blank');
    expect(a).toHaveAttribute('rel', 'noopener noreferrer');
  });

  it('never loads images; renders a link with the alt text', () => {
    const { container } = render(<Markdown>{'![pixel](https://t.example/p.gif)'}</Markdown>);
    expect(container.querySelector('img')).toBeNull();
    expect(screen.getByRole('link', { name: 'pixel' })).toHaveAttribute('href', 'https://t.example/p.gif');
  });

  it('fenced code without a language gets no inline styling', () => {
    const { container } = render(<Markdown>{'```\nplain\n```'}</Markdown>);
    expect(container.querySelector('pre code')?.className).not.toMatch(/bg-slate-100|px-1/);
  });
});
