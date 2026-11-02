---
cargo/rpr: minor (Added)
---

StrykerJS adapter (`reapr-stryker`):
- Config detection in Stryker's own lookup order; an error with a `npx stryker init` hint when there is no
  config file.
- Plan step (`plan.mjs`, Node 20.10 or later) that keeps the changed ranges selected by the config's own
  `mutate` patterns, or Stryker's installed default.
- Wrapper config that overrides only `mutate`, `reporters` (adds `json`) and `jsonReporter.fileName`.
- Report parsing (mutation-testing-report-schema v2), filtered to the diff scope with Stryker's containment
  rule; results carried over by `incremental` are excluded with a warning.
- Warning for Vitest 4 or later with bail enabled. The config is never changed.
- CPU time and peak RSS of the Stryker process tree on Unix.
