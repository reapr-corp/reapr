---
cargo/rpr: minor (Added)
---

Telemetry v2: the `run_completed` event adds `trigger`, `via_action`, `agent`, `first_run`, `build`, `project_id` (a salted hash of the repository's root commits), and `invoked_by` and `session_id` (set by the mutation-audit skill to measure the score change per session). See TELEMETRY.md.
