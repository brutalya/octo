import type { AgentEvent } from './types';

interface SseMessage {
  event: string;
  data: string;
}

/** Parse as many complete SSE messages as `buffer` holds; returns the unconsumed tail. */
export function parseSse(buffer: string): { messages: SseMessage[]; rest: string } {
  const text = buffer.replace(/\r\n/g, '\n');
  const blocks = text.split('\n\n');
  const rest = blocks.pop() ?? '';
  const messages: SseMessage[] = [];
  for (const block of blocks) {
    let event = 'message';
    const data: string[] = [];
    let hasData = false;
    for (const line of block.split('\n')) {
      if (line === '' || line.startsWith(':')) continue;
      const i = line.indexOf(':');
      const field = i === -1 ? line : line.slice(0, i);
      let value = i === -1 ? '' : line.slice(i + 1);
      if (value.startsWith(' ')) value = value.slice(1);
      if (field === 'event') event = value;
      else if (field === 'data') {
        data.push(value);
        hasData = true;
      }
    }
    if (hasData) messages.push({ event, data: data.join('\n') });
  }
  return { messages, rest };
}

const KNOWN = new Set(['session', 'text_delta', 'tool', 'done', 'error']);

export function toAgentEvent(msg: SseMessage): AgentEvent | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(msg.data);
  } catch {
    return null;
  }
  if (typeof parsed !== 'object' || parsed === null) return null;
  const obj = parsed as Record<string, unknown>;
  const type = typeof obj.type === 'string' ? obj.type : msg.event;
  if (!KNOWN.has(type)) return null;
  return { ...obj, type } as AgentEvent;
}

export async function* readAgentEvents(body: ReadableStream<Uint8Array>): AsyncGenerator<AgentEvent> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  let ended = false;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) {
        ended = true;
        buffer += decoder.decode();
        if (buffer.trim()) buffer += '\n\n';
      } else {
        buffer += decoder.decode(value, { stream: true });
      }
      const parsed = parseSse(buffer);
      buffer = parsed.rest;
      for (const m of parsed.messages) {
        const e = toAgentEvent(m);
        if (e) yield e;
      }
      if (ended) break;
    }
  } finally {
    if (!ended) await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}
