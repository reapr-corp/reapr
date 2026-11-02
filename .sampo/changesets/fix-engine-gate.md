---
cargo/rpr: patch (Fixed)
---

`engine_gate_failed` only when Stryker exits 1 and the config has a numeric `thresholds.break`; any other
non-zero exit is an error (exit 2) that names the report path.
