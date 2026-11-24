import { describe, it, expect } from 'vitest';
import { classifyCode, codeHost, joinRelay, relayFromCode, splitRelay } from './relay';

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
    const url = 'wss://relay.example.com/relay?key=s3cr3tKey';
    const { address, key } = splitRelay(url);
    expect(joinRelay(address, key)).toBe(url);
  });
});

describe('pasted codes', () => {
  const tok = 'a'.repeat(52);

  it('tells a device code from a relay code', () => {
    expect(classifyCode('')).toBe('empty');
    expect(classifyCode(`wss://relay.example.com/relay?key=k#${tok}`)).toBe('device');
    expect(classifyCode(` ws://10.0.0.2:7443/relay#${tok.toUpperCase()} `)).toBe('device');
    expect(classifyCode('wss://relay.example.com/relay?key=k')).toBe('relay');
    expect(classifyCode('relay.example.com?key=k')).toBe('relay');
    expect(classifyCode('relay.example.com')).toBe('relay');
    expect(classifyCode('localhost:7443')).toBe('relay');
    expect(classifyCode('hello')).toBe('invalid');
    expect(classifyCode('ws://relay/relay')).toBe('relay');
    expect(classifyCode('wss://relay.example.com/relay#short')).toBe('invalid');
    expect(classifyCode('https://relay.example.com')).toBe('invalid');
  });

  it('turns a relay code into the relay url the node stores', () => {
    expect(relayFromCode('relay.example.com?key=k')).toBe('wss://relay.example.com/relay?key=k');
    expect(relayFromCode(' wss://relay.example.com/relay?key=k ')).toBe('wss://relay.example.com/relay?key=k');
    expect(relayFromCode('ws://h:7443')).toBe('ws://h:7443/relay');
  });

  it('names the relay host of either code', () => {
    expect(codeHost(`wss://relay.example.com/relay?key=k#${tok}`)).toBe('relay.example.com');
    expect(codeHost('relay.example.com?key=k')).toBe('relay.example.com');
    expect(codeHost('hello')).toBe('');
  });
});
