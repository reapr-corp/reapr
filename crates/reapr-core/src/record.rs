//! Raw facts for the run record (`run.json`): git, environment, time. Facts as found, never judgments.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::diff::git;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct GitFacts {
    pub head_sha: Option<String>,
    /// As given on the command line.
    pub base: String,
    pub merge_base_sha: Option<String>,
    /// `merge_base..HEAD`, newest first (git log order).
    pub commits: Vec<Commit>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub author_email: String,
    /// Raw `Key: value` lines as written (unfolded), e.g. `Co-Authored-By: ...`.
    pub trailers: Vec<String>,
}

/// Fields `\x1f`-separated, commits NUL-separated (`git log -z`).
const LOG_FORMAT: &str = "--format=%H%x1f%ae%x1f%(trailers:only,unfold)";

/// Collects what git can tell about `base...HEAD`. Never fails: each git command that fails becomes a
/// warning and leaves its fact empty.
pub async fn git_facts(project_dir: &Path, base: &str) -> (GitFacts, Vec<String>) {
    let mut warnings = vec![];
    let mut fact = |r: Result<String, crate::diff::DiffError>| match r {
        Ok(out) => Some(out.trim().to_owned()),
        Err(e) => {
            warnings.push(format!("git metadata: {e}"));
            None
        }
    };
    let head_sha = fact(git(project_dir, &["rev-parse", "HEAD"]).await);
    let merge_base_sha = fact(
        git(
            project_dir,
            &["merge-base", "--end-of-options", base, "HEAD"],
        )
        .await,
    );
    let commits = match &merge_base_sha {
        Some(mb) => {
            let range = format!("{mb}..HEAD");
            fact(
                git(
                    project_dir,
                    &["log", "-z", LOG_FORMAT, "--end-of-options", &range, "--"],
                )
                .await,
            )
            .map(|out| parse_log(&out))
            .unwrap_or_default()
        }
        None => vec![],
    };
    let facts = GitFacts {
        head_sha,
        base: base.to_owned(),
        merge_base_sha,
        commits,
    };
    (facts, warnings)
}

/// Parses `git log -z` output in [`LOG_FORMAT`].
pub fn parse_log(out: &str) -> Vec<Commit> {
    out.split('\0')
        .filter_map(|rec| {
            let mut fields = rec.trim_start_matches('\n').splitn(3, '\x1f');
            let sha = fields.next().filter(|s| !s.is_empty())?;
            Some(Commit {
                sha: sha.to_owned(),
                author_email: fields.next().unwrap_or_default().to_owned(),
                trailers: fields
                    .next()
                    .unwrap_or_default()
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(str::to_owned)
                    .collect(),
            })
        })
        .collect()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    /// `github-actions` or `local`.
    pub kind: String,
    pub runner_name: Option<String>,
    /// `RUNNER_OS` / `RUNNER_ARCH` as set by the CI, else Rust's `std::env::consts`.
    pub os: String,
    pub arch: String,
    /// `std::thread::available_parallelism` (respects affinity and cgroup quotas).
    pub cpu_count: Option<usize>,
}

impl Environment {
    pub fn detect() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let gha = var("GITHUB_ACTIONS").as_deref() == Some("true");
        Environment {
            kind: if gha { "github-actions" } else { "local" }.to_owned(),
            runner_name: var("RUNNER_NAME"),
            os: var("RUNNER_OS").unwrap_or_else(|| std::env::consts::OS.to_owned()),
            arch: var("RUNNER_ARCH").unwrap_or_else(|| std::env::consts::ARCH.to_owned()),
            cpu_count: std::thread::available_parallelism().ok().map(|n| n.get()),
        }
    }
}

/// RFC 3339 UTC with second precision, e.g. `2026-11-02T17:20:00Z`. Times before 1970 clamp to the epoch.
pub fn rfc3339(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Howard Hinnant's civil_from_days, for days >= 0.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn rfc3339_known_timestamps() {
        let at = |s| rfc3339(UNIX_EPOCH + Duration::from_secs(s));
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z"); // leap day, century leap year
        assert_eq!(at(1_709_210_096), "2024-02-29T12:34:56Z"); // leap day
        assert_eq!(at(1_793_640_000), "2026-11-02T17:20:00Z");
        assert_eq!(at(4_107_542_399), "2100-02-28T23:59:59Z"); // 2100 is not a leap year
        assert_eq!(at(253_402_300_799), "9999-12-31T23:59:59Z");
        assert_eq!(
            rfc3339(UNIX_EPOCH - Duration::from_secs(1)),
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn rfc3339_march_first_of_a_common_year_before_2000() {
        // 1999 is not a leap year, so the day after Feb 28 is March 1, not Feb 29.
        let at = |s| rfc3339(UNIX_EPOCH + Duration::from_secs(s));
        assert_eq!(at(920_246_400), "1999-03-01T00:00:00Z");
    }

    #[test]
    fn parses_log_with_trailers() {
        let out = "7ae48700c070e1eeaa460868d5e063fe77160a01\x1fc@d\x1f\0\
                   b84248f3709e21292e7b9c21319976e906aea48\x1fa@b\x1f\
                   Co-Authored-By: Claude <noreply@anthropic.com>\n\
                   Signed-off-by: X Y <x@y>\n\0";
        assert_eq!(
            parse_log(out),
            vec![
                Commit {
                    sha: "7ae48700c070e1eeaa460868d5e063fe77160a01".into(),
                    author_email: "c@d".into(),
                    trailers: vec![],
                },
                Commit {
                    sha: "b84248f3709e21292e7b9c21319976e906aea48".into(),
                    author_email: "a@b".into(),
                    trailers: vec![
                        "Co-Authored-By: Claude <noreply@anthropic.com>".into(),
                        "Signed-off-by: X Y <x@y>".into(),
                    ],
                },
            ]
        );
        assert_eq!(parse_log(""), vec![]);
    }

    /// Runs in a child copy of this test binary with the CI variables set, since the environment is
    /// process-global; the child prints what it detected.
    #[test]
    fn environment_from_github_actions_runner_variables() {
        const NAME: &str = "record::tests::environment_from_github_actions_runner_variables";
        if std::env::var_os("REAPR_TEST_ENV_CHILD").is_some() {
            let e = Environment::detect();
            println!(
                "detected={}|{:?}|{}|{}",
                e.kind, e.runner_name, e.os, e.arch
            );
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([NAME, "--exact", "--nocapture", "--test-threads=1"])
            .env("REAPR_TEST_ENV_CHILD", "1")
            .env("GITHUB_ACTIONS", "true")
            .env("RUNNER_NAME", "runner-7")
            .env("RUNNER_OS", "TestOS")
            .env("RUNNER_ARCH", "TestArch")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{stdout}");
        assert!(
            stdout.contains(r#"detected=github-actions|Some("runner-7")|TestOS|TestArch"#),
            "{stdout}"
        );
    }

    fn git_cmd(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[tokio::test]
    async fn git_facts_from_real_repo_and_failures_become_warnings() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        git_cmd(dir, &["init", "-q"]);
        git_cmd(dir, &["commit", "-q", "--allow-empty", "-m", "one"]);
        git_cmd(
            dir,
            &[
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "two\n\nCo-Authored-By: Bot <bot@x>",
            ],
        );

        let (facts, warnings) = git_facts(dir, "HEAD~1").await;
        assert_eq!(warnings, Vec::<String>::new());
        assert_eq!(facts.base, "HEAD~1");
        assert_eq!(facts.head_sha.as_ref().map(String::len), Some(40));
        assert_eq!(facts.commits.len(), 1);
        assert_eq!(facts.commits[0].sha, facts.head_sha.clone().unwrap());
        assert_eq!(facts.commits[0].author_email, "t@t");
        assert_eq!(facts.commits[0].trailers, ["Co-Authored-By: Bot <bot@x>"]);

        let (facts, warnings) = git_facts(dir, "HEAD").await;
        assert_eq!(facts.commits, vec![]);
        assert!(warnings.is_empty());

        let (facts, warnings) = git_facts(dir, "no-such-ref").await;
        assert!(facts.head_sha.is_some());
        assert_eq!((facts.merge_base_sha, facts.commits), (None, vec![]));
        assert_eq!(warnings.len(), 1, "{warnings:?}");

        // A base that looks like an option is a revision to git, not an option.
        let (facts, warnings) = git_facts(dir, "--octopus").await;
        assert_eq!(facts.merge_base_sha, None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }
}
