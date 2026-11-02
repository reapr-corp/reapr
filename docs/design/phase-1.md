# Phase 1: local CLI, no cloud

Status: agreed design, 2026-11-02. Changes to the `MutationEngineAdapter` interface need the owner's approval.

## Why

Teams now ship large amounts of AI-generated code. The question Reapr answers is: how do you know that code
does not add bugs or regressions? Coverage measures execution, not verification; the mutation score measures
whether the tests actually protect behavior. Speed matters (a 40-90 min signal ends up in a nightly job), but
it is a separate problem to solve on top of this base.

Phase 1 is the base: one portable binary that runs the same on a laptop, on Blacksmith or on any CI, with the
configuration each repo already has. Every run leaves raw data behind; that data is what later becomes
visibility and business (trends per repo, human vs agent, minutes per run). Trust comes first: Reapr's verdict
must match the engine's own run, mutant by mutant.

## Principles

- **We invent nothing.** Each repo configures its own engine and tests. Reapr runs the user's config and adds
  only what the product is: the diff scope and a machine-readable report. No forced flags, no generated
  configs, no opinion on thresholds, bail, coverage analysis or concurrency.
- **Not only Stryker.** The core knows no engine. Everything engine-specific (what a test file is, config
  formats, report formats, CLI flags) lives in the adapter. PIT, Infection, cargo-mutants, mutmut and
  Stryker.NET must fit the same trait later.
- **Zero CPU while the engine runs.** The binary awaits the child process (`tokio::process::Child::wait`); no
  polling, no busy loops, no output parsing loops. Engine output goes straight to our stderr.
- **No `unwrap`/`expect` on error paths.** Errors are typed (`thiserror`) in libraries, `anyhow` only in the CLI.
- Nothing moves to Rust without profiling that shows the time is there.

## Workspace

```
Cargo.toml                 # [workspace], resolver 3, edition 2024, shared deps
crates/reapr-core          # FSL-1.1-ALv2. Types, trait, diff parsing, score. No engine knowledge.
crates/reapr-stryker       # MIT. StrykerJS adapter.
crates/reapr-cargo-mutants # MIT. cargo-mutants adapter.
crates/rpr                 # FSL-1.1-ALv2. Binary `rpr`.
```

Dependencies: `serde`/`serde_json`, `thiserror` (libs), `anyhow` + `clap` (derive) in the CLI, `tokio`
(`rt`, `process`, `macros`, `fs`, `io-util`) where async is needed, `dunce` for real paths without Windows'
`\\?\` prefix. Nothing else without a reason.

## reapr-core

### Types (all `Serialize + Deserialize + Debug + Clone`)

```rust
/// 1-based, inclusive line range in one file. `path` is relative to the project dir.
pub struct FileRange { pub path: PathBuf, pub start: u32, pub end: u32 }

/// What to mutate: changed line ranges, relative to `project_dir`.
pub struct MutationScope { pub ranges: Vec<FileRange> }

/// A slice of the work. Engines with native sharding (cargo-mutants `--shard k/n`, PIT) use index/total;
/// engines without it (Stryker) use `scope`. Phase 1 always runs one shard: index 0, total 1.
pub struct Shard { pub index: u32, pub total: u32, pub scope: MutationScope }

pub struct Position { pub line: u32, pub column: u32 }   // 1-based

pub struct Mutant {
    pub id: String,                 // engine's own id
    pub file: PathBuf,              // relative to project dir
    pub start: Position,
    pub end: Position,
    pub mutator: String,
    pub replacement: Option<String>,
}

pub enum MutantStatus { Killed, Survived, NoCoverage, Timeout, CompileError, RuntimeError, Ignored, Pending }

pub struct MutantResult {
    pub mutant: Mutant,
    pub status: MutantStatus,
    pub killed_by: Vec<String>,
    pub status_reason: Option<String>,
}

pub struct MutationReport {
    pub engine: String,
    pub results: Vec<MutantResult>,
    pub duration: Duration,         // serialize as milliseconds
}

pub struct CacheKey(pub String);
pub struct CoverageMap { pub tests_by_mutant: BTreeMap<String, Vec<String>> }

pub struct Detection { pub engine: &'static str, pub config: Option<PathBuf>, pub reason: String }

pub struct RunContext {
    pub project_dir: PathBuf,       // engine cwd
    pub config: Option<PathBuf>,    // explicit or detected engine config
    pub run_dir: PathBuf,           // <git dir>/reapr/runs/<run_id>/, scratch + artifacts for this run
}
```

### Score

`MutationReport::score() -> Option<f64>` uses the mutation-testing-metrics definition, so verdicts compare 1:1
with the engine's own report:

- detected = Killed + Timeout
- undetected = Survived + NoCoverage
- score = detected / (detected + undetected) × 100
- CompileError, RuntimeError, Ignored and Pending are excluded. No valid mutants → `None`.

Also: `counts() -> StatusCounts` (one field per status) and `survivors() -> impl Iterator<Item=&MutantResult>`
(Survived + NoCoverage).

`merge_reports(engine, Vec<MutationReport>) -> MutationReport`: concatenates results, duration = max (shards run in
parallel). The trait's default `merge` calls it.

### The trait

Native `async fn` in trait. Returned futures must be `Send`, so declare methods as
`fn x(&self, ..) -> impl Future<Output = Result<T, AdapterError>> + Send`. Not dyn-compatible on purpose:
the CLI dispatches through `enum Engine { Stryker(StrykerAdapter), CargoMutants(CargoMutantsAdapter) }`
([ADR 0001](../adr/0001-engine-adapter-trait.md)).

```rust
pub trait MutationEngineAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn detect(&self, project_dir: &Path) -> impl Future<Output = Result<Option<Detection>, AdapterError>> + Send;
    fn list_mutants(&self, ctx: &RunContext, scope: &MutationScope) -> impl Future<Output = Result<Vec<Mutant>, AdapterError>> + Send;
    fn coverage(&self, ctx: &RunContext, scope: &MutationScope) -> impl Future<Output = Result<CoverageMap, AdapterError>> + Send;
    fn run(&self, ctx: &RunContext, shard: &Shard) -> impl Future<Output = Result<RunOutcome, AdapterError>> + Send;
    fn merge(&self, reports: Vec<MutationReport>) -> Result<MutationReport, AdapterError> { /* default: merge_reports */ }
    fn cache_key(&self, ctx: &RunContext, mutant: &Mutant) -> impl Future<Output = Result<CacheKey, AdapterError>> + Send;
}

pub struct RunOutcome {
    pub report: Option<MutationReport>,  // None = the adapter found nothing to mutate in scope
    pub engine_gate_failed: bool,        // the engine's own configured threshold failed (e.g. Stryker thresholds.break)
    pub warnings: Vec<String>,
    pub resources: Option<Resources>,    // engine process tree CPU/RSS; None where unmeasured (Windows)
}

pub struct Resources { pub cpu_user_ms: u64, pub cpu_sys_ms: u64, pub max_rss_kb: u64 }

#[derive(thiserror::Error)]
pub enum AdapterError {
    Unsupported { engine: &'static str, operation: &'static str },
    ConfigMissing { engine: &'static str, hint: String },
    Io { context: String, source: std::io::Error },
    Process { command: String, code: Option<i32>, detail: String },
    Parse { context: String, detail: String },
}
```

### Diff (`reapr_core::diff`)

- `parse_unified_zero(diff: &str) -> Vec<FileRange>` (pure, unit-tested). Paths relative to the git root.
  - File from `+++ b/<path>`; `+++ /dev/null` (deleted file) contributes nothing. Handle C-quoted paths
    (`+++ "b/sp\303\251c ial.ts"`) by unquoting octal escapes and `\"`, `\\`, `\t`, `\n`.
  - Hunk `@@ -a[,b] +c[,d] @@`: missing `d` means 1; `d == 0` (pure deletion) contributes nothing; else
    lines c..=c+d-1.
  - Binary files, mode-only changes, renames without content changes: nothing.
  - Merge overlapping or adjacent ranges per file; sort by path then start.
- `changed_ranges(project_dir, base) -> Result<Vec<FileRange>, DiffError>` (async): runs
  `git -c core.quotePath=false diff --unified=0 --no-color --no-ext-diff --find-renames --inter-hunk-context=0
  --no-textconv --src-prefix=a/ --dst-prefix=b/ --end-of-options <base>...HEAD --` in the repo root
  (`git rev-parse --show-toplevel` from project_dir), parses, then re-bases paths to `project_dir` and drops
  files outside it. Non-zero git exit → `DiffError` with git's stderr. `--inter-hunk-context=0` and
  `--no-textconv` keep the user's git config (`diff.interHunkContext`, textconv filters) from fusing hunks or
  shifting lines; `--end-of-options` makes a base that starts with `-` a revision, never an option (the CLI
  also rejects such a base). The merge-base call in `record` uses `--end-of-options` too.
- Known limit: git splits lines on `\n` only, while Babel (Stryker's parser) also breaks lines on U+2028,
  U+2029 and a lone CR. In a file containing those, Stryker's line numbers and git's differ and the ranges
  can point at the wrong lines.
- No "is this a test" filter here. What counts as mutable source is the engine config's business.

### Run store (`reapr_core::store`)

Runs live in `<git dir>/reapr/runs/<id>/`, outside the work tree: engines that copy the project into a
sandbox (Stryker) never copy `.git`, and git never tracks it, so no `.gitignore` is needed. Each worktree has
its own git dir, so worktrees never share runs.

- `git_dir(project_dir) -> Result<PathBuf, DiffError>` (async): `git rev-parse --absolute-git-dir` from
  project_dir.
- `new_run_dir(git_dir) -> io::Result<(String, PathBuf)>`: id = `<unix seconds>-<pid>`, creates
  `<git_dir>/reapr/runs/<id>/`.
- `write_report(run_dir, &MutationReport)` → `report.json`; `write_json(run_dir, file, &impl Serialize)`
  for `run.json`.

### Run record (`run.json`, pending owner review)

Raw facts only, never derived judgments; metrics are computed later from these. Stays local in
`<git dir>/reapr/runs/<id>/`; never sent (telemetry is a separate event). Written by the CLI after the run, also when
there is nothing to mutate; not on errors. Gathering facts never fails the run: a git command that fails
becomes a warning and leaves its fact empty (`null` / `[]`). Facts come from `reapr_core::record`
(`git_facts`, `Environment::detect`, `rfc3339`); the engine version from the adapter (Stryker:
`version` of the `@stryker-mutator/core` package Node resolves from project_dir; cargo-mutants: what
`cargo mutants --version` prints).

- `run_id`, `scope` (`diff` or `all`).
- git: `head_sha`, `base`, `merge_base_sha`; commits in `merge_base..HEAD`: sha, author email, trailers
  (e.g. `Co-Authored-By`) as written (`git log -z --format=%H%x1f%ae%x1f%(trailers:only,unfold)`,
  raw `Key: value` strings).
- engine: `engine`, `engine_version`.
- time: `started_at` (RFC 3339 UTC, when rpr started), `duration_ms` (engine wall time, the same window
  as `resources`).
- environment: `kind` (`local` | `github-actions`), `runner_name`, `os`, `arch` (`RUNNER_*` when set, else
  Rust's `std::env::consts`), `cpu_count` (`available_parallelism`).
- resources: `cpu_user_ms`, `cpu_sys_ms`, `max_rss_kb` of the whole engine process tree: the adapter reads
  `getrusage(RUSAGE_CHILDREN)` (`reapr_core::resources::children_usage`) right before spawning the engine and right after its `wait` returns (CPU =
  delta; RSS = the high-water mark after, normalized to KiB). `null` on Windows for now. CPU vs wall time is
  the first profiling signal: it says where to optimize (and what to rewrite in Rust) before anything moves.
- result: ranges, counts, score, `min_score`, `passed`, `engine_gate_failed`, `warnings`.

## reapr-stryker

### detect

- Config files in project_dir, Stryker's own 16 names in its order (`SUPPORTED_CONFIG_FILE_NAMES` in
  `config-file-formats.js`, `CONFIG_FILES` in the adapter): `stryker.conf.{json,js,mjs,cjs}`,
  `stryker.config.{json,js,mjs,cjs}`, `.stryker.conf.{json,js,mjs,cjs}`, `.stryker.config.{json,js,mjs,cjs}`.
  First hit → `Detection { config: Some(path) }`.
- Otherwise, `package.json` (dependencies, devDependencies) mentions `@stryker-mutator/core`, `vitest` or `jest`
  → `Detection { config: None, reason: "..." }`. `run` then fails with `ConfigMissing` and a hint
  (`npx stryker init`). Reapr never generates a config.
- No `package.json` → `Ok(None)`. One that exists but cannot be read (a directory, permission denied) →
  `AdapterError::Io`, not "no Stryker".
- `--config <path>` on the CLI skips detection of the config file (a monorepo can have one config per package).

### run (two Node steps, both via `node`, cwd = project_dir)

1. **Plan** (`plan.mjs`, embedded with `include_str!`, written to run_dir): loads the user's config
   (`import(pathToFileURL(config))`, JSON with `with { type: 'json' }`), takes its `mutate` (or Stryker's own
   default read from the installed `@stryker-mutator/core/schema/stryker-schema.json`,
   `properties.mutate.default`), and keeps each changed range whose file the patterns select as Stryker's
   `project-reader.js` does: patterns apply in order, a `!` pattern excludes what earlier patterns included.
   Matching is Stryker's `FileMatcher`: the `minimatch` that the installed `@stryker-mutator/core` resolves
   (`createRequire` from its `package.json`), absolute forward-slash paths resolved against the cwd,
   case-sensitive, `{ dot: false }`. No minimatch resolvable → the plan fails with an error, no fallback.
   Node >= 20.10 (import attributes). Prints JSON to stdout:
   `{ "mutate": ["path:start-end", ...], "ranges": [...], "thresholds_break": bool, "warnings": [...] }`.
   `mutate` escapes glob characters in paths (`[`→`[[]` and so on) because Stryker rejects and glob-matches a
   ranged pattern that minimatch sees as a glob; `ranges` is the same selection unescaped, the scope the report
   is filtered to; `thresholds_break` says whether the user config has a numeric `thresholds.break`.
   Known limit: a user `mutate` entry that carries its own range (`file:1-9`) never matches here.
   Warning (never a change): `testRunner === 'vitest'`, installed vitest major ≥ 4, and `disableBail` not true →
   "vitest >= 4 with bail can report killed mutants as survived (observed: 98.0 vs 87.8 on the same code); consider disableBail: true".
   Changed ranges come in through a JSON file path argument, not argv (argv limits on Windows).
2. Empty `mutate` → `RunOutcome { report: None }`. Stryker is not started.
3. **Wrapper** (`stryker.wrapper.mjs` in run_dir, generated by Rust; string literals via `serde_json` so they are
   valid JS): imports the user's config and exports `{ ...base, mutate, reporters, jsonReporter }` where
   `reporters = unique([...(base.reporters ?? ['clear-text', 'progress', 'html']), 'json'])` and
   `jsonReporter = { ...base.jsonReporter, fileName: '<run_dir>/mutation.json' }`. These are the only overrides.
   The private report path means a stale report can never be read.
4. `npx --no-install stryker run <wrapper>`, cwd = project_dir, stdin null, stdout and stderr → our stderr,
   awaited. Afterwards:
   - No report → `AdapterError::Process` with the exit code. A report path that exists but cannot be read (a
     directory, permission denied) → `AdapterError::Io`.
   - Report and exit 0 → parse it, `engine_gate_failed = false`. Exit 1 and `thresholds_break` (Stryker sets
     exit code 1 only for a score under a numeric `thresholds.break`, `mutation-test-report-helper.js`) →
     `engine_gate_failed = true`. Any other non-zero exit → `AdapterError::Process` naming the report path,
     even though a report exists.

### Report parsing (`schema` module, unit-tested)

mutation-testing-report-schema v2: `files: { "<path>": { mutants: [{ id, mutatorName, replacement?, location:
{ start: {line, column}, end }, status, statusReason?, killedBy? }] } }`. Status strings: `Killed`,
`Survived`, `NoCoverage`, `Timeout`, `CompileError`, `RuntimeError`, `Ignored`, `Pending`. Unknown status →
`Parse` error. File keys may be absolute or relative to the report's `projectRoot` (Stryker writes
`process.cwd()`, a real path): both directories are compared as real paths (`dunce::canonicalize`), so a
project dir given through `..` or a symlink still matches; keys become relative to project_dir. Results are
then filtered to the plan's `ranges` with Stryker's containment rule (`isInMutatedScope` + `locationIncluded`
in `incremental-differ.js`): a range `a-b` is (a-1, 0)..(b-1, MAX_SAFE_INTEGER) in Stryker's 0-based positions,
i.e. (a, 1)..(b, 2^53) in the report's 1-based positions, compared as (line, column) pairs. Results outside
(carried over by `incremental`, possibly with column ≤ 0) are dropped and a warning gives the count. Keep the module
engine-agnostic in shape (it takes the JSON and an engine name) so it can move to core when a second engine
needs it; it stays in this crate until then.

### Not in Phase 1

`list_mutants`, `coverage`, `cache_key` → `AdapterError::Unsupported`. Stryker cannot list mutants without
running them; these arrive with the Node-side bridge.

## reapr-cargo-mutants

Verified against cargo-mutants 26.2.0 and 27.1.0 (`SUPPORTED_VERSIONS = >=26.2.0, <28.0.0`, real fixtures in
`tests/fixtures/<version>/`); another version runs with a warning ([ADR 0002](../adr/0002-engine-version-support.md)).

- **detect:** `Cargo.toml` in project_dir → `Detection { config: None }` (cargo-mutants reads
  `.cargo/mutants.toml` in the workspace root itself); the reason says whether `cargo mutants --version` works
  and, if not, how to install it.
- **run:** Rust ranges only (cargo-mutants ignores other files); none → `report: None`, nothing spawned. Not
  installed → `AdapterError::Process` with the install hint. The scope diff is synthesized from the ranges,
  since the trait gives ranges, not the base: one insertion-only hunk per range whose `+` lines are the file's
  current lines, paths relative to the workspace root (`cargo locate-project --workspace`). cargo-mutants
  checks the diff's new text against the tree (exit 5 otherwise) and takes inserted lines as the changed ones,
  so it selects the mutants that intersect the ranges by its own rule. Then
  `cargo mutants --in-diff <run_dir>/scope.diff --output <run_dir> [--workspace] [--config <path>]
  [--shard k/n]`, cwd = project_dir, stdout and stderr → our stderr, awaited; `--workspace` only when
  project_dir is the workspace root; `--config` only when given on the CLI.
- **exit codes** (`exit_code.rs`): 0, 2 (missed) and 3 (timeouts) are finished runs, `engine_gate_failed =
  false` (cargo-mutants has no thresholds); 0 without `outcomes.json` = no mutant in the diff → `report: None`;
  4 (baseline tests fail) → `Process` naming `mutants.out/log/baseline.log`; anything else → `Process`.
- **outcomes.json:** mutant scenarios only (the baseline is skipped). `CaughtMutant` → Killed, `MissedMutant` →
  Survived, `Unviable` → CompileError, `Timeout` → Timeout; any other summary → `Parse`. id = the mutant's
  `name`, mutator = `genre`, positions = `span` (1-based), file re-based to project_dir.
- **list_mutants:** the same diff and args with `--list --json`; empty stdout = no mutants.
- `coverage`, `cache_key` → `Unsupported`.

## rpr (CLI)

```
rpr run (--diff <base> | --all) [--json] [--min-score <N>] [--config <path>] [--engine <stryker|cargo-mutants>] [--project <dir>]
rpr telemetry [status|enable|disable]
```

- `--project` defaults to cwd and is canonicalized once (`dunce::canonicalize`: real path, no Windows `\\?\`
  prefix). `--diff` values that start with `-` are rejected. Engine: `--engine`, else the engine whose own
  config file name `--config` has, else detection; more than one engine detected → error asking for
  `--engine`. Then detect the config (unless `--config`), compute ranges,
  create run dir, collect git facts, run one shard, store `report.json` and `run.json`, print.
- `--all` (exactly one of `--diff` and `--all` is required): the ranges are every file `git ls-files -z` lists
  under the project dir, each `1..=line_count` from the work tree (binary files, by git's own test of a NUL in
  the first 8000 bytes, empty files and tracked files missing from the work tree are skipped). The adapters
  get a `MutationScope` like any other and intersect it with their engine config unchanged: Stryker with the
  config's `mutate` globs (`plan.mjs`), cargo-mutants through the synthesized `--in-diff`, which then selects
  the same mutants as a plain `cargo mutants` (checked with `--list`). Git facts use `HEAD` as the base, so
  `merge_base_sha` is HEAD and `commits` is empty; `run.json` has `scope: "all"` (else `"diff"`).
- Human output (stdout):
  ```
  rpr · stryker · diff vs origin/main · 12 ranges in 4 files
  score 87.50%  (14 killed, 0 timeout, 2 survived, 0 no coverage)  1m 23s
  survivors:
    packages/lib/src/x.ts:42:7  ConditionalExpression  → false
  run: /home/me/repo/.git/reapr/runs/1793640000-4242
  ```
  Empty diff scope: `nothing to mutate in diff vs <base>`. With `--all`, `all files` replaces `diff vs <base>`
  in both lines. Warnings go to stderr.
- `--json` (stdout, nothing else on stdout):
  `{ run_id, run_dir, engine, base, ranges, score, counts, survivors: [{file, line, column, mutator, replacement,
  status}], duration_ms, min_score, passed, engine_gate_failed, warnings, resources }`. `score` is `null` when
  nothing was mutated; `base` is `null` with `--all`. `run_dir` is absolute. The git commits list is only in `run.json`. On exit 2 (also for
  argument errors caught by clap), stdout holds `{"error": "<message>"}` instead; `--help` and `--version`
  print normally and exit 0.
- Exit codes: 0 pass; 1 score < `--min-score` or `engine_gate_failed`; 2 any error. No valid mutants → pass.

## Telemetry

Anonymous, opt-out usage telemetry in `rpr` (`crates/rpr/src/telemetry.rs`); the user-facing contract, with
every property, is [TELEMETRY.md](../../TELEMETRY.md).

- One PostHog event, `run_completed`, per `rpr run`, also on errors after argument parsing. Built and sent after
  the output is printed: nothing runs while the engine does, stdout is untouched, the notice and the debug
  payload go to stderr.
- Transport: spawn `curl -sS -m 1 -X POST -H 'Content-Type: application/json' --data-binary @- <capture URL>`,
  write the JSON to its stdin, `tokio::time::timeout` 1.5 s on the child, then kill. No HTTP client dependency;
  no curl → nothing sent; every failure is silent.
- `flags` lists the optional flags by name (`all` included); `scope_mode` is `diff` or `all`.
- Properties are numbers, bools, fixed enums and buckets only: no free text. Errors map to an `error_kind`
  enum by downcasting the `anyhow` chain (`CliError`, `AdapterError`, `DiffError`, `io::Error`) in one
  function; warnings map to a `warning_kinds` enum by their adapter text; versions pass only if they are
  `[0-9A-Za-z.+-]`. A unit test serializes events built from runs full of paths, emails and shas and rejects
  any string with `/`, `\`, `@` or a 40-hex sha.
- Opt-out precedence: `DO_NOT_TRACK` (non-empty, not `0`) > `RPR_TELEMETRY` (`0/false/off/no` vs
  `1/true/on/yes`) > the saved setting (`rpr telemetry enable|disable`) > on.
- State: `${XDG_CONFIG_HOME:-$HOME/.config}/reapr/telemetry.json` = `install_id` (UUID v4 from `/dev/urandom`),
  `enabled` (`null` until set), `notice_shown`. Unreadable or unwritable → defaults, never an error. In CI
  (a well-known CI variable is set) nothing is persisted: a fresh id and the notice on every run.
- `POSTHOG_PROJECT_KEY` is one constant; while it is empty telemetry is a no-op (no notice, no state, no send),
  except that `RPR_TELEMETRY_DEBUG=1` still prints the payload. Tests pass a key to the internal `prepare`
  function; there is no runtime override.

## Distribution

- Phase 1 supports Linux only (tested on Ubuntu); macOS and Windows are planned. `dist` (cargo-dist 0.34) for
  x86_64/aarch64 Linux (gnu); shell installer; GitHub CI on Ubuntu; repo `reapr-corp/reapr`.
- `reapr-corp/run` (separate repo, `../run/action.yml`): composite action. Inputs `base` (default
  `${{ github.base_ref }}`), `min-score`, `config`, `working-directory`, `version` (default `latest`). Installs
  through the dist installer from the release, fetches the base ref, runs `rpr run --diff origin/<base> --json`,
  renders `$GITHUB_STEP_SUMMARY` from the JSON with `jq`, and exits with rpr's exit code.

## Adapter conformance kit

`reapr_core::testkit` (feature `testkit`, dev-dependencies only): `empty_scope_reports_none`, `status_mapping`,
`assert_parity`, `unsupported`. Every adapter's tests call them; see CONTRIBUTING.md.

## Out of Phase 1

MCP server (`rpr mcp`), incremental cache, sharding across machines, generated configs.
