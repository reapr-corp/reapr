# Contributing to Reapr

Thanks for helping. Read [docs/design/phase-1.md](docs/design/phase-1.md) first: it is the agreed design, and
changes are reviewed against it.

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
(`crates/reapr-core/src/lib.rs`). Use `crates/reapr-stryker` as the reference.

### The trait

| Method | Phase 1 | What it does |
|---|---|---|
| `id` | Required | Short engine id, for example `stryker`. Appears in output and `run.json`. |
| `detect` | Required | Looks at the project directory. Returns `Some(Detection)` with the config path, or `config: None` when the engine is used but has no config file. |
| `run` | Required | Runs one `Shard` with the user's own config and returns a `RunOutcome`: the report (or `None` when nothing in scope), whether the engine's own gate failed, warnings, and resources. |
| `list_mutants` | May return `Unsupported` | Lists mutants in scope without running them. |
| `coverage` | May return `Unsupported` | Maps mutants to the tests that cover them. |
| `cache_key` | May return `Unsupported` | Content key for one mutant, for a future cache. |
| `merge` | Default provided | Combines shard reports with `merge_reports`. Override only if the engine needs it. |

The trait is not dyn-compatible on purpose. To wire a new adapter into the CLI, add a variant to `enum Engine`
in `crates/rpr/src/main.rs`, add it to `Engine::all()` (detection order), and dispatch each method. Also
provide the engine version for `run.json` (the StrykerJS adapter has an `engine_version` method for this).

### Where engine knowledge lives

In the adapter, and only there. The core knows no engine. What a test file is, config file names and formats,
report formats, CLI flags and the engine's scoping rules all belong to the adapter crate.

If the engine writes [mutation-testing-report-schema](https://github.com/stryker-mutator/mutation-testing-elements/tree/master/packages/report-schema)
JSON, reuse the parser in `crates/reapr-stryker/src/schema.rs`. It is engine-agnostic in shape and moves to
`reapr-core` when a second engine needs it; propose that move in the same pull request.

### Rules for adapters

- Run the user's own config unchanged, except for the diff scope and a report path Reapr owns. If the engine has
  no config file, return `AdapterError::ConfigMissing` with a hint for creating one.
- Read the report from a private path in `RunContext::run_dir`, so a stale report can never be read.
- Filter the report to the diff scope with the engine's own containment rule, and count what you drop in a
  warning.
- Measure the engine's process tree resources around the child process, as the StrykerJS adapter does, or
  return `None`.
- Adapters are MIT licensed. Add a `LICENSE` file (MIT) to the crate and set `license = "MIT"` in its
  `Cargo.toml`.

### Acceptance bar: verdict parity

An adapter is accepted when its verdicts match the engine's own run, mutant by mutant. Run the engine directly
on the same ranges (for StrykerJS, `npx stryker run --mutate <ranges>`), run Reapr on the same diff, and compare
every in-scope mutant's status. Include the comparison in the pull request: the project, engine version, the
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

- **Package:** always `cargo/rpr`. Its changelog is Reapr's, and `reapr-core`, `reapr-stryker` and `rpr` share one
  version (a fixed group in `.sampo/config.toml`), so a change in an adapter is still a `cargo/rpr` changeset.
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
  `docs: ...`, `ci: ...`. Scopes: `core`, `cli`, `stryker`, `ci`, `docs`.
- Each commit message describes the change and why.
- One topic per pull request. Fill in the checklist in the template.

## License of contributions

Contributions are licensed under the license of the crate they change: FSL-1.1-ALv2 for `reapr-core` and
`rpr`, MIT for engine adapters.

## Conduct and security

See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). Report vulnerabilities privately as described in
[SECURITY.md](SECURITY.md), not in a public issue.
