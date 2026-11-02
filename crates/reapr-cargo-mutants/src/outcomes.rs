//! cargo-mutants' JSON: `mutants.out/outcomes.json` (a run) and `--list --json` (a listing) →
//! core types. Shapes verified on cargo-mutants 26.2.0 and 27.1.0 (`tests/fixtures/<version>/`).

use std::path::Path;
use std::time::Duration;

use reapr_core::{AdapterError, Mutant, MutantResult, MutantStatus, MutationReport, Position};
use serde::Deserialize;

#[derive(Deserialize)]
struct Outcomes {
    outcomes: Vec<Outcome>,
}

#[derive(Deserialize)]
struct Outcome {
    scenario: Scenario,
    /// `SummaryOutcome` in cargo-mutants' `outcome.rs`.
    summary: String,
}

/// Serialized as `"Baseline"` or `{ "Mutant": { ... } }`.
#[derive(Deserialize)]
enum Scenario {
    Baseline,
    Mutant(RawMutant),
}

/// cargo-mutants' `impl Serialize for Mutant` (`mutant.rs`); `package`, `function` and `diff` unused.
#[derive(Deserialize)]
struct RawMutant {
    /// Unique per mutant, e.g. `src/lib.rs:7:5: replace is_even -> bool with true`.
    name: String,
    /// Relative to the cargo workspace root, `/`-separated.
    file: String,
    span: Span,
    replacement: String,
    /// `FnValue`, `BinaryOperator`, ...
    genre: String,
}

#[derive(Deserialize)]
struct Span {
    start: LineCol,
    end: LineCol,
}

/// 1-based, as in the mutant names cargo-mutants prints.
#[derive(Deserialize)]
struct LineCol {
    line: u32,
    column: u32,
}

/// Engine summary → core status. cargo-mutants has no "no coverage": an untested mutant is missed.
pub(crate) fn status(summary: &str) -> Result<MutantStatus, AdapterError> {
    match summary {
        "CaughtMutant" => Ok(MutantStatus::Killed),
        "MissedMutant" => Ok(MutantStatus::Survived),
        "Unviable" => Ok(MutantStatus::CompileError),
        "Timeout" => Ok(MutantStatus::Timeout),
        // `Success` (only with `--check`) and `Failure` (unclassified) never describe a tested mutant.
        other => Err(AdapterError::Parse {
            context: "cargo-mutants outcomes".into(),
            detail: format!("unexpected mutant outcome `{other}`"),
        }),
    }
}

impl RawMutant {
    /// `prefix`: the project dir relative to the workspace root; paths become relative to the project dir.
    fn into_mutant(self, prefix: &Path) -> Mutant {
        let file = Path::new(&self.file);
        let pos = |p: LineCol| Position {
            line: p.line,
            column: p.column,
        };
        Mutant {
            file: file.strip_prefix(prefix).unwrap_or(file).to_path_buf(),
            id: self.name,
            start: pos(self.span.start),
            end: pos(self.span.end),
            mutator: self.genre,
            replacement: Some(self.replacement),
        }
    }
}

fn parse_err(context: &str) -> impl FnOnce(serde_json::Error) -> AdapterError + '_ {
    move |e| AdapterError::Parse {
        context: context.into(),
        detail: e.to_string(),
    }
}

/// Parses `outcomes.json`. The baseline scenario is not a mutant and is skipped.
pub fn parse_outcomes(
    json: &str,
    engine: &str,
    prefix: &Path,
    duration: Duration,
) -> Result<MutationReport, AdapterError> {
    let o: Outcomes = serde_json::from_str(json).map_err(parse_err("cargo-mutants outcomes"))?;
    let mut results = Vec::new();
    for outcome in o.outcomes {
        let Scenario::Mutant(m) = outcome.scenario else {
            continue;
        };
        results.push(MutantResult {
            status: status(&outcome.summary)?,
            mutant: m.into_mutant(prefix),
            killed_by: vec![],
            status_reason: None,
        });
    }
    Ok(MutationReport {
        engine: engine.to_owned(),
        results,
        duration,
    })
}

/// Parses the stdout of `cargo mutants --list --json`. Empty stdout (cargo-mutants found no mutant in
/// the diff and printed nothing) is an empty list.
pub fn parse_list(stdout: &str, prefix: &Path) -> Result<Vec<Mutant>, AdapterError> {
    if stdout.trim().is_empty() {
        return Ok(vec![]);
    }
    let raw: Vec<RawMutant> =
        serde_json::from_str(stdout).map_err(parse_err("cargo-mutants --list --json"))?;
    Ok(raw.into_iter().map(|m| m.into_mutant(prefix)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use MutantStatus::*;

    fn fixture(version: &str, file: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(version)
            .join(file);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// The fixture crate's run (`tests/fixtures/crate`, `.cargo/mutants.toml` with
    /// `minimum_test_timeout = 2`): `cargo mutants --in-diff <diff> --output <dir>`, exit 3.
    #[test]
    fn parses_real_outcomes_of_every_supported_version() {
        for version in ["26.2.0", "27.1.0"] {
            let r = parse_outcomes(
                &fixture(version, "outcomes.json"),
                "cargo-mutants",
                Path::new(""),
                Duration::from_millis(5),
            )
            .unwrap();
            let c = r.counts();
            assert_eq!(
                (c.killed, c.survived, c.timeout, c.compile_error),
                (8, 6, 2, 1),
                "{version}"
            );
            assert_eq!(
                r.results.len(),
                17,
                "{version}: the baseline is not a mutant"
            );
            assert_eq!(r.duration, Duration::from_millis(5));
            assert_eq!(r.score(), Some(62.5), "{version}: (8 + 2) / 16");

            let first = &r.results[0];
            assert_eq!(
                first.mutant,
                Mutant {
                    id: "src/lib.rs:7:5: replace is_even -> bool with true".into(),
                    file: "src/lib.rs".into(),
                    start: Position { line: 7, column: 5 },
                    end: Position {
                        line: 7,
                        column: 15
                    },
                    mutator: "FnValue".into(),
                    replacement: Some("true".into()),
                },
                "{version}"
            );
            assert_eq!(first.status, Killed);
            let unviable = r.results.iter().find(|x| x.status == CompileError).unwrap();
            assert!(
                unviable.mutant.id.contains("Default::default()"),
                "{version}"
            );
        }
    }

    #[test]
    fn baseline_failure_has_no_mutants() {
        let r = parse_outcomes(
            &fixture("27.1.0", "outcomes-baseline-failed.json"),
            "cargo-mutants",
            Path::new(""),
            Duration::ZERO,
        )
        .unwrap();
        assert!(r.results.is_empty());
    }

    #[test]
    fn paths_become_relative_to_the_project_dir() {
        let json =
            fixture("27.1.0", "outcomes.json").replace("\"src/lib.rs\"", "\"crates/a/src/lib.rs\"");
        let r = parse_outcomes(
            &json,
            "cargo-mutants",
            Path::new("crates/a"),
            Duration::ZERO,
        )
        .unwrap();
        assert!(
            r.results
                .iter()
                .all(|x| x.mutant.file == Path::new("src/lib.rs"))
        );
    }

    #[test]
    fn parses_real_listing_of_every_supported_version() {
        for version in ["26.2.0", "27.1.0"] {
            let list = parse_list(&fixture(version, "list.json"), Path::new("")).unwrap();
            assert_eq!(list.len(), 12, "{version}");
            assert_eq!(
                list[0].id,
                "src/lib.rs:7:5: replace is_even -> bool with true"
            );
            assert_eq!(
                list[11].id,
                "src/lib.rs:19:11: replace -= with /= in countdown"
            );
            assert_eq!(
                list[11].start,
                Position {
                    line: 19,
                    column: 11
                }
            );
            assert_eq!(list[11].mutator, "BinaryOperator");
            assert_eq!(list[11].replacement.as_deref(), Some("/="));
        }
        assert_eq!(parse_list("", Path::new("")).unwrap(), vec![]);
        assert!(matches!(
            parse_list("[{", Path::new("")),
            Err(AdapterError::Parse { .. })
        ));
    }

    #[test]
    fn invalid_outcomes_are_parse_errors() {
        for json in [
            "{",
            r#"{ "outcomes": [{ "scenario": "Other", "summary": "Success" }] }"#,
        ] {
            assert!(matches!(
                parse_outcomes(json, "cargo-mutants", Path::new(""), Duration::ZERO),
                Err(AdapterError::Parse { .. })
            ));
        }
    }

    /// Through the real parsing path: one mutant with the given summary.
    #[test]
    fn conformance_status_mapping() {
        let parse = |summary: &str| {
            let json = fixture("27.1.0", "outcomes.json");
            let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
            let one = v["outcomes"][1].clone();
            v["outcomes"] = serde_json::json!([one]);
            v["outcomes"][0]["summary"] = summary.into();
            let r = parse_outcomes(
                &v.to_string(),
                "cargo-mutants",
                Path::new(""),
                Duration::ZERO,
            )?;
            Ok(r.results[0].status)
        };
        reapr_core::testkit::status_mapping(
            parse,
            &[
                ("CaughtMutant", Killed),
                ("MissedMutant", Survived),
                ("Unviable", CompileError),
                ("Timeout", Timeout),
            ],
        );
        for not_a_tested_mutant in ["Success", "Failure"] {
            assert!(parse(not_a_tested_mutant).is_err(), "{not_a_tested_mutant}");
        }
    }
}
