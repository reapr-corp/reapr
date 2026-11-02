# Mutation audit: instructions for coding agents

Use these instructions when asked to check, strengthen or review the tests of a branch or pull request. They
are the same workflow as the Claude Code skill in
[skills/mutation-audit/SKILL.md](skills/mutation-audit/SKILL.md), written for any agent (Cursor, Codex,
Copilot, Aider and others). Copy this file into your agent's instructions, or point the agent at it.

Mutation testing is the objective evidence: the engine changes the code on purpose (a mutant) and checks that
a test fails (the mutant is killed). A surviving mutant is a change in behavior no test catches. Reapr (`rpr`)
runs the project's own engine on the lines the branch changed. Your reading of the code adds a semantic audit
on top. Keep the two apart in the report.

## Rules

- Never change production code to kill a mutant. If a test shows the code is wrong, report it.
- Never edit, create or loosen engine config, thresholds or Reapr flags. Each repo owns its config.
- Never delete or weaken tests. Change existing tests only with the human's OK.
- One behavior per test. Follow the repo's test conventions (location, naming, helpers, assertion style).
- Stay inside the diff scope unless the human asks for more.
- Assert intended behavior, not current behavior. When they may differ, ask.
- No `// Stryker disable` comments without the human's approval.

## 0. Preconditions

When available, run `rpr doctor` first. Until then:

1. `rpr --version` must work. If not, show the human the installer and run it only after they agree:
   `curl -LsSf https://github.com/reapr-corp/reapr/releases/latest/download/rpr-installer.sh | sh`
   (Linux x86_64 and aarch64 only for now).
2. An engine config must exist (StrykerJS: `stryker.config.*`, `stryker.conf.*`, a leading-dot variant, or a
   named config passed with `--config`). If there is none, stop and tell the human to run `npx stryker init`.
   Do not write a config yourself.
3. Reapr mutates committed changes only (`<base>...HEAD`). Tell the human if source files in scope have
   uncommitted changes. New tests you write need not be committed; the engine runs the work tree's tests.
4. Monorepos with several configs: list changed files (`git diff --name-only <base>...HEAD`), find the
   configs whose package contains them, and run each with `--project <dir> --config <path>`.

## 1. Evidence

Run `rpr run --diff <base> --json` (base default `origin/main`; ask if unclear). stdout is one JSON document;
note `score`, `counts`, `survivors` and `run_dir`. Exit code 2 is an error (`{"error": "..."}`): report it.
Field meanings and run directory files: [skills/mutation-audit/references/reapr-output.md](skills/mutation-audit/references/reapr-output.md).

## 2. Survivors to tests

For each survivor (`file`, `line`, `column`, `mutator`, `replacement`):

1. Read the code and the mutant. State in one sentence the behavior it changes.
2. Find what specifies the intended behavior: docs, comments, types, names, callers, specs, README.
3. Find the existing tests for that unit and follow their conventions.
4. Write ONE focused test asserting the intended behavior. Run the project's own test command for that file.
5. Re-run `rpr run --diff <base> --json` and confirm the mutant is now Killed (match by file, line, column,
   mutator, replacement). You may batch several tests per re-run. A test that kills nothing is not done.

Equivalent mutants (no observable behavior change): do not force a test. Explain why and list them for the
human.

Unclear intent: if the code disagrees with its docs, types, names, callers or specs, or nothing specifies the
behavior, ask the human and leave the survivor open. A test written from what the code does turns a bug into
expected behavior; Trail of Bits makes this point in
[Mutation testing for the agentic era](https://blog.trailofbits.com/2026/04/01/mutation-testing-for-the-agentic-era/).

## 3. Weak tests

Run `node skills/mutation-audit/scripts/test-kills.mjs <run_dir>` (path relative to this file) on the first
run. It lists tests that cover in-scope mutants but kill none. With bail on (Stryker's default) only the
first failing test is recorded, so confirm each by reading it. Also look for assertions that cannot fail: no
assertions, `toBeDefined()` on values that always exist, snapshots of mocks, tautologies. Report each with
evidence and a proposed fix; change it only with the human's OK.

## 4. Semantic audit

Within the diff scope, check the edge cases mutation operators do not represent:
[skills/mutation-audit/references/edge-cases.md](skills/mutation-audit/references/edge-cases.md) (boundaries,
empty/null/undefined, error and timeout paths, ordering and concurrency, idempotency, time zones, locale and
calendar edges, numeric precision and overflow, input validation at trust boundaries). Propose each with the
behavior it protects and why. Implement only those whose expected behavior is specified; ask about the rest.

## 5. Report

- Score before → after, both from `rpr` runs (give the run ids).
- Table: mutant (file:line:col, mutator → replacement) | behavior it changes | test | killed?
- Equivalent mutants, with reasons.
- Open questions where intent is unclear, with both sides quoted.
- Weak tests, with evidence and proposed fixes.
- Edge cases added (and what specifies them) and proposed.
- Findings: tests showing the code disagrees with its spec. Production code was not changed.
