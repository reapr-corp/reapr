# Semantic audit checklist

Mutation operators change what is there. They cannot add an input nobody thought of. Go through this list
for the code in the diff scope only, and keep only the items that apply. For each one you keep, write down:
the input, the behavior it would protect, why it matters here, and where the expected behavior is
specified. If nothing specifies it, it is a question for the human, not a test.

## Boundaries
- Exactly at, one below and one above every threshold, limit and range end (`>=` vs `>`).
- First and last element; off-by-one in loops, slices, pagination (`page * size`), substring ends.
- Minimum and maximum lengths, sizes and counts.

## Empty, null, undefined
- Empty string, empty array, empty object, empty map or set.
- `null` vs `undefined` vs missing key, where the types allow them.
- Whitespace-only strings; `0`, `NaN` and `false` where the code tests truthiness (`if (x)`, `x || default`).

## Errors and timeouts
- Each `throw`, rejected promise and error return: is the error type (and message, if it is part of the
  contract) asserted?
- Dependency failures: network error, timeout, partial response, malformed payload.
- Cleanup on failure: resources released, state not left half-written, retries bounded.

## Ordering and concurrency
- Input order: does the result depend on it when it should not (or the reverse)?
- Ties in sorting; stable vs unstable sort.
- Two concurrent calls: races on shared state, double submission, out-of-order async responses.

## Idempotency
- Calling twice gives the same result and the same side effects as calling once, where that is the contract
  (handlers, migrations, retries, webhooks).

## Time, locale, calendar
- Time zones (UTC vs local), DST transitions, midnight and end of month, leap years (Feb 29), year
  boundaries.
- Locale-dependent formatting and parsing (decimal separators, date formats, case mapping such as Turkish
  `i`).
- Clock injection: tests must not depend on the real current time.

## Numbers
- Floating point precision (`0.1 + 0.2`), rounding mode and where rounding happens (per line or on the
  total, especially with money).
- Integer overflow, `Number.MAX_SAFE_INTEGER`, BigInt boundaries; division by zero; negative zero.
- Negative values where only positives make sense; non-integers where integers are expected.

## Input validation at trust boundaries
- Data from users, files, network, environment or other services: wrong types, missing fields, extra
  fields, oversized input, injection-shaped strings (path traversal, SQL, HTML).
- That validation rejects bad input with the documented error, and accepts every valid input.

## How to propose

```markdown
- Edge case: coupon code with surrounding whitespace ("  SAVE10 ")
  Protects: a code pasted with spaces is still accepted
  Why: `applyCoupon` compares the raw string; no test passes padded input
  Specified by: README "codes are case- and whitespace-insensitive" → implemented
```
