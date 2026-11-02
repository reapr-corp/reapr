---
cargo/rpr: minor (Added)
---

Anonymous, opt-out usage telemetry: one event per `rpr run` (outcome, counts, durations, bucketed diff size, platform; never code, paths or names), a one-time notice, `rpr telemetry [status|enable|disable]`, `DO_NOT_TRACK` / `RPR_TELEMETRY`, and `RPR_TELEMETRY_DEBUG=1` to print the payload. See TELEMETRY.md.
