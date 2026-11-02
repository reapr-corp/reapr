//! The run summary (human text, the `--json` document, the pass/fail verdict) and the run record.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reapr_core::record::{Environment, GitFacts};
use reapr_core::{FileRange, MutantStatus, Resources, RunOutcome, StatusCounts};
use serde::Serialize;

/// Also the `--json` document, field for field.
#[derive(Serialize, Debug)]
pub struct Summary {
    pub run_id: String,
    /// Absolute.
    pub run_dir: PathBuf,
    pub engine: String,
    pub base: String,
    pub ranges: Vec<FileRange>,
    /// `None` when nothing was mutated (or no valid mutants).
    pub score: Option<f64>,
    pub counts: StatusCounts,
    pub survivors: Vec<Survivor>,
    pub duration_ms: u64,
    pub min_score: Option<f64>,
    pub passed: bool,
    pub engine_gate_failed: bool,
    pub warnings: Vec<String>,
    pub resources: Option<Resources>,
    /// The adapter found nothing to mutate in scope (no report).
    #[serde(skip)]
    nothing_to_mutate: bool,
}

#[derive(Serialize, Debug)]
pub struct Survivor {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub mutator: String,
    pub replacement: Option<String>,
    pub status: MutantStatus,
}

impl Summary {
    pub fn new(
        run_id: String,
        run_dir: PathBuf,
        engine: &str,
        base: String,
        ranges: Vec<FileRange>,
        outcome: RunOutcome,
        min_score: Option<f64>,
    ) -> Self {
        let report = outcome.report.as_ref();
        let score = report.and_then(|r| r.score());
        let mut survivors: Vec<Survivor> = report
            .into_iter()
            .flat_map(|r| r.survivors())
            .map(|s| Survivor {
                file: s.mutant.file.clone(),
                line: s.mutant.start.line,
                column: s.mutant.start.column,
                mutator: s.mutant.mutator.clone(),
                replacement: s.mutant.replacement.clone(),
                status: s.status,
            })
            .collect();
        survivors.sort_by(|a, b| (&a.file, a.line, a.column).cmp(&(&b.file, b.line, b.column)));
        Summary {
            run_id,
            run_dir,
            engine: engine.to_owned(),
            base,
            ranges,
            score,
            counts: report.map(|r| r.counts()).unwrap_or_default(),
            survivors,
            duration_ms: report.map_or(0, |r| {
                u64::try_from(r.duration.as_millis()).unwrap_or(u64::MAX)
            }),
            min_score,
            passed: passed(score, min_score, outcome.engine_gate_failed),
            engine_gate_failed: outcome.engine_gate_failed,
            warnings: outcome.warnings,
            resources: outcome.resources,
            nothing_to_mutate: outcome.report.is_none(),
        }
    }

    pub fn to_json(&self) -> String {
        // Plain data with string keys: serialization cannot fail.
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn render_human(&self) -> String {
        let files: BTreeSet<&Path> = self.ranges.iter().map(|r| r.path.as_path()).collect();
        let mut out = format!(
            "rpr · {} · diff vs {} · {} ranges in {} files\n",
            self.engine,
            self.base,
            self.ranges.len(),
            files.len()
        );
        // `write!` into a String cannot fail.
        if self.nothing_to_mutate {
            let _ = writeln!(out, "nothing to mutate in diff vs {}", self.base);
        } else {
            let score = self
                .score
                .map_or_else(|| "n/a".to_owned(), |s| format!("{s:.2}%"));
            let c = &self.counts;
            let _ = writeln!(
                out,
                "score {score}  ({} killed, {} timeout, {} survived, {} no coverage)  {}",
                c.killed,
                c.timeout,
                c.survived,
                c.no_coverage,
                format_duration(Duration::from_millis(self.duration_ms))
            );
            if !self.survivors.is_empty() {
                out.push_str("survivors:\n");
                for s in &self.survivors {
                    let _ = write!(
                        out,
                        "  {}:{}:{}  {}",
                        s.file.display(),
                        s.line,
                        s.column,
                        s.mutator
                    );
                    if let Some(r) = &s.replacement {
                        let _ = write!(out, "  → {r}");
                    }
                    out.push('\n');
                }
            }
        }
        let _ = writeln!(out, "run: {}", self.run_dir.display());
        out
    }

    /// Why the run failed, for stderr. `None` when it passed.
    pub fn failure(&self) -> Option<String> {
        if self.engine_gate_failed {
            Some(format!(
                "{} failed its own configured threshold",
                self.engine
            ))
        } else if self.passed {
            None
        } else {
            Some(format!(
                "score {:.2}% is below --min-score {}",
                self.score.unwrap_or_default(),
                self.min_score.unwrap_or_default()
            ))
        }
    }
}

/// `run.json`: raw facts about one run, kept in the run dir for later trends. No survivors list
/// (`report.json` has every mutant); `duration_ms` and `resources` cover the engine process only.
#[derive(Serialize, Debug)]
pub struct RunRecord<'a> {
    pub run_id: &'a str,
    pub git: &'a GitFacts,
    pub engine: &'a str,
    pub engine_version: Option<&'a str>,
    pub started_at: &'a str,
    pub duration_ms: u64,
    pub environment: &'a Environment,
    pub ranges: &'a [FileRange],
    pub counts: StatusCounts,
    pub score: Option<f64>,
    pub min_score: Option<f64>,
    pub passed: bool,
    pub engine_gate_failed: bool,
    pub warnings: &'a [String],
    pub resources: Option<Resources>,
}

impl<'a> RunRecord<'a> {
    pub fn new(
        s: &'a Summary,
        git: &'a GitFacts,
        engine_version: Option<&'a str>,
        started_at: &'a str,
        environment: &'a Environment,
    ) -> Self {
        RunRecord {
            run_id: &s.run_id,
            git,
            engine: &s.engine,
            engine_version,
            started_at,
            duration_ms: s.duration_ms,
            environment,
            ranges: &s.ranges,
            counts: s.counts,
            score: s.score,
            min_score: s.min_score,
            passed: s.passed,
            engine_gate_failed: s.engine_gate_failed,
            warnings: &s.warnings,
            resources: s.resources,
        }
    }
}

/// Exit 0 vs 1. No valid mutants (score `None`) passes.
fn passed(score: Option<f64>, min_score: Option<f64>, engine_gate_failed: bool) -> bool {
    let above_min = match (score, min_score) {
        (Some(s), Some(m)) => s >= m,
        _ => true,
    };
    above_min && !engine_gate_failed
}

/// `1m 23s`, `45s`, `850ms`.
pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs == 0 {
        format!("{}ms", d.as_millis())
    } else if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reapr_core::record::Commit;
    use reapr_core::{Mutant, MutantResult, MutationReport, Position};

    use MutantStatus::*;

    fn result(file: &str, line: u32, column: u32, status: MutantStatus) -> MutantResult {
        MutantResult {
            mutant: Mutant {
                id: format!("{file}:{line}:{column}"),
                file: file.into(),
                start: Position { line, column },
                end: Position {
                    line,
                    column: column + 1,
                },
                mutator: "ConditionalExpression".into(),
                replacement: Some("false".into()),
            },
            status,
            killed_by: vec![],
            status_reason: None,
        }
    }

    fn range(path: &str, start: u32, end: u32) -> FileRange {
        FileRange {
            path: path.into(),
            start,
            end,
        }
    }

    fn outcome(results: Vec<MutantResult>, gate: bool) -> RunOutcome {
        RunOutcome {
            report: Some(MutationReport {
                engine: "stryker".into(),
                results,
                duration: Duration::from_millis(83_400),
            }),
            engine_gate_failed: gate,
            warnings: vec!["consider disableBail: true".into()],
            resources: Some(Resources {
                cpu_user_ms: 250_000,
                cpu_sys_ms: 30_000,
                max_rss_kb: 812_000,
            }),
        }
    }

    fn summary(outcome: RunOutcome, min_score: Option<f64>) -> Summary {
        Summary::new(
            "1793640000-4242".into(),
            "/r/.git/reapr/runs/1793640000-4242".into(),
            "stryker",
            "origin/main".into(),
            vec![
                range("src/a.ts", 1, 3),
                range("src/a.ts", 10, 10),
                range("src/b.ts", 5, 6),
            ],
            outcome,
            min_score,
        )
    }

    /// 7 killed, 1 timeout, 2 survived, 1 no coverage, 1 compile error → 8/11 = 72.73%.
    fn mixed() -> Vec<MutantResult> {
        let mut r: Vec<_> = (1..=7).map(|l| result("src/a.ts", l, 1, Killed)).collect();
        r.push(result("src/a.ts", 2, 4, Timeout));
        r.push(result("src/b.ts", 6, 2, Survived));
        r.push(result("src/a.ts", 10, 9, NoCoverage));
        r.push(result("src/a.ts", 10, 3, Survived));
        r.push(result("src/a.ts", 3, 1, CompileError));
        r
    }

    #[test]
    fn human_output() {
        let s = summary(outcome(mixed(), false), None);
        assert_eq!(
            s.render_human(),
            "\
rpr · stryker · diff vs origin/main · 3 ranges in 2 files
score 72.73%  (7 killed, 1 timeout, 2 survived, 1 no coverage)  1m 23s
survivors:
  src/a.ts:10:3  ConditionalExpression  → false
  src/a.ts:10:9  ConditionalExpression  → false
  src/b.ts:6:2  ConditionalExpression  → false
run: /r/.git/reapr/runs/1793640000-4242
"
        );
    }

    #[test]
    fn failures_and_nothing_to_mutate() {
        let s = summary(outcome(mixed(), true), Some(90.0));
        assert_eq!(
            s.failure().as_deref(),
            Some("stryker failed its own configured threshold")
        );
        let s = summary(outcome(mixed(), false), Some(90.0));
        assert_eq!(
            s.failure().as_deref(),
            Some("score 72.73% is below --min-score 90")
        );
        assert_eq!(summary(outcome(mixed(), false), None).failure(), None);

        let empty = RunOutcome {
            report: None,
            engine_gate_failed: false,
            warnings: vec![],
            resources: None,
        };
        let s = summary(empty, Some(90.0));
        assert_eq!(
            s.render_human(),
            "\
rpr · stryker · diff vs origin/main · 3 ranges in 2 files
nothing to mutate in diff vs origin/main
run: /r/.git/reapr/runs/1793640000-4242
"
        );
    }

    #[test]
    fn json_output() {
        let s = summary(outcome(mixed(), false), Some(50.0));
        let v: serde_json::Value = serde_json::from_str(&s.to_json()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "run_id": "1793640000-4242",
                "run_dir": "/r/.git/reapr/runs/1793640000-4242",
                "engine": "stryker",
                "base": "origin/main",
                "ranges": [
                    {"path": "src/a.ts", "start": 1, "end": 3},
                    {"path": "src/a.ts", "start": 10, "end": 10},
                    {"path": "src/b.ts", "start": 5, "end": 6},
                ],
                "score": 8.0 / 11.0 * 100.0,
                "counts": {
                    "killed": 7, "survived": 2, "no_coverage": 1, "timeout": 1,
                    "compile_error": 1, "runtime_error": 0, "ignored": 0, "pending": 0,
                },
                "survivors": [
                    {"file": "src/a.ts", "line": 10, "column": 3, "mutator": "ConditionalExpression",
                     "replacement": "false", "status": "Survived"},
                    {"file": "src/a.ts", "line": 10, "column": 9, "mutator": "ConditionalExpression",
                     "replacement": "false", "status": "NoCoverage"},
                    {"file": "src/b.ts", "line": 6, "column": 2, "mutator": "ConditionalExpression",
                     "replacement": "false", "status": "Survived"},
                ],
                "duration_ms": 83400,
                "min_score": 50.0,
                "passed": true,
                "engine_gate_failed": false,
                "warnings": ["consider disableBail: true"],
                "resources": {"cpu_user_ms": 250000, "cpu_sys_ms": 30000, "max_rss_kb": 812000},
            })
        );
    }

    #[test]
    fn run_record() {
        let s = summary(outcome(mixed(), false), Some(50.0));
        let git = GitFacts {
            head_sha: Some("bbbb".into()),
            base: "origin/main".into(),
            merge_base_sha: Some("aaaa".into()),
            commits: vec![Commit {
                sha: "bbbb".into(),
                author_email: "dev@example.com".into(),
                trailers: vec!["Co-Authored-By: Bot <bot@example.com>".into()],
            }],
        };
        let env = Environment {
            kind: "github-actions".into(),
            runner_name: Some("GitHub Actions 7".into()),
            os: "Linux".into(),
            arch: "X64".into(),
            cpu_count: Some(4),
        };
        let r = RunRecord::new(&s, &git, Some("9.6.1"), "2026-11-02T17:20:00Z", &env);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "run_id": "1793640000-4242",
                "git": {
                    "head_sha": "bbbb", "base": "origin/main", "merge_base_sha": "aaaa",
                    "commits": [{"sha": "bbbb", "author_email": "dev@example.com",
                                 "trailers": ["Co-Authored-By: Bot <bot@example.com>"]}],
                },
                "engine": "stryker",
                "engine_version": "9.6.1",
                "started_at": "2026-11-02T17:20:00Z",
                "duration_ms": 83400,
                "environment": {"kind": "github-actions", "runner_name": "GitHub Actions 7",
                                "os": "Linux", "arch": "X64", "cpu_count": 4},
                "ranges": [
                    {"path": "src/a.ts", "start": 1, "end": 3},
                    {"path": "src/a.ts", "start": 10, "end": 10},
                    {"path": "src/b.ts", "start": 5, "end": 6},
                ],
                "counts": {
                    "killed": 7, "survived": 2, "no_coverage": 1, "timeout": 1,
                    "compile_error": 1, "runtime_error": 0, "ignored": 0, "pending": 0,
                },
                "score": 8.0 / 11.0 * 100.0,
                "min_score": 50.0,
                "passed": true,
                "engine_gate_failed": false,
                "warnings": ["consider disableBail: true"],
                "resources": {"cpu_user_ms": 250000, "cpu_sys_ms": 30000, "max_rss_kb": 812000},
            })
        );
    }

    #[test]
    fn json_nothing_to_mutate_has_null_score() {
        let empty = RunOutcome {
            report: None,
            engine_gate_failed: false,
            warnings: vec![],
            resources: None,
        };
        let v: serde_json::Value =
            serde_json::from_str(&summary(empty, Some(80.0)).to_json()).unwrap();
        assert_eq!(v["score"], serde_json::Value::Null);
        assert_eq!(v["passed"], true);
        assert_eq!(v["duration_ms"], 0);
        assert_eq!(v["survivors"], serde_json::json!([]));
    }

    #[test]
    fn verdict() {
        // pass, with and without a threshold
        assert!(passed(Some(80.0), None, false));
        assert!(passed(Some(80.0), Some(80.0), false));
        // below --min-score
        assert!(!passed(Some(79.99), Some(80.0), false));
        // engine gate failed, even when above the threshold
        assert!(!passed(Some(100.0), Some(10.0), true));
        assert!(!passed(None, None, true));
        // nothing to mutate / no valid mutants
        assert!(passed(None, Some(100.0), false));
        // end-to-end through Summary
        assert!(!summary(outcome(mixed(), false), Some(80.0)).passed);
        assert!(summary(outcome(mixed(), false), Some(70.0)).passed);
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(Duration::from_millis(0)), "0ms");
        assert_eq!(format_duration(Duration::from_millis(850)), "850ms");
        assert_eq!(format_duration(Duration::from_millis(1_000)), "1s");
        assert_eq!(format_duration(Duration::from_millis(45_999)), "45s");
        assert_eq!(format_duration(Duration::from_secs(60)), "1m 0s");
        assert_eq!(format_duration(Duration::from_secs(83)), "1m 23s");
        assert_eq!(format_duration(Duration::from_secs(3_725)), "62m 5s");
    }
}
