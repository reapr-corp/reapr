---
cargo/rpr: patch (Fixed)
---

A `--diff` base that starts with `-` is rejected, and git calls use `--end-of-options`, so a base is never
read as a git option.
