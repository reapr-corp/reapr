#!/usr/bin/env bash
# Build one setup eval fixture, a fresh git repo on branch main with one commit.
#
# Usage: build-fixture.sh <variant> <dest>
#   js-no-config  the mutation-audit TypeScript + Vitest fixture (npm install included) with no Stryker config
#   rust          a small Rust crate with unit tests; no .cargo/mutants.toml
# Set NODE_MODULES_FROM=<dir> to copy an existing node_modules instead of running `npm install` (js-no-config).
set -euo pipefail

V=${1:?variant}
D=${2:?dest}
HERE=$(cd "$(dirname "$0")" && pwd)
G=(git -c user.name=dev -c user.email=dev@example.com -c commit.gpgsign=false)

case "$V" in
js-no-config)
  "$HERE/../../../mutation-audit/evals/files/build-fixture.sh" no-config "$D"
  # Setup starts from main, before any feature branch.
  git -C "$D" checkout -q main
  ;;
rust)
  rm -rf "$D"
  mkdir -p "$D/src"
  cd "$D"
  # Its own workspace root, wherever the fixture is built.
  printf '[package]\nname = "discount"\nversion = "0.1.0"\nedition = "2021"\n\n[workspace]\n' > Cargo.toml
  printf 'target/\n' > .gitignore
  cat > src/lib.rs <<'RS'
/// Orders of 10 units or more get 10% off, in cents.
pub fn price(units: u32, unit_cents: u32) -> u32 {
    let total = units * unit_cents;
    if units >= 10 {
        total - total / 10
    } else {
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_orders_pay_full_price() {
        assert_eq!(price(2, 500), 1000);
    }
}
RS
  git init -q -b main .
  git add -A
  "${G[@]}" commit -qm "feat: price with bulk discount"
  ;;
*)
  echo "unknown variant: $V" >&2
  exit 2
  ;;
esac
