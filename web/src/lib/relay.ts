// The node stores one relay URL (`wss://host/relay?key=<secret>`); the UI edits address and key
// separately.

export function splitRelay(url: string | null | undefined): { address: string; key: string } {
  if (!url) return { address: '', key: '' };
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return { address: url, key: '' };
  }
  const key = u.searchParams.get('key') ?? '';
  u.searchParams.delete('key');
  const address = u.toString().replace(/\?$/, '');
  return { address, key };
}

/** Full relay URL from what the user typed: a bare host gets `wss://` and `/relay`. */
export function joinRelay(address: string, key: string): string {
  let a = address.trim();
  if (!a) return '';
  if (!/^wss?:\/\//i.test(a)) a = `wss://${a}`;
  let u: URL;
  try {
    u = new URL(a);
  } catch {
    return a;
  }
  if (u.pathname === '/' || u.pathname === '') u.pathname = '/relay';
  const k = key.trim();
  if (k) u.searchParams.set('key', k);
  else u.searchParams.delete('key');
  return u.toString().replace(/\?$/, '');
}
