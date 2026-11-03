# bench/oss: rpr against full StrykerJS runs on public repos

The claim this benchmark exists to back, or refute:

> `rpr` is N× faster than running StrykerJS on every pull request, with the same verdict on the changed lines.

Every number comes from public repos, public merged pull requests, unmodified configs, and a script anyone can
rerun. This directory holds the design (this file), the pilot harness (`pilot.py`) and the pilot results
(`results/`). The full suite on GitHub Actions is the next step and is not built yet.

## Repos

Found with `gh search code --filename stryker.{config,conf}.{mjs,js,json}` (313 repos), then filtered to
active, non-fork repos whose config runs Vitest or Jest. Numbers as of 2026-11-02. "Files under `mutate`" counts
the files in the default branch's tree that the config's `mutate` patterns select (approximate: globs evaluated
by hand). "PR fit" is how many of the last 40 merged PRs touch those files.

| Repo | Stars | Last commit | Runner | Stryker | Config | Files under `mutate` | Full run < 60 min on 4 vCPU? | License | PR fit |
|---|---|---|---|---|---|---|---|---|---|
| [brazilian-utils/javascript](https://github.com/brazilian-utils/javascript) | 1.7k | 2026-10-09 | Vitest (vite-plus), `perTest` | 10.0.0 | `stryker.config.json` | 263 | Yes. Upstream runs it in CI on ubuntu-latest: full `--force` on main 24 min. Its PR job uses `--incremental`: 2 to 4 min when the cached state hits (about 6090 of 6199 results reused), about 27 min on a cache miss (a lockfile change, or main's cache not saved yet) | MIT | 28/40 (many large) |
| [javierbrea/eslint-plugin-boundaries](https://github.com/javierbrea/eslint-plugin-boundaries) | 1.0k | 2026-11-02 | Jest 30, `perTest` | 9.6.1 | `packages/{config,elements,policies}/stryker.config.mjs` | 134 | Yes. Upstream runs all three on every PR: elements 21 min, config 4.5 min, policies 2 min | MIT | 15/40 in 10 days |
| [voidcosmos/npkill](https://github.com/voidcosmos/npkill) | 9.5k | 2026-11-02 | Jest 30 via `command` runner, `coverageAnalysis: off` | 9.6.1 | `stryker.conf.js` | 88 | Unknown: every mutant runs the whole Jest suite; no upstream CI job. Measure first | MIT | 29/40 (since 2023) |
| [onury/accesscontrol](https://github.com/onury/accesscontrol) | 2.3k | 2026-10-03 | Vitest 4, `perTest` | ^9.6.1 | `stryker.config.json` | 27 | Likely (small) | MIT | Pushes to the default branch; almost no PRs. Commits would replace PRs |
| [bennycode/trading-signals](https://github.com/bennycode/trading-signals) | 1.0k | 2026-11-01 | Vitest 5 | ^10.0.0 | `packages/exchange/stryker.config.json` | 55 | Likely | MIT (package.json) | 4/40. Config sets `incremental` and `inPlace`, so B is the repo's own setup |
| [evloghq/evlog](https://github.com/evloghq/evlog) | 1.9k | 2026-11-02 | Vitest 4 | ^9.6.1 | `packages/evlog/stryker.config.json` | 10 (large core modules) | Yes. Upstream weekly cron: 21 to 35 min | MIT | 1/40 |
| [inversify/monorepo](https://github.com/inversify/monorepo) | 0.3k | 2026-11-02 | Vitest 5 + TypeScript checker | 10.1.1 | 37 per-package configs (shared preset, `concurrency: 1`) | ~2.5k in all packages | Per package yes; the whole repo no | MIT | 17/40 |
| [tiagoporto/gerador-validador-cpf](https://github.com/tiagoporto/gerador-validador-cpf) | 0.4k | 2026-10-03 | Jest 30 + TypeScript checker | ^9.4.0 | `stryker.config.mjs` | 16 | Yes. Upstream CI job 1 to 2 min | MIT | 4/40 |

Excluded: recharts and handsontable (the config mutates one file), mastra (`mutate: []`), marionette (the config
throws unless run through the repo's own wrapper script), typeorm, docxtemplater, typed-inject and jsonschema
(Mocha), knip (Stryker configs are test fixtures).

## Scenarios

All scenarios run on the same commit (the PR head), with the same installed `node_modules`, the same config file,
unmodified. No scenario gets a flag the others do not, except the one flag that defines it.

| id | What | Command |
|---|---|---|
| A | Full Stryker, as the repo configures it | `npx --no-install stryker run` |
| B | Stryker incremental, only when the repo allows it from the CLI: the state file comes from a full run on the PR's base (a fresh runner has no state file, so the seed tests every mutant), then the head runs with it. This is incremental's best case: a cache that always hits and is never stale | `npx --no-install stryker run --incremental` (on base, then on head) |
| C | rpr on the diff | `rpr run --diff <merge^1> --json` |
| D (later) | rpr after applying `rpr doctor` fixes | depends on doctor |
| E (later) | rpr sharded | depends on sharding |

A is the baseline: it is what "mutation testing on every PR" costs today. B is the strongest no-new-tool
alternative and must be shown, even when it wins on some PRs. When a config already sets `incremental: true`
(trading-signals), A is that config as is, and B is the same run seeded from the base.

## PR selection

The integration branch is the default branch, unless the repo merges features into another branch first and
runs its mutation workflow there (eslint-plugin-boundaries squash-merges into `release`, and its workflow runs
on every push to `release`). Release PRs that only move the integration branch into the default branch are
left out, because their diff is the sum of PRs already counted.

Per repo, the last N (N = 10 for the suite, 3 for the pilot) merged PRs into the integration branch that:

1. touch at least one file selected by the config's `mutate` patterns at the PR head, and
2. have a Stryker config and `@stryker-mutator/core` at the PR head (brazilian-utils #498 has neither: Stryker
   came later), install, and leave rpr at least one changed file that the config's `mutate` selects.

Survivors: parity only means something if some in-diff mutants survive. The suite's scan job runs C once per
candidate PR, newest first, and stops when it has N valid PRs and at least 3 of them (or of older ones) have
in-diff survivors. If fewer than 3 of the last N have survivors, the newest older PRs with survivors are added
(marked "older PR added for in-diff survivors"). The scan uses C only to choose PRs; its runs are not results,
and A's verdicts are what parity is checked against.

Base and head come from the merge commit: base is `merge^1` (the integration branch just before the merge), head is
`merge^2` (the PR's last commit), or the merge commit itself for a squash merge. `rpr run --diff <base>` from
`head` then scores `merge-base(base, head)...head`, which is the PR's diff. PRs are not hand-picked: the selection
is a `gh pr list --state merged` query, recorded with the results. Large PRs stay in; they are where C gains least.

## Measurements

Per run:

- wall time of the command (spawn to exit), and CPU user + sys of the whole process tree (`wait4` rusage)
- peak RSS of the largest process
- `/proc/loadavg` just before and after
- exit code and whether the run hit the timeout
- mutants and score: for A over the whole report; for C from rpr's JSON (in-scope only)

Per PR:

- speedup = median wall of A / median wall of C, and the same for CPU
- parity: A's report and C's raw Stryker report (`<run_dir>/mutation.json`) are both filtered with the diff ranges
  from C's JSON, using Stryker's containment rule (re-implemented in `pilot.py`, the same rule as
  `crates/reapr-stryker/src/schema.rs` `in_scope`). Mutants are matched on file, mutator, start and end position
  and replacement. Reported as matched / total, with every mismatch listed. The score of A restricted to the diff
  is reported next to C's.

Engine time, not job time: dependency install and checkout are the same for A and C and are excluded from wall
time. They are recorded once per PR so a "job minutes" figure can be given too.

## Environment

- Suite: GitHub-hosted `ubuntu-latest` (4 vCPU, 16 GB), one job per (repo, PR, scenario, rep), so runs never
  share a machine. Node pinned per repo to the version in the repo's own CI (`.nvmrc` or workflow), stated in
  the results.
- npm cache: warm (`actions/setup-node` cache) for all scenarios, stated. A cold-cache run per repo is reported
  separately, since install time is outside the measured window anyway.
- Pilot: a local machine (below). `--cpus 4` pins every run to 4 CPUs with `taskset`. Node's
  `os.availableParallelism()` follows the affinity mask, so Stryker picks the same default concurrency (4) as on
  ubuntu-latest.

## Fairness rules

1. Same commit, same lockfile, same `node_modules`, same config file for A, B and C. No config edits, no extra
   flags beyond the one that defines the scenario. The only exception: if a config has no `json` reporter, A
   gets `--reporters <its reporters>,json` so parity can be computed (rpr adds the same reporter).
2. Report every run: failures, timeouts (60 min limit) and engine threshold failures (Stryker exit 1 on
   `thresholds.break`) stay in the table with their exit code. A threshold failure is a completed run.
3. Medians of at least 3 runs per scenario and PR in the suite (pilot: A once, C 3 times).
4. Runs alternate between scenarios, so drift on the runner image affects all of them.
5. Telemetry off (`DO_NOT_TRACK=1`) for every rpr run.
6. The rpr version, Stryker version, Node version and runner image are recorded in every result file.
7. No cherry-picking: the PR list is a query, and every PR it returns is reported, including those where
   C is not faster.

## Publishing

- `results/<repo>/summary.json`: every run as recorded (command, times, rusage, loadavg, counts, parity with
  mismatches).
- `results/<repo>/summary.md`: one row per PR, produced from the JSON.
- A top-level table per suite run: repo, PRs, median speedup (wall and CPU), parity (mutants matched / total),
  runner minutes for A vs C. The headline N is the median speedup over all PRs of all repos, next to its range.

## Next step: the suite on GitHub Actions

Repos, in order of value:

1. **brazilian-utils/javascript**: Vitest, Stryker 10, 4855 mutants. Upstream pays 24 min for a full run, and 2 to 4
   min for an incremental PR run when its cache hits (about 27 min on a miss). A and B both come from the repo's
   own workflow, and B is the fair best case for incremental: its state comes from a full run on the PR's base.
2. **eslint-plugin-boundaries**, packages `elements` (21 min upstream) and `config` (4.5 min): Jest, pnpm + nx
   monorepo, `--project`. The pilot repo, rerun on clean runners.
3. **voidcosmos/npkill**: dropped. Timed once on ubuntu-latest (run 41544110724, PR #277 head): the full run
   (Jest through the `command` runner, `coverageAnalysis: off`) hit the 60 min limit, and rpr alone needed about
   13 min for the PR's 196 in-diff mutants. Over the limit for A, so not in the suite.
4. **inversify/monorepo**, its most-touched package: Vitest 5 + TypeScript checker, Stryker 10.1.1, shared
   preset with `concurrency: 1`. Also measure A once first.
5. Optional, for B: **bennycode/trading-signals** (`incremental: true` in its config).

Runner-minute estimate for 10 PRs per repo, with A, B and C 3 times each (B seeded once per PR from a full run on
the base) and about 1.5 min of checkout and install per job. Upstream timings where known; the npkill and inversify
figures are guesses, to be replaced by the one-off measurement:

| Repo | A per run | Per PR (A×3 + seed + B×3 + C×3 + setup) | 10 PRs |
|---|---|---|---|
| brazilian-utils | 24 min | ~150 min | ~1,500 min |
| boundaries elements (6 PRs) + config (4 PRs) | 21 / 4 min | ~130 / ~40 min | ~950 min |
| npkill | ≤ 30 min (guess) | ≤ 180 min | ≤ 1,800 min |
| inversify, one package | ~10 min (guess) | ~65 min | ~650 min |
| **Total** | | | **~4,900 min (~82 h)** |

Standard GitHub-hosted runners are free for public repos, and public repos get the 4-vCPU `ubuntu-latest`, so
the suite should run from a public repo. The jobs are independent, so the wall time is about the total
divided by the number of concurrent jobs the account allows. B's head runs are assumed at 40% of A; without B the total drops by about a third.

## Running the suite

`.github/workflows/oss-benchmark.yml` (workflow_dispatch only) runs `suite.py` on GitHub-hosted `ubuntu-latest`:

```sh
gh workflow run oss-benchmark.yml --ref feat/oss-benchmark \
  -f repos=all -f prs=10 -f reps=3 -f scenarios=A,B,C -f min_survivor_prs=3 -f rpr_version=0.3.0
```

Jobs: `scan` (one per repo, picks the PRs), `plan` (builds the matrices), then one job per (repo, PR, scenario,
rep): `A`, `C`, `seed` (A on the PR base with the B flags, kept for its incremental file) and `B` (A on the head
with the seed's incremental file). `aggregate` writes the markdown table to the job summary and uploads
`summary.json`, `summary.md` and the run's job list (for runner minutes). Each job installs cold (no cache
restored), times install apart from the measured command, and uploads one small result JSON. CI commits nothing.
Node is pinned per repo in `repos.json` (the version each repo's own CI uses). `python3 bench/oss/suite.py
selftest` checks the diff parser, the plan and the aggregate on synthetic data.

## Pilot harness

```sh
python3 bench/oss/pilot.py --repo <clone> --gh <owner/name> --pr <n> [--pr <n> ...] \
  --rpr <path to rpr> --cpus 4 --install "npm ci" --out <results dir> [--project <package dir>] \
  [--a-reps 1] [--c-reps 3] [--a-report reports/mutation/mutation.json] [--a-args "..."] [--timeout-min 60]
```

It checks out each PR head, installs, runs A `--a-reps` times and C `--c-reps` times, and writes
`<out>/<pr>/{A,C}-<rep>.{json,stdout,stderr}`, A's raw report, `<out>/summary.json` and `<out>/summary.md`. It
never edits the repo; before A it deletes only Stryker's temp dir and A's previous report.
`python3 bench/oss/pilot.py --self-test` checks the containment rule and the score formula.

## Pilot: eslint-plugin-boundaries, `packages/config` (2026-11-02)

Why this one: upstream runs a full Stryker job for this package on every push to `release` and on every PR to
`master` (4.5 min on ubuntu-latest, including setup), it uses Jest (not Vitest, so it is not the case rpr was
built on), and the three PRs are recent and picked by the selection rule. brazilian-utils was tried first and
stopped: its full run has 4855 mutants, and Stryker projected more than 1.5 h on 4 pinned CPUs under load 18
(upstream does it in 24 min on ubuntu-latest). It goes to the GitHub Actions suite instead.

Setup: Node 24.15.0, rpr 0.2.0, Stryker 9.6.1 (jest-runner), pnpm 10.34.8 through corepack, install
`pnpm install --frozen-lockfile && pnpm nx build elements` (the package imports `@boundaries/elements`).
Local 22-thread Linux machine, **shared and loaded**: every run pinned to 4 CPUs with `--cpus 4`, and the 1-min
load average before runs was 13.6 to 46.4. A ran once per PR, C three times. The config has no `json`
reporter, so A got `--reporters html,clear-text,progress,dashboard,json` (its own list plus `json`).

```sh
python3 bench/oss/pilot.py --repo <clone> --gh javierbrea/eslint-plugin-boundaries --project packages/config \
  --pr 591 --pr 578 --pr 577 --rpr <rpr> --cpus 4 \
  --install "corepack pnpm install --frozen-lockfile && corepack pnpm nx build elements" \
  --a-args "--reporters html,clear-text,progress,dashboard,json" --out <results>
```

| PR | Diff (package src) | A wall | A CPU | A mutants | A score | C wall, median of 3 (range) | C CPU | C mutants | C score | Speedup wall / CPU | Parity in diff | Load before A / C |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| [#591](https://github.com/javierbrea/eslint-plugin-boundaries/pull/591) | types only (`Settings.types.ts`) + spec | 1031 s | 2091 s | 1698 | 99.23 | 30.0 s (29.0-36.1) | 26 s | 0 | n/a | 34.3× / 80.2× | 0 / 0 | 23.5 / 38.4-46.4 |
| [#578](https://github.com/javierbrea/eslint-plugin-boundaries/pull/578) | `Config.ts`, `Settings.types.ts` + spec; 13 ranges, 10 in source | 757 s | 1959 s | 1698 | 99.23 | 19.5 s (17.0-23.0) | 48 s | 42 | 100 | 38.8× / 40.7× | 42 / 42 same | 32.0 / 17.6-22.6 |
| [#577](https://github.com/javierbrea/eslint-plugin-boundaries/pull/577) | `Settings.ts`, `Validator.ts`, `RuleOptions.ts`, `Config.ts` + specs; adds `stryker.config.mjs`; 112 ranges in all files | 642 s | 2400 s | 1693 | 99.23 | 32.5 s (31.0-39.0) | 81 s | 43 | 100 | 19.7× / 29.6× | 43 / 43 same | 13.6 / 25.6-43.3 |

Median wall speedup over the three PRs: 34×. Every run exited 0. The three C runs of each PR returned identical
counts. Raw per-run records: `results/2026-11-02-eslint-plugin-boundaries-config/` (local paths replaced with
`<repo>` and `<scratch>`; A's 9 MB reports are not committed).

What the pilot does and does not show:

- **Load inflates A more than C.** Upstream's whole job for this package takes 4.5 min on ubuntu-latest; A took
  10.7 to 17.2 min here. These numbers prove the harness and the parity check, not the headline N. The
  headline has to come from GitHub-hosted runners.
- **Parity is weak evidence here.** Every in-diff mutant was killed in both A and C, so the check never saw a
  survivor. The suite needs PRs whose diff has survivors (brazilian-utils and the elements package have some).
- **Type-only diffs still start Stryker.** #591's package diff touches only a types file and specs; the
  `mutate` patterns select the types file, Stryker produces 0 mutants, and rpr still spends ~30 s on startup and
  the dry run. Skipping ranges with no mutable code would make this case near zero.
- The A score is package-wide (1693-1698 mutants); C scores only the diff. Both are reported, never compared
  as one number.
