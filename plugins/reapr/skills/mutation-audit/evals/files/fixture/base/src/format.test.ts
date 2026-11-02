import { describe, expect, it } from 'vitest';
import { formatCents } from './format';

describe('formatCents', () => {
  it('formats whole dollars', () => {
    expect(formatCents(500)).toBe('$5.00');
  });
});
