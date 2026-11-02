# Accepted equivalent mutants

Mutants that survive `cargo mutants --workspace` on purpose: no input the code can receive tells them apart from
the original, so no test can kill them. Each was reviewed and accepted without a production change. Positions
are `file:line:column` as cargo-mutants reports them (cargo-mutants 27.1.0).

## `crates/reapr-core/src/diff.rs`, `parse_unified_zero`

Input domain: the output of `git diff --unified=0` (the only producer, `changed_ranges`). In a `-U0` hunk git
lists every removed line before every added line, the counts in the `@@` header match the lines that follow, and
`\ No newline at end of file` only follows the last line of a file.

- `34:21` `>` → `>=` in `old_left > 0`: only changes whether the hunk block runs; outside a hunk the catch-all
  arm resets both counters and the line is parsed as usual, and a skipped `\` line is not a header anyway.
- `34:21` `>` → `<` in `old_left > 0`: `u32` is never below 0, so the block runs only on `new_left`; a pure
  deletion then parses its `-` lines at the top level, and git's `-` content lines are never headers there.
- `34:21` `>` → `==` in `old_left > 0`: the block also runs between hunks, where the catch-all arm resets the
  counters and the line falls through to the same header parsing.
- `34:37` `>` → `>=` in `new_left > 0`: as `34:21` `>=`, it only makes the block run outside a hunk, where the
  catch-all arm restores the original parse.
- `36:31` guard `old_left > 0` → `true`: differs only for a `-` line after the old side is used up, which git
  never emits (removed lines come first and match the count); on such input the mutant underflows a `u32`.
- `36:40` `>` → `>=` in the `-` guard: the same input as `36:31`, the same argument.
- `37:30` `-=` → `+=` in `old_left -= 1`: `old_left` never reaches 0, so the block stays active after the
  added lines; the only lines git emits next (`diff --git`, `@@`, `\`) are reset by the catch-all arm or ignored.
- `37:30` `-=` → `/=` in `old_left -= 1`: `old_left / 1` is `old_left`, so as with `+=` it never reaches 0
  and the same next lines follow.
- `40:31` guard `new_left > 0` → `true`: differs only for a `+` line while removed lines are still expected,
  which git never emits (removed lines come first); on such input the mutant underflows a `u32`.
- `40:40` `>` → `>=` in the `+` guard: the same input as `40:31`, the same argument.
- `44:17` delete the `Some(b'\\')` arm: the marker then ends the hunk early, but git writes it only after the
  last line of a file, so no later hunk of that file exists to move, and the next file sets its own path.
