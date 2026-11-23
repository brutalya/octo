import { useEffect, useRef, useState, type DragEvent } from 'react';
import { FolderPlus, Lock, PanelRightClose, PanelRightOpen, Upload, X } from 'lucide-react';
import { useFilesStore } from '../stores/files';
import { useThreadsStore } from '../stores/threads';
import { FileTree } from './FileTree';
import type { Space } from '../api/types';

const TABS: { space: Space; label: string }[] = [
  { space: 'private', label: 'Private' },
  { space: 'public', label: 'Public' },
];

export function FilesPanel({ sheet = false, onClose }: { sheet?: boolean; onClose?: () => void } = {}) {
  const threadSpace = useThreadsStore((s) => s.threads.find((t) => t.id === s.activeId)?.space ?? null);
  const preview = useFilesStore((s) => s.preview);
  const error = useFilesStore((s) => s.error);
  const [collapsed, setCollapsed] = useState(false);
  const [choice, setChoice] = useState<{ forThread: Space | null; space: Space } | null>(null);
  const [newRoot, setNewRoot] = useState(false);
  const [over, setOver] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);

  // An explicit tab pick is kept until the active thread's space changes.
  const space: Space = choice && choice.forThread === threadSpace ? choice.space : (threadSpace ?? 'public');

  useEffect(() => {
    if (sheet || !collapsed) void useFilesStore.getState().load(space);
  }, [space, collapsed, sheet]);

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
        <FileTree space={space} newRoot={newRoot} onNewRootDone={() => setNewRoot(false)} />
      </div>
      {error && <div className="px-3 py-1 text-xs text-red-500">{error}</div>}
      {!sheet && (
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
