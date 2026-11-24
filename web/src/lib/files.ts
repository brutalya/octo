/** `name` if free, else `stem (n).ext` with the smallest free n. */
export function uniqueName(name: string, taken: Iterable<string>): string {
  const used = new Set(taken);
  if (!used.has(name)) return name;
  const dot = name.lastIndexOf('.');
  const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ''];
  for (let n = 1; ; n++) {
    const candidate = `${stem} (${n})${ext}`;
    if (!used.has(candidate)) return candidate;
  }
}

const EXT: Record<string, string> = {
  'image/png': '.png',
  'image/jpeg': '.jpg',
  'image/webp': '.webp',
  'image/gif': '.gif',
};

/** Name for a file pasted from the clipboard (browsers call every screenshot `image.png`). */
export function pastedName(type: string, at: Date = new Date()): string {
  const p = (n: number) => String(n).padStart(2, '0');
  const stamp = `${at.getFullYear()}${p(at.getMonth() + 1)}${p(at.getDate())}-${p(at.getHours())}${p(at.getMinutes())}${p(at.getSeconds())}`;
  return `pasted-${stamp}${EXT[type] ?? ''}`;
}

/** Folder for files sent from the chat input: `Attachments/<YYYY-MM-DD> <chat title>`. */
export function attachDir(title: string | null | undefined, at: Date = new Date()): string {
  const p = (n: number) => String(n).padStart(2, '0');
  const day = `${at.getFullYear()}-${p(at.getMonth() + 1)}-${p(at.getDate())}`;
  // Only characters every device accepts in a folder name; dots at the ends are dropped too.
  const name = (title ?? '')
    .split('')
    .map((c) => (c < ' ' || '\\/:*?"<>|'.includes(c) ? ' ' : c))
    .join('')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 60)
    .replace(/^[.\s]+|[.\s]+$/g, '');
  return `Attachments/${name ? `${day} ${name}` : day}`;
}
