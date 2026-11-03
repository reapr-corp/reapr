# Reapr: instructions for coding agents

For any agent (Cursor, Codex, Copilot, Aider and others) asked to check, strengthen or review the tests of a
branch or pull request. Point the agent at this file or copy it into the agent's instructions.

The skill runs Reapr (`rpr run --diff <base> --json`, or `rpr run --all --json` for a whole-codebase audit) and
uses the surviving mutants as evidence: one focused test per survivor, confirmed killed by a second run. It flags
tests that cover the scope but kill nothing, then audits the code for edge cases that mutation operators do not
represent.

## Hard rules

- Never change production code, thresholds or engine config, and never create an engine config. If rpr, the
  engine or a StrykerJS config is missing, follow Setup below (`npx stryker init` is the human's to run).
  Rust projects use `--engine cargo-mutants`, whose config is optional; it has no thresholds, so only
  `--min-score` (the human's choice) gates it.
- When the intended behavior is ambiguous (the code disagrees with docs, types, names, callers, specs or an
  existing test, or nothing specifies it), ask the human. Do not write a test that pins the current behavior.
- A test is done only when a later `rpr` run (the same `--diff <base>` or `--all` command as the first) shows
  it kills a mutant. Engine-only checks (`cargo mutants -f <file>`) are for iterating, not for the report.
  Equivalent mutants are listed for the human with the input domain that makes them equivalent, not forced.
- Never delete tests; change existing tests only with the human's OK. One behavior per test, in the repo's
  conventions, inside the diff scope. Rust tests go in the mutated package; commit them before re-running
  `rpr --diff`, or the scope shifts.
- Do not change process-wide state (env vars, cwd, PATH) in tests; use a child process or a project-local fake.

- Generate one UUID v4 per session (`uuidgen` or `node -e 'console.log(crypto.randomUUID())'`) and prefix
  every `rpr` command with `RPR_INVOKED_BY=mutation-audit RPR_SESSION_ID=<uuid>`. It only tags Reapr's
  anonymous telemetry and changes nothing when `DO_NOT_TRACK` is set.

## Setup (first use)

Ask before every step that changes anything. 1. `rpr --version`, else offer the installer
(`curl -LsSf https://github.com/reapr-corp/reapr/releases/latest/download/rpr-installer.sh | sh`). 2. Detect
StrykerJS (package.json with vitest/jest, `stryker.*config*` files, per package in monorepos) or cargo-mutants
(`Cargo.toml`); both → `--engine`. 3. cargo-mutants installed (`cargo install --locked cargo-mutants`); for
StrykerJS a config must exist: never write one, ask the human to run `npx stryker init`, and stop until it
exists. 4. Baseline: `RPR_INVOKED_BY=setup RPR_SESSION_ID=<uuid> rpr run --all --json` per config; report score,
survivors per file and time. 5. Optionally, with consent, a `reapr-corp/run@v1` pull-request job and an agent
rule. Full steps: [skills/setup/SKILL.md](skills/setup/SKILL.md).

## Workflow

Follow the full workflow in [skills/mutation-audit/SKILL.md](skills/mutation-audit/SKILL.md). Ignore its YAML
header; where it says `/reapr:setup`, use the Setup section above. `${CLAUDE_SKILL_DIR}` in it means the
`skills/mutation-audit` folder next to this file.
