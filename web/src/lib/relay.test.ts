import { describe, it, expect } from 'vitest';
import { joinRelay, splitRelay } from './relay';

describe('relay url', () => {
  it('splits the key off and keeps the rest', () => {
    expect(splitRelay('wss://relay.example.com/relay?key=abc')).toEqual({
      address: 'wss://relay.example.com/relay',
      key: 'abc',
    });
    expect(splitRelay('ws://h:7443/relay')).toEqual({ address: 'ws://h:7443/relay', key: '' });
    expect(splitRelay(null)).toEqual({ address: '', key: '' });
    expect(splitRelay('not a url')).toEqual({ address: 'not a url', key: '' });
  });

  it('builds a full relay url from a bare host and a key', () => {
    expect(joinRelay('relay.example.com', 'abc')).toBe('wss://relay.example.com/relay?key=abc');
    expect(joinRelay(' relay.example.com ', ' ')).toBe('wss://relay.example.com/relay');
    expect(joinRelay('ws://h:7443/relay', '')).toBe('ws://h:7443/relay');
    expect(joinRelay('wss://h/custom', 'k')).toBe('wss://h/custom?key=k');
    expect(joinRelay('', 'k')).toBe('');
  });

  it('round-trips', () => {
    const url = 'wss://relay.example.com/relay?key=9dgnJSQV';
    const { address, key } = splitRelay(url);
    expect(joinRelay(address, key)).toBe(url);
  });
});
