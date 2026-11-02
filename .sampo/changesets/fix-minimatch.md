---
cargo/rpr: patch (Fixed)
---

The `mutate` patterns are matched with the `minimatch` the installed Stryker uses (case-sensitive on every
OS, as Stryker); this also lowers the Node requirement from 22.5 to 20.10.
