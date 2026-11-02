#!/usr/bin/env bash
# usage: install-rpr.sh VERSION   Installs the x86_64 Linux release of rpr into ~/.local/bin after checking its sha256.
set -euo pipefail
v=${1:?usage: install-rpr.sh VERSION}
t=rpr-x86_64-unknown-linux-gnu.tar.xz
u=https://github.com/reapr-corp/reapr/releases/download/rpr-v$v
d=$(mktemp -d)
cd "$d"
curl -fsSL --retry 3 -O "$u/$t" -O "$u/$t.sha256"
sha256sum -c "$t.sha256"
tar xf "$t"
install -D rpr-x86_64-unknown-linux-gnu/rpr "$HOME/.local/bin/rpr"
echo "$HOME/.local/bin" >> "${GITHUB_PATH:-/dev/null}"
DO_NOT_TRACK=1 "$HOME/.local/bin/rpr" --version
