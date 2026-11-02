# bench: where the wall time of a diff-scoped Stryker run goes

Measurement only. The target repo is never edited: `run.py` restores the incremental state file
(`$INCREMENTAL`) from `backup/` before every run, so each run starts from the same incremental
state, and `bench.sh` restores it and `reports/mutation/mutation.json` on exit.
Never add `--inPlace` (it writes into the target's sources).

## Files

- `run.py OUT -- CMD...`: one run. Restores state, spawns CMD in `$TARGET` (required), and stamps
  every stdout/stderr line with epoch ms as it is read. The stamping is done in this one long-lived process (two reader threads); nothing forks
  per line. Writes `OUT.log` (`<ms> <out|err|meta> <line>`), `OUT.stdout`, and `OUT.json` (start/end
  ms, exit code, `wait4` rusage of the whole waited-for tree, loadavg, and the top-3 CPU users as
  `pcpu comm` only). `--sample-sandbox` also
  counts the files and bytes in the new `.stryker-tmp/sandbox-*` when "Starting initial test run"
  appears. It runs during the dry run, so use it only in a run you do not time.
- `phases.py RAW_DIR`: phase table (median, min-max) per variant from `RAW_DIR/<variant>-<rep>.log`.
- `bench.sh [FIRST] [LAST]`: takes the target from env (see Reproduce) and runs reps FIRST..LAST (default 1..5) of variants A B B0 C D E,
  round-robin, one run at a time, then `phases.py`. `VARIANTS="E"` runs a subset. `QUIET_WAIT=1`
  waits before each run until the 1-min loadavg is <= 4, polling every 30 s with a 20 min total
  budget, and stops if load never drops. `phases.py` leaves runs that started above load 4 out of
  the medians, unless no quiet runs exist; the table header then says "MEASURED UNDER LOAD".
- `results/`: reports. `results/raw/` holds the logs and is git-ignored (about 166 MB).

## Reproduce

```sh
REAPR_BIN=/path/to/rpr TARGET=<repo> CONFIG=<config> BASE=<base> MUTATE="<ranges>" \
  IGNORE="<the config's ignorePatterns>,.turbo" [INCREMENTAL=<state file>] \
  bench/bench.sh 1 5    # 30 runs, about 30 min under load
python3 bench/phases.py bench/results/raw
```

Variants (all run from the target root, with `--mutate` equal to the list Reapr derives from the
diff (`MUTATE`). That list was checked against Reapr's generated `stryker.wrapper.mjs`):

| id | command |
|----|---------|
| A  | `rpr run --diff <base> --config <config> --json` |
| B  | `npx stryker run <config> --mutate "<ranges>" --logLevel debug` |
| B0 | B without `--logLevel debug`. This is the like-for-like control for A, which runs at the config's `info` |
| C  | B + `--checkers ""` (the CLI splits on `,` and drops empty strings, so the result is `[]`) |
| D  | B + `--concurrency 8` |
| E  | B0 + `--ignorePatterns "<the config's list>,.turbo"`. The CLI value replaces the config's list, so the whole list is repeated |

The phase markers are Stryker 9.6.1 log messages. Their sources, under
`node_modules/@stryker-mutator/core/dist/src/`, are listed in `results/2026-11-02-turborepo-monorepo.md`.

Sandbox lower bound (measured once; the timed commands ran 3× each with `date +%s%N` around them):

```sh
# Stryker's file list, from the "ProjectReader All input files: [" dump in a --logLevel debug log
sed 's/\x1b\[[0-9;]*m//g' results/raw/explore/B-debug-sample.log \
  | awk '/All input files: \[/{f=1;next} f&&/^[0-9]+ out \]/{exit} f{sub(/^[0-9]+ out +"/,"");sub(/",?$/,"");sub("^'"$TARGET"'/","");print}' \
  > results/raw/sandbox-files.txt
cd "$TARGET"; D=$OLDPWD/scratch-copy; L=$OLDPWD/results/raw/sandbox-files.txt
rsync -a --files-from=$L . $D/                     # and: tar -cf - -T $L | tar -C $D -xf -
xargs -d '\n' cp --parents -p -t $D < $L           # -l instead of -p for hardlinks
rm -rf $D                                          # timed too; delete scratch-copy/ afterwards
npx --no-install tsc -p <checker tsconfig> --listFilesOnly   # the checker's TS project
```
