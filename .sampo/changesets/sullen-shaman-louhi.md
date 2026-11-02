---
cargo/rpr: minor (Added)
---

cargo-mutants support: `rpr run` mutates the diff of a Rust project with cargo-mutants (26.2 to 27.x) and its own `.cargo/mutants.toml`. A new `--engine <stryker|cargo-mutants>` flag picks the engine when several are detected; `--config` picks it from the file name (`mutants.toml`, `stryker.config.*`).
