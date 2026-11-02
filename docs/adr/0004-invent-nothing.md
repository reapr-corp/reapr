---
status: accepted
---

# Reapr invents nothing

Reapr runs each project's own engine with the project's own configuration and adds only what the product is:
the diff scope, a report location Reapr owns, and a machine-readable report. It forces no flags, generates no
configs, and has no opinion on thresholds, bail, coverage analysis, timeouts or concurrency. The reason is
trust: Reapr's verdicts must match the engine's own run, mutant by mutant, and that is only true if the engine
runs as the project runs it. Every override is a place where the two could disagree.

## Consequences

- Adapters pass the scope and the output path and nothing else. StrykerJS gets a wrapper that sets only
  `mutate` and a json reporter; cargo-mutants gets only `--in-diff`, `--output` and, at the workspace root,
  `--workspace` (so the diff, not cargo's default members, decides what is mutated).
- A missing config is an error with a hint, never a default (StrykerJS: `npx stryker init`). An engine that
  works without a config file (cargo-mutants) runs with its own defaults, which are the engine's, not Reapr's.
- A risky setting in the user's config gets a warning, never a change (for example, vitest bail with
  StrykerJS).
- Reapr does not add its own gate on top of the engine's: an engine threshold that fails sets
  `engine_gate_failed`, and Reapr's `--min-score` is opt-in. An engine without thresholds (cargo-mutants)
  never fails its own gate: missed mutants are results, not errors.
- Acceptance for a new adapter is verdict parity with the engine's own run on the same ranges
  (CONTRIBUTING.md), checked with the conformance kit's `assert_parity`.
