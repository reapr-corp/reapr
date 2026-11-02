//! Adapter conformance kit (feature `testkit`): checks that every adapter's tests call, so each engine
//! keeps the same contract with the core. Use it from `[dev-dependencies]` only. Each check panics with
//! a message when the adapter breaks the contract.

use std::collections::BTreeMap;
use std::path::Path;

use crate::{
    AdapterError, Mutant, MutantStatus, MutationEngineAdapter, MutationReport, MutationScope,
    Position, RunContext, Shard,
};

/// An operation an adapter may answer with [`AdapterError::Unsupported`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    ListMutants,
    Coverage,
    CacheKey,
}

impl Operation {
    /// The name adapters put in [`AdapterError::Unsupported::operation`].
    pub fn name(self) -> &'static str {
        match self {
            Operation::ListMutants => "list_mutants",
            Operation::Coverage => "coverage",
            Operation::CacheKey => "cache_key",
        }
    }
}

/// An empty scope gives `report: None` without starting the engine: no resources measured and nothing
/// written to the run dir. `ctx` must be otherwise runnable (for example, with a config file that exists)
/// and its `run_dir` must exist and be empty.
pub async fn empty_scope_reports_none<A: MutationEngineAdapter>(adapter: &A, ctx: &RunContext) {
    let shard = Shard {
        index: 0,
        total: 1,
        scope: MutationScope::default(),
    };
    let outcome = match adapter.run(ctx, &shard).await {
        Ok(outcome) => outcome,
        Err(e) => panic!("{}: run with an empty scope failed: {e}", adapter.id()),
    };
    assert_eq!(outcome.report, None, "{}: empty scope", adapter.id());
    assert!(!outcome.engine_gate_failed, "{}: empty scope", adapter.id());
    assert_eq!(
        outcome.resources,
        None,
        "{}: resources were measured, so the engine ran",
        adapter.id()
    );
    let written = entries(&ctx.run_dir);
    assert!(
        written.is_empty(),
        "{}: the empty scope wrote {written:?} to the run dir",
        adapter.id()
    );
}

fn entries(dir: &Path) -> Vec<String> {
    match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(e) => panic!("reading {}: {e}", dir.display()),
    }
}

/// Every engine status in `cases` maps to its core status through `parse` (the adapter's real parsing
/// path), and a status the engine does not define is a [`AdapterError::Parse`] error, never a guess.
pub fn status_mapping(
    parse: impl Fn(&str) -> Result<MutantStatus, AdapterError>,
    cases: &[(&str, MutantStatus)],
) {
    for (engine_status, expected) in cases {
        match parse(engine_status) {
            Ok(status) => assert_eq!(status, *expected, "engine status {engine_status:?}"),
            Err(e) => panic!("engine status {engine_status:?}: {e}"),
        }
    }
    let unknown = "ReaprUnknownStatus";
    match parse(unknown) {
        Err(AdapterError::Parse { .. }) => {}
        other => panic!("unknown status {unknown:?}: expected a Parse error, got {other:?}"),
    }
}

/// Verdict parity: Reapr's report and the engine's own report on the same ranges hold the same mutants
/// with the same statuses, and so the same score. Mutants are matched by file, position, mutator and
/// replacement, not by id (engines may number mutants per run).
pub fn assert_parity(reapr: &MutationReport, native: &MutationReport) {
    let by_mutant = |r: &MutationReport| -> BTreeMap<MutantKey, MutantStatus> {
        r.results
            .iter()
            .map(|x| (MutantKey::of(&x.mutant), x.status))
            .collect()
    };
    let (ours, theirs) = (by_mutant(reapr), by_mutant(native));
    for (key, status) in &theirs {
        match ours.get(key) {
            Some(s) => assert_eq!(s, status, "status of {key:?}"),
            None => panic!("{key:?} ({status:?}) is in the native report but not in Reapr's"),
        }
    }
    if let Some(key) = ours.keys().find(|k| !theirs.contains_key(*k)) {
        panic!("{key:?} is in Reapr's report but not in the native one");
    }
    assert_eq!(reapr.score(), native.score(), "score");
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MutantKey {
    file: String,
    start: (u32, u32),
    end: (u32, u32),
    mutator: String,
    replacement: Option<String>,
}

impl MutantKey {
    fn of(m: &Mutant) -> Self {
        let pos = |p: Position| (p.line, p.column);
        MutantKey {
            file: m.file.to_string_lossy().into_owned(),
            start: pos(m.start),
            end: pos(m.end),
            mutator: m.mutator.clone(),
            replacement: m.replacement.clone(),
        }
    }
}

/// Each operation in `ops` returns [`AdapterError::Unsupported`] naming the adapter's own engine id and
/// the operation.
pub async fn unsupported<A: MutationEngineAdapter>(
    adapter: &A,
    ctx: &RunContext,
    ops: &[Operation],
) {
    let scope = MutationScope::default();
    let mutant = Mutant {
        id: "1".into(),
        file: "a".into(),
        start: Position { line: 1, column: 1 },
        end: Position { line: 1, column: 2 },
        mutator: "M".into(),
        replacement: None,
    };
    for op in ops {
        let err = match op {
            Operation::ListMutants => adapter.list_mutants(ctx, &scope).await.err(),
            Operation::Coverage => adapter.coverage(ctx, &scope).await.err(),
            Operation::CacheKey => adapter.cache_key(ctx, &mutant).await.err(),
        };
        match err {
            Some(AdapterError::Unsupported { engine, operation }) => {
                assert_eq!(engine, adapter.id(), "{op:?}: engine id");
                assert_eq!(operation, op.name(), "{op:?}: operation name");
            }
            other => panic!(
                "{}: {op:?}: expected Unsupported, got {other:?}",
                adapter.id()
            ),
        }
    }
}

/// The kit checks itself: each check passes on a conforming adapter and fails on a broken one.
#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::panic::AssertUnwindSafe;
    use std::time::Duration;

    use super::*;
    use crate::{CacheKey, CoverageMap, Detection, MutantResult, Resources, RunOutcome};

    /// `ran`: report a result and resources even for an empty scope. `engine`: the id put in errors.
    struct Fake {
        ran: bool,
        engine: &'static str,
        operation: Option<&'static str>,
    }

    const GOOD: Fake = Fake {
        ran: false,
        engine: "fake",
        operation: None,
    };

    impl Fake {
        fn err(&self, op: &'static str) -> AdapterError {
            AdapterError::Unsupported {
                engine: self.engine,
                operation: self.operation.unwrap_or(op),
            }
        }
    }

    impl MutationEngineAdapter for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        async fn detect(&self, _: &Path) -> Result<Option<Detection>, AdapterError> {
            Ok(None)
        }
        async fn list_mutants(
            &self,
            _: &RunContext,
            _: &MutationScope,
        ) -> Result<Vec<Mutant>, AdapterError> {
            Err(self.err("list_mutants"))
        }
        async fn coverage(
            &self,
            _: &RunContext,
            _: &MutationScope,
        ) -> Result<CoverageMap, AdapterError> {
            Err(self.err("coverage"))
        }
        async fn run(&self, _: &RunContext, _: &Shard) -> Result<RunOutcome, AdapterError> {
            Ok(RunOutcome {
                report: self.ran.then(|| report(&[MutantStatus::Killed])),
                engine_gate_failed: false,
                warnings: vec![],
                resources: self.ran.then(Resources::default),
            })
        }
        async fn cache_key(&self, _: &RunContext, _: &Mutant) -> Result<CacheKey, AdapterError> {
            Err(self.err("cache_key"))
        }
    }

    fn report(statuses: &[MutantStatus]) -> MutationReport {
        MutationReport {
            engine: "fake".into(),
            results: statuses
                .iter()
                .enumerate()
                .map(|(i, s)| MutantResult {
                    mutant: Mutant {
                        id: i.to_string(),
                        file: "a.rs".into(),
                        start: Position {
                            line: 1,
                            column: u32::try_from(i).unwrap_or(0) + 1,
                        },
                        end: Position { line: 1, column: 9 },
                        mutator: "M".into(),
                        replacement: None,
                    },
                    status: *s,
                    killed_by: vec![],
                    status_reason: None,
                })
                .collect(),
            duration: Duration::ZERO,
        }
    }

    fn ctx(dir: &Path) -> RunContext {
        RunContext {
            project_dir: dir.into(),
            config: None,
            run_dir: dir.into(),
        }
    }

    fn panics(f: impl FnOnce()) -> bool {
        std::panic::catch_unwind(AssertUnwindSafe(f)).is_err()
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        match tokio::runtime::Builder::new_current_thread().build() {
            Ok(rt) => rt.block_on(f),
            Err(e) => panic!("runtime: {e}"),
        }
    }

    #[test]
    fn empty_scope_check() {
        let dir = tempfile::tempdir().unwrap();
        block_on(empty_scope_reports_none(&GOOD, &ctx(dir.path())));
        let ran = Fake { ran: true, ..GOOD };
        assert!(panics(|| block_on(empty_scope_reports_none(
            &ran,
            &ctx(dir.path())
        ))));
        std::fs::write(dir.path().join("scope.diff"), "").unwrap();
        assert!(panics(|| block_on(empty_scope_reports_none(
            &GOOD,
            &ctx(dir.path())
        ))));
    }

    #[test]
    fn status_mapping_check() {
        let strict = |s: &str| match s {
            "K" => Ok(MutantStatus::Killed),
            _ => Err(AdapterError::Parse {
                context: "c".into(),
                detail: s.into(),
            }),
        };
        status_mapping(strict, &[("K", MutantStatus::Killed)]);
        assert!(panics(|| status_mapping(
            strict,
            &[("K", MutantStatus::Survived)]
        )));
        assert!(panics(|| status_mapping(
            strict,
            &[("X", MutantStatus::Killed)]
        )));
        let guessing = |_: &str| Ok(MutantStatus::Killed);
        assert!(panics(|| status_mapping(
            guessing,
            &[("K", MutantStatus::Killed)]
        )));
    }

    #[test]
    fn parity_check() {
        use MutantStatus::*;
        let r = report(&[Killed, Survived]);
        assert_parity(&r, &r.clone());
        assert!(panics(|| assert_parity(&r, &report(&[Killed, Killed]))));
        assert!(panics(|| assert_parity(&r, &report(&[Killed]))));
        assert!(panics(|| assert_parity(&report(&[Killed]), &r)));
        let mut moved = r.clone();
        moved.results[1].mutant.end.line = 2;
        assert!(panics(|| assert_parity(&r, &moved)));
    }

    #[test]
    fn unsupported_check() {
        let dir = tempfile::tempdir().unwrap();
        let all = [
            Operation::ListMutants,
            Operation::Coverage,
            Operation::CacheKey,
        ];
        block_on(unsupported(&GOOD, &ctx(dir.path()), &all));
        let other_engine = Fake {
            engine: "other",
            ..GOOD
        };
        assert!(panics(|| block_on(unsupported(
            &other_engine,
            &ctx(dir.path()),
            &all
        ))));
        let wrong_op = Fake {
            operation: Some("x"),
            ..GOOD
        };
        for op in all {
            assert!(panics(|| block_on(unsupported(
                &wrong_op,
                &ctx(dir.path()),
                &[op]
            ))));
        }
    }
}
