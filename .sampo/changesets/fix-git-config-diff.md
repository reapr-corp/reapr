---
cargo/rpr: patch (Fixed)
---

`diff.interHunkContext` and textconv in the user's git config no longer change the diff scope
(`--inter-hunk-context=0 --no-textconv`).
