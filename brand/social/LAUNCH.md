# Launch copy (draft)

Every number here is measured:

| Claim | Source |
| --- | --- |
| 100% line and branch coverage, 83.33% mutation score, 1 survivor (`>=` became `>`) | The two-test fixture behind `card-terminal.png`: `node --test --experimental-test-coverage` and `rpr run --diff main` (rpr 0.2.0) |
| 85% of a small diff run happens before the first test; 97% of the sandbox files were `.turbo/cache` | `bench/results/2026-11-02-turborepo-monorepo.md` |
| 38.4 s → 12.5 s, same results, after adding `.turbo` to `ignorePatterns` | Same file, run E against B0 |
| Diff score 100% where Stryker reported 87.0% package-wide | README, Measurements |
| 365 mutants, 287 killed, 46 survived, 32 unviable, 86.2% | cargo-mutants on Reapr's own Rust code |
| 21 of 21 rule checks pass with the skill, 14 of 21 without | The `mutation-audit` skill evals (4 evals, PR #14) |
| 31 merged PRs in 4 open-source repos: 27.4× faster (median) than a full Stryker run, same verdicts | `bench/oss/results/2026-11-03-suite-run-41556588144/summary.md` (CI run 41556588144) |

Images: `launch-1-problem.png` to `launch-4-number.png` go with the X thread in that order. `card-*.png` are
share cards (1200×630) for links and replies.

## X thread

**1/6** (image: `launch-1-problem.png`)

AI agents write tests that pass. Passing does not mean they protect behavior.

Coverage tells you which lines ran. Mutation testing tells you whether a test fails when a line is wrong.

Reapr runs mutation testing on the lines your pull request changes. Open source.

**2/6** (image: `card-terminal.png`)

A small example. `price()` takes 10% off orders of 100 or more. Two tests, 100% line and branch coverage.

Change `>=` to `>` and both tests still pass. `rpr run --diff main` shows that mutant and where it is.

**3/6** (image: `launch-2-diff.png`)

Mutation testing usually runs nightly because a full run takes too long.

Across 31 merged PRs in 4 open-source repos, Reapr was 27.4× faster (median) than a full Stryker run, same verdicts.

In a Turborepo monorepo, one ignore pattern cut a small run from 38.4 s to 12.5 s.

**4/6** (image: `launch-3-loop.png`)

`rpr run --diff origin/main --json` lists every survivor with file, line, mutator and replacement.

The `/reapr:mutation-audit` agent skill writes one test per survivor, then runs Reapr again to confirm the
mutant is killed. In our evals, 21 of 21 rule checks passed with the skill and 14 of 21 without it.

**5/6** (image: `launch-4-number.png`)

We ran mutation testing on Reapr's own code with cargo-mutants. 365 mutants, 86.2%, 46 survived.

Our tests miss things too. That is the point.

**6/6**

- CLI: `rpr`
- GitHub Action: `reapr-corp/run@v1`
- Agent skill: `/reapr:mutation-audit`

Phase 1: Linux, StrykerJS (JavaScript and TypeScript). Your config, your engine.

github.com/reapr-corp/reapr

## Show HN

**Title:** Show HN: Reapr – mutation testing scoped to the pull-request diff

**First comment:**

Reapr (`rpr`) runs your existing mutation engine on the lines your branch changed and reports a score and the
surviving mutants. It uses your own config. It adds only the diff scope and a JSON reporter, and it never edits
the config file.

Why: more test code is now written by agents, and agents are good at tests that pass. Coverage cannot tell those
tests apart from tests that check behavior. A mutation score can. In a small example, two tests give
100% line and branch coverage of a discount function, and both still pass when `>=` becomes `>`.

Why scoped to the diff: teams skip mutation testing on pull requests because of time, not tooling. While
measuring a pnpm + Turborepo monorepo, we found that 85% of a small diff's run happened before the first test,
mostly walking and copying Turborepo's cache into Stryker's sandbox. Adding `.turbo` to `ignorePatterns` took that
run from 38.4 s to 12.5 s with identical results. Across 31 merged PRs in 4 open-source repos, Reapr was 27.4×
faster (median) than a full Stryker run, with the same verdicts. Scoping also fixes the
number itself: with `incremental: true`, Stryker reported 87.0% package-wide; the diff's own mutants scored 100%.

What it is today: Linux only, one engine (StrykerJS), a CLI, a GitHub Action (`reapr-corp/run@v1`) and an agent
skill that writes one test per survivor and reruns to confirm the kill. PIT, cargo-mutants, mutmut, Infection
and Stryker.NET are planned. We ran cargo-mutants on Reapr itself: 365 mutants, 86.2%, 46 survivors.

The benchmark harness and raw results are in `bench/`. I'd like to hear where the diff scope gets it wrong for
your setup.

## LinkedIn

AI agents now write a large share of our tests. Most of those tests pass. Fewer of them protect behavior.

Coverage measures which lines ran. A mutation score measures something else: change the code on purpose, and see
whether a test fails. Two tests can give a function 100% line and branch coverage and still pass when `>=`
becomes `>`.

Today we're releasing Reapr, open-source mutation testing scoped to the pull-request diff. It runs your existing
engine (StrykerJS for now) with your own config, only on the lines your branch changed, and lists every mutant
that survived.

Some measurements from a pnpm + Turborepo monorepo:

- 85% of a small diff's run happened before the first test.
- One ignore pattern (`.turbo`) took a run from 38.4 s to 12.5 s, with identical results.
- Across 31 merged PRs in 4 open-source repos, Reapr was 27.4× faster (median) than a full Stryker run, with the
  same verdicts.
- Scoped to the diff, the score was 100%. Package-wide, Stryker reported 87.0%.

It comes as a CLI (`rpr`), a GitHub Action (`reapr-corp/run@v1`) and an agent skill (`/reapr:mutation-audit`) that
writes one test per surviving mutant and reruns to confirm it is killed.

We also ran mutation testing on Reapr's own code: 86.2%, 46 survivors. Our tests miss things too.

github.com/reapr-corp/reapr

## README or blog intro

AI agents write tests that pass but do not always protect behavior, and coverage cannot tell the difference: it
measures which lines ran, not whether a test would fail if they were wrong. A mutation score measures that.
Reapr runs your existing mutation engine, with your own config, on the lines your pull request changes, and
reports the score and every surviving mutant, so a person or an agent can write the missing test. On a pnpm +
Turborepo monorepo, the time of a small diff's run was mostly engine startup, not testing. Across 31 merged PRs in
4 open-source repos, Reapr was 27.4× faster (median) than a full Stryker run, with the same verdicts.
