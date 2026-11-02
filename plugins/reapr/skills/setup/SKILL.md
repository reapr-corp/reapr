---
name: setup
description: First-time setup of Reapr (`rpr`) in a repository, end to end. Checks that rpr is installed, detects the stack (StrykerJS for JavaScript/TypeScript, cargo-mutants for Rust), checks the mutation engine and its config without ever writing one, runs a whole-project baseline with `rpr run --all`, and offers a GitHub Actions gate and an agent rule. Use it when the user wants to start with Reapr or mutation testing, set it up, install it, onboard a repo, or when the mutation-audit skill finds a missing prerequisite.
license: MIT
---

# Reapr setup

Take a first-time user from nothing to a measured baseline. **Ask before every action that changes anything**
(installing, writing a file, adding a CI job); reading and running `rpr` read-only checks need no consent. Say
what you found at each step in one or two lines.

Generate one session id for the whole setup, a UUID v4 (`uuidgen`, or `node -e
'console.log(crypto.randomUUID())'`), and run every `rpr` command with `RPR_INVOKED_BY=setup
RPR_SESSION_ID=<uuid>` in front. It only tags Reapr's anonymous telemetry and changes nothing when
`DO_NOT_TRACK` is set.

## Hard rules

- **Reapr invents nothing: never write, edit or "fix" an engine config** (`stryker.*config*`,
  `.cargo/mutants.toml`), not even a minimal one. Its test runner, `mutate` patterns and thresholds are the
  team's choices. Read it and warn; do not change it.
- Never change production code or tests during setup.
- Install nothing and write nothing without an explicit yes.

## 1. rpr

Run `rpr --version`. If it fails, show the installer and run it only after the user agrees:

```sh
curl -LsSf https://github.com/reapr-corp/reapr/releases/latest/download/rpr-installer.sh | sh
```

Then check `rpr --version` again (a new shell may be needed for PATH; the installer prints where it put
`rpr`). Reapr runs on Linux (x86_64, aarch64) only for now: on another OS, say so and stop.

## 2. Stack

Read, from the repo root:

- **JavaScript/TypeScript**: `package.json` listing `vitest` or `jest`, or a Stryker config: `stryker.conf.*`,
  `stryker.config.*` (or with a leading dot), and named configs such as `stryker.<name>.config.mjs`. In a
  monorepo, look in each package dir too (`git ls-files | grep -E '(^|/)\.?stryker\.'`); each config is one
  `rpr` run with `--project <package dir> --config <config path>`.
- **Rust**: `Cargo.toml`.
- Both in the same dir: every `rpr` command there needs `--engine stryker` or `--engine cargo-mutants`.

## 3. Engine and config

**Rust (cargo-mutants).** `cargo mutants --version` must work. If not, offer `cargo binstall cargo-mutants`
when `cargo binstall --version` works (prebuilt, fast), else `cargo install --locked cargo-mutants`. No config
is needed; if `.cargo/mutants.toml` exists, rpr and cargo-mutants respect it as it is.

**JavaScript/TypeScript (StrykerJS).**

1. `@stryker-mutator/core` must resolve from the package dir (`npm ls @stryker-mutator/core`, or
   `node_modules/@stryker-mutator/core`). If not, say so; the user's own `npx stryker init` installs it.
2. If there is **no Stryker config**, do not write one. Stryker's own wizard creates it, and it is
   interactive: tell the user to run it themselves, in Claude Code with `! npx stryker init` (other agents:
   in their terminal). **Stop here** until the config exists, then continue at step 3.
3. With a config, read it and warn (do not change it) about known pitfalls, linking
   <https://reapr.dev/guides/speed-up-stryker>:
   - `testRunner: 'vitest'` with vitest 4 or later and no `disableBail: true`: killed mutants can be
     reported as survived (rpr also warns).
   - A Turborepo, Nx or similar cache dir in the project (`.turbo`, `.nx`, `.next`, `dist`, `coverage`) not in
     `ignorePatterns`: Stryker copies it into every sandbox, which makes runs slow.
   - `coverageAnalysis` other than `perTest`: per-test kill data (used by the mutation-audit skill) is missing.

## 4. Baseline

Run, once per config (with `--project`/`--config`/`--engine` as found above):

```sh
RPR_INVOKED_BY=setup RPR_SESSION_ID=<uuid> rpr run --all --json
```

`--all` mutates every tracked file the engine config selects, so it can take a while on a large project; say
so before starting. Report from the JSON: `score`, `counts`, the survivors grouped per file (count per file,
most first), and the time (`duration_ms`). Exit code 2 is an error (`{"error": "..."}`): show it and help fix
the cause (missing engine, failing tests), never by changing the engine config.

## 5. Integration (optional, each with consent)

**GitHub Actions gate.** Offer a workflow, for example `.github/workflows/mutation.yml`, that runs on
`pull_request` and uses `reapr-corp/run@v1`: `actions/checkout` with `fetch-depth: 0`, then the project's own
setup (its node/pnpm version and install command, or its Rust toolchain plus cargo-mutants, for example
`taiki-e/install-action` with `tool: cargo-mutants`), then:

```yaml
      - uses: reapr-corp/run@v1
        with:
          min-score: <a little below the baseline>   # cargo-mutants only, see below
          # configs: |                                # monorepo: one config path per line
          #   packages/api/stryker.config.mjs
```

For cargo-mutants, which has no threshold of its own, propose `min-score` just below the baseline score.
For Stryker, leave `min-score` out: the config's own `thresholds.break` is the gate. The gate scores each PR's
diff, so a PR that changes only tests has nothing to mutate.

**Agent rule.** Offer to add to `AGENTS.md` or `CLAUDE.md` (whichever the repo uses):

> Before opening a PR, run /reapr:mutation-audit on your diff and write the missing test for each surviving
> mutant. Never change production code to kill a mutant, and never lower thresholds or edit the engine config.

## 6. Next step

End with the baseline and one line: "Run `/reapr:mutation-audit` to start killing the N survivors." (N from
the baseline; other agents: the workflow in `AGENTS.md`). For a whole-codebase audit, that skill uses
`rpr run --all`; for a branch, `rpr run --diff <base>`.
