---
cargo/rpr: patch (Fixed)
---

The scope filter compares (line, column) as Stryker does: carried-over mutants that start before a range's
first column are dropped instead of counted or failing the parse.
