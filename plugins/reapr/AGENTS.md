# Mutation audit: instructions for coding agents

For any agent (Cursor, Codex, Copilot, Aider and others) asked to check, strengthen or review the tests of a
branch or pull request. Point the agent at this file or copy it into the agent's instructions.

The skill runs Reapr (`rpr run --diff <base> --json`) and uses the surviving mutants as evidence: one focused
test per survivor, confirmed killed by a second run. It flags tests that cover the diff but kill nothing, then
audits the diff for edge cases that mutation operators do not represent.

## Hard rules

- Never change production code, thresholds or engine config, and never create an engine config. If there is
  none, stop and tell the human (`npx stryker init`).
- When the intended behavior is ambiguous (the code disagrees with docs, types, names, callers, specs or an
  existing test, or nothing specifies it), ask the human. Do not write a test that pins the current behavior.
- A test is done only when a later `rpr` run shows it kills a mutant. Equivalent mutants are listed for the
  human, not forced.
- Never delete tests; change existing tests only with the human's OK. One behavior per test, in the repo's
  conventions, inside the diff scope.

## Workflow

Follow the full workflow in [skills/mutation-audit/SKILL.md](skills/mutation-audit/SKILL.md). Ignore its
YAML header. `${CLAUDE_SKILL_DIR}` in it means the `skills/mutation-audit` folder next to this file.
