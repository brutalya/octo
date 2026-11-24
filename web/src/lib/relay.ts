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

/** What a pasted code is: a device code (`<relay url>#<token>`, from Add device) or a relay
 *  code (the relay address with its key). */
export type CodeKind = 'empty' | 'device' | 'relay' | 'invalid';

const TOKEN = /^[a-z2-7]{52}$/i;

function relayUrlOf(s: string): URL | null {
  const explicit = /^[a-z][a-z0-9+.-]*:\/\//i.test(s);
  const withScheme = explicit ? s : `wss://${s}`;
  let u: URL;
  try {
    u = new URL(withScheme);
  } catch {
    return null;
  }
  if (u.protocol !== 'ws:' && u.protocol !== 'wss:') return null;
  const h = u.hostname;
  // A bare word is not a relay; with ws:// or wss:// typed out any host counts.
  if (!explicit && !(h.includes('.') || h.includes(':') || h === 'localhost' || u.port)) return null;
  return u;
}

export function classifyCode(code: string): CodeKind {
  const s = code.trim();
  if (!s) return 'empty';
  const hash = s.lastIndexOf('#');
  if (hash >= 0) {
    const url = s.slice(0, hash);
    return /^wss?:\/\//i.test(url) && relayUrlOf(url) && TOKEN.test(s.slice(hash + 1)) ? 'device' : 'invalid';
  }
  return relayUrlOf(s) ? 'relay' : 'invalid';
}

/** The relay URL a relay code stands for (a bare host gets `wss://` and `/relay`). */
export function relayFromCode(code: string): string {
  const s = code.trim();
  const { address, key } = splitRelay(/^wss?:\/\//i.test(s) ? s : `wss://${s}`);
  return joinRelay(address, key);
}

/** Relay host named by a device or relay code; '' if it names none. */
export function codeHost(code: string): string {
  const s = code.trim();
  const hash = s.lastIndexOf('#');
  return relayUrlOf(hash >= 0 ? s.slice(0, hash) : s)?.hostname ?? '';
}
