//! Engine-agnostic core of Reapr: shared types, the [`MutationEngineAdapter`] trait, diff scoping
//! and scoring.

pub mod diff;
pub mod record;
pub mod resources;
pub mod store;
// Compiled in core's own tests too, so the kit is tested where it lives.
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// 1-based, inclusive line range in one file. `path` is relative to the project dir.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileRange {
    pub path: PathBuf,
    pub start: u32,
    pub end: u32,
}

/// What to mutate: changed line ranges, relative to `project_dir`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct MutationScope {
    pub ranges: Vec<FileRange>,
}

/// A slice of the work. Engines with native sharding (cargo-mutants `--shard k/n`, PIT) use index/total;
/// engines without it (Stryker) use `scope`. Phase 1 always runs one shard: index 0, total 1.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Shard {
    pub index: u32,
    pub total: u32,
    pub scope: MutationScope,
}

/// 1-based line and column.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Mutant {
    /// The engine's own id.
    pub id: String,
    /// Relative to the project dir.
    pub file: PathBuf,
    pub start: Position,
    pub end: Position,
    pub mutator: String,
    pub replacement: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MutantStatus {
    Killed,
    Survived,
    NoCoverage,
    Timeout,
    CompileError,
    RuntimeError,
    Ignored,
    Pending,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MutantResult {
    pub mutant: Mutant,
    pub status: MutantStatus,
    pub killed_by: Vec<String>,
    pub status_reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MutationReport {
    pub engine: String,
    pub results: Vec<MutantResult>,
    #[serde(with = "duration_ms")]
    pub duration: Duration,
}

/// Number of results per status.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusCounts {
    pub killed: u32,
    pub survived: u32,
    pub no_coverage: u32,
    pub timeout: u32,
    pub compile_error: u32,
    pub runtime_error: u32,
    pub ignored: u32,
    pub pending: u32,
}

impl MutationReport {
    /// Mutation score in percent, per the mutation-testing-metrics definition:
    /// (Killed + Timeout) / (Killed + Timeout + Survived + NoCoverage) × 100.
    /// `None` when there are no valid mutants.
    pub fn score(&self) -> Option<f64> {
        let c = self.counts();
        let detected = f64::from(c.killed) + f64::from(c.timeout);
        let undetected = f64::from(c.survived) + f64::from(c.no_coverage);
        let valid = detected + undetected;
        // Multiply first: `29.0 / 50.0 * 100.0` is 57.99999999999999.
        (valid > 0.0).then(|| detected * 100.0 / valid)
    }

    pub fn counts(&self) -> StatusCounts {
        let mut c = StatusCounts::default();
        for r in &self.results {
            let slot = match r.status {
                MutantStatus::Killed => &mut c.killed,
                MutantStatus::Survived => &mut c.survived,
                MutantStatus::NoCoverage => &mut c.no_coverage,
                MutantStatus::Timeout => &mut c.timeout,
                MutantStatus::CompileError => &mut c.compile_error,
                MutantStatus::RuntimeError => &mut c.runtime_error,
                MutantStatus::Ignored => &mut c.ignored,
                MutantStatus::Pending => &mut c.pending,
            };
            *slot += 1;
        }
        c
    }

    /// Undetected mutants: Survived and NoCoverage.
    pub fn survivors(&self) -> impl Iterator<Item = &MutantResult> {
        self.results
            .iter()
            .filter(|r| matches!(r.status, MutantStatus::Survived | MutantStatus::NoCoverage))
    }
}

/// Concatenates the results of several shards. Duration is the max, since shards run in parallel.
pub fn merge_reports(engine: &str, reports: Vec<MutationReport>) -> MutationReport {
    let duration = reports.iter().map(|r| r.duration).max().unwrap_or_default();
    MutationReport {
        engine: engine.to_owned(),
        results: reports.into_iter().flat_map(|r| r.results).collect(),
        duration,
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct CoverageMap {
    pub tests_by_mutant: BTreeMap<String, Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub engine: &'static str,
    pub config: Option<PathBuf>,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RunContext {
    /// Engine cwd.
    pub project_dir: PathBuf,
    /// Explicit or detected engine config.
    pub config: Option<PathBuf>,
    /// `<git dir>/reapr/runs/<run_id>/`: scratch and artifacts for this run.
    pub run_dir: PathBuf,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RunOutcome {
    /// `None`: the adapter found nothing to mutate in scope.
    pub report: Option<MutationReport>,
    /// The engine's own configured threshold failed (e.g. Stryker `thresholds.break`).
    pub engine_gate_failed: bool,
    pub warnings: Vec<String>,
    /// CPU and memory of the engine's process tree; `None` where it cannot be measured (Windows).
    pub resources: Option<Resources>,
}

/// CPU time and peak RSS of an engine's process tree (every descendant that was waited for).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Resources {
    pub cpu_user_ms: u64,
    pub cpu_sys_ms: u64,
    /// High-water mark of the largest single process, in KiB.
    pub max_rss_kb: u64,
}

#[derive(thiserror::Error, Debug)]
pub enum AdapterError {
    #[error("{engine} does not support {operation}")]
    Unsupported {
        engine: &'static str,
        operation: &'static str,
    },
    #[error("{engine} config not found: {hint}")]
    ConfigMissing { engine: &'static str, hint: String },
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error("`{command}` failed (exit code {code:?}): {detail}")]
    Process {
        command: String,
        code: Option<i32>,
        detail: String,
    },
    #[error("failed to parse {context}: {detail}")]
    Parse { context: String, detail: String },
}

/// One mutation testing engine. Not dyn-compatible on purpose: callers dispatch through an enum.
pub trait MutationEngineAdapter: Send + Sync {
    fn id(&self) -> &'static str;

    fn detect(
        &self,
        project_dir: &Path,
    ) -> impl Future<Output = Result<Option<Detection>, AdapterError>> + Send;

    fn list_mutants(
        &self,
        ctx: &RunContext,
        scope: &MutationScope,
    ) -> impl Future<Output = Result<Vec<Mutant>, AdapterError>> + Send;

    fn coverage(
        &self,
        ctx: &RunContext,
        scope: &MutationScope,
    ) -> impl Future<Output = Result<CoverageMap, AdapterError>> + Send;

    fn run(
        &self,
        ctx: &RunContext,
        shard: &Shard,
    ) -> impl Future<Output = Result<RunOutcome, AdapterError>> + Send;

    fn merge(&self, reports: Vec<MutationReport>) -> Result<MutationReport, AdapterError> {
        Ok(merge_reports(self.id(), reports))
    }

    fn cache_key(
        &self,
        ctx: &RunContext,
        mutant: &Mutant,
    ) -> impl Future<Output = Result<CacheKey, AdapterError>> + Send;
}

mod duration_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        u64::deserialize(d).map(Duration::from_millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(status: MutantStatus) -> MutantResult {
        MutantResult {
            mutant: Mutant {
                id: "1".into(),
                file: "a.ts".into(),
                start: Position { line: 1, column: 1 },
                end: Position { line: 1, column: 2 },
                mutator: "BooleanLiteral".into(),
                replacement: None,
            },
            status,
            killed_by: vec![],
            status_reason: None,
        }
    }

    fn report(statuses: &[MutantStatus], ms: u64) -> MutationReport {
        MutationReport {
            engine: "test".into(),
            results: statuses.iter().copied().map(result).collect(),
            duration: Duration::from_millis(ms),
        }
    }

    use MutantStatus::*;

    fn killed_of(killed: usize, total: usize) -> MutationReport {
        let mut statuses = vec![Killed; killed];
        statuses.resize(total, Survived);
        report(&statuses, 0)
    }

    /// `detected / valid * 100` gives 57.99999999999999 for 29/50, failing `--min-score 58`.
    #[test]
    fn score_is_exact_for_whole_percentages() {
        assert_eq!(killed_of(29, 50).score(), Some(58.0));
        assert_eq!(killed_of(29, 100).score(), Some(29.0));
        assert_eq!(killed_of(57, 100).score(), Some(57.0));
        assert_eq!(killed_of(58, 100).score(), Some(58.0));
        assert!(killed_of(29, 50).score().is_some_and(|s| s >= 58.0));
    }

    #[test]
    fn score_mixed_statuses() {
        let r = report(
            &[
                Killed,
                Killed,
                Timeout,
                Survived,
                NoCoverage,
                CompileError,
                RuntimeError,
                Ignored,
                Pending,
            ],
            0,
        );
        assert_eq!(r.score(), Some(60.0));
        let c = r.counts();
        assert_eq!(
            c,
            StatusCounts {
                killed: 2,
                survived: 1,
                no_coverage: 1,
                timeout: 1,
                compile_error: 1,
                runtime_error: 1,
                ignored: 1,
                pending: 1,
            }
        );
        let survivors: Vec<_> = r.survivors().map(|s| s.status).collect();
        assert_eq!(survivors, vec![Survived, NoCoverage]);
    }

    #[test]
    fn score_none_when_only_excluded() {
        assert_eq!(
            report(&[CompileError, RuntimeError, Ignored, Pending], 0).score(),
            None
        );
        assert_eq!(report(&[], 0).score(), None);
    }

    #[test]
    fn timeout_counts_as_detected() {
        assert_eq!(report(&[Timeout], 0).score(), Some(100.0));
    }

    #[test]
    fn no_coverage_counts_as_undetected() {
        assert_eq!(report(&[NoCoverage], 0).score(), Some(0.0));
        assert_eq!(report(&[Killed, NoCoverage], 0).score(), Some(50.0));
    }

    #[test]
    fn merge_takes_max_duration_and_concatenates() {
        let merged = merge_reports(
            "stryker",
            vec![report(&[Killed], 300), report(&[Survived, Timeout], 1200)],
        );
        assert_eq!(merged.engine, "stryker");
        assert_eq!(merged.duration, Duration::from_millis(1200));
        assert_eq!(merged.results.len(), 3);
        assert_eq!(merge_reports("x", vec![]).duration, Duration::ZERO);
    }

    #[test]
    fn duration_serializes_as_millis() {
        let r = report(&[], 1500);
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["duration"], 1500);
        let back: MutationReport = serde_json::from_value(json).unwrap();
        assert_eq!(back.duration, Duration::from_millis(1500));
    }
}

#[cfg(test)]
mod trait_tests {
    use super::*;

    struct Dummy;

    impl MutationEngineAdapter for Dummy {
        fn id(&self) -> &'static str {
            "dummy"
        }
        async fn detect(&self, _: &Path) -> Result<Option<Detection>, AdapterError> {
            Ok(None)
        }
        async fn list_mutants(
            &self,
            _: &RunContext,
            _: &MutationScope,
        ) -> Result<Vec<Mutant>, AdapterError> {
            Err(AdapterError::Unsupported {
                engine: "dummy",
                operation: "list_mutants",
            })
        }
        async fn coverage(
            &self,
            _: &RunContext,
            _: &MutationScope,
        ) -> Result<CoverageMap, AdapterError> {
            Ok(CoverageMap::default())
        }
        async fn run(&self, _: &RunContext, _: &Shard) -> Result<RunOutcome, AdapterError> {
            Ok(RunOutcome {
                report: None,
                engine_gate_failed: false,
                warnings: vec![],
                resources: None,
            })
        }
        async fn cache_key(&self, _: &RunContext, m: &Mutant) -> Result<CacheKey, AdapterError> {
            Ok(CacheKey(m.id.clone()))
        }
    }

    #[test]
    fn default_merge_uses_engine_id() {
        let merged = Dummy.merge(vec![]).unwrap();
        assert_eq!(merged.engine, "dummy");
    }
}
