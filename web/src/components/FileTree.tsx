import { useEffect, useState, type KeyboardEvent } from 'react';
import { ChevronDown, ChevronRight, File as FileIcon, Folder } from 'lucide-react';
import { useFilesStore } from '../stores/files';
import { buildTree, type TreeNode } from '../lib/tree';
import type { FileEntry, Space } from '../api/types';

export const DRAG_TYPE = 'application/x-octo-file';

const parentOf = (path: string) => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '');
const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

type Editing = { kind: 'rename'; path: string } | { kind: 'mkdir'; dir: string };

function NameInput({ initial, onDone }: { initial: string; onDone: (name: string | null) => void }) {
  const [value, setValue] = useState(initial);
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') onDone(value.trim() || null);
    else if (e.key === 'Escape') onDone(null);
  };
  return (
    <input
      autoFocus
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onKeyDown={onKeyDown}
      onBlur={() => onDone(null)}
      className="min-w-0 flex-1 rounded border border-indigo-500 bg-transparent px-1 text-sm text-slate-800 outline-none"
    />
  );
}

export function FileTree({ space, newRoot = false, onNewRootDone }: { space: Space; newRoot?: boolean; onNewRootDone?: () => void }) {
  const entries = useFilesStore((s) => s.entries[space]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [menu, setMenu] = useState<{ x: number; y: number; entry: FileEntry } | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    document.addEventListener('click', close);
    return () => document.removeEventListener('click', close);
  }, [menu]);

  const toggle = (path: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(path)) next.add(path);
      return next;
    });

  const finish = async (name: string | null) => {
    const e = editing;
    setEditing(null);
    if (!name || !e) return;
    const st = useFilesStore.getState();
    if (e.kind === 'rename') {
      const next = join(parentOf(e.path), name);
      if (next !== e.path) await st.rename(space, e.path, next);
    } else {
      await st.mkdir(space, join(e.dir, name));
    }
  };

  const row = (n: TreeNode, depth: number) => {
    const { entry } = n;
    const isDir = entry.kind === 'dir';
    const open = expanded.has(entry.path);
    const renaming = editing?.kind === 'rename' && editing.path === entry.path;
    return (
      <li key={entry.path}>
        <div
          draggable={!isDir}
          onDragStart={(e) => e.dataTransfer.setData(DRAG_TYPE, JSON.stringify({ space, path: entry.path }))}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu({ x: e.clientX, y: e.clientY, entry });
          }}
          onClick={() => (isDir ? toggle(entry.path) : void useFilesStore.getState().open(space, entry.path))}
          style={{ paddingLeft: depth * 12 + 4 }}
          className="flex cursor-pointer items-center gap-1 rounded py-0.5 pr-1 text-sm text-slate-600 hover:bg-slate-100"
        >
          {isDir ? (open ? <ChevronDown size={14} /> : <ChevronRight size={14} />) : <span className="w-3.5" />}
          {isDir ? <Folder size={14} className="shrink-0" /> : <FileIcon size={14} className="shrink-0" />}
          {renaming ? <NameInput initial={n.name} onDone={(v) => void finish(v)} /> : <span className="truncate">{n.name}</span>}
        </div>
        {isDir && open && (
          <ul>
            {editing?.kind === 'mkdir' && editing.dir === entry.path && (
              <li style={{ paddingLeft: (depth + 1) * 12 + 22 }} className="flex py-0.5">
                <NameInput initial="" onDone={(v) => void finish(v)} />
              </li>
            )}
            {n.children.map((c) => row(c, depth + 1))}
          </ul>
        )}
      </li>
    );
  };

  const item = 'block w-full px-3 py-1 text-left text-sm text-slate-700 hover:bg-slate-100';
  return (
    <>
      <ul>
        {newRoot && (
          <li className="flex px-1 py-0.5">
            <NameInput
              initial=""
              onDone={(v) => {
                onNewRootDone?.();
                if (v) void useFilesStore.getState().mkdir(space, v);
              }}
            />
          </li>
        )}
        {buildTree(entries).map((n) => row(n, 0))}
      </ul>
      {menu && (
        <div
          role="menu"
          style={{ position: 'fixed', left: menu.x, top: menu.y }}
          className="z-20 min-w-32 rounded-lg border border-slate-200 bg-white py-1 shadow-lg"
        >
          <button
            role="menuitem"
            className={item}
            onClick={() => {
              setEditing({ kind: 'rename', path: menu.entry.path });
              setMenu(null);
            }}
          >
            Rename
          </button>
          {menu.entry.kind === 'dir' && (
            <button
              role="menuitem"
              className={item}
              onClick={() => {
                setExpanded((p) => new Set(p).add(menu.entry.path));
                setEditing({ kind: 'mkdir', dir: menu.entry.path });
                setMenu(null);
              }}
            >
              New folder
            </button>
          )}
          <button
            role="menuitem"
            className={`${item} text-red-500`}
            onClick={() => {
              void useFilesStore.getState().remove(space, menu.entry.path);
              setMenu(null);
            }}
          >
            Delete
          </button>
        </div>
      )}
    </>
  );
}
