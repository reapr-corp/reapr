---
status: accepted
---

# Each adapter declares the engine versions it supports, and warns outside them

Engines change their report formats, flags and exit codes between versions, and a misread report gives a
wrong score with no error. So each adapter declares the range of engine versions it is verified against in
one constant (cargo-mutants: `SUPPORTED_VERSIONS = >=26.2.0, <28.0.0` in
`crates/reapr-cargo-mutants/src/lib.rs`), reads the installed version before running, and outside that range
runs anyway with a warning in `RunOutcome.warnings`, never an error: blocking a user on a newer engine that
still works is worse than a clear warning. A version enters the range only with real fixtures captured from
that version (`tests/fixtures/<version>/` in the adapter crate), parsed by the adapter's tests. CI runs the
engine end to end with one pinned version inside the range; a matrix over the range is added when a second
version needs a code path of its own. The engine version is recorded in `run.json` and telemetry, so a
problem can be tied to a version.

## Consequences

- Raising the upper bound is a pull request with new fixtures from the new version, not a constant edit.
- The pinned CI version (`.github/workflows/ci.yml`) must stay inside the declared range.
- The StrykerJS adapter predates this decision and declares no range yet.
