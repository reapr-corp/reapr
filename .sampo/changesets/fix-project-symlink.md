---
cargo/rpr: patch (Fixed)
---

A `--project` given through `..` or a symlink no longer puts every mutant out of scope (which scored `null`
and passed): the project dir is canonicalized, and report keys are resolved against the report's
`projectRoot`.
