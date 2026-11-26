import { useCallback, useId, useRef, useState, type KeyboardEvent } from 'react';
import { Check, ChevronDown, ChevronRight, Cloud, CloudOff, File as FileIcon, Folder, HardDrive, Loader2, Lock, Minus, MoreHorizontal } from 'lucide-react';
import { useFilesStore } from '../stores/files';
import { useClickOutside } from '../lib/useClickOutside';
import { buildTree, type TreeNode } from '../lib/tree';
import { mergeDevices, missingUnder, type Reach, type SharedEntry } from '../lib/shared';
import type { FileEntry, Space } from '../api/types';

export const DRAG_TYPE = 'application/x-octo-file';

const parentOf = (path: string) => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '');
const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);
const EMPTY: never[] = [];

type Editing = { kind: 'rename'; path: string } | { kind: 'mkdir'; dir: string };
/** `path` '' = the space itself (the root row). */
type Menu = { x: number; y: number; entry: SharedEntry };

function NameInput({ initial, onDone }: { initial: string; onDone: (name: string | null) => void }) {
  const [value, setValue] = useState(initial);
  const done = useRef(false);
  const finish = (name: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(name);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') finish(value.trim() || null);
    else if (e.key === 'Escape') finish(null);
  };
  return (
    <input
      autoFocus
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onKeyDown={onKeyDown}
      onBlur={() => finish(null)}
      className="min-w-0 flex-1 rounded border border-indigo-500 bg-transparent px-1 text-sm text-slate-800 outline-none"
    />
  );
}

/** A checkbox that also shows "some inside are picked". */
function Pick({ label, state, onChange }: { label: string; state: 'all' | 'some' | 'none'; onChange: (on: boolean) => void }) {
  return (
    <span className="relative flex h-4 w-4 shrink-0 items-center justify-center max-md:h-5 max-md:w-5">
      <input
        type="checkbox"
        aria-label={label}
        checked={state === 'all'}
        ref={(el) => {
          if (el) el.indeterminate = state === 'some';
        }}
        onClick={(e) => e.stopPropagation()}
        onChange={(e) => onChange(e.target.checked)}
        className="peer absolute inset-0 m-0 cursor-pointer appearance-none rounded border border-slate-300 checked:border-indigo-500 checked:bg-indigo-500 indeterminate:border-indigo-500 focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500"
      />
      {state === 'all' && <Check size={12} strokeWidth={3.5} aria-hidden="true" className="pointer-events-none relative text-slate-50" />}
      {state === 'some' && <Minus size={12} strokeWidth={3.5} aria-hidden="true" className="pointer-events-none relative text-indigo-500" />}
    </span>
  );
}

const fmtSize = (n: number) =>
  n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${(n / 1024).toFixed(0)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`;

/**
 * One tree of the space across devices, seen from this device (`device`): files that are
 * only elsewhere are dimmed with the devices that have them.
 */
export function FileTree({
  space,
  device = 'this device',
  chatNode = null,
  selected,
  onSelect,
  onAttach,
  autoSync,
  newRoot = false,
  onNewRootDone,
}: {
  space: Space;
  /** Name of this device (where "here" is). */
  device?: string;
  /** The device the open chat runs on (null = this one): only files it can read are attachable. */
  chatNode?: string | null;
  /** Picked file paths; with `onSelect` the rows get checkboxes. */
  selected?: Set<string>;
  onSelect?: (paths: string[], on: boolean) => void;
  /** Offered in a file's menu when a chat can take it. */
  onAttach?: (e: SharedEntry) => void;
  /** This device's Public auto-sync. */
  autoSync?: { on: boolean; toggle: () => void };
  newRoot?: boolean;
  onNewRootDone?: () => void;
}) {
  const entries = useFilesStore((s) => s.entries[space]);
  const others = useFilesStore((s) => s.others?.[space] ?? EMPTY);
  const [expanded, setExpanded] = useState<Set<string>>(new Set(['']));
  const [menu, setMenuRaw] = useState<Menu | null>(null);
  /** The entry whose new folder the open menu asks for. */
  const [moving, setMoving] = useState<SharedEntry | null>(null);
  const setMenu = (m: Menu | null) => {
    setMenuRaw(m);
    setMoving(null);
    setNotice(null);
  };
  const [editing, setEditing] = useState<Editing | null>(null);
  const [uploadDir, setUploadDir] = useState('');
  /** Why the last action did nothing (a holder does not answer). */
  const [notice, setNotice] = useState<string | null>(null);
  const inputId = useId();

  const menuRef = useRef<HTMLDivElement>(null);
  const closeMenu = useCallback(() => {
    setMenuRaw(null);
    setMoving(null);
  }, []);
  useClickOutside(menuRef, menu !== null, closeMenu);

  const merged = mergeDevices({ id: null, name: device, entries }, others);
  const byPath = new Map(merged.map((e) => [e.path, e]));
  const label = space === 'private' ? 'Private' : 'Public';
  const root: SharedEntry = { path: '', kind: 'dir', size: 0, modified: 0, here: true, on: [] };

  const toggle = (path: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(path)) next.add(path);
      return next;
    });

  const filesUnder = (dir: string) =>
    merged.filter((e) => e.kind === 'file' && (dir === '' || e.path.startsWith(dir + '/'))).map((e) => e.path);
  const pickState = (paths: string[]): 'all' | 'some' | 'none' => {
    const n = paths.filter((p) => selected?.has(p)).length;
    return n === 0 ? 'none' : n === paths.length ? 'all' : 'some';
  };

  const finish = async (name: string | null) => {
    const e = editing;
    setEditing(null);
    if (!name || !e) return;
    const st = useFilesStore.getState();
    if (e.kind === 'rename') {
      const next = join(parentOf(e.path), name);
      const entry = byPath.get(e.path);
      if (next !== e.path && entry) await st.renameEverywhere(space, entry, next);
    } else {
      await st.mkdir(space, join(e.dir, name));
    }
  };

  const openMenu = (el: HTMLElement, entry: SharedEntry) => {
    const r = el.getBoundingClientRect();
    setMenu({ x: r.left, y: r.bottom, entry });
  };

  /** Only holders that have not answered this session: their entries are last known, not confirmed. */
  const cachedOnly = (e: SharedEntry) => !e.here && e.on.every((h) => h.stale);
  /** Holders that do not answer: a change must reach every device that has the entry. */
  const silent = (e: SharedEntry) => e.on.filter((h) => h.stale);
  /** A copy can be fetched from a device that answered. */
  const fetchable = (e: SharedEntry) => e.on.some((h) => !h.stale);
  /** Devices that stand in the way: a message naming them, or null when nothing does. */
  const blocker = (hs: { name: string; stale?: Reach }[]): string | null => {
    if (hs.length === 0) return null;
    const off = hs.filter((h) => h.stale === 'offline').map((h) => h.name);
    return off.length > 0 ? `${off.join(', ')} ${off.length === 1 ? 'is' : 'are'} offline` : `Connecting to ${hs.map((h) => h.name).join(', ')}`;
  };
  /** Runs `f` unless a device in `hs` is in the way; then says which. */
  const unless = (hs: { name: string; stale?: Reach }[], f: () => void) => () => {
    const why = blocker(hs);
    if (!why) return f();
    closeMenu();
    setNotice(why);
  };
  /** The open chat can read the file where it runs. */
  const onChat = (e: SharedEntry) => (chatNode === null ? e.here : e.on.some((h) => h.id === chatNode));
  /** What stands in the way of attaching: this device's chat needs the file here (fetched if need be), another's needs that device. */
  const attachBlock = (e: SharedEntry) =>
    chatNode === null ? (e.here || fetchable(e) ? [] : e.on) : e.on.filter((h) => h.id === chatNode && h.stale);

  const elsewhere = (e: SharedEntry) => {
    if (e.here) return null;
    const names = e.on.map((h) => h.name).join(', ');
    const offline = cachedOnly(e) && e.on.every((h) => h.stale === 'offline');
    return (
      <span className="flex shrink-0 items-center gap-1 text-xs text-slate-400" title={`Only on ${names}`}>
        {!cachedOnly(e) ? (
          <Cloud size={13} role="img" aria-label="Only on other devices" />
        ) : offline ? (
          <CloudOff size={13} role="img" aria-label="Offline" />
        ) : (
          <Loader2 size={13} role="img" aria-label="Connecting" className="animate-spin" />
        )}
        <span className="max-w-24 truncate">{names}</span>
      </span>
    );
  };

  const actions = (entry: SharedEntry, name: string) => (
    <button
      aria-label={entry.path === '' ? `${name} actions` : 'Actions'}
      onClick={(e) => {
        e.stopPropagation();
        openMenu(e.currentTarget, entry);
      }}
      className={`ml-auto h-6 w-6 shrink-0 items-center justify-center rounded text-slate-400 hover:text-slate-700 max-md:flex max-md:h-10 max-md:w-10 ${
        entry.path === '' ? 'flex' : 'hidden group-hover:flex'
      }`}
    >
      <MoreHorizontal size={16} />
    </button>
  );

  const row = (n: TreeNode, depth: number) => {
    const entry = byPath.get(n.entry.path) ?? { ...n.entry, here: true, on: [] };
    const isDir = entry.kind === 'dir';
    const open = expanded.has(entry.path);
    const renaming = editing?.kind === 'rename' && editing.path === entry.path;
    const inside = isDir ? filesUnder(entry.path) : [entry.path];
    return (
      <li key={entry.path}>
        {/* The name is the button; the checkbox and actions sit beside it, never inside. */}
        <div
          draggable={!isDir && onChat(entry)}
          onDragStart={(e) => e.dataTransfer.setData(DRAG_TYPE, JSON.stringify({ space, path: entry.path }))}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu({ x: e.clientX, y: e.clientY, entry });
          }}
          style={{ paddingLeft: depth * 14 + 4 }}
          className={`group flex items-center gap-1.5 rounded py-0.5 pr-1 text-sm hover:bg-slate-100 max-md:min-h-11 ${
            entry.here ? 'text-slate-600' : 'text-slate-400'
          } ${cachedOnly(entry) ? 'opacity-60' : ''} ${!isDir && selected?.has(entry.path) ? 'bg-indigo-50' : ''}`}
        >
          {onSelect && inside.length > 0 && (
            <Pick label={`Select ${n.name}`} state={pickState(inside)} onChange={(on) => onSelect(inside, on)} />
          )}
          {onSelect && inside.length === 0 && <span className="w-4 shrink-0 max-md:w-5" />}
          {renaming ? (
            <span className="flex min-w-0 flex-1 items-center gap-1.5">
              {isDir ? <ChevronRight size={14} className="shrink-0" /> : <span className="w-3.5 shrink-0" />}
              {isDir ? <Folder size={14} className="shrink-0" /> : <FileIcon size={14} className="shrink-0" />}
              <NameInput initial={n.name} onDone={(v) => void finish(v)} />
            </span>
          ) : (
            <button
              aria-expanded={isDir ? open : undefined}
              onClick={(e) => {
                setNotice(null);
                if (isDir) toggle(entry.path);
                else if (entry.here) void useFilesStore.getState().open(space, entry.path);
                else if (!fetchable(entry)) setNotice(blocker(entry.on)); // only offline devices have it
                else openMenu(e.currentTarget, entry); // not here: what can be done with it
              }}
              className="flex min-w-0 flex-1 items-center gap-1.5 self-stretch text-left"
            >
              {isDir ? (open ? <ChevronDown size={14} className="shrink-0" /> : <ChevronRight size={14} className="shrink-0" />) : <span className="w-3.5 shrink-0" />}
              {isDir ? <Folder size={14} className="shrink-0" /> : <FileIcon size={14} className="shrink-0" />}
              <span className="min-w-0 flex-1 truncate">{n.name}</span>
              {elsewhere(entry)}
            </button>
          )}
          {actions(entry, n.name)}
        </div>
        {isDir && open && (
          <ul>
            {editing?.kind === 'mkdir' && editing.dir === entry.path && (
              <li style={{ paddingLeft: (depth + 1) * 14 + 22 }} className="flex py-0.5">
                <NameInput initial="" onDone={(v) => void finish(v)} />
              </li>
            )}
            {n.children.map((c) => row(c, depth + 1))}
          </ul>
        )}
      </li>
    );
  };

  const item = 'flex w-full items-center gap-2 px-3 py-1.5 max-md:py-3 text-left text-sm text-slate-700 hover:bg-slate-100';
  const act = (f: () => void) => () => {
    f();
    setMenu(null);
  };
  const moveBody = (e: SharedEntry) => {
    const name = e.path.slice(e.path.lastIndexOf('/') + 1);
    const from = parentOf(e.path);
    const places = [
      '',
      ...merged
        .filter((d) => d.kind === 'dir')
        .map((d) => d.path)
        .sort((a, b) => a.localeCompare(b)),
    ].filter((d) => d !== from && d !== e.path && !d.startsWith(e.path + '/'));
    return (
      <>
        <div className="truncate border-b border-slate-200 px-3 pb-1.5 pt-1 text-xs text-slate-400">Move {name} to</div>
        <div className="max-h-64 overflow-y-auto">
          {places.map((d) => (
            <button
              key={d || '/'}
              role="menuitem"
              className={item}
              onClick={act(() => void useFilesStore.getState().renameEverywhere(space, e, join(d, name)))}
            >
              <Folder size={14} className="shrink-0 text-slate-400" />
              <span className="truncate">{d || label}</span>
            </button>
          ))}
          {places.length === 0 && <p className="px-3 py-2 text-sm text-slate-400">No other folders</p>}
        </div>
      </>
    );
  };

  const menuBody = (m: Menu) => {
    const e = m.entry;
    const isRoot = e.path === '';
    const name = isRoot ? label : e.path.slice(e.path.lastIndexOf('/') + 1);
    const st = useFilesStore.getState();
    if (moving) return moveBody(moving);
    const moveItem = (
      <button role="menuitem" className={item} onClick={unless(silent(e), () => setMoving(e))}>
        Move to folder
      </button>
    );
    if (e.kind === 'dir') {
      const missing = missingUnder(merged, e.path);
      return (
        <>
          <div className="border-b border-slate-200 px-3 pb-1.5 pt-1 text-xs text-slate-400">
            <div className="truncate text-sm font-medium text-slate-700">{name}</div>
            {filesUnder(e.path).length} files{missing.length > 0 && ` · ${missing.length} not on ${device}`}
          </div>
          <button
            role="menuitem"
            className={item}
            onClick={act(() => {
              setExpanded((p) => new Set(p).add(e.path));
              setEditing({ kind: 'mkdir', dir: e.path });
            })}
          >
            New folder
          </button>
          <button
            role="menuitem"
            className={item}
            onClick={act(() => {
              // The chooser must open synchronously inside the tap; closing the menu first would be too late.
              setUploadDir(e.path);
              document.getElementById(inputId)?.click();
            })}
          >
            Upload here
          </button>
          {missing.length > 0 && (
            <button
              role="menuitem"
              className={item}
              onClick={unless(missing.some(fetchable) ? [] : missing.flatMap((m) => m.on), act(() => void st.getHere(space, missing)))}
            >
              Get all on {device}
            </button>
          )}
          {isRoot && autoSync && (
            <button
              role="menuitemcheckbox"
              aria-checked={autoSync.on}
              className={item}
              onClick={act(autoSync.toggle)}
            >
              <span className="flex-1">Auto-sync {label}</span>
              <span className={`relative h-5 w-9 shrink-0 rounded-full ${autoSync.on ? 'bg-indigo-500' : 'bg-slate-200'}`}>
                <span className={`absolute top-0.5 h-4 w-4 rounded-full transition-[left] ${autoSync.on ? 'left-[18px] bg-slate-50' : 'left-0.5 bg-slate-400'}`} />
              </span>
            </button>
          )}
          {!isRoot && (
            <>
              <button role="menuitem" className={item} onClick={unless(silent(e), act(() => setEditing({ kind: 'rename', path: e.path })))}>
                Rename
              </button>
              {moveItem}
              <button role="menuitem" className={`${item} text-red-500`} onClick={unless(silent(e), act(() => void st.removeEverywhere(space, e)))}>
                {e.on.length ? 'Delete on all devices' : 'Delete'}
              </button>
            </>
          )}
        </>
      );
    }
    const where = e.here ? (e.on.length ? `on ${device} and ${e.on.map((h) => h.name).join(', ')}` : `on ${device}`) : `on ${e.on.map((h) => h.name).join(', ')} · not on ${device}`;
    return (
      <>
        <div className="border-b border-slate-200 px-3 pb-1.5 pt-1 text-xs text-slate-400">
          <div className="truncate text-sm font-medium text-slate-700">{name}</div>
          {fmtSize(e.size)} · {where}
        </div>
        {!e.here && (
          <button role="menuitem" className={item} onClick={unless(fetchable(e) ? [] : e.on, act(() => void st.getHere(space, [e])))}>
            Get on {device}
          </button>
        )}
        {onAttach && (chatNode === null || onChat(e)) && (
          <button role="menuitem" className={item} onClick={unless(attachBlock(e), act(() => onAttach(e)))}>
            Attach to chat
          </button>
        )}
        <button role="menuitem" className={item} onClick={unless(silent(e), act(() => setEditing({ kind: 'rename', path: e.path })))}>
          Rename
        </button>
        {moveItem}
        {space === 'public' && (
          <button role="menuitem" className={item} onClick={unless(silent(e), act(() => void st.moveToPrivate(e)))}>
            Move to Private
          </button>
        )}
        <button role="menuitem" className={`${item} text-red-500`} onClick={unless(silent(e), act(() => void st.removeEverywhere(space, e)))}>
          {e.on.length ? 'Delete on all devices' : 'Delete'}
        </button>
      </>
    );
  };

  const all = filesUnder('');
  const rootOpen = expanded.has('');
  return (
    <>
      <input
        id={inputId}
        type="file"
        multiple
        aria-label="Upload to folder"
        className="hidden"
        onChange={(e) => {
          const files = Array.from(e.target.files ?? []);
          e.target.value = '';
          if (files.length) void useFilesStore.getState().upload(space, uploadDir, files);
        }}
      />
      <div
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ x: e.clientX, y: e.clientY, entry: root });
        }}
        className="flex items-center gap-1.5 rounded border-b border-slate-200 py-1 pl-1 pr-1 text-sm max-md:min-h-11"
      >
        {onSelect && all.length > 0 && <Pick label={`Select all in ${label}`} state={pickState(all)} onChange={(on) => onSelect(all, on)} />}
        <button aria-expanded={rootOpen} onClick={() => toggle('')} className="flex min-w-0 flex-1 items-center gap-1.5 self-stretch text-left">
          {rootOpen ? <ChevronDown size={14} className="shrink-0 text-slate-400" /> : <ChevronRight size={14} className="shrink-0 text-slate-400" />}
          {space === 'private' ? <Lock size={14} className="shrink-0 text-violet-400" /> : <HardDrive size={14} className="shrink-0 text-slate-500" />}
          <span className="font-semibold text-slate-800">{label}</span>
          {autoSync && <span className="min-w-0 flex-1 truncate text-xs text-slate-400">{autoSync.on ? 'Auto-sync on' : 'Auto-sync off'}</span>}
        </button>
        {actions(root, label)}
      </div>
      {notice && (
        <p role="status" className="px-1 py-1 text-xs text-slate-500">
          {notice}
        </p>
      )}
      {rootOpen && (
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
          {editing?.kind === 'mkdir' && editing.dir === '' && (
            <li className="flex py-0.5 pl-6">
              <NameInput initial="" onDone={(v) => void finish(v)} />
            </li>
          )}
          {buildTree(merged as FileEntry[]).map((n) => row(n, 0))}
        </ul>
      )}
      {menu && (
        <div
          ref={menuRef}
          role="menu"
          style={{ position: 'fixed', left: Math.max(8, Math.min(menu.x, window.innerWidth - 232)), top: Math.max(8, Math.min(menu.y, window.innerHeight - 300)) }}
          className="z-20 w-56 rounded-lg border border-slate-200 bg-white py-1 shadow-lg"
        >
          {menuBody(menu)}
        </div>
      )}
    </>
  );
}
