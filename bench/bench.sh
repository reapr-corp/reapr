#!/usr/bin/env bash
# Runs variants round-robin, one run at a time: for rep in FIRST..LAST: A B B0 C D E.
# usage: REAPR_BIN=<rpr> TARGET=<repo> CONFIG=<config> BASE=<base> MUTATE=<ranges> bench/bench.sh [FIRST] [LAST]
#   TARGET            the repo to measure (never edited; state files are restored)
#   CONFIG            Stryker config, relative to TARGET
#   BASE              base ref for variant A (`rpr run --diff BASE`)
#   MUTATE            the --mutate list Reapr derives from the diff, e.g. "pkg/src/a.ts:5-9,pkg/src/b.ts:1-1"
#   IGNORE            variant E only: the config's ignorePatterns plus the extra entry (the CLI value replaces the list)
#   INCREMENTAL       incremental state file, relative to TARGET (default reports/stryker-incremental.json)
#   VARIANTS="A B"    subset (default: A B B0 C D E)
#   QUIET_WAIT=1      before each run, poll the 1-min loadavg every 30 s until it is <= 4; the total
#                     wait budget is 20 min, after which the script stops (results stay "measured under load")
set -euo pipefail
# Benchmark runs never send telemetry.
export DO_NOT_TRACK=1
HERE=$(cd "$(dirname "$0")" && pwd)
FIRST=${1:-1}
LAST=${2:-5}
BIN=${REAPR_BIN:?set REAPR_BIN to the rpr binary}
: "${TARGET:?set TARGET to the repo to measure}" "${CONFIG:?set CONFIG}" "${BASE:?set BASE}" "${MUTATE:?set MUTATE}"
export TARGET INCREMENTAL=${INCREMENTAL:-reports/stryker-incremental.json}
RAW=$HERE/results/raw
STRYKER=(npx stryker run "$CONFIG" --mutate "$MUTATE")
mkdir -p "$RAW" "$HERE/backup"

# Back up the state files once; run.py restores the incremental file before every run.
[ -f "$HERE/backup/incremental.json" ] || cp -p "$TARGET/$INCREMENTAL" "$HERE/backup/incremental.json"
[ -f "$HERE/backup/mutation.json" ] || cp -p "$TARGET/reports/mutation/mutation.json" "$HERE/backup/mutation.json"
restore() {
  cp -p "$HERE/backup/incremental.json" "$TARGET/$INCREMENTAL"
  cp -p "$HERE/backup/mutation.json" "$TARGET/reports/mutation/mutation.json"
}
trap restore EXIT

budget=1200
wait_quiet() {
  [ "${QUIET_WAIT:-0}" = 1 ] || return 0
  while (( budget > 0 )) && awk '{exit !($1 > 4)}' /proc/loadavg; do
    sleep 30; budget=$((budget - 30))
  done
  echo "wait_quiet: load $(cut -d' ' -f1 /proc/loadavg), budget left ${budget}s"
  # Budget spent and still loaded: stop rather than add more noisy runs.
  awk '{exit !($1 > 4)}' /proc/loadavg && { echo "wait_quiet: never dropped below 4; stopping"; exit 0; }
  return 0
}

run() {  # run VARIANT REP
  local v=$1 out=$RAW/$1-$2
  wait_quiet
  case $v in
    A)  python3 "$HERE/run.py" "$out" -- "$BIN" run --diff "$BASE" --config "$CONFIG" --json ;;
    B)  python3 "$HERE/run.py" "$out" -- "${STRYKER[@]}" --logLevel debug ;;
    B0) python3 "$HERE/run.py" "$out" -- "${STRYKER[@]}" ;;
    C)  python3 "$HERE/run.py" "$out" -- "${STRYKER[@]}" --logLevel debug --checkers "" ;;
    D)  python3 "$HERE/run.py" "$out" -- "${STRYKER[@]}" --logLevel debug --concurrency 8 ;;
    E)  python3 "$HERE/run.py" "$out" -- "${STRYKER[@]}" --ignorePatterns "${IGNORE:?set IGNORE for variant E}" ;;
  esac
}

for rep in $(seq "$FIRST" "$LAST"); do
  for v in ${VARIANTS:-A B B0 C D E}; do run "$v" "$rep"; done
done
python3 "$HERE/phases.py" "$RAW" | tee "$HERE/results/phases.txt"
git -C "$TARGET" status --short
