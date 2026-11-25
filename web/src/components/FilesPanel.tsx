import { useEffect, useRef, useState, type DragEvent } from 'react';
import { FolderPlus, Lock, PanelRightClose, PanelRightOpen, Upload, X } from 'lucide-react';
import { useFilesStore } from '../stores/files';
import { useThreadsStore } from '../stores/threads';
import { useNodesStore } from '../stores/nodes';
import { useAttachStore } from '../stores/attach';
import { FileTree } from './FileTree';
import { mergeDevices, type SharedEntry } from '../lib/shared';
import { api } from '../api/client';
import type { Space } from '../api/types';

/** Most files one message can carry (the server's limit). */
const MAX_ATTACH = 10;

const REFRESH_MS = 20_000;
const RETRY_MS = 3_000;

/** Loads the open device and (Public only) the other devices. `force` asks even those marked offline. */
async function pull(space: Space, force: boolean) {
  const st = useFilesStore.getState();
  const { nodes: all, selectedId: open } = useNodesStore.getState();
  const others = all
    .filter((n) => (open ? n.node_id !== open : !n.is_self))
    .map((n) => ({ id: n.is_self ? null : n.node_id, name: n.name, online: force || n.is_self || n.online }));
  // An empty node list means it is not loaded yet: nothing may be dropped from the cache for that.
  await Promise.all([st.load(space), space === 'public' && all.length > 0 ? st.loadOthers('public', others) : undefined]);
}

const TABS: { space: Space; label: string }[] = [
  { space: 'private', label: 'Private' },
  { space: 'public', label: 'Public' },
];

export function FilesPanel({ sheet = false, onClose }: { sheet?: boolean; onClose?: () => void } = {}) {
  const threadSpace = useThreadsStore((s) => s.threads.find((t) => t.id === s.activeId)?.space ?? null);
  const preview = useFilesStore((s) => s.preview);
  const error = useFilesStore((s) => s.error);
  const links = useFilesStore((s) => s.links);
  const [collapsed, setCollapsed] = useState(false);
  const [choice, setChoice] = useState<{ forThread: Space | null; space: Space } | null>(null);
  const [newRoot, setNewRoot] = useState(false);
  const [over, setOver] = useState(false);
  const [picked, setPicked] = useState<{ space: Space; paths: Set<string> }>({ space: 'public', paths: new Set() });
  const [busy, setBusy] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const nodes = useNodesStore((s) => s.nodes);
  const selectedId = useNodesStore((s) => s.selectedId);

  // An explicit tab pick is kept until the active thread's space changes.
  const space: Space = choice && choice.forThread === threadSpace ? choice.space : (threadSpace ?? 'public');
  const selected = picked.space === space ? picked.paths : new Set<string>();

  const openNode = nodes.find((n) => (selectedId ? n.node_id === selectedId : n.is_self));
  const device = openNode?.name ?? 'this device';
  // The other devices whose public files join the tree (private files already sync on their own).
  // Every member counts, online or not: one that drops out for a moment keeps its files on
  // screen (it is just not asked), so the tree does not flicker with the online status.
  const othersKey = nodes
    .filter((n) => n !== openNode)
    .map((n) => n.node_id)
    .join(',');
  const shown = sheet || !collapsed;
  // Auto-sync is this device's own setting: offered while this device is the open one.
  const [publicSync, setPublicSync] = useState<boolean | null>(null);
  const local = !selectedId;
  useEffect(() => {
    if (!shown || !local) return;
    let live = true;
    api.settings().then(
      (v) => live && setPublicSync(v.public_sync),
      () => live && setPublicSync(null),
    );
    return () => {
      live = false;
    };
  }, [shown, local]);
  const toggleSync = () => {
    const next = !publicSync;
    setPublicSync(next);
    api.patchSettings({ public_sync: next }).then(
      (v) => {
        setPublicSync(v.public_sync);
        // The first pull starts right away: show what it brought soon.
        if (v.public_sync) setTimeout(() => void useFilesStore.getState().load('public'), 3000);
      },
      () => setPublicSync(!next),
    );
  };

  useEffect(() => {
    if (!shown) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // Files change on other devices (and arrive by sync): the open panel follows every 20 s,
    // and a device that has not answered is tried again every few seconds.
    const tick = async () => {
      await pull(space, false);
      if (!live) return;
      const waiting = Object.values(useFilesStore.getState().links ?? {}).some((l) => l.state === 'connecting');
      timer = setTimeout(() => void tick(), waiting ? RETRY_MS : REFRESH_MS);
    };
    void tick();
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [space, shown, othersKey]);

  const offline = Object.entries(links ?? {})
    .filter(([, l]) => l.state === 'offline')
    .map(([k, l]) => (k === 'open' ? device : l.name));

  const select = (paths: string[], on: boolean) =>
    setPicked(() => {
      const next = new Set(selected);
      for (const p of paths) {
        if (on) next.add(p);
        else next.delete(p);
      }
      return { space, paths: next };
    });

  const canAttach = threadSpace === space;
  /** Brings files that are only elsewhere here first: the chat's agent reads them here. */
  const attach = async (items: SharedEntry[]) => {
    setBusy(true);
    try {
      const missing = items.filter((e) => !e.here);
      if (missing.length) await useFilesStore.getState().getHere(space, missing);
      const here = new Set(useFilesStore.getState().entries[space].map((e) => e.path));
      useAttachStore.getState().add(items.filter((e) => here.has(e.path)).map((e) => ({ space, path: e.path })));
      setPicked({ space, paths: new Set() });
      if (sheet) onClose?.();
    } finally {
      setBusy(false);
    }
  };
  const attachPicked = () => {
    const st = useFilesStore.getState();
    const all = new Map(mergeDevices({ id: null, name: device, entries: st.entries[space] }, st.others[space]).map((e) => [e.path, e]));
    void attach([...selected].map((p) => all.get(p)).filter((e): e is SharedEntry => !!e));
  };

  if (collapsed && !sheet) {
    return (
      <aside className="flex h-full shrink-0 flex-col border-l border-slate-200 bg-white p-2">
        <button
          aria-label="Expand files"
          onClick={() => setCollapsed(false)}
          className="rounded-md p-1.5 text-slate-500 hover:bg-slate-100 hover:text-slate-800"
        >
          <PanelRightOpen size={16} />
        </button>
      </aside>
    );
  }

  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    setOver(false);
    const files = Array.from(e.dataTransfer.files ?? []);
    if (files.length) void useFilesStore.getState().upload(space, '', files);
  };

  return (
    <aside
      className={
        sheet
          ? 'fixed inset-0 z-30 flex flex-col bg-white pb-[env(safe-area-inset-bottom)]'
          : 'flex h-full w-72 shrink-0 flex-col border-l border-slate-200 bg-white'
      }
    >
      <div className="flex h-14 shrink-0 items-center gap-1 border-b border-slate-200 px-3">
        <span className="flex-1 text-sm font-semibold text-slate-800">Files</span>
        <div className="flex gap-0.5">
          <button
            aria-label="New folder"
            title="New folder"
            onClick={() => setNewRoot(true)}
            className="rounded-md p-2 max-md:p-3 text-slate-500 hover:bg-slate-100 hover:text-slate-800"
          >
            <FolderPlus size={16} />
          </button>
          <input
            ref={fileInput}
            type="file"
            multiple
            aria-label="Upload file"
            className="hidden"
            onChange={(e) => {
              const files = Array.from(e.target.files ?? []);
              if (files.length) void useFilesStore.getState().upload(space, '', files);
              e.target.value = '';
            }}
          />
          <button
            aria-label="Upload"
            title="Upload"
            onClick={() => fileInput.current?.click()}
            className="rounded-md p-2 max-md:p-3 text-slate-500 hover:bg-slate-100 hover:text-slate-800"
          >
            <Upload size={16} />
          </button>
          <button
            aria-label={sheet ? 'Close files' : 'Collapse files'}
            onClick={() => (sheet ? onClose?.() : setCollapsed(true))}
            className="rounded-md p-2 max-md:p-3 text-slate-500 hover:bg-slate-100 hover:text-slate-800"
          >
            {sheet ? <X size={18} /> : <PanelRightClose size={16} />}
          </button>
        </div>
      </div>
      <div className="px-3 py-2.5">
        <div role="tablist" className="flex rounded-lg border border-slate-200 bg-slate-50 p-0.5">
          {TABS.map((t) => (
            <button
              key={t.space}
              role="tab"
              aria-selected={space === t.space}
              onClick={() => setChoice({ forThread: threadSpace, space: t.space })}
              className={`flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md text-xs max-md:h-10 ${
                space === t.space ? 'bg-slate-200 text-slate-800' : 'text-slate-500 hover:text-slate-800'
              }`}
            >
              {t.space === 'private' && <Lock size={12} />}
              {t.label}
            </button>
          ))}
        </div>
      </div>
      <div
        data-testid="files-drop"
        onDragOver={(e) => {
          if (e.dataTransfer.types?.includes('Files')) {
            e.preventDefault();
            setOver(true);
          }
        }}
        onDragLeave={() => setOver(false)}
        onDrop={onDrop}
        className={`min-h-24 flex-1 overflow-y-auto px-2 ${over ? 'bg-indigo-50' : ''}`}
      >
        <FileTree
          space={space}
          device={device}
          selected={selected}
          onSelect={select}
          onAttach={canAttach ? (e) => void attach([e]) : undefined}
          autoSync={local && space === 'public' && publicSync !== null ? { on: publicSync, toggle: toggleSync } : undefined}
          newRoot={newRoot}
          onNewRootDone={() => setNewRoot(false)}
        />
      </div>
      {error && <div className="px-3 py-1 text-xs text-red-500">{error}</div>}
      {offline.length > 0 && (
        <div className="flex items-center gap-2 px-3 py-1 text-xs text-slate-500">
          <span className="min-w-0 flex-1 truncate">
            {offline.join(', ')} {offline.length === 1 ? 'is' : 'are'} offline
          </span>
          <button onClick={() => void pull(space, true)} className="rounded px-1.5 py-0.5 text-slate-600 hover:bg-slate-100 max-md:py-2">
            Retry
          </button>
        </div>
      )}
      {selected.size > 0 && (
        <div className="flex items-center gap-2 border-t border-slate-200 bg-slate-50 px-3 py-2">
          <span className="min-w-0 flex-1 truncate text-sm text-slate-700">
            {busy ? `Getting files on ${device}…` : `${selected.size} selected`}
            {!busy && !canAttach && <span className="block text-xs text-slate-400">Open a {space} chat to attach</span>}
            {!busy && canAttach && selected.size > MAX_ATTACH && <span className="block text-xs text-slate-400">Up to {MAX_ATTACH} files</span>}
          </span>
          <button
            onClick={() => setPicked({ space, paths: new Set() })}
            className="h-9 rounded-md px-2 text-sm text-slate-500 hover:bg-slate-100 hover:text-slate-800 max-md:h-11"
          >
            Clear
          </button>
          {canAttach && (
            <button
              disabled={busy || selected.size > MAX_ATTACH}
              onClick={attachPicked}
              className="h-9 rounded-md bg-indigo-500 px-3 text-sm font-medium text-slate-50 hover:bg-indigo-600 disabled:opacity-40 max-md:h-11"
            >
              Attach to chat
            </button>
          )}
        </div>
      )}
      {!sheet && selected.size === 0 && (
        <div className="m-3 rounded-lg border border-dashed border-slate-300 p-3 text-center text-xs text-slate-400">
          Drop files here
        </div>
      )}
      {preview && <PreviewDialog />}
    </aside>
  );
}

/** The open file over the page, big enough to read; Escape or a click outside closes it. */
function PreviewDialog() {
  const preview = useFilesStore((s) => s.preview);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') useFilesStore.getState().closePreview();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, []);
  if (!preview) return null;
  const close = () => useFilesStore.getState().closePreview();
  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center p-4 max-md:p-0">
      <div data-testid="preview-backdrop" className="absolute inset-0 bg-slate-900/30" onClick={close} />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={preview.path}
        className="relative flex max-h-[85vh] w-full max-w-3xl flex-col rounded-lg bg-white shadow-xl max-md:h-full max-md:max-h-none max-md:rounded-none"
      >
        <div className="flex items-center justify-between gap-2 border-b border-slate-200 px-3 py-1.5">
          <span className="truncate text-sm text-slate-600">{preview.path}</span>
          <button
            aria-label="Close preview"
            onClick={close}
            className="rounded-md p-1 text-slate-400 hover:bg-slate-100 hover:text-slate-700 max-md:p-3"
          >
            <X size={16} />
          </button>
        </div>
        {preview.imageUrl ? (
          <img src={preview.imageUrl} alt={preview.path} className="min-h-0 flex-1 object-contain p-3" />
        ) : (
          <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap p-3 font-mono text-sm text-slate-700">{preview.text}</pre>
        )}
      </div>
    </div>
  );
}
