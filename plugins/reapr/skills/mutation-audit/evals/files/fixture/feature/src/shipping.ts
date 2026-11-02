/** Orders at or above this subtotal ship free. */
export const FREE_SHIPPING_THRESHOLD_CENTS = 5000;
export const FLAT_FEE_CENTS = 499;

/**
 * Shipping fee for an order, in cents.
 * Orders of $50.00 or more ship free; smaller orders pay a flat fee.
 *
 * @throws RangeError if `subtotalCents` is negative or not an integer.
 */
export function shippingCost(subtotalCents: number): number {
  if (!Number.isInteger(subtotalCents) || subtotalCents < 0) {
    throw new RangeError('subtotalCents must be a non-negative integer');
  }
  if (subtotalCents >= FREE_SHIPPING_THRESHOLD_CENTS) {
    return 0;
  }
  return FLAT_FEE_CENTS;
}

/** Sum of line totals, in cents. */
export function sumCents(lines: number[]): number {
  if (lines.length === 0) return 0;
  return lines.reduce((total, line) => total + line, 0);
}
