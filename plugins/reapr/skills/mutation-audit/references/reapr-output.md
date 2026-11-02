# Reapr output and run directory

Read this when you need a field's exact meaning, the full mutant span, or per-test data.
The source of truth is the Reapr README (Output, Run record).

## `rpr run --json`

stdout holds one JSON document and nothing else. Engine output, warnings and notices go to stderr.

```json
{
  "run_id": "1793640000-4242",
  "run_dir": "/home/me/repo/.git/reapr/runs/1793640000-4242",
  "engine": "stryker",
  "base": "origin/main",
  "ranges": [{ "path": "src/price.ts", "start": 8, "end": 12 }],
  "score": 80.0,
  "counts": { "killed": 8, "survived": 2, "no_coverage": 0, "timeout": 0,
              "compile_error": 0, "runtime_error": 0, "ignored": 0, "pending": 0 },
  "survivors": [
    { "file": "src/price.ts", "line": 10, "column": 3, "mutator": "ConditionalExpression",
      "replacement": "false", "status": "Survived" }
  ],
  "duration_ms": 83400,
  "min_score": null,
  "passed": true,
  "engine_gate_failed": false,
  "warnings": [],
  "resources": { "cpu_user_ms": 250000, "cpu_sys_ms": 30000, "max_rss_kb": 812000 }
}
```

- `score`: detected / (detected + undetected) × 100, where detected = Killed + Timeout and undetected =
  Survived + NoCoverage. `null` when nothing was mutated or there were no valid mutants.
- `base`: the `--diff` value; `null` with `--all`.
- `ranges`: changed line ranges from the diff, before the config's `mutate` patterns apply (test files can
  appear here and still not be mutated). With `--all`: every tracked text file, lines `1` to its last line.
- `survivors`: Survived and NoCoverage mutants in the diff scope, sorted by file, line, column. `line` and
  `column` are the 1-based start position.
- `warnings`: read them. For example, Vitest 4+ with bail on can report killed mutants as survived; Reapr
  warns and does not change the setting. Pass that warning on to the human; do not edit the config.
- `mutator` and `replacement` depend on the engine. StrykerJS: mutator names such as `EqualityOperator`
  (table below) and the replacement code. cargo-mutants: the genre (`BinaryOperator`, `FnValue`,
  `MatchArmGuard`, `MatchArm`, `UnaryOperator`) and only the new token (`>=`, `true`, `()`), without the
  original; the full description (`replace > with >= in parse_unified_zero`) is the mutant `id` in
  `report.json`.
- Exit codes: 0 passed (also nothing to mutate), 1 below `--min-score` or the engine's own threshold
  failed, 2 error (stdout is `{"error": "<message>"}`).

Flags: `--diff <BASE>` or `--all` (exactly one), `--json`, `--min-score <N>`, `--config <PATH>`, `--project <DIR>`.
Do not add `--min-score` unless the human asks; it does not change which mutants survive.

## Run directory: `<git dir>/reapr/runs/<run_id>/`

Outside the work tree, never committed. Each worktree has its own git dir: in a linked worktree that is
`<main repo>/.git/worktrees/<name>/reapr/runs/`, not `.git/reapr/runs/`. `run_dir` in the JSON is always right.

| File | Use it for |
|---|---|
| `report.json` | Reapr's normalized report: every mutant in the scope with `mutant` (`id`, `file`, `start`, `end`, `mutator`, `replacement`), `status`, `killed_by` (test ids; always `[]` for cargo-mutants), `status_reason`. |
| `run.json` | Facts about the run: `scope` (`diff` or `all`), git shas, base, engine version, duration, counts, score. |

StrykerJS runs also have:

| File | Use it for |
|---|---|
| `mutation.json` | Stryker's raw report (mutation-testing-report-schema v2), unfiltered: per mutant `coveredBy` and `killedBy` (test ids), `statusReason`; `testFiles` maps each test file to its tests (`id`, `name`). |
| `stryker.wrapper.mjs` | The config that actually ran: the project's config with only `mutate`, `reporters` and `jsonReporter.fileName` overridden. |

cargo-mutants runs also have:

| File | Use it for |
|---|---|
| `scope.diff` | The diff cargo-mutants was given (`--in-diff`): one insertion-only hunk per range. |
| `mutants.out/` | cargo-mutants' own output: `outcomes.json`, `mutants.json`, `diff/` (each mutant as a patch) and `log/<mutant>.log`, the build and `cargo test` output per mutant. The `test <name> ... FAILED` lines in a log are the tests that killed it. |

`mutation.json` covers more than the diff (and with `incremental: true`, results carried over from earlier
runs). Use `report.json` ids to restrict to the diff scope. `scripts/test-kills.mjs` does this.

`coveredBy` is only present when the engine records per-test coverage (Stryker: `coverageAnalysis:
"perTest"`). Without it, find weak tests by reading them. Do not change the config to get the data.

## Mutators you will see (StrykerJS)

| Mutator | What it changes | Usually killed by |
|---|---|---|
| `EqualityOperator` | `<` ↔ `<=`, `>` ↔ `>=`, `===` ↔ `!==` | a test at the exact boundary value |
| `ConditionalExpression` | a condition becomes `true` or `false` | one test where the condition is true and one where it is false |
| `LogicalOperator` | `&&` ↔ `\|\|`, `??` → `&&` | a case where only one operand is true |
| `BlockStatement` | a block body becomes `{}` | asserting the block's effect (return, throw, side effect) |
| `StringLiteral` | a string becomes `""` (or `"Stryker was here!"`) | asserting the string, if its content is part of the contract |
| `ArithmeticOperator` | `+` ↔ `-`, `*` ↔ `/` | a non-trivial input (avoid 0 and 1) |
| `BooleanLiteral`, `UnaryOperator`, `UpdateOperator` | `true` ↔ `false`, `-x` → `+x`, `i++` → `i--` | asserting the value |
| `OptionalChaining` | `a?.b` → `a.b` | a test with `a` null or undefined |
| `ArrayDeclaration`, `ObjectLiteral` | `[...]` → `[]`, `{...}` → `{}` | asserting contents |
| `MethodExpression` | `startsWith` ↔ `endsWith`, `toUpperCase` ↔ `toLowerCase`, `.filter(...)` removed | an input where the two differ |
| `Regex` | a regex changed | inputs at each regex edge |

A `StringLiteral` on an error message is only worth a test when the message is part of the contract
(documented, shown to users, matched by callers). Otherwise assert the error type and treat the message
mutant as a question for the human.
