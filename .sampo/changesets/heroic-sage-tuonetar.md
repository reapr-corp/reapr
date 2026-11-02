---
cargo/rpr: minor (Added)
---

rpr run --all audits a whole project: every git-tracked file under the project dir is the scope, narrowed by each engine's own config as a diff is. JSON `base` is null, run.json records `scope`, telemetry adds the `all` flag and `scope_mode`.
