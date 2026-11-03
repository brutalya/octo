import { describe, it, expect } from 'vitest';
import { parseSse, toAgentEvent, readAgentEvents } from './sse';
import type { AgentEvent } from './types';

describe('parseSse', () => {
  it('parses a complete message', () => {
    const { messages, rest } = parseSse('event: text_delta\ndata: {"a":1}\n\n');
    expect(messages).toEqual([{ event: 'text_delta', data: '{"a":1}' }]);
    expect(rest).toBe('');
  });

  it('keeps an incomplete tail as rest', () => {
    const { messages, rest } = parseSse('data: one\n\ndata: tw');
    expect(messages).toEqual([{ event: 'message', data: 'one' }]);
    expect(rest).toBe('data: tw');
  });

  it('is resumable across arbitrary chunk boundaries', () => {
    const full = 'event: a\ndata: 1\n\nevent: b\ndata: 2\n\n';
    for (let i = 0; i <= full.length; i++) {
      const first = parseSse(full.slice(0, i));
      const second = parseSse(first.rest + full.slice(i));
      expect([...first.messages, ...second.messages]).toEqual([
        { event: 'a', data: '1' },
        { event: 'b', data: '2' },
      ]);
    }
  });

  it('joins multi-line data with newline', () => {
    const { messages } = parseSse('data: a\ndata: b\n\n');
    expect(messages).toEqual([{ event: 'message', data: 'a\nb' }]);
  });

  it('handles CRLF line endings', () => {
    const { messages } = parseSse('event: x\r\ndata: y\r\n\r\n');
    expect(messages).toEqual([{ event: 'x', data: 'y' }]);
  });

  it('ignores comments and strips a single leading space', () => {
    const { messages } = parseSse(': ping\n\ndata:  two spaces\n\n');
    expect(messages).toEqual([{ event: 'message', data: ' two spaces' }]);
  });

  it('keeps unknown events as raw messages', () => {
    const { messages } = parseSse('event: weird\ndata: {}\n\n');
    expect(messages).toEqual([{ event: 'weird', data: '{}' }]);
  });
});

describe('toAgentEvent', () => {
  it('decodes tagged json', () => {
    expect(toAgentEvent({ event: 'text_delta', data: '{"type":"text_delta","text":"hi"}' })).toEqual({
      type: 'text_delta',
      text: 'hi',
    });
  });

  it('falls back to the event name when json has no type', () => {
    expect(toAgentEvent({ event: 'error', data: '{"message":"boom"}' })).toEqual({
      type: 'error',
      message: 'boom',
    });
  });

  it('returns null for unknown events and bad json', () => {
    expect(toAgentEvent({ event: 'weird', data: '{}' })).toBeNull();
    expect(toAgentEvent({ event: 'done', data: 'not json' })).toBeNull();
  });
});

function streamOf(parts: string[]): ReadableStream<Uint8Array> {
  const enc = new TextEncoder();
  return new ReadableStream({
    start(c) {
      parts.forEach((p) => c.enqueue(enc.encode(p)));
      c.close();
    },
  });
}

describe('readAgentEvents', () => {
  it('yields events across chunks, skipping unknown ones', async () => {
    const body = streamOf([
      'event: session\ndata: {"type":"sess',
      'ion","id":"s1"}\n\nevent: weird\ndata: {}\n\n',
      'data: {"type":"text_delta","text":"é"}\n\ndata: {"type":"done"}\n\n',
    ]);
    const out: AgentEvent[] = [];
    for await (const e of readAgentEvents(body)) out.push(e);
    expect(out).toEqual([
      { type: 'session', id: 's1' },
      { type: 'text_delta', text: 'é' },
      { type: 'done' },
    ]);
  });

  it('does not split multibyte characters across chunks', async () => {
    const bytes = new TextEncoder().encode('data: {"type":"text_delta","text":"é"}\n\n');
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        c.enqueue(bytes.slice(0, 35));
        c.enqueue(bytes.slice(35));
        c.close();
      },
    });
    const out: AgentEvent[] = [];
    for await (const e of readAgentEvents(body)) out.push(e);
    expect(out).toEqual([{ type: 'text_delta', text: 'é' }]);
  });
});
