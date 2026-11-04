import { describe, it, expect } from 'vitest';
import { relativeTime, shortCwd } from './format';

describe('shortCwd', () => {
  it('keeps the last two segments', () => {
    expect(shortCwd('/srv/apps/demo')).toBe('apps/demo');
    expect(shortCwd('C:\\work\\my app')).toBe('work/my app');
    expect(shortCwd('/one')).toBe('one');
    expect(shortCwd('/')).toBe('/');
    expect(shortCwd(null)).toBe('');
  });
});

describe('relativeTime', () => {
  const now = Date.UTC(2026, 2, 10, 12, 0, 0);
  it('formats coarse units', () => {
    expect(relativeTime(now - 20_000, now)).toBe('now');
    expect(relativeTime(now - 5 * 60_000, now)).toBe('5m');
    expect(relativeTime(now - 3 * 3_600_000, now)).toBe('3h');
    expect(relativeTime(now - 2 * 86_400_000, now)).toBe('2d');
    expect(relativeTime(now - 40 * 86_400_000, now)).toBe('1mo');
    expect(relativeTime(now - 400 * 86_400_000, now)).toBe('1y');
    expect(relativeTime(now + 5000, now)).toBe('now');
  });
});
