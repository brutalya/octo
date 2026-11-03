import type { FileEntry } from '../api/types';

export interface TreeNode {
  entry: FileEntry;
  name: string;
  children: TreeNode[];
}

const baseName = (p: string) => p.slice(p.lastIndexOf('/') + 1);

/** Builds a nested tree from the server's flat file list. Missing parent dirs are synthesized. */
export function buildTree(entries: FileEntry[]): TreeNode[] {
  const byPath = new Map<string, TreeNode>();
  const roots: TreeNode[] = [];
  const attach = (node: TreeNode) => {
    byPath.set(node.entry.path, node);
    const i = node.entry.path.lastIndexOf('/');
    (i >= 0 ? ensure(node.entry.path.slice(0, i)).children : roots).push(node);
  };
  const ensure = (path: string): TreeNode => {
    const found = byPath.get(path);
    if (found) return found;
    const node: TreeNode = { entry: { path, kind: 'dir', size: 0, modified: 0 }, name: baseName(path), children: [] };
    attach(node);
    return node;
  };
  const sorted = [...entries].sort((a, b) => a.path.split('/').length - b.path.split('/').length);
  for (const entry of sorted) {
    const existing = byPath.get(entry.path);
    if (existing) {
      existing.entry = entry;
      continue;
    }
    attach({ entry, name: baseName(entry.path), children: [] });
  }
  const order = (ns: TreeNode[]) => {
    ns.sort((a, b) => (a.entry.kind === b.entry.kind ? a.name.localeCompare(b.name) : a.entry.kind === 'dir' ? -1 : 1));
    ns.forEach((n) => order(n.children));
  };
  order(roots);
  return roots;
}
