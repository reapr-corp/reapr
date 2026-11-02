# Fast iteration loops

A full `rpr run` re-tests every mutant in scope (a whole-workspace cargo-mutants run can take many minutes).
While you write tests for one file, check them with the engine's own narrowing. These checks are for you
only: the before and after scores in the report come from the same `rpr` command (`--diff <base>` or `--all`),
run after the last test exists.

## cargo-mutants

```sh
cargo mutants -f <file> [--re <function name regex>]          # one file, optionally one function
cargo mutants -f <file> --re <fn> --list                       # what it would test, without running
```

- Run it from the workspace root (add `--workspace` there, as `rpr` does) or from the package dir.
- It runs only the mutated package's tests, so a test in another crate kills nothing.
- Which test killed a mutant: `mutants.out/log/<mutant>.log` holds the `cargo test` output; the
  `test <name> ... FAILED` lines are the killing tests. In an `rpr` run the same logs are in
  `<run_dir>/mutants.out/log/`.
- It writes `mutants.out/` in the directory it runs from; do not commit it.

## StrykerJS

```sh
npx stryker run --mutate <file>:<start>-<end>                  # the form rpr passes Stryker
```

- Run it in the package dir, with the project's own config (the default lookup, or `<config path>` as the
  first argument). Never edit the config to speed it up.
- It writes the config's own reports (and, with `incremental: true`, its incremental file), which can carry
  results from outside your range. Read only the mutants in the range you passed, and do not commit report
  files.
- `rpr` itself narrows the same way: its `mutate` is the diff's `file:start-end` ranges intersected with the
  config's patterns, so a short `--diff` (or `--project <package>`) gives a faster run with the final numbers.

## Verdicts that change between runs

A mutant that is Killed in one run and Survived in the next, with no relevant change, is flaky: the code or
the test depends on random data, the clock, or load (a slow machine turns kills into timeouts, which still
count as detected but change the label). Report it as flaky and prefer a test that does not depend on
chance, for example checking an invariant over many generated values. Tracking issue:
[reapr-corp/reapr#24](https://github.com/reapr-corp/reapr/issues/24).
