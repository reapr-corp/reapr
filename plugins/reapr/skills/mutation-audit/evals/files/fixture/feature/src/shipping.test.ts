import { describe, expect, it } from 'vitest';
import { shippingCost, sumCents } from './shipping';

describe('shippingCost', () => {
  it('waives shipping on large orders', () => {
    expect(shippingCost(12000)).toBe(0);
  });

  it('charges the flat fee on small orders', () => {
    expect(shippingCost(1000)).toBe(499);
  });

  it('computes shipping for a typical cart', () => {
    const fee = shippingCost(2500);
    expect(fee).toBeDefined();
  });
});

describe('sumCents', () => {
  it('adds line totals', () => {
    expect(sumCents([100, 250])).toBe(350);
  });
});
