//! mutation-testing-report-schema v2 → [`MutationReport`]. Engine-agnostic in shape so it can move
//! to core when a second engine writes this format.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reapr_core::{
    AdapterError, FileRange, Mutant, MutantResult, MutantStatus, MutationReport, Position,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    files: BTreeMap<String, FileResult>,
    project_root: Option<PathBuf>,
}

#[derive(Deserialize)]
struct FileResult {
    mutants: Vec<RawMutant>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMutant {
    id: String,
    mutator_name: String,
    replacement: Option<String>,
    location: Location,
    status: MutantStatus,
    status_reason: Option<String>,
    killed_by: Option<Vec<String>>,
}

/// Signed on purpose: Stryker's incremental mode carries over stale results whose positions can be
/// negative (seen in the wild: `column: -3`). Those are out of scope and dropped before conversion.
#[derive(Deserialize)]
struct Location {
    start: RawPosition,
    end: RawPosition,
}

#[derive(Deserialize, Clone, Copy)]
struct RawPosition {
    line: i64,
    column: i64,
}

/// A parsed report, filtered to the scope.
#[derive(Debug)]
pub struct ScopedReport {
    pub report: MutationReport,
    /// Results the engine reported outside `scope` (e.g. carried over by Stryker `incremental`).
    pub out_of_scope: usize,
}

/// Stryker's containment rule (`isInMutatedScope` + `locationIncluded` in
/// `@stryker-mutator/core/dist/src/mutants/incremental-differ.js`), in report coordinates.
/// Stryker parses `path:a-b` (`project-reader.js`) to the 0-based range (a-1, 0)..(b-1,
/// MAX_SAFE_INTEGER) and reads report positions as (line-1, column-1); so in the report's 1-based
/// positions the range spans (a, 1)..(b, MAX_SAFE_INTEGER + 1), compared as (line, column) pairs.
fn in_scope(scope: &[FileRange], file: &Path, loc: &Location) -> bool {
    const END_COLUMN: i64 = 1 << 53;
    let pos = |p: RawPosition| (p.line, p.column);
    scope.iter().any(|r| {
        r.path == file
            && pos(loc.start) >= (i64::from(r.start), 1)
            && pos(loc.end) <= (i64::from(r.end), END_COLUMN)
    })
}

fn position(p: RawPosition, id: &str, file: &Path) -> Result<Position, AdapterError> {
    match (u32::try_from(p.line), u32::try_from(p.column)) {
        (Ok(line), Ok(column)) => Ok(Position { line, column }),
        _ => Err(AdapterError::Parse {
            context: "mutation report".into(),
            detail: format!(
                "mutant {id} in {} has an invalid position (line {}, column {})",
                file.display(),
                p.line,
                p.column
            ),
        }),
    }
}

/// Parses a v2 report and keeps only the results inside `scope` (paths relative to `project_dir`).
/// File keys (absolute, or relative to `projectRoot` / `project_dir`) become relative to
/// `project_dir`; keys outside it stay absolute. Both directories are compared as real paths:
/// Stryker's `projectRoot` is `process.cwd()`, which has no `..` and no symlinks, while
/// `project_dir` may have either. `duration` is measured by the caller.
pub fn parse_report(
    json: &str,
    engine: &str,
    project_dir: &Path,
    duration: Duration,
    scope: &[FileRange],
) -> Result<ScopedReport, AdapterError> {
    let report: Report = serde_json::from_str(json).map_err(|e| AdapterError::Parse {
        context: "mutation report".into(),
        detail: e.to_string(),
    })?;
    let real = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let dir = real(project_dir);
    let root = report
        .project_root
        .as_deref()
        .map_or_else(|| dir.clone(), real);
    let mut results = Vec::new();
    let mut out_of_scope = 0;
    for (key, file) in report.files {
        let abs = root.join(key);
        let rel = abs
            .strip_prefix(&dir)
            .or_else(|_| abs.strip_prefix(project_dir))
            .map_or(abs.clone(), Path::to_path_buf);
        for m in file.mutants {
            if !in_scope(scope, &rel, &m.location) {
                out_of_scope += 1;
                continue;
            }
            results.push(MutantResult {
                mutant: Mutant {
                    start: position(m.location.start, &m.id, &rel)?,
                    end: position(m.location.end, &m.id, &rel)?,
                    id: m.id,
                    file: rel.clone(),
                    mutator: m.mutator_name,
                    replacement: m.replacement,
                },
                status: m.status,
                killed_by: m.killed_by.unwrap_or_default(),
                status_reason: m.status_reason,
            });
        }
    }
    Ok(ScopedReport {
        report: MutationReport {
            engine: engine.to_owned(),
            results,
            duration,
        },
        out_of_scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(path: &str, start: u32, end: u32) -> FileRange {
        FileRange {
            path: path.into(),
            start,
            end,
        }
    }

    fn whole(paths: &[&str]) -> Vec<FileRange> {
        paths.iter().map(|p| range(p, 1, u32::MAX)).collect()
    }

    const FIXTURE: &str = r#"{
      "schemaVersion": "2",
      "thresholds": { "high": 80, "low": 60 },
      "files": {
        "/proj/src/a.ts": {
          "language": "typescript",
          "source": "export const a = 1 > 0;",
          "mutants": [
            { "id": "0", "mutatorName": "ConditionalExpression", "replacement": "false",
              "location": { "start": { "line": 1, "column": 18 }, "end": { "line": 1, "column": 23 } },
              "status": "Killed", "killedBy": ["t1", "t2"], "coveredBy": ["t1", "t2"] },
            { "id": "1", "mutatorName": "EqualityOperator", "replacement": "1 >= 0",
              "location": { "start": { "line": 1, "column": 18 }, "end": { "line": 1, "column": 23 } },
              "status": "Survived", "statusReason": "no test failed", "killedBy": [] },
            { "id": "2", "mutatorName": "BooleanLiteral",
              "location": { "start": { "line": 2, "column": 1 }, "end": { "line": 2, "column": 5 } },
              "status": "NoCoverage" },
            { "id": "3", "mutatorName": "BlockStatement", "replacement": "{}",
              "location": { "start": { "line": 3, "column": 1 }, "end": { "line": 5, "column": 2 } },
              "status": "Timeout", "statusReason": null, "killedBy": null }
          ]
        },
        "src/b.ts": {
          "language": "typescript",
          "source": "",
          "mutants": [
            { "id": "4", "mutatorName": "StringLiteral", "replacement": "\"\"",
              "location": { "start": { "line": 1, "column": 1 }, "end": { "line": 1, "column": 4 } },
              "status": "CompileError", "statusReason": "TS2322: Type 'string' is not assignable" },
            { "id": "5", "mutatorName": "ArithmeticOperator",
              "location": { "start": { "line": 2, "column": 3 }, "end": { "line": 2, "column": 4 } },
              "status": "RuntimeError", "statusReason": "TypeError: x is not a function" },
            { "id": "6", "mutatorName": "Regex",
              "location": { "start": { "line": 3, "column": 1 }, "end": { "line": 3, "column": 9 } },
              "status": "Ignored", "statusReason": "Ignored by user comment" },
            { "id": "7", "mutatorName": "UpdateOperator",
              "location": { "start": { "line": 4, "column": 1 }, "end": { "line": 4, "column": 4 } },
              "status": "Pending" }
          ]
        },
        "/elsewhere/c.ts": { "language": "typescript", "source": "", "mutants": [] }
      },
      "testFiles": {}
    }"#;

    #[test]
    fn parses_every_status_and_normalizes_paths() {
        let parsed = parse_report(
            FIXTURE,
            "stryker",
            Path::new("/proj"),
            Duration::from_secs(3),
            &whole(&["src/a.ts", "src/b.ts"]),
        )
        .unwrap();
        assert_eq!(parsed.out_of_scope, 0);
        let r = parsed.report;
        assert_eq!(r.engine, "stryker");
        assert_eq!(r.duration, Duration::from_secs(3));
        use MutantStatus::*;
        let statuses: Vec<_> = r.results.iter().map(|x| x.status).collect();
        assert_eq!(
            statuses,
            // BTreeMap order: "/proj/src/a.ts" < "src/b.ts".
            [
                Killed,
                Survived,
                NoCoverage,
                Timeout,
                CompileError,
                RuntimeError,
                Ignored,
                Pending
            ]
        );
        let files: Vec<_> = r.results.iter().map(|x| x.mutant.file.clone()).collect();
        assert!(files[..4].iter().all(|f| f == Path::new("src/a.ts")));
        assert!(files[4..].iter().all(|f| f == Path::new("src/b.ts")));

        let killed = &r.results[0];
        assert_eq!(killed.killed_by, ["t1", "t2"]);
        assert_eq!(killed.mutant.id, "0");
        assert_eq!(killed.mutant.mutator, "ConditionalExpression");
        assert_eq!(killed.mutant.replacement.as_deref(), Some("false"));
        assert_eq!(
            killed.mutant.start,
            Position {
                line: 1,
                column: 18
            }
        );
        assert_eq!(
            killed.mutant.end,
            Position {
                line: 1,
                column: 23
            }
        );

        assert_eq!(
            r.results[1].status_reason.as_deref(),
            Some("no test failed")
        );
        let no_cov = &r.results[2];
        assert_eq!(no_cov.mutant.replacement, None);
        assert!(no_cov.killed_by.is_empty());
        assert_eq!(no_cov.status_reason, None);
        assert!(r.results[3].killed_by.is_empty());
        assert_eq!(r.score(), Some(50.0));
    }

    #[test]
    fn relative_keys_resolve_against_project_root() {
        let json = r#"{ "schemaVersion": "2", "projectRoot": "/proj", "files": { "pkg/x.ts": { "mutants": [
            { "id": "1", "mutatorName": "M", "status": "Killed",
              "location": { "start": { "line": 1, "column": 1 }, "end": { "line": 1, "column": 2 } } } ] } } }"#;
        let r = parse_report(
            json,
            "stryker",
            Path::new("/proj/pkg"),
            Duration::ZERO,
            &whole(&["x.ts"]),
        )
        .unwrap()
        .report;
        assert_eq!(r.results[0].mutant.file, Path::new("x.ts"));
    }

    #[test]
    fn unknown_status_is_parse_error() {
        let json = FIXTURE.replace(r#""status": "Pending""#, r#""status": "Exploded""#);
        let err =
            parse_report(&json, "stryker", Path::new("/proj"), Duration::ZERO, &[]).unwrap_err();
        match err {
            AdapterError::Parse { detail, .. } => assert!(detail.contains("Exploded"), "{detail}"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn invalid_json_is_parse_error() {
        let err = parse_report("{", "stryker", Path::new("/p"), Duration::ZERO, &[]).unwrap_err();
        assert!(matches!(err, AdapterError::Parse { .. }));
    }

    fn mutant(id: &str, start: (i64, i64), end: (i64, i64)) -> String {
        format!(
            r#"{{ "id": "{id}", "mutatorName": "M", "status": "Killed", "location": {{
                "start": {{ "line": {}, "column": {} }}, "end": {{ "line": {}, "column": {} }} }} }}"#,
            start.0, start.1, end.0, end.1
        )
    }

    fn report(files: &[(&str, Vec<String>)]) -> String {
        let files: Vec<String> = files
            .iter()
            .map(|(k, ms)| format!(r#""{k}": {{ "mutants": [{}] }}"#, ms.join(",")))
            .collect();
        format!(
            r#"{{ "schemaVersion": "2", "files": {{ {} }} }}"#,
            files.join(",")
        )
    }

    fn ids(r: &ScopedReport) -> Vec<&str> {
        r.report
            .results
            .iter()
            .map(|x| x.mutant.id.as_str())
            .collect()
    }

    /// Observed on a real Turborepo monorepo, StrykerJS 9.6.1, `incremental: true`: the report carried old results from outside the
    /// mutate ranges, one of them with `column: -3`.
    #[test]
    fn drops_incremental_carryover_outside_scope() {
        let json = report(&[
            (
                "/proj/packages/lib/src/a.ts",
                vec![
                    mutant("in1", (10, 5), (10, 9)),
                    mutant("in2", (11, 1), (12, 3)),
                    mutant("old-garbage", (20, 1), (20, -3)),
                    mutant("old-before", (2, 1), (2, 4)),
                ],
            ),
            (
                "packages/lib/src/other.ts",
                vec![mutant("old-other", (1, 1), (1, 2))],
            ),
        ]);
        let r = parse_report(
            &json,
            "stryker",
            Path::new("/proj"),
            Duration::ZERO,
            &[range("packages/lib/src/a.ts", 10, 12)],
        )
        .unwrap();
        assert_eq!(ids(&r), ["in1", "in2"]);
        assert_eq!(r.out_of_scope, 3);
        assert_eq!(
            r.report.results[0].mutant.file,
            Path::new("packages/lib/src/a.ts")
        );
    }

    #[test]
    fn containment_boundaries_follow_stryker() {
        let json = report(&[(
            "src/a.ts",
            vec![
                mutant("exact", (5, 1), (7, 80)),
                mutant("past-end", (7, 1), (8, 1)),
                mutant("before-start", (4, 9), (5, 2)),
                mutant("second-range", (20, 3), (20, 4)),
            ],
        )]);
        let r = parse_report(
            &json,
            "stryker",
            Path::new("/proj"),
            Duration::ZERO,
            &[range("src/a.ts", 5, 7), range("src/a.ts", 20, 20)],
        )
        .unwrap();
        assert_eq!(ids(&r), ["exact", "second-range"]);
        assert_eq!(r.out_of_scope, 2);
    }

    /// Stryker writes `projectRoot: process.cwd()` (the real path) and keys relative to it, while
    /// `project_dir` may name the same directory through `..` or a symlink (macOS `/tmp`).
    #[test]
    fn project_root_real_path_matches_project_dir_through_dotdot_or_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let real = dunce::canonicalize(tmp.path()).unwrap().join("pkg");
        std::fs::create_dir_all(real.join("src")).unwrap();
        let abs_key = serde_json::to_string(&real.join("src/b.ts")).unwrap();
        let abs_key = abs_key.trim_matches('"');
        let one = vec![mutant("1", (1, 1), (1, 2))];
        let json = report(&[("src/a.ts", one.clone()), (abs_key, one)]).replacen(
            "{",
            &format!(r#"{{ "projectRoot": {},"#, serde_json::json!(real)),
            1,
        );
        let mut dirs = vec![real.join("src").join("..")];
        #[cfg(unix)]
        {
            let link = tmp.path().join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            dirs.push(link);
        }
        for dir in dirs {
            let r = parse_report(
                &json,
                "stryker",
                &dir,
                Duration::ZERO,
                &whole(&["src/a.ts", "src/b.ts"]),
            )
            .unwrap();
            assert_eq!(r.out_of_scope, 0, "{}", dir.display());
            let files: Vec<_> = r.report.results.iter().map(|x| &x.mutant.file).collect();
            assert_eq!(files, [Path::new("src/b.ts"), Path::new("src/a.ts")]);
        }
    }

    /// `locationIncluded` compares (line, column): a range `a-b` starts at line a column 0 in
    /// Stryker's 0-based positions, which is column 1 in the report. A carried-over mutant on line
    /// a with column 0 or below is outside the scope (Stryker reports it as carried over).
    #[test]
    fn start_column_before_range_start_is_out_of_scope() {
        let json = report(&[(
            "src/a.ts",
            vec![
                mutant("col-1", (5, 1), (5, 4)),
                mutant("col-0", (5, 0), (5, 4)),
                mutant("col-neg", (5, -3), (5, 4)),
                mutant("end-on-last-line", (6, 2), (7, 120)),
                mutant("end-past-max", (6, 2), (7, 9_007_199_254_740_993)),
            ],
        )]);
        let r = parse_report(
            &json,
            "stryker",
            Path::new("/proj"),
            Duration::ZERO,
            &[range("src/a.ts", 5, 7)],
        )
        .unwrap();
        assert_eq!(ids(&r), ["col-1", "end-on-last-line"]);
        assert_eq!(r.out_of_scope, 3);
    }

    #[test]
    fn negative_position_in_scope_is_parse_error() {
        let json = report(&[("src/a.ts", vec![mutant("bad", (3, 1), (3, -3))])]);
        let err = parse_report(
            &json,
            "stryker",
            Path::new("/proj"),
            Duration::ZERO,
            &[range("src/a.ts", 1, 9)],
        )
        .unwrap_err();
        match err {
            AdapterError::Parse { detail, .. } => assert!(detail.contains("-3"), "{detail}"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }
}
