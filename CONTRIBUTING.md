# Contributing to Reapr

Thanks for helping. Read [docs/design/phase-1.md](docs/design/phase-1.md) first: it is the agreed design, and
changes are reviewed against it.

## Issues and discussions

Start in [Discussions](https://github.com/reapr-corp/reapr/discussions); open an issue when there is
concrete work to do.

| You have | Where it goes | What happens next |
| --- | --- | --- |
| A question ("how do I...") | Discussions → Q&A | Answered there; the answer is marked |
| An idea or feature request | Discussions → Ideas | If accepted, a maintainer turns it into an issue labelled `accepted` |
| A reproducible bug | Issue, bug report form | Starts as `needs-triage` |
| A request for another engine | Issue, engine request form | Labelled `engine`; prioritised by demand (👍 on the issue) |
| A security problem | [Private advisory](https://github.com/reapr-corp/reapr/security/advisories/new) | Never in public |

Triage moves each new issue out of `needs-triage` to `confirmed`, `needs-repro`, `duplicate` or `wontfix`
(with the reason), and adds one area label: `area:core`, `area:stryker`, `area:cargo-mutants`, `area:cli`,
`area:action` or `area:skill`. Small, well-scoped issues get `good first issue`.

To resolve an issue: branch, open a pull request that says `Fixes #<number>`, add a changeset if it touches
`crates/`, get CI green, squash-merge. The next release ships it and closes the issue.

All issues live here, including those about the GitHub Action: reporters often cannot tell which part
failed, so triage decides. The bug form's "Component" field adds the area label automatically. When the
fix belongs in another repository (for example `reapr-corp/run`), the pull request there says
`Fixes reapr-corp/reapr#<number>`, which closes the issue here when it merges.

## Development setup

Develop on Linux (CI runs on Ubuntu). macOS and Windows are not supported yet.

- **Rust 1.97.** `rust-toolchain.toml` pins it, with `rustfmt` and `clippy`; rustup picks it up in this
  directory (run `rustup toolchain install` if it does not install by itself).
- **Node.js 20.10 or later** for the StrykerJS adapter tests. They run `plan.mjs` and the generated wrapper in
  Node. Without `node` on `PATH` these tests print `node not on PATH, skipping` and pass, so run them with Node
  before you open a pull request that touches `crates/reapr-stryker`. CI uses Node 24. The plan tests also
  need a real `minimatch` (the one Stryker uses) to copy into their fake project: they take it from the npm
  that ships with Node (`npm root -g`), and print `no minimatch to copy, skipping` without one.
- **git**, for the diff and run store tests.
- **cargo-mutants 27.1.0** (`cargo install --locked cargo-mutants@27.1.0`) for the cargo-mutants adapter's
  end-to-end test, which is `#[ignore]`d by default. Run it with
  `cargo test -p reapr-cargo-mutants --test e2e -- --ignored` before you open a pull request that touches
  `crates/reapr-cargo-mutants`; CI runs it in the `mutation` job.

## Checks

All four must pass; CI runs them on Ubuntu.

```sh
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

CI also runs `cargo deny check` (licenses, advisories, sources) with [deny.toml](deny.toml). A new dependency
with a license that is not in its `allow` list fails the build.

On pull requests, the `mutation` job dogfoods Reapr: it builds the pull request's `rpr` and runs
`rpr run --diff origin/<base> --engine cargo-mutants --min-score 80` on the repository. A change whose new code
the tests do not protect fails it; the survivors are listed in the job log.

## Code rules

- **No `unwrap` or `expect` outside tests.** Handle the error or return it.
- **Typed errors in libraries** (`thiserror`: `AdapterError`, `DiffError`). `anyhow` only in `rpr`.
- **Zero CPU while the engine runs.** Await the child process. No polling, no busy loops, no loops that parse
  engine output. Engine output goes straight to Reapr's stderr; stdout belongs to the report.
- **Invent nothing in the user's config.** Reapr adds the diff scope and a machine-readable report, nothing
  else: no forced flags, no generated configs, no opinions on thresholds, bail, coverage analysis or concurrency.
  If something in a config is risky, warn; never change it. No config means an error, not a default.
- **Measure before optimizing.** Nothing moves to Rust, and nothing gets a cache or a fast path, without
  profiling that shows the time is there. Each run's `run.json` has wall time and CPU of the engine process tree
  as a first signal.
- **Dependencies:** `serde`, `serde_json`, `thiserror`, `anyhow`, `clap`, `tokio` (and `libc` for resource
  usage). Add another only with a reason in the pull request.

## Adding an engine adapter

An adapter is a crate, `crates/reapr-<engine>`, that implements `MutationEngineAdapter` from `reapr-core`
(`crates/reapr-core/src/lib.rs`). Use `crates/reapr-cargo-mutants` (CLI engine, JSON report, native diff
filter) or `crates/reapr-stryker` (config wrapper, Node plan step) as the reference, whichever is closer.

Read the decisions first, in [docs/adr/](docs/adr/):

- [0001](docs/adr/0001-engine-adapter-trait.md): one async trait, not dyn-compatible, dispatched through
  `enum Engine` in the CLI.
- [0002](docs/adr/0002-engine-version-support.md): declare the engine versions you support in one constant,
  with real fixtures per version; warn outside the range.
- [0003](docs/adr/0003-out-of-process-adapters.md) (proposed): adapters written in the engine's own language
  speak JSON over stdio.
- [0004](docs/adr/0004-invent-nothing.md): run the user's own config; add only the scope and the report path.

The domain words (mutant, verdict, survived, scope, range, gate, ...) are defined in [GLOSSARY.md](GLOSSARY.md).

### The trait

| Method | Phase 1 | What it does |
|---|---|---|
| `id` | Required | Short engine id, for example `stryker` or `cargo-mutants`. Appears in output and `run.json`. |
| `detect` | Required | Looks at the project directory. Returns `Some(Detection)` with the config path, or `config: None` when the engine has no config file (or finds its own). |
| `run` | Required | Runs one `Shard` with the user's own config and returns a `RunOutcome`: the report (or `None` when nothing in scope), whether the engine's own gate failed, warnings, and resources. |
| `list_mutants` | May return `Unsupported` | Lists mutants in scope without running them. |
| `coverage` | May return `Unsupported` | Maps mutants to the tests that cover them. |
| `cache_key` | May return `Unsupported` | Content key for one mutant, for a future cache. |
| `merge` | Default provided | Combines shard reports with `merge_reports`. Override only if the engine needs it. |

The trait is not dyn-compatible on purpose (ADR 0001). To wire a new adapter into the CLI, in
`crates/rpr/src/main.rs`: add a variant to `enum Engine` and to `EngineId` (the `--engine` values), add it to
`Engine::all()` (detection order) and `Engine::for_config` (an `owns_config(file_name)` function on the adapter),
and dispatch each method. Also provide the engine version for `run.json` (an `engine_version` method on the
adapter), and add the engine id to [TELEMETRY.md](TELEMETRY.md). Add the crate to the fixed group in
`.sampo/config.toml`.

### Where engine knowledge lives

In the adapter, and only there. The core knows no engine. What a test file is, config file names and formats,
report formats, CLI flags, exit codes and the engine's scoping rules all belong to the adapter crate.

If the engine writes [mutation-testing-report-schema](https://github.com/stryker-mutator/mutation-testing-elements/tree/master/packages/report-schema)
JSON, reuse the parser in `crates/reapr-stryker/src/schema.rs`. It is engine-agnostic in shape and moves to
`reapr-core` when a second engine needs it; propose that move in the same pull request.

### Rules for adapters

- Run the user's own config unchanged, except for the diff scope and a report path Reapr owns (ADR 0004). If
  the engine needs a config file and there is none, return `AdapterError::ConfigMissing` with a hint for
  creating one.
- Read the report from a private path in `RunContext::run_dir`, so a stale report can never be read.
- Scope with the engine's own containment rule: either hand the engine the scope in its own terms (cargo-mutants
  `--in-diff`) or filter its report with its rule (StrykerJS), and count what you drop in a warning.
- Map every engine status to a `MutantStatus`; an unknown one is `AdapterError::Parse`, never a guess.
- A non-zero exit that only means "mutants survived" is a result, not an error. Set `engine_gate_failed` only
  for the engine's own configured threshold.
- Measure the engine's process tree resources around the child process with
  `reapr_core::resources::{children_usage, usage_since}`, or return `None`.
- Declare the supported engine versions and keep real fixtures for each (ADR 0002).
- Adapters are MIT licensed. Add a `LICENSE` file (MIT) to the crate and set `license = "MIT"` in its
  `Cargo.toml`.

### Conformance kit

`reapr-core` has a `testkit` feature with the checks every adapter's tests call. Enable it in the adapter's
`[dev-dependencies]` only (`reapr-core = { workspace = true, features = ["testkit"] }`):

| Check | What it asserts |
|---|---|
| `testkit::empty_scope_reports_none(&adapter, &ctx)` | An empty scope gives `report: None`, no resources and nothing in the run dir: the engine was not started. |
| `testkit::status_mapping(parse, &[("Killed", Killed), ...])` | Each engine status maps to its core status through the adapter's real parsing path, and an unknown status is a `Parse` error. |
| `testkit::assert_parity(&reapr_report, &native_report)` | Reapr's report and the engine's own report on the same ranges have the same mutants (by file, position, mutator, replacement), the same statuses and the same score. |
| `testkit::unsupported(&adapter, &ctx, &[Operation::Coverage, ...])` | Each listed operation returns `AdapterError::Unsupported` naming the adapter's engine id and the operation. |

### Acceptance bar: verdict parity

An adapter is accepted when its verdicts match the engine's own run, mutant by mutant. Run the engine directly
on the same ranges (for StrykerJS, `npx stryker run --mutate <ranges>`; for cargo-mutants,
`cargo mutants --in-diff <git diff>`), run Reapr on the same diff, and compare every in-scope mutant's status
with `testkit::assert_parity`. Where the engine can run in CI, make this an end-to-end test (the cargo-mutants
adapter's `tests/e2e.rs`). Include the comparison in the pull request: the project, engine version, the
commands, and the counts on both sides. Any difference must be explained, for example results the engine
carried over from outside the scope.

## Changes to the design

Changes to the `MutationEngineAdapter` trait, its types (`RunOutcome`, `Shard`, `AdapterError`, ...), the
`--json` fields, the exit codes or `run.json` need maintainer approval. Open an issue first, and update
`docs/design/phase-1.md` in the same pull request as the code.

## Changesets

Versions and the changelog ([crates/rpr/CHANGELOG.md](crates/rpr/CHANGELOG.md)) are written by
[Sampo](https://github.com/bruits/sampo/blob/main/crates/sampo/README.md) from changesets: small Markdown files
in `.sampo/changesets/`, one per user-facing change, in the same format as Changesets for npm. Install Sampo with
`cargo install sampo --locked` and add one in the same pull request as the change:

```sh
sampo add -p cargo/rpr -b patch -t Fixed -m 'A `--diff` base that starts with `-` is rejected.'
```

`sampo add` without flags asks for each value. Commit the file it creates.

- **Package:** always `cargo/rpr`. Its changelog is Reapr's, and every crate (`reapr-core`, the adapters and
  `rpr`) shares one version (a fixed group in `.sampo/config.toml`), so a change in an adapter is still a
  `cargo/rpr` changeset.
- **Bump:** `patch` for fixes, `minor` for features and, before 1.0, for breaking changes; `major` only after 1.0.
- **Tag:** the Keep a Changelog section: `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed` or `Security`.
- **Message:** what changed for a user of `rpr`, in the changelog's style. It is copied as is. Continuation lines
  and `- ` sub-items under the first line are kept.

Not needed for changes users do not see: tests, CI, refactors, docs. CI fails a pull request that changes
`crates/` without a changeset; a maintainer adds the `no-changeset` label for those.

### How releases happen

1. On every push to `main` with pending changesets, the Sampo workflow (`.github/workflows/sampo.yml`) opens or
   updates the release pull request: version bump in `Cargo.toml`, `Cargo.lock`, new changelog sections, and the
   changesets removed.
2. Merging that pull request is the release. The workflow tags the merge commit `rpr-v<version>` and starts the
   Release workflow (`.github/workflows/release.yml`) for that tag.
3. `dist` builds the binaries and installers and creates the GitHub Release, with the changelog section as notes.
   Nothing is published to crates.io.

Sampo writes the entries of a section in the order it reads the changeset files, which is not the order they
were added. To order them, edit `crates/rpr/CHANGELOG.md` in the release pull request just before merging it
(the next push to `main` rewrites the pull request).

## Commits and pull requests

- [Conventional Commits](https://www.conventionalcommits.org/): `feat(stryker): ...`, `fix(core): ...`,
  `docs: ...`, `ci: ...`. Scopes: `core`, `cli`, `stryker`, `cargo-mutants`, `plugins`, `ci`, `docs`.
- Each commit message describes the change and why.
- One topic per pull request. Fill in the checklist in the template.

## License of contributions

Contributions are licensed under the license of the crate they change: FSL-1.1-ALv2 for `reapr-core` and
`rpr`, MIT for engine adapters and agent skills (`plugins/`).

## Conduct and security

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). Report vulnerabilities privately as described in
[SECURITY.md](SECURITY.md), not in a public issue.
