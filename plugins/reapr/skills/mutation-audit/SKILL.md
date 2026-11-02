---
name: mutation-audit
description: Find and fix weak tests with mutation testing as objective evidence, using Reapr (`rpr`). Runs `rpr run --diff` against the base branch, writes one focused test per surviving mutant and re-runs until it is killed, flags tests that cover code but kill nothing, then audits the diff for edge cases mutation operators miss. Use it whenever the user asks if their tests are good enough, wants to strengthen, harden or review the tests of a branch or pull request (including AI-written tests), or mentions mutation testing, mutation score, surviving mutants, Stryker or rpr, even if they never say "mutation testing".
license: MIT
---

# Mutation audit

Coverage says which lines ran. A mutation score says whether any test would notice if those lines were wrong:
the engine changes the code on purpose (a mutant) and checks that a test fails (the mutant is killed). A
surviving mutant is a concrete change in behavior that no test catches. Reapr (`rpr`) runs the project's own
mutation engine on the lines the branch changed and reports the score and every survivor.

Your job: use that evidence to make the tests protect the behavior the code is meant to have, then look past
it for edge cases no mutant represents. Mutation results are the objective part; your reading of the code is
the semantic part. Keep the two apart in the report.

## Hard rules

These exist because each shortcut makes the score go up while the tests get worse, or takes a decision that
belongs to the human.

- **Never change production code to kill a mutant.** If a test shows the code is wrong, report it and stop
  for that mutant. Fixing bugs is a separate decision.
- **Never edit engine config, thresholds or Reapr flags to change the result.** Never create an engine
  config. Reapr invents nothing: each repo owns its config.
- **Never delete or weaken tests.** Change an existing test only with the human's OK.
- **One behavior per test.** A test that kills five mutants by asserting five unrelated things is five
  tests.
- **Follow the repo's test conventions**: file location and naming, framework, helpers, fixtures, assertion
  style, how `describe`/`it` names read.
- **Stay inside the diff scope** unless the human asks for more.
- **Assert intended behavior, not current behavior.** See "When intent is unclear" below.
- No disable comments (`// Stryker disable ...`) without the human's approval.

## Step 0: Preconditions

When available, run `rpr doctor` first. (It does not exist yet; until it does, check the points below.)

1. `rpr --version` works. If not, tell the human and offer the installer; run it only after they agree:
   ```sh
   curl -LsSf https://github.com/reapr-corp/reapr/releases/latest/download/rpr-installer.sh | sh
   ```
   Reapr runs on Linux (x86_64, aarch64) only for now.
2. An engine config exists. For StrykerJS: `stryker.conf.*`, `stryker.config.*` (or the same with a leading
   dot), or a named config such as `stryker.unit.config.mjs` passed with `--config`. If there is none,
   **stop** and tell the human to create one (`npx stryker init`). Do not write one yourself, not even a
   minimal one: the config decides the test runner, what is mutated and the thresholds, and those are the
   team's choices.
3. Reapr mutates committed changes only (`<base>...HEAD`). If `git status` shows uncommitted changes to
   source files, tell the human: those lines are not in scope. New test files you write do not need to be
   committed; the engine runs the tests in the work tree.
4. Monorepos with several configs: list the changed files (`git diff --name-only <base>...HEAD`), find the
   configs whose package directory contains them, and plan one run per config with
   `--project <package dir> --config <config path>`. A run that reports nothing to mutate is fine.

## Step 1: Evidence

```sh
rpr run --diff <base> --json
```

`<base>` defaults to `origin/main`. If the repo has no `origin/main`, or the branch clearly targets another
branch, ask. stdout holds one JSON document; stderr has engine output and notices, ignore it unless the run
fails. Exit code 2 means an error (the JSON is `{"error": "..."}`): report it, do not work around it.

Note the `score`, `counts`, `survivors` and `run_dir`. Field meanings and the run directory files are in
[references/reapr-output.md](references/reapr-output.md).

If `score` is `null` and nothing was mutated, say so and go to Step 4.

## Step 2: Survivors to tests

Work through `survivors` one at a time. For each:

1. Read the code at `file:line:column` and the mutant (`mutator`, `replacement`). The full mutant span is in
   `<run_dir>/report.json` if you need it.
2. State in one sentence the behavior the mutant changes. Example: "`quantity >= 10` became
   `quantity > 10`, so an order of exactly 10 units would lose the bulk discount."
3. Decide what the code is meant to do there (see "When intent is unclear").
4. Find the existing tests for that unit and follow their conventions. Add to the existing test file when
   there is one.
5. Write ONE focused test that asserts the intended behavior at the point the mutant changes. Name it after
   the behavior, not the mutant.
6. Run the project's own test command for that file (from `package.json` scripts, for example
   `npx vitest run src/price.test.ts`). It must pass against the real code.
7. Re-run `rpr run --diff <base> --json` and confirm the mutant is now Killed. Match mutants across runs by
   file, line, column, mutator and replacement (ids can change). A test that kills nothing is not done:
   rethink it or treat the mutant as equivalent.

You may write tests for several survivors before one re-run, since runs take time, but every survivor you
claim as fixed must show as Killed in a run made after its test existed.

`NoCoverage` survivors mean no test even executes that code. The first test that reaches it often kills
several mutants at once; that is fine as long as the test asserts one behavior.

### Equivalent mutants

Some mutants change the code but not its observable behavior, for example removing a fast path that returns
the same value the general path would. Do not force a test (a test that inspects internals to tell them
apart is worse than none). Explain why it is equivalent in one or two sentences and list it for the human.
They decide whether to accept it, rewrite the code, or add a disable comment.

### When intent is unclear

A test written from what the code does, instead of what it should do, turns a bug into expected behavior.
Trail of Bits warns about exactly this in
[Mutation testing for the agentic era](https://blog.trailofbits.com/2026/04/01/mutation-testing-for-the-agentic-era/):
an agent that kills mutants without thinking does not know whether it is writing down the correct behavior or
copying a bug into the test suite. The goal is tests that encode requirements, not accidents of the current
implementation. A passing test that defends a bug makes the bug harder to find.

So before writing a test, look for what specifies the behavior: docs and comments, types, names, callers,
specs, issues, the README. If they agree with the code, write the test. If they disagree with the code
(the doc says "10 units or more get the bulk price" but the code uses `> 10`), or nothing specifies the behavior
at that point, **ask the human** and leave that survivor open. Say what you found, quote both sides, and
propose the test you would write for each answer.

## Step 3: Weak tests

Mutation data also shows which tests do no work. Run:

```sh
node ${CLAUDE_SKILL_DIR}/scripts/test-kills.mjs <run_dir>
```

It lists, for in-scope mutants, how many each test covers and kills, and the tests that cover mutants but
kill none. Use the run from Step 1 (before your new tests), so the result reflects the original suite.
Caveat: with bail on (Stryker's default), `killedBy` holds only the first test that failed, so "kills none"
is a lead, not proof. Confirm by reading the test.

Also read the tests in scope for assertions that cannot fail:

- no assertion at all, or only `toBeDefined()` / `not.toThrow()` on a value that always exists
- snapshots or assertions of a mock's own return value
- tautologies (`expect(x).toBe(x)`, asserting the input against itself)
- assertions inside a callback or loop that never runs

Report each with evidence (test name, file, what it asserts, mutants covered and killed) and propose a fix.
Change existing tests only with the human's OK.

## Step 4: Semantic audit

Mutation operators flip operators and literals; they do not invent missing inputs. Within the diff scope,
read the changed code for its logic and check the edge cases in
[references/edge-cases.md](references/edge-cases.md): boundaries, empty/null/undefined, error and timeout
paths, ordering and concurrency, idempotency, time zones, locale and calendar edges, numeric precision and
overflow, input validation at trust boundaries.

Only list cases that apply to this code. For each, give the behavior it would protect and why it matters.
Implement only those whose expected behavior is specified somewhere (same standard as Step 2), run them, and
ask about the rest. A new edge-case test that fails against the real code is a finding: report it, do not
change the code.

## Step 5: Report

Use this structure:

```markdown
## Mutation audit: <branch> vs <base>

Score: <before>% → <after>% (rpr run <first run_id> → <last run_id>)
Counts after: <killed> killed, <survived> survived, <no_coverage> no coverage, <timeout> timeout

### Survivors
| Mutant (file:line:col, mutator → replacement) | Behavior it changes | Test | Killed? |
|---|---|---|---|

### Equivalent mutants (need your decision)
- <mutant>: <why no observable behavior changes>

### Open questions (intent unclear)
- <mutant or edge case>: <what the code does> vs <what the docs/types/callers say>. Test I would write for each answer.

### Weak tests
| Test | Evidence | Proposed fix |
|---|---|---|

### Edge cases
Added: <test> protects <behavior> (specified by <source>)
Proposed: <case>, <behavior it would protect>, <why>, <what I need to know>

### Findings
<tests that show the code disagrees with its spec; nothing was changed in production code>
```

Every score in the report comes from an `rpr` run, never from an estimate. If a survivor stays open, say why.
