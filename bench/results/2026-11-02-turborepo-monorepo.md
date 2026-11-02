# Where the wall time of a diff-scoped Stryker run goes (reference monorepo, one package, 2026-11-02)

Case: `rpr run --diff <base> --config <config>` in the reference monorepo, a pnpm + Turborepo
monorepo (~55k files including build caches; StrykerJS 9.6.1, Vitest 4.1.5, typescript-checker
9.6.1, Node 24.15.0). `<config>` is the Stryker config of one package. The diff gives 3 mutants in
that package. With `incremental` reused, the report holds
293. All 30 timed runs produced the same tallies: 86.94%, 252 killed, 1 timeout, 38 survived. Reapr
scored 100% (3/3 killed) on every run. How to reproduce: `../README.md`. Raw logs:
`raw/` (git-ignored).

**Every number below was measured under load.** The 1-min loadavg was 14.8-41.7 before every
run, on 22 hardware threads, with other agents building and running node and cargo jobs. Each
run's load and top-3 CPU users are in `phases.txt`. The coordinator's rule was to keep runs above
load 4 out of the medians and to rerun them once load fell below 4. A quiet-gated pass waited the
full 20 min budget, polling every 30 s, and load never fell below 4 (it ended at 31.9), so no quiet
runs exist. **Noisy vs quiet:** no quiet baseline exists. Within the noisy runs, the 13 that
started at load ≥ 26 took 1.31× their variant's median wall time, and the 10 below load 20 took
0.91× (Spearman ρ = 0.71, n = 30). Treat the absolute seconds as inflated, and the wide ranges as
mostly load.

Machine: Intel Core Ultra 9 185H (16 cores, 22 threads, up to 5.1 GHz), 93 GiB RAM (67 GiB in use,
8 GiB swap full), NVMe SSD (GIGABYTE AG4732TB, `rota=0`), ext4 on LVM.

Method: n = 5 per variant. The runs were sequential and round-robin (A B B0 C D ×5, then E ×5), and
the package's incremental state file was restored before every run. Each cell is the median (min-max) in seconds. Each
stdout/stderr line was timestamped at receipt by one long-lived Python process.

## Phase markers (Stryker 9.6.1 source, `@stryker-mutator/core/dist/src/`)

| marker | log line | source |
|---|---|---|
| found | `INFO ProjectReader Found N of M file(s)` | `fs/project.js:41` |
| pool | `INFO ConcurrencyTokenProvider Creating …process(es)` (before `checkerPool.init()`) | `concurrent/concurrency-token-provider.js:23/29`, `process/2-mutant-instrumenter-executor.js` |
| sandbox | `DEBUG Sandbox Creating a sandbox for files in …` (after the checker pool is up) | `sandbox/sandbox.js:40` |
| symlink | `DEBUG Sandbox Start symlink node_modules` (after `fillSandbox()`) | `sandbox/sandbox.js:75` |
| runner | `DEBUG ChildProcessProxy Started ChildProcessTestRunnerWorker …` | `child-proxy/child-process-proxy.js:36` |
| dry_start / dry_ok | `INFO DryRunExecutor Starting initial test run` / `Initial test run succeeded` | `process/3-dry-run-executor.js:89/131` |
| done | `INFO MutationTestExecutor Done in` (after `reporter.wrapUp()`) | `process/4-mutation-test-executor.js:82` |
| rmtmp | `DEBUG TemporaryDirectory Deleting stryker temp directory` | `utils/temporary-directory.js:35` |

At `info` level, "Found" is the first Stryker line, so P0 and P1 can only be separated in the debug
variants (B, C, D). The `pool` marker had a single receipt lag over 1.1 s, against Stryker's own
1-second clock (1.16 s in D). All other markers arrived within 1 s of Stryker's stamp.

## Variant tables

| phase | A Reapr | B0 plain, info | B plain, debug | C B, no checker | D B, conc. 8 | E B0 + ignore `.turbo` |
|---|---|---|---|---|---|---|
| P0 start→first line | 18.0 (16.0-37.1)¹ | 13.6 (12.2-24.4)¹ | 1.6 (1.3-5.0) | 1.3 (1.0-1.5) | 1.6 (1.2-3.6) | 2.6 (2.3-4.4)¹ |
| ↳ P0a start→npx up | 2.1 (1.9-4.5) | 0.4 (0.3-0.7) | 0.5 (0.4-1.4) | 0.4 (0.3-0.5) | 0.4 (0.4-1.0) | 0.4 (0.4-0.7) |
| P1 →Found (walk M files) | in P0 | in P0 | 17.7 (11.2-27.0) | 11.6 (10.0-13.9) | 14.3 (10.2-22.9) | in P0 |
| P2 →Starting initial test run | 28.6 (21.3-39.6) | 19.5 (14.9-31.9) | 36.2 (17.8-46.6) | 20.8 (15.3-33.2) | 27.4 (23.8-34.1) | 5.7 (3.9-6.6) |
| ↳ P2a instrument + preprocess | 4.1 (3.8-10.1) | 3.2 (2.8-9.1) | 5.0 (3.4-9.6) | 4.0 (3.2-6.0) | 4.7 (2.8-6.2) | 0.4 (0.3-0.6) |
| ↳ P2b checker init | | | 2.9 (2.5-5.5) | 1.1 (1.0-2.2) | 8.9 (6.2-9.2) | |
| ↳ P2c sandbox copy | | | 21.0 (10.4-28.1) | 14.5 (9.4-25.7) | 13.4 (12.1-16.6) | |
| ↳ P2d symlink + reporters | | | 0.4 (0.2-0.7) | 0.3 (0.3-0.5) | 0.3 (0.3-0.5) | |
| ↳ P2e runner startup | | | 1.9 (1.3-2.8) | 1.5 (1.3-1.6) | 1.6 (1.3-1.8) | |
| ↳ P2b-e at info level | 24.8 (17.5-35.2) | 16.5 (12.1-22.8) | 28.5 | 17.6 | 23.5 | 5.1 (3.5-6.1) |
| P3 dry run | 1.4 (1.2-2.7) | 1.3 (0.7-1.3) | 1.8 (1.0-2.4) | 1.2 (1.1-1.6) | 1.4 (1.3-1.5) | 1.2 (1.1-1.5) |
| P4 mutation testing | 2.7 (2.5-6.9) | 2.7 (1.8-3.6) | 3.1 (2.2-4.9) | 1.7 (1.5-2.1) | 2.0 (1.5-2.5) | 2.5 (2.0-2.7) |
| P5 →exit | 1.7 (1.6-4.4) | 1.5 (1.1-1.6) | 1.5 (1.2-2.6) | 1.6 (1.3-1.8) | 2.3 (1.6-2.3) | 0.2 (0.2-0.2) |
| ↳ P5b temp-dir delete + exit | | | 1.3 (1.2-2.4) | 1.5 (1.2-1.7) | 1.4 (1.1-1.9) | |
| **TOTAL wall** | **54.0 (50.2-81.4)** | **38.4 (30.7-58.7)** | **59.4 (35.0-86.9)** | **37.2 (33.4-52.4)** | **49.1 (40.6-65.8)** | **12.5 (9.4-15.3)** |
| CPU user+sys (rusage, whole process tree) | 78.3 (71.5-104.6) | 62.8 (46.2-79.9) | 76.1 (52.8-107.9) | 56.5 (51.9-72.6) | 98.9 (81.8-117.4) | 19.8 (16.2-23.3) |
| peak RSS of the largest process, MB | 607 | 607 | 633 | 635 | 616 | 399 |

¹ At info level, P0 includes P1 (config load plus the walk of the file tree).

Reapr's own JSON (A-1…5): `duration_ms` was 77697, 51552, 52374, 47863, 59781.
`cpu_user_ms` was 80035, 53371, 55016, 53355, 59626. `cpu_sys_ms` was 21298, 17192, 21483, 15868,
21895. `max_rss_kb` was 622560, 621200, 614720, 621856, 646624.

## Where A's time goes (median 54.0 s)

```
P0+P1  18.0 s  33%  ███████████████  Reapr git diff + plan.mjs (1.7) + npx/node + config + walking 55,389 files
P2a     4.1 s   8%  ███              instrument + preprocess (preprocessors iterate every file)
P2b-e  24.8 s  46%  ███████████████████████  B's debug split: checker ~2.9, sandbox copy ~14-21, symlink 0.4, runner 1.9
P3      1.4 s   3%  █                dry run (83 tests)
P4      2.7 s   5%  ██               the 3 mutants
P5      1.7 s   3%  █                reporters + deleting a 1.29 GB temp dir
```

About 85% of A happens before the first test runs. About 8% is spent testing (P3 + P4).

## Comparisons

- **Reapr overhead, A vs B0:** the medians are 54.0 vs 38.4 s (+15.6 s). The only Reapr code that
  runs is before and after the Stryker child. That time is A's wall minus Reapr's own Stryker-span
  `duration_ms`: **1.7 s (1.6-3.7)**. Within it, the time from start to npx up is 2.1 s for A
  against 0.4 s for B0 (`git diff` plus the `plan.mjs` node process). The other ~14 s falls inside
  Stryker phases, mostly P0/P1 and the copy in P2. Reapr does nothing there except wait. Its config
  differs only by the `json` reporter, which can only affect P4 and P5 (equal within 0.2 s). So the
  ~14 s is noise or an ordering effect: A always ran right after D, which had just deleted a
  1.29 GB sandbox. That cause is a hypothesis, not tested. B (debug) is not a valid control for A:
  debug logging alone accounts for +21 s of median wall (B 59.4 vs B0 38.4). In an exploratory run
  it held Stryker's event loop for ~4 s while dumping the 55k-file list. The harness drains 55k
  lines in 0.25 s (measured), so that stall is Stryker's own.
- **Checker cost, B vs C:** by phase, 2.9 → 1.1 s in P2b (−1.8) and 3.1 → 1.7 s in P4 (−1.4,
  because with a checker only 1 of the 2 workers runs tests until checking finishes). That is about
  **3.2 s**. The total-wall gap (59.4 vs 37.2) is dominated by P1 and P2c, which the checker does
  not touch, so it is load noise. The checker's TS project is the repo's Stryker tsconfig, which has no
  `include`. `tsc --listFilesOnly` shows it covers only three shared Vitest config files plus lib
  `.d.ts` files. The mutated package is outside it, so `check()` passes these mutants
  through (`typescript-checker.js:65`). Here the checker costs about 3 s and checks nothing.
- **Concurrency, B vs D (`--concurrency 8`, giving 4 checkers + 4 runners):** P2b grows
  2.9 → 8.9 s (+6.0, four TS programs built in parallel), P4 shrinks 3.1 → 2.0 s (−1.1), and P5
  grows by 0.8. The phase sum is **+5.7 s slower**. Total wall (49.1 vs 59.4) is again dominated by
  copy and walk noise. With 3 mutants, a bigger pool does not help.
- **Sandbox (measured once, exploratory run B-debug-sample):** Stryker copies **55,390 files,
  1.288 GB** (plus 29 symlinks). `.turbo/cache` (Turborepo's local cache, absent from the config's
  `ignorePatterns`) makes up **53,832 files (97%) and 1,277.6 MB (99.2%)**. Stryker's copy (P2c,
  pooled B/C/D) takes **14.3 s (9.4-28.1, n=15)**. Copying the same 55,389-file list to the same
  ext4 volume, 3 reps each, warm cache, load 15-31:

  | method | copy | `rm -rf` |
  |---|---|---|
  | `rsync -a --files-from` | 17.2 (16.4-18.4) | 1.6-2.6 |
  | `tar \| tar` | 5.5 (5.2-5.8) | 1.8-2.1 |
  | `xargs cp --parents -p` | 5.3 (4.7-8.5) | 1.6-2.7 |
  | `xargs cp --parents -l` (hardlinks) | 1.7 (1.6-2.5) | 0.8-1.1 |
  | rsync, list without `.turbo/` (1,557 files) | **0.31 (0.27-0.34)** | 0.04-0.05 |

  Stryker's own temp-dir delete (P5b) takes 1.4 s (1.1-2.4).
- **E (counterfactual, same flags as B0 plus `.turbo` in `--ignorePatterns`):** M falls from 55,389
  to 1,523 files and the tallies are identical. Wall goes from **38.4 to 12.5 s (−25.9 s, −67%)**.
  The saving spreads over P0+P1 (−11.0), P2 (−13.8), and P5 (−1.3). CPU drops 62.8 → 19.8 s and
  peak RSS 607 → 399 MB. E ran at load 18-21; B0 ran at 19-41. The B0 runs nearest E's load (B0-4
  at load 19.4 took 30.7 s; B0-5 at 23.6 took 38.4 s) still leave a gap of ≥ 18 s.

## Measured vs inferred

Measured: every table cell, the per-run load and top-3, the sandbox file and byte counts, the
`.turbo` share, the copy and delete timings, the TS project file list, Reapr's time outside the
Stryker child, the E counterfactual, and identical tallies across all 30 runs.

Inferred, not measured:
- The leftover ~14 s in A − B0 is noise or ordering.
- Absolute numbers on a quiet machine would be lower. The load/wall correlation suggests this; it
  was never observed, because no quiet window came.
- What warm workers or a Rust sandbox would save. These are upper bounds below, never built.
- What P2a's time is spent on: the preprocessors iterating every file. This comes from reading
  `2-mutant-instrumenter-executor.js`, and is consistent with P2a falling 3.2 → 0.4 s in E.

## Biggest levers, by seconds

1. **Stop walking and copying files the tests never read (`.turbo/cache` here): −25.9 s measured**
   (B0 → E, 38.4 → 12.5 s, identical results). This one change removes the P1 walk, most of
   preprocessing, the copy, and the delete. Turbo caches will exist in most Turborepo targets.
2. **Warm, pre-started workers: at most ~6-8 s, inferred.** This is the fixed cost left in E's
   12.5 s that a warm process could skip: node + npx + config + plugin load (E's P0, 2.6 s),
   checker init (2.9 s in B, of which ~1.8 s is the checker itself), runner startup (1.5-1.9 s),
   and possibly the 1.2 s dry run if coverage were cached. Before lever 1, this is under 15% of A.
3. **A faster sandbox in Rust: ≤ 0.3 s once lever 1 is applied, measured bound.** The remaining
   1,557 files copy in 0.31 s with rsync. Without lever 1, the bound is ~9-13 s (Stryker's 14.3 s
   copy vs `cp` at 5.3 s, or hardlinks at 1.7 s). Even then it does not touch the 11-18 s P1 walk.

Sharding is not a lever for diffs of this size. All mutant testing (P4) is 2.7 s, and widening the
pool cost +5.7 s (D). Disabling the checker is worth ~3.2 s here, because its tsconfig covers none
of the mutated files.
