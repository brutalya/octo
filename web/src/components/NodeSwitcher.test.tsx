import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { NodeSwitcher } from './NodeSwitcher';
import { useNodesStore } from '../stores/nodes';
import { setNodeBase } from '../api/client';
import type { NodeInfo } from '../api/types';

const nodes: NodeInfo[] = [
  { node_id: 'me', name: 'Home', is_self: true, online: true, trust: null },
  { node_id: 'vps', name: 'Vps', is_self: false, online: true, trust: 'public' },
  { node_id: 'ph', name: 'Phone', is_self: false, online: false, trust: 'public' },
];
beforeEach(() => {
  localStorage.clear();
  setNodeBase(null);
  useNodesStore.setState({ nodes, selectedId: null });
  vi.stubGlobal('fetch', vi.fn(async (url: string) => new Response(url.endsWith('/nodes') ? JSON.stringify(nodes) : '[]')));
});
afterEach(() => vi.unstubAllGlobals());

describe('NodeSwitcher', () => {
  it('shows the current node name', () => {
    render(<NodeSwitcher />);
    expect(screen.getByRole('button', { name: 'Node: Home' })).toBeInTheDocument();
  });

  it('lists self and members with online state', async () => {
    render(<NodeSwitcher />);
    await userEvent.click(screen.getByRole('button', { name: 'Node: Home' }));
    expect(screen.getAllByRole('option')).toHaveLength(3);
    expect(screen.getByRole('option', { name: /Vps/ })).toHaveAttribute('data-online', 'true');
    expect(screen.getByRole('option', { name: /Phone/ })).toHaveAttribute('data-online', 'false');
  });

  it('selecting a member switches and closes the list', async () => {
    render(<NodeSwitcher />);
    await userEvent.click(screen.getByRole('button', { name: 'Node: Home' }));
    await userEvent.click(screen.getByRole('option', { name: /Vps/ }));
    expect(useNodesStore.getState().selectedId).toBe('vps');
    expect(screen.queryByRole('listbox')).toBeNull();
    expect(screen.getByRole('button', { name: 'Node: Vps' })).toBeInTheDocument();
  });

  it('selected offline node is greyed', () => {
    useNodesStore.setState({ selectedId: 'ph' });
    render(<NodeSwitcher />);
    expect(screen.getByRole('button', { name: 'Node: Phone' })).toHaveClass('opacity-50');
  });
});
