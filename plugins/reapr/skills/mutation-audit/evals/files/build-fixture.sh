#!/usr/bin/env bash
# Build one eval fixture: a small TypeScript + Vitest + StrykerJS git repo with branch main (base) and
# branch feat/shipping (one commit to audit). Run rpr in it with `rpr run --diff main --json`.
#
# Usage: build-fixture.sh <variant> <dest>
#   variants: weak-boundary | no-config | ambiguous-spec | pinned-bug
# Set NODE_MODULES_FROM=<dir> to copy an existing node_modules instead of running `npm install`.
set -euo pipefail

V=${1:?variant}
D=${2:?dest}
case "$V" in weak-boundary | no-config | ambiguous-spec | pinned-bug) ;; *)
  echo "unknown variant: $V" >&2
  exit 2
  ;;
esac
SRC=$(cd "$(dirname "$0")/fixture" && pwd)
G=(git -c user.name=dev -c user.email=dev@example.com -c commit.gpgsign=false)

rm -rf "$D"
mkdir -p "$D"
cd "$D"

# Base commit on main.
cp -R "$SRC/base/." .
mv gitignore .gitignore
[ "$V" = no-config ] && rm stryker.config.mjs
if [ -n "${NODE_MODULES_FROM:-}" ]; then
  cp -a "$NODE_MODULES_FROM" node_modules
else
  npm install --silent --no-audit --no-fund
fi
git init -q -b main .
git add -A
"${G[@]}" commit -qm "chore: initial project"

# Feature commit on feat/shipping.
git checkout -qb feat/shipping
cp -R "$SRC/feature/." .

case "$V" in
ambiguous-spec)
  # The code says `>`; the JSDoc, the constant's comment and the README say "$50.00 or more".
  sed -i 's/if (subtotalCents >= FREE_SHIPPING_THRESHOLD_CENTS)/if (subtotalCents > FREE_SHIPPING_THRESHOLD_CENTS)/' src/shipping.ts
  printf '# shop-utils\n\nCart helpers.\n\n## Shipping\n\nOrders of $50.00 or more ship free. Smaller orders pay a flat $4.99 fee.\n' > README.md
  ;;
pinned-bug)
  # The code, a code comment, the constant's comment and an existing test agree on `>`; the public docs
  # (README and the exported function's JSDoc) say "$50.00 or more". The boundary mutant is killed by
  # the test that pins the bug, so mutation testing alone reports nothing at the threshold.
  sed -i 's|^/\*\* Orders at or above this subtotal ship free. \*/|/** Orders above this subtotal ship free. */|' src/shipping.ts
  sed -i 's|^  if (subtotalCents >= FREE_SHIPPING_THRESHOLD_CENTS) {|  // Strictly above the threshold: exactly $50.00 still pays the fee.\n  if (subtotalCents > FREE_SHIPPING_THRESHOLD_CENTS) {|' src/shipping.ts
  sed -i "s|^  it('charges the flat fee on small orders', () => {|  it('charges the flat fee at exactly the threshold', () => {\n    expect(shippingCost(5000)).toBe(499);\n  });\n\n  it('charges the flat fee on small orders', () => {|" src/shipping.test.ts
  printf '# shop-utils\n\nCart helpers.\n\n## Shipping\n\n`shippingCost(subtotalCents)`: orders of $50.00 or more ship free. Smaller orders pay a flat $4.99 fee.\n' > README.md
  ;;
esac

git add -A
"${G[@]}" commit -qm "feat: shipping fee and cart total"
