/** Last two path segments of a cwd (either separator). */
export function shortCwd(cwd: string | null): string {
  if (!cwd) return '';
  const parts = cwd.split(/[\\/]+/).filter(Boolean);
  if (parts.length === 0) return cwd;
  return parts.slice(-2).join('/');
}

export function relativeTime(ms: number, now: number = Date.now()): string {
  const s = Math.max(0, (now - ms) / 1000);
  if (s < 60) return 'now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86_400) return `${Math.floor(s / 3600)}h`;
  const d = s / 86_400;
  if (d < 30) return `${Math.floor(d)}d`;
  if (d < 365) return `${Math.floor(d / 30)}mo`;
  return `${Math.floor(d / 365)}y`;
}

export const errMsg = (e: unknown): string => (e instanceof Error ? e.message : String(e));
