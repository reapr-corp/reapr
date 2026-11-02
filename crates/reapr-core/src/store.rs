//! Per-run directories under `<git dir>/reapr/runs/`: outside the work tree, so engines never copy them
//! into their sandboxes and git never tracks them. Each worktree has its own git dir.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::MutationReport;
use crate::diff::{DiffError, git};

/// `git rev-parse --absolute-git-dir`, run from `project_dir`, canonicalized so it compares equal to other
/// canonical paths (git prints `C:/...` on Windows and does not resolve symlinks).
pub async fn git_dir(project_dir: &Path) -> Result<PathBuf, DiffError> {
    let args = ["rev-parse", "--absolute-git-dir"];
    let out = git(project_dir, &args).await?;
    let raw = PathBuf::from(out.trim_end_matches(['\r', '\n']));
    dunce::canonicalize(&raw).map_err(|source| DiffError::Io {
        command: format!("canonicalize {}", raw.display()),
        source,
    })
}

/// Creates `<git_dir>/reapr/runs/<unix seconds>-<pid>/` and returns `(run_id, run_dir)`.
pub fn new_run_dir(git_dir: &Path) -> io::Result<(String, PathBuf)> {
    let runs = git_dir.join("reapr").join("runs");
    fs::create_dir_all(&runs)?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let id = format!("{secs}-{}", std::process::id());
    let dir = runs.join(&id);
    fs::create_dir(&dir)?;
    Ok((id, dir))
}

/// Writes `report.json` into `run_dir`.
pub fn write_report(run_dir: &Path, report: &MutationReport) -> io::Result<()> {
    write_json(run_dir, "report.json", report)
}

/// Writes `value` as pretty JSON to `run_dir/<file>`.
pub fn write_json(run_dir: &Path, file: &str, value: &impl Serialize) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(value)?;
    fs::write(run_dir.join(file), json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_dir_and_report() {
        let tmp = tempfile::tempdir().unwrap();
        let (id, dir) = new_run_dir(tmp.path()).unwrap();
        assert!(id.ends_with(&format!("-{}", std::process::id())));
        assert_eq!(dir, tmp.path().join("reapr/runs").join(&id));

        let report = MutationReport {
            engine: "test".into(),
            results: vec![],
            duration: std::time::Duration::from_millis(42),
        };
        write_report(&dir, &report).unwrap();
        let back: MutationReport =
            serde_json::from_slice(&fs::read(dir.join("report.json")).unwrap()).unwrap();
        assert_eq!(back, report);
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
    async fn run_store_lands_in_the_git_dir_of_each_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(tmp.path()).unwrap();
        let sub = root.join("main/packages/app");
        fs::create_dir_all(&sub).unwrap();
        git_cmd(&root.join("main"), &["init", "-q"]);
        git_cmd(
            &root.join("main"),
            &["commit", "-q", "--allow-empty", "-m", "one"],
        );

        let main_git = git_dir(&sub).await.unwrap();
        assert_eq!(main_git, root.join("main/.git"));
        let (id, dir) = new_run_dir(&main_git).unwrap();
        assert_eq!(dir, root.join("main/.git/reapr/runs").join(id));

        git_cmd(&root.join("main"), &["worktree", "add", "-q", "../wt"]);
        assert_eq!(
            git_dir(&root.join("wt")).await.unwrap(),
            root.join("main/.git/worktrees/wt")
        );

        assert!(git_dir(&root).await.is_err());
    }
}
