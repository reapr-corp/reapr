# Reapr

[![CI](https://github.com/reapr-corp/reapr/actions/workflows/ci.yml/badge.svg)](https://github.com/reapr-corp/reapr/actions/workflows/ci.yml)
[![License: FSL-1.1-ALv2 / MIT](https://img.shields.io/badge/license-FSL--1.1--ALv2%20%2F%20MIT-blue)](#licensing)

Mutation testing scoped to your diff.

Teams now ship large amounts of AI-generated code. How do you know it does not add bugs or regressions?
Coverage measures which lines ran, not whether any test would notice if they changed. Agents write tests that
pass but do not protect behavior. The mutation score measures exactly that: it changes your code on purpose and
checks that a test fails. Reapr runs your existing mutation engine on the lines your branch changed, with your
own config, and gives you a score and a list of surviving mutants that a person or an agent can act on.

What keeps teams from running mutation testing on every pull request is time, not the lack of tools. A signal
that takes 40 to 90 minutes ends up in a nightly job. Making it fast is the goal Reapr is working toward.
Phase 1 does not make the engine faster yet: it builds the base (a correct diff scope, a machine-readable
verdict, and raw timing data for every run) that the speed work will be measured against.

## Contents

- [Status](#status)
- [Install](#install)
- [Quick start](#quick-start)
- [What Reapr changes in your config](#what-reapr-changes-in-your-config)
- [Output](#output)
- [Run record](#run-record)
- [GitHub Action](#github-action)
- [Agent skills](#agent-skills)
- [How it works](#how-it-works)
- [Measurements](#measurements)
- [Known limitations](#known-limitations)
- [Telemetry](#telemetry)
- [Engines](#engines)
- [Roadmap](#roadmap)
- [Licensing](#licensing)

## Status

Phase 1, released: see the [latest release](https://github.com/reapr-corp/reapr/releases/latest) and the
[changelog](crates/rpr/CHANGELOG.md). Releases come from changesets: see
[CONTRIBUTING.md](CONTRIBUTING.md#changesets). Website: [reapr.dev](https://reapr.dev).

- A CLI, `rpr`: `rpr run`, plus `rpr telemetry` to see or change [telemetry](#telemetry).
- A GitHub Action, [`reapr-corp/run@v1`](https://github.com/marketplace/actions/reapr-mutation-testing).
- Agent skills for Claude Code (and AGENTS.md for other agents): [Agent skills](#agent-skills).
- Two engines: StrykerJS (JavaScript and TypeScript) and cargo-mutants (Rust).
- Linux only (x86_64 and aarch64), tested on Ubuntu. macOS and Windows are not supported yet: see the
  [roadmap](#roadmap). Nothing runs in the cloud. The only thing sent anywhere is anonymous usage
  [telemetry](#telemetry), which you can turn off.

The agreed design for this phase is in [docs/design/phase-1.md](docs/design/phase-1.md).

## Install

Requirements on the machine that runs Reapr:

- Linux, x86_64 or aarch64 (tested on Ubuntu). macOS and Windows are not supported yet.
- `git`.
- For StrykerJS: Node.js 20.10 or later (StrykerJS 9 needs Node 20; Reapr's plan step and wrapper use import
  attributes), StrykerJS installed in the project (`npx --no-install stryker` must work), and a Stryker config
  file.
- For cargo-mutants: cargo-mutants 26.2 to 27.x (`cargo install --locked cargo-mutants`). Other versions run
  with a warning. A `.cargo/mutants.toml` is optional, as for cargo-mutants itself.

### From source (works today)

Needs Rust 1.97 or later.

```sh
cargo install --git https://github.com/reapr-corp/reapr rpr
```

Or build it from a clone:

```sh
git clone https://github.com/reapr-corp/reapr
cd reapr
cargo build --release   # binary: target/release/rpr
```

### Prebuilt binaries (from v0.1.0)

Not available until v0.1.0 is released. Releases will ship binaries for x86_64 and aarch64 Linux (glibc), with a
shell installer:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/reapr-corp/reapr/releases/latest/download/rpr-installer.sh | sh
```

The installer puts `rpr` in `$CARGO_HOME/bin` (`~/.cargo/bin` by default).

## Quick start

Commit your changes first: Reapr mutates what changed in `<base>...HEAD` (from the merge base to `HEAD`).
Uncommitted changes are not part of the diff.

```sh
# Mutate the lines changed since the merge base with origin/main.
rpr run --diff origin/main

# Exit 1 when the diff's mutation score is below 80%.
rpr run --diff origin/main --min-score 80

# One JSON document on stdout, for CI and agents.
rpr run --diff origin/main --json

# Monorepo: run in one package (the engine's working directory), with that package's config.
rpr run --diff origin/main --project packages/lib
rpr run --diff origin/main --project packages/lib --config packages/lib/stryker.config.mjs

# A repo where several engines are detected (a Rust crate with a JS frontend): pick one.
rpr run --diff origin/main --engine cargo-mutants

# Whole-project audit: every git-tracked file, not a diff.
rpr run --all
```

| Flag | Meaning |
|---|---|
| `--diff <BASE>` | Base ref; the diff is `<BASE>...HEAD`. Must not start with `-`. Required unless `--all`. |
| `--all` | Mutate every git-tracked file under the project directory, in full, instead of a diff. Not with `--diff`. |
| `--json` | Print a single JSON document on stdout instead of the human summary. |
| `--min-score <N>` | Fail (exit 1) when the score is below `N` percent (0 to 100). |
| `--config <PATH>` | Engine config file. Skips config detection. Its file name picks the engine when it is one the engine looks for (`stryker.config.mjs` and Stryker's other names, `mutants.toml`). |
| `--engine <ENGINE>` | `stryker` or `cargo-mutants`. Needed when several engines are detected in the project. |
| `--project <DIR>` | Project directory and engine working directory. Default: the current directory. |

Relative paths in `--config` and `--project` resolve against the current directory. Only changed files inside
the project directory count.

`--all` is for auditing a whole codebase. Reapr lists the tracked files (`git ls-files`) under the project
directory and gives each engine every line of each one (binary and empty files are skipped). The engine then
narrows that scope with its own config, as it does a diff: Stryker keeps what your `mutate` patterns select,
cargo-mutants what it would mutate in a plain `cargo mutants` run. Lines are read from the work tree, so
uncommitted edits to tracked files count; untracked files do not.

### Engine and config detection

Reapr picks the engine from `--engine`, else from the file name of `--config`, else from what it finds in the
project directory: StrykerJS for a Stryker config file or a `package.json` that lists `@stryker-mutator/core`,
`vitest` or `jest`; cargo-mutants for a `Cargo.toml`. When more than one engine is found, Reapr stops and asks
for `--engine`.

For StrykerJS, without `--config`, Reapr looks in the project directory for a Stryker config file in Stryker's own lookup
order (`stryker.conf.{json,js,mjs,cjs}`, `stryker.config.{json,js,mjs,cjs}`, then the same names with a leading
dot) and uses the first one it finds.

If there is no config file, Reapr stops with an error. When `package.json` lists `@stryker-mutator/core`,
`vitest` or `jest`, the error suggests `npx stryker init`. Reapr never generates a config.

For cargo-mutants, Reapr passes no config: cargo-mutants reads `.cargo/mutants.toml` in the workspace root
itself, as it does when you run `cargo mutants`. `--config <PATH>` is passed on as `cargo mutants --config`. If
cargo-mutants is not installed, the run stops with an error that says how to install it.

### For agents

`rpr run --diff origin/main --json` prints every surviving mutant with its file, line, column, mutator and
replacement. That is enough for an agent to find the change no test noticed, write the missing test, and run
Reapr again. Reapr reports; it does not write tests.

## What Reapr changes in your config

Reapr writes nothing into your work tree and never edits your config file. It runs your engine config as it is
and adds only the diff scope and a machine-readable report. For each run it writes a wrapper config (`stryker.wrapper.mjs`, in the run directory)
that imports your config and overrides exactly three keys:

| Key | Value |
|---|---|
| `mutate` | The changed line ranges (`path:start-end`) of the files that your own `mutate` patterns select. Patterns apply in order and a `!` pattern excludes what earlier patterns included, matched with the `minimatch` your installed Stryker uses, exactly as Stryker matches them (case-sensitive, hidden files excluded). Without `mutate` in your config, Stryker's default from the installed version is used. |
| `reporters` | Your reporters (or Stryker's default `clear-text`, `progress`, `html`) plus `json`. |
| `jsonReporter.fileName` | A private path in the run directory, so Reapr never reads a stale report. Other `jsonReporter` keys are kept. |

Everything else is yours and stays as written: test runner, thresholds, `disableBail`, `coverageAnalysis`,
`concurrency`, `incremental`, checkers, timeouts, plugins. Reapr does not force flags and has no opinion on any
of them.

It may warn. With `testRunner: 'vitest'`, Vitest 4 or later installed and `disableBail` not set to `true`, it
prints a warning that bail can report killed mutants as survived. It does not change the setting.

If the diff touches no file your `mutate` patterns select, Stryker is not started and the run reports
`nothing to mutate`.

### cargo-mutants

Reapr runs `cargo mutants --in-diff <run dir>/scope.diff --output <run dir>` in the project directory, plus
`--workspace` when the project directory is the workspace root (so the diff, not cargo's default members,
decides which packages are mutated). Nothing else: jobs, timeouts, test tool, features, exclusions and every
other setting come from your `.cargo/mutants.toml` and environment, as written. `--output` overrides an `output`
key in your config, so Reapr reads the report of this run only.

`scope.diff` marks exactly the diff's ranges in your Rust files, so cargo-mutants selects the mutants that
touch those lines by its own rule. If the diff changes no Rust file, cargo-mutants is not started and the run
reports `nothing to mutate`. cargo-mutants has no thresholds, so it never fails its own gate: missed mutants
and timeouts are results (its exit codes 2 and 3). Tests that fail without any mutation (exit code 4) are an
error, with the path of cargo-mutants' baseline log.

## Output

The summary goes to stdout. Stryker's own output, warnings (`warning: ...`), the failure reason
(`failed: ...`) and errors (`error: ...`) go to stderr.

```text
rpr · stryker · diff vs origin/main · 3 ranges in 2 files
score 80.00%  (8 killed, 0 timeout, 2 survived, 0 no coverage)  1m 23s
survivors:
  src/a.ts:10:3  ConditionalExpression  → false
  src/b.ts:6:2  ConditionalExpression  → false
run: /home/me/repo/.git/reapr/runs/1793640000-4242
```

When nothing in the diff is mutated, the second line reads `nothing to mutate in diff vs origin/main`. With
`--all`, both lines say `all files` instead of `diff vs <base>`.

### Score

The score uses the [mutation-testing-metrics](https://stryker-mutator.io/docs/mutation-testing-elements/mutant-states-and-metrics/)
definition, so it compares one to one with the engine's own report:

- detected = Killed + Timeout
- undetected = Survived + NoCoverage
- score = detected / (detected + undetected) × 100

CompileError, RuntimeError, Ignored and Pending are excluded. With no valid mutants the score is `null` and the
run passes.

Only mutants inside the diff scope count. Reapr filters the engine's report with Stryker's own containment rule:
a mutant counts when it starts on or after the first column of a changed range's first line and ends on or
before its last line (positions compared as line and column, as Stryker does).
Results that Stryker carries over from outside the scope (it does this with `incremental: true`) are excluded,
and a warning says how many.

### JSON

With `--json`, stdout holds one JSON document and nothing else.

```json
{
  "run_id": "1793640000-4242",
  "run_dir": "/home/me/repo/.git/reapr/runs/1793640000-4242",
  "engine": "stryker",
  "base": "origin/main",
  "ranges": [{ "path": "src/a.ts", "start": 8, "end": 12 }, { "path": "src/b.ts", "start": 5, "end": 6 }],
  "score": 80.0,
  "counts": { "killed": 8, "survived": 2, "no_coverage": 0, "timeout": 0,
              "compile_error": 0, "runtime_error": 0, "ignored": 0, "pending": 0 },
  "survivors": [
    { "file": "src/a.ts", "line": 10, "column": 3, "mutator": "ConditionalExpression",
      "replacement": "false", "status": "Survived" },
    { "file": "src/b.ts", "line": 6, "column": 2, "mutator": "ConditionalExpression",
      "replacement": "false", "status": "Survived" }
  ],
  "duration_ms": 83400,
  "min_score": 80.0,
  "passed": true,
  "engine_gate_failed": false,
  "warnings": [],
  "resources": { "cpu_user_ms": 250000, "cpu_sys_ms": 30000, "max_rss_kb": 812000 }
}
```

| Field | Type | Meaning |
|---|---|---|
| `run_id` | string | `<unix seconds>-<pid>`. |
| `run_dir` | string | Absolute path of the run directory. |
| `engine` | string | `stryker` or `cargo-mutants`. |
| `base` | string or null | The `--diff` value, as given. `null` with `--all`. |
| `ranges` | array | Changed line ranges from the diff: `path` (relative to the project directory), `start`, `end` (1-based, inclusive). Before your `mutate` patterns are applied. |
| `score` | number or null | Percent. `null` when nothing was mutated or there were no valid mutants. |
| `counts` | object | Mutants in scope per status: `killed`, `survived`, `no_coverage`, `timeout`, `compile_error`, `runtime_error`, `ignored`, `pending`. |
| `survivors` | array | Survived and NoCoverage mutants, sorted by file, line, column: `file`, `line`, `column` (start position, 1-based), `mutator`, `replacement` (string or null), `status` (`Survived` or `NoCoverage`). |
| `duration_ms` | integer | Engine wall time. `0` when the engine did not run. |
| `min_score` | number or null | The `--min-score` value. |
| `passed` | boolean | `false` when the score is below `--min-score` or the engine's own gate failed. |
| `engine_gate_failed` | boolean | The engine's own threshold failed: Stryker exited 1 and your config has a numeric `thresholds.break`. Any other non-zero Stryker exit is an error (exit code 2). Always `false` for cargo-mutants, which has no threshold. |
| `warnings` | array of strings | The same warnings printed to stderr. |
| `resources` | object or null | `cpu_user_ms`, `cpu_sys_ms` and `max_rss_kb` of the engine's process tree. `null` when the engine did not run. |

On an error (exit code 2) with `--json`, stdout holds `{ "error": "<message>" }`. This includes invalid arguments.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Passed. Also when nothing in the diff was mutated, or there were no valid mutants. |
| 1 | The score is below `--min-score`, or the engine's own configured threshold failed. |
| 2 | Error: invalid arguments, no git repository, unknown base, no engine or config found, several engines found without `--engine`, engine not installed, engine crash, no report, failing tests without mutations, or an engine exit other than its threshold failure. |

## Run record

Every run that completes leaves its raw data in `<git dir>/reapr/runs/<run_id>/`, for example
`.git/reapr/runs/1793640000-4242/`. This is outside the work tree: git does not track it, engines that copy the
project into a sandbox do not copy it, and you need no `.gitignore` entry. Each git worktree has its own git dir,
so worktrees never share runs. Reapr never sends this data anywhere and never deletes it; remove the
`reapr` directory under your git dir to clear it.

| File | Contents |
|---|---|
| `run.json` | Facts about the run (below). Written also when nothing was mutated; not written on errors. |
| `report.json` | Reapr's report: every mutant in the diff scope with its status, `killed_by` and `status_reason`. |
| `mutation.json` | StrykerJS: Stryker's own JSON report, unfiltered. |
| `stryker.wrapper.mjs`, `plan.mjs`, `plan-input.json` | StrykerJS: the wrapper config and the plan step, as they ran. |
| `scope.diff`, `mutants.out/` | cargo-mutants: the diff it was given and its own output directory (`outcomes.json`, logs, mutant diffs). |

`run.json` holds raw facts, not judgments:

| Field | Meaning |
|---|---|
| `run_id` | Same as in the JSON output. |
| `scope` | `diff` (`--diff`) or `all` (`--all`). |
| `git` | `head_sha`, `base`, `merge_base_sha`, and `commits` in `merge_base..HEAD`: `sha`, `author_email`, and `trailers` as written (for example `Co-Authored-By: ...`). With `--all`, `base` is `HEAD` and `commits` is empty. |
| `engine`, `engine_version` | For StrykerJS, the version of `@stryker-mutator/core` that Node resolves from the project directory; for cargo-mutants, the version `cargo mutants --version` prints. |
| `started_at` | When Reapr started, RFC 3339 UTC. |
| `duration_ms` | Engine wall time, the same window as `resources`. |
| `environment` | `kind` (`local` or `github-actions`), `runner_name`, `os`, `arch`, `cpu_count`. |
| `ranges`, `counts`, `score`, `min_score`, `passed`, `engine_gate_failed`, `warnings` | As in the JSON output. |
| `resources` | CPU time and peak RSS of the engine's process tree. |

If a git command fails while collecting these facts, the fact is left empty (`null` or `[]`) and a warning is
added. It never fails the run.

## GitHub Action

[reapr-corp/run](https://github.com/reapr-corp/run) runs Reapr on a pull request, writes a job summary
(score, counts, survivors, warnings) and exits with Reapr's exit code. It installs Reapr from a published
release, so it works once v0.1.0 is released.

```yaml
on: pull_request

jobs:
  rpr:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0 # full history, so origin/<base>...HEAD has a merge base
      - uses: actions/setup-node@v5
        with:
          node-version: 22
          cache: npm
      - run: npm ci # rpr runs your engine (StrykerJS) with your installed deps
      - name: rpr
        uses: reapr-corp/run@v1
        id: rpr
        with:
          min-score: 80
```

Linux runners only (`ubuntu-latest`, or Blacksmith Ubuntu labels such as `blacksmith-2vcpu-ubuntu-2404`; x86_64
and aarch64). No token is needed: the action installs from the public `reapr-corp/reapr` releases.

Inputs: `base` (default: the pull request base), `min-score`, `config`, `configs` (one config path per line, for
monorepos with one config per package; not with `config`), `working-directory`, `version`, and `github-token`
(only for a private fork or mirror of the releases). Outputs: `score`, `passed`, `run-id`, and `results` (path to
a JSON file with one entry per config). See the [action's README](https://github.com/reapr-corp/run) for details.

## Agent skills

The `reapr` plugin has two skills. `setup` (`/reapr:setup`) onboards a repo: it checks that `rpr` and the engine
are installed, finds the engine config (it never writes one; for StrykerJS it asks you to run `npx stryker init`),
runs a whole-project baseline with `rpr run --all`, and offers a pull-request gate with the GitHub Action and an
agent rule, asking before each change.

The `mutation-audit` skill lets a coding agent improve tests with Reapr's results as evidence. It runs
`rpr run --diff <base> --json` (or `rpr run --all --json` for a whole-codebase audit), writes one focused test per surviving mutant and runs Reapr again to confirm
the mutant is killed, lists tests that cover the diff but kill no mutant, and then checks the diff for edge
cases that mutation operators do not represent (boundaries, empty input, error paths, time zones, numeric
precision). It never changes production code or your engine config, and it asks you when the intended
behavior is unclear instead of writing a test that locks in the current behavior.

Claude Code:

```text
/plugin marketplace add reapr-corp/reapr
/plugin install reapr@reapr
```

On Claude Code 2.1.275 or later, one step does both:

```text
/plugin install reapr --marketplace reapr-corp/reapr
```

Then, in three steps:

1. Install the plugin (above).
2. Run `/reapr:setup`: installs what is missing (with your consent) and measures a baseline.
3. Run `/reapr:mutation-audit`, or ask Claude to check the tests on your branch.

Other agents (Cursor, Codex and others): point them at [plugins/reapr/AGENTS.md](plugins/reapr/AGENTS.md). It
has the hard rules, a short setup section, and links the full workflow in the skills' `SKILL.md` files.

## How it works

1. `git diff --unified=0 --inter-hunk-context=0 --no-textconv <base>...HEAD` gives the changed line ranges,
   whatever your git config says about hunk context or textconv. Reapr keeps the files inside the project
   directory.
2. `plan.mjs` runs in Node from the project directory. It loads your Stryker config and keeps the ranges whose
   files your `mutate` patterns select.
3. Reapr writes the wrapper config: your config plus `mutate`, the `json` reporter and its private path.
4. It runs `npx --no-install stryker run <wrapper>` in the project directory and waits for it. The `rpr`
   process uses no CPU while Stryker runs: it does not poll and does not parse Stryker's output.
5. It reads Stryker's JSON report, keeps the mutants inside the diff scope, computes the score and writes the
   run record.

## Measurements

Measured on one package of the reference monorepo: a pnpm + Turborepo monorepo (~55k files including build
caches; StrykerJS 9.6.1, Vitest 4), on a local 22-thread Linux machine, 2026-11-02.

**Scope.** The package's config has `incremental: true`. A plain `npx stryker run --mutate <ranges>` reported
291 mutants, a score of 87.0% and 38 survivors, package-wide: Stryker carries old results from outside the
`--mutate` ranges into its report (`incremental-differ.js`, lines 130-157). The diff itself had 3 mutants, all
killed: 100%. Reapr scores only the diff, and its verdict for each mutant was identical to the plain Stryker run
on the same ranges.

**Time.** Wall time was 39 to 82 seconds across four runs for 3 mutants. The initial test run took 2 to 3
seconds. In the slower runs, 44 to 52 seconds went to reading the project and building the sandbox (55,365
files) and to starting the checker, before the dry run. Four runs are too few to explain the variance. The time
is in engine startup, not in the orchestrator, so that is where speed work will go.

Earlier measurements on the reference monorepo: 82% to 98% of the initial test run was Vitest startup and transformation; Stryker's
incremental cache was reused 0 of 525 times; and Vitest 4 with bail on miscounted killed mutants (score 98.0
against 87.8 with bail off). This last one is why Reapr warns about bail, and why it only warns.

## Known limitations

- Only committed changes count: the diff is `<base>...HEAD`, so uncommitted and untracked changes are not
  mutated. (`--all` reads the tracked files from the work tree, so it includes uncommitted edits, but still
  not untracked files.)
- cargo-mutants mutates the working tree. Uncommitted edits to a changed Rust file shift its lines against the
  committed diff, so the ranges can select different code; commit or stash first.
- A `mutate` entry in your config that carries its own range (`src/a.ts:1-9`) never matches; use plain globs.
- Line breaks: git splits lines on `\n` only, while Babel (Stryker's parser) also treats U+2028, U+2029 and a
  lone CR as line breaks. In files that contain them, Stryker's line numbers drift from git's and the diff
  ranges can point at the wrong lines.

## Telemetry

`rpr run` sends one anonymous usage event (outcome, counts, durations, bucketed diff size, platform, CI kind,
flag names) to PostHog after the output is printed. It never sends code, paths, names, shas, emails or messages.
It is on by default with a one-time notice on stderr; turn it off with `DO_NOT_TRACK=1`, `RPR_TELEMETRY=0` or
`rpr telemetry disable`, and see the exact payload with `RPR_TELEMETRY_DEBUG=1`. Every property is listed in
[TELEMETRY.md](TELEMETRY.md).
Projects are counted by `project_id`, a salted hash of the repository's root commit, never its name or URL.

## Engines

| Engine | Language | Status |
|---|---|---|
| [StrykerJS](https://stryker-mutator.io/) | JavaScript, TypeScript | Supported |
| [cargo-mutants](https://mutants.rs/) | Rust | Supported: 26.2 to 27.x |
| [PIT](https://pitest.org/) | Java, JVM | Planned |
| [Infection](https://infection.github.io/) | PHP | Planned |
| [mutmut](https://github.com/boxed/mutmut) | Python | Planned |
| [Stryker.NET](https://stryker-mutator.io/docs/stryker-net/introduction/) | C# | Planned |

Reapr does not replace mutation engines; it runs them. Every engine plugs in through one Rust trait,
`MutationEngineAdapter` in `reapr-core`. The core knows no engine: config formats, report formats, what counts as
a test file and CLI flags all live in the adapter. An adapter implements `detect` and `run`; `list_mutants`,
`coverage` and `cache_key` may return `Unsupported` (the StrykerJS adapter does, in Phase 1; the cargo-mutants
adapter implements `list_mutants`). See [CONTRIBUTING.md](CONTRIBUTING.md#adding-an-engine-adapter) to add one,
and [docs/adr/](docs/adr/) for the decisions behind the adapter design.

## Roadmap

Planned, not built:

- **Sharded and parallel execution.** The `Shard` type exists in the adapter trait; Phase 1 always runs one shard.
- **Speed work on engine startup**, where the measurements above put the time. Nothing moves to Rust without
  profiling that shows the time is there.
- **Content cache** of mutant results, so unchanged mutants are not run again.
- **MCP server** (`rpr mcp`) so agents can call Reapr as a tool.
- **macOS and Windows** support (binaries, installers and CI). Phase 1 is Linux only.
- **More engines**: PIT, Infection, mutmut, Stryker.NET.
- **Hosted compute** (warm workers, parallel shards), a **global cache**, and **history and metrics** across
  runs. This is the planned paid layer. The CLI and the engine adapters stay open source and work without it.

## Licensing

Reapr is open core. The tool is free to use, modify and self-host; the planned hosted layer is the business.

| Crate | License |
|---|---|
| `reapr-core`, `rpr` | [FSL-1.1-ALv2](LICENSE.md) |
| Engine adapters (`reapr-stryker`, `reapr-cargo-mutants`) | MIT ([reapr-stryker](crates/reapr-stryker/LICENSE), [reapr-cargo-mutants](crates/reapr-cargo-mutants/LICENSE)) |
| Agent skills (`plugins/reapr`) | [MIT](plugins/reapr/LICENSE) |

FSL-1.1-ALv2 (Functional Source License) lets you use, modify and self-host Reapr for any purpose except
offering it as a competing commercial product or service. Each release converts to Apache 2.0 two years after it
is published.

Engine adapters are MIT because they are a community surface: people who know an engine best should be able to
write, fork and reuse an adapter without any restriction. Agent skills are MIT for the same reason: copy and
adapt them into any agent's instructions.
