---
cargo/rpr: minor (Added)
---

Diff scope from `git diff --unified=0 <base>...HEAD`: changed line ranges, merged per file, relative to the
project directory. Deleted files, pure deletions, binary files and mode-only changes contribute nothing;
renames and quoted paths are handled.
