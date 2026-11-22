import { describe, it, expect } from 'vitest';
import { pastedName, uniqueName } from './files';

describe('uniqueName', () => {
  it('keeps a free name', () => {
    expect(uniqueName('a.png', ['b.png'])).toBe('a.png');
  });

  it('numbers a taken name before the extension', () => {
    expect(uniqueName('a.png', ['a.png'])).toBe('a (1).png');
    expect(uniqueName('a.png', ['a.png', 'a (1).png', 'a (2).png'])).toBe('a (3).png');
  });

  it('handles names without or with several dots', () => {
    expect(uniqueName('README', ['README'])).toBe('README (1)');
    expect(uniqueName('x.tar.gz', ['x.tar.gz'])).toBe('x.tar (1).gz');
    expect(uniqueName('.env', ['.env'])).toBe('.env (1)');
  });
});

describe('pastedName', () => {
  it('names a pasted image by time and type', () => {
    const at = new Date(2026, 10, 22, 9, 5, 7);
    expect(pastedName('image/png', at)).toBe('pasted-20261122-090507.png');
    expect(pastedName('image/jpeg', at)).toBe('pasted-20261122-090507.jpg');
    expect(pastedName('application/octet-stream', at)).toBe('pasted-20261122-090507');
  });
});
