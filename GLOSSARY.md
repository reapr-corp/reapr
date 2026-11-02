# Reapr

Reapr runs a project's own mutation testing engine on the lines a change touched, and reports whether the
tests would catch bugs in that change.

## Mutants and their verdicts

**Mutant**:
A copy of the code with one small deliberate change (an operator swapped, a value replaced), made to check
whether some test notices it.
_Avoid_: mutation (for the changed copy), variant

**Verdict**:
What the engine concluded about one mutant: killed, survived, no coverage, timeout, compile error, runtime
error, ignored or pending.
_Avoid_: result, outcome (for a single mutant)

**Killed**:
At least one test failed against the mutant: the tests caught the change.
_Avoid_: caught, detected

**Survived**:
The mutant's code was run by tests and every test still passed: the tests missed the change.
_Avoid_: missed, alive, escaped

**No coverage**:
No test runs the mutant's code at all. Counted as undetected, like survived.
_Avoid_: uncovered, not tested

**Timeout**:
The tests did not finish against the mutant within the engine's time limit, usually because the change made
a loop endless. Counted as detected.
_Avoid_: hang

**Compile error**:
The mutant does not build, so it says nothing about the tests and is left out of the score.
_Avoid_: unviable, invalid mutant

**Survivor**:
A mutant whose verdict is survived or no coverage.

**Equivalent mutant**:
A mutant that behaves exactly like the original code, so no test could ever kill it. It survives without
revealing a gap in the tests.
_Avoid_: false positive

**Finding**:
A survivor reported to the person who owns the change, as a likely gap in the tests.
_Avoid_: issue, alert

## Scope

**Scope**:
What a run may mutate: the ranges of the change under test, and nothing else.
_Avoid_: target, selection

**Range**:
A run of consecutive changed lines in one file. Lines a change only deleted are in no range.
_Avoid_: hunk, chunk

**Shard**:
One slice of a run's mutants, run separately from the other slices so they can run in parallel.
_Avoid_: partition, batch

## Results

**Score**:
The share of mutants the tests detected: killed and timeout over killed, timeout, survived and no coverage.
Compile errors, runtime errors, ignored and pending mutants are left out.
_Avoid_: mutation coverage, kill rate

**Gate**:
A minimum score that fails the run when the score falls below it: Reapr's own minimum score, or a threshold
configured in the engine.
_Avoid_: threshold (alone), quality gate

**Run record**:
The raw facts Reapr keeps about one run, on the machine where it ran: what was tested, with which engine,
how long it took, and what each mutant's verdict was. Never derived judgments.
_Avoid_: run log, history

## Engines

**Engine**:
A mutation testing tool for one language or ecosystem (StrykerJS, cargo-mutants, PIT), configured and run
by the project itself.
_Avoid_: framework, mutator, backend

**Adapter**:
The part of Reapr that knows one engine: how to find its configuration, how to run it on a scope, and how to
read its report. The rest of Reapr knows no engine.
_Avoid_: plugin, driver, integration
