# Changelog

All notable changes to Reapr are documented in this file. `reapr-core`, `reapr-stryker` and `rpr` share one
version.

Sections follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Entries come from the changesets in
[`.sampo/changesets/`](../../.sampo/changesets); see [CONTRIBUTING.md](../../CONTRIBUTING.md#changesets).

## 0.4.0 — 2026-11-02

### Added

- rpr run --all audits a whole project: every git-tracked file under the project dir is the scope, narrowed by each engine's own config as a diff is. JSON `base` is null, run.json records `scope`, telemetry adds the `all` flag and `scope_mode`.
- Telemetry: `invoked_by` also accepts `setup`, the onboarding skill.

## 0.3.0 — 2026-11-02

### Added

- Telemetry v2: the `run_completed` event adds `trigger`, `via_action`, `agent`, `first_run`, `build`, `project_id` (a salted hash of the repository's root commits), and `invoked_by` and `session_id` (set by the mutation-audit skill to measure the score change per session). See TELEMETRY.md.
- cargo-mutants support: `rpr run` mutates the diff of a Rust project with cargo-mutants (26.2 to 27.x) and its own `.cargo/mutants.toml`. A new `--engine <stryker|cargo-mutants>` flag picks the engine when several are detected; `--config` picks it from the file name (`mutants.toml`, `stryker.config.*`).

## 0.2.0 — 2026-11-02

### Added

- Telemetry is live: events go to the Reapr PostHog project (US cloud). See TELEMETRY.md to see what is sent or turn it off.
- Anonymous, opt-out usage telemetry: one event per `rpr run` (outcome, counts, durations, bucketed diff size, platform; never code, paths or names), a one-time notice, `rpr telemetry [status|enable|disable]`, `DO_NOT_TRACK` / `RPR_TELEMETRY`, and `RPR_TELEMETRY_DEBUG=1` to print the payload. See TELEMETRY.md.

## 0.1.0 — 2026-11-02

### Added

- `MutationEngineAdapter` trait and shared types in `reapr-core`; score per the mutation-testing-metrics
  definition.
- Human summary and `--json` output with survivors (file, line, column, mutator, replacement, status).
- Licensing: FSL-1.1-ALv2 for `reapr-core` and `rpr`, MIT for `reapr-stryker`.
- StrykerJS adapter (`reapr-stryker`):
  - Config detection in Stryker's own lookup order; an error with a `npx stryker init` hint when there is no
    config file.
  - Plan step (`plan.mjs`, Node 20.10 or later) that keeps the changed ranges selected by the config's own
    `mutate` patterns, or Stryker's installed default.
  - Wrapper config that overrides only `mutate`, `reporters` (adds `json`) and `jsonReporter.fileName`.
  - Report parsing (mutation-testing-report-schema v2), filtered to the diff scope with Stryker's containment
    rule; results carried over by `incremental` are excluded with a warning.
  - Warning for Vitest 4 or later with bail enabled. The config is never changed.
  - CPU time and peak RSS of the Stryker process tree on Unix.
- Phase 1: a local CLI, StrykerJS only. See the
  [Phase 1 design](https://github.com/reapr-corp/reapr/blob/main/docs/design/phase-1.md).
- Release configuration with `dist`: binaries for x86_64 and aarch64 Linux (glibc) only, and a shell installer.
  macOS and Windows are not supported yet.
- Diff scope from `git diff --unified=0 <base>...HEAD`: changed line ranges, merged per file, relative to the
  project directory. Deleted files, pure deletions, binary files and mode-only changes contribute nothing;
  renames and quoted paths are handled.
- Exit codes: 0 pass, 1 below `--min-score` or the engine's own threshold failed, 2 error.
- Run store in `<git dir>/reapr/runs/<run_id>/` with `report.json` and `run.json` (git facts including commit
  trailers, engine version, timing, resources, environment, result). Local only.
- `rpr run --diff <base>` with `--json`, `--min-score`, `--config` and `--project`.

### Changed

- Renamed the CLI to `rpr`: the binary and its package (`crates/rpr`, formerly `reapr-cli`). The installer is
  `rpr-installer.sh`. The product, the crates `reapr-core` and `reapr-stryker`, and the
  run store in `<git dir>/reapr/runs/` keep the Reapr name.

### Fixed

- The scope filter compares (line, column) as Stryker does: carried-over mutants that start before a range's
  first column are dropped instead of counted or failing the parse.
- The `mutate` patterns are matched with the `minimatch` the installed Stryker uses (case-sensitive on every
  OS, as Stryker); this also lowers the Node requirement from 22.5 to 20.10.
- Score arithmetic: 29 of 50 is exactly 58.0, so `--min-score 58` passes.
- A `--diff` base that starts with `-` is rejected, and git calls use `--end-of-options`, so a base is never
  read as a git option.
- `diff.interHunkContext` and textconv in the user's git config no longer change the diff scope
  (`--inter-hunk-context=0 --no-textconv`).
- `engine_gate_failed` only when Stryker exits 1 and the config has a numeric `thresholds.break`; any other
  non-zero exit is an error (exit 2) that names the report path.
- A `--project` given through `..` or a symlink no longer puts every mutant out of scope (which scored `null`
  and passed): the project dir is canonicalized, and report keys are resolved against the report's
  `projectRoot`.
- With `--json`, argument errors also print a `{"error": ...}` document on stdout (exit 2).

