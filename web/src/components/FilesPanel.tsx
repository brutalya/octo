import { useEffect, useRef, useState, type DragEvent } from 'react';
import { FolderPlus, PanelRightClose, PanelRightOpen, Upload, X } from 'lucide-react';
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
  const [collapsed, setCollapsed] = useState(false);
  const [choice, setChoice] = useState<{ forThread: Space | null; space: Space } | null>(null);
  const [newRoot, setNewRoot] = useState(false);
  const [over, setOver] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);

  // An explicit tab pick is kept until the active thread's space changes.
  const space: Space = choice && choice.forThread === threadSpace ? choice.space : (threadSpace ?? 'public');

  useEffect(() => {
    if (sheet || !collapsed) void useFilesStore.getState().load(space).catch(() => {});
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
      <div className="flex items-center justify-between border-b border-slate-200 px-2 py-1.5">
        <div role="tablist" className="flex gap-1">
          {TABS.map((t) => (
            <button
              key={t.space}
              role="tab"
              aria-selected={space === t.space}
              onClick={() => setChoice({ forThread: threadSpace, space: t.space })}
              className={`rounded-md px-2 py-0.5 text-xs ${
                space === t.space ? 'bg-slate-100 text-slate-800' : 'text-slate-500 hover:text-slate-800'
              }`}
            >
              {t.label}
            </button>
          ))}
        </div>
        <div className="flex gap-1">
          <button
            aria-label="New folder"
            onClick={() => setNewRoot(true)}
            className="rounded-md p-1 max-md:p-3 text-slate-500 hover:bg-slate-100 hover:text-slate-800"
          >
            <FolderPlus size={14} />
          </button>
          {sheet && (
            <>
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
                onClick={() => fileInput.current?.click()}
                className="rounded-md p-3 text-slate-500 hover:bg-slate-100 hover:text-slate-800"
              >
                <Upload size={16} />
              </button>
            </>
          )}
          <button
            aria-label={sheet ? 'Close files' : 'Collapse files'}
            onClick={() => (sheet ? onClose?.() : setCollapsed(true))}
            className="rounded-md p-1 max-md:p-3 text-slate-500 hover:bg-slate-100 hover:text-slate-800"
          >
            {sheet ? <X size={16} /> : <PanelRightClose size={14} />}
          </button>
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
        className={`min-h-24 flex-1 overflow-y-auto p-1 ${over ? 'bg-indigo-50' : ''}`}
      >
        <FileTree space={space} newRoot={newRoot} onNewRootDone={() => setNewRoot(false)} />
      </div>
      {preview && (
        <div className="flex max-h-[45%] flex-col border-t border-slate-200">
          <div className="flex items-center justify-between px-2 py-1">
            <span className="truncate text-xs text-slate-500">{preview.path}</span>
            <button
              aria-label="Close preview"
              onClick={() => useFilesStore.getState().closePreview()}
              className="rounded p-0.5 text-slate-400 hover:text-slate-700"
            >
              <X size={12} />
            </button>
          </div>
          <pre className="overflow-auto whitespace-pre-wrap px-2 pb-2 font-mono text-xs text-slate-700">{preview.text}</pre>
        </div>
      )}
    </aside>
  );
}
