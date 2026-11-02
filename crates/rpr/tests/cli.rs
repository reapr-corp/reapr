use std::process::{Command, Output};

fn rpr(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rpr"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_and_version() {
    let out = rpr(&["--help"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("run"));

    let out = rpr(&["run", "--help"]);
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for flag in ["--diff", "--json", "--min-score", "--config", "--project"] {
        assert!(help.contains(flag), "{flag} missing from:\n{help}");
    }

    let out = rpr(&["--version"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("rpr {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn min_score_out_of_range_is_rejected() {
    let out = rpr(&["run", "--diff", "main", "--min-score", "101"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn non_git_dir_exits_2_with_json_error() {
    // Outside any git repo; an explicit config skips engine detection so the diff step fails.
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("stryker.config.json");
    std::fs::write(&config, "{}").unwrap();
    let project = dir.path().to_str().unwrap();
    let config = config.to_str().unwrap();

    let out = rpr(&[
        "run",
        "--diff",
        "main",
        "--project",
        project,
        "--config",
        config,
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let msg = v["error"].as_str().unwrap();
    assert!(msg.contains("failed to compute the diff vs main"), "{msg}");
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with("error: "),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = rpr(&["run", "--diff", "main", "--project", project]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());

    let out = rpr(&[
        "run",
        "--diff",
        "main",
        "--project",
        project,
        "--config",
        "/no/such/stryker.config.json",
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("config file not found"));
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "dev@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "dev@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn nothing_to_mutate_writes_run_record_in_the_git_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("stryker.config.json"), "{}").unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(
        &root,
        &[
            "commit",
            "-q",
            "-m",
            "one\n\nCo-Authored-By: Bot <bot@example.com>",
        ],
    );

    let out = rpr(&[
        "run",
        "--diff",
        "HEAD~0",
        "--project",
        root.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let run_dir = std::path::PathBuf::from(v["run_dir"].as_str().unwrap());
    let run_id = v["run_id"].as_str().unwrap();
    assert_eq!(run_dir, root.join(".git/reapr/runs").join(run_id));
    assert_eq!(v["resources"], serde_json::Value::Null);
    assert!(v.get("commits").is_none() && v.get("git").is_none());
    assert!(!root.join(".reapr").exists());

    let record = std::fs::read_to_string(run_dir.join("run.json")).unwrap();
    eprintln!("{record}");
    let r: serde_json::Value = serde_json::from_str(&record).unwrap();
    assert_eq!(r["run_id"], run_id);
    assert_eq!(r["git"]["base"], "HEAD~0");
    assert_eq!(r["git"]["head_sha"], r["git"]["merge_base_sha"]);
    assert_eq!(r["git"]["commits"], serde_json::json!([]));
    assert_eq!(r["engine"], "stryker");
    assert_eq!(r["engine_version"], serde_json::Value::Null);
    assert_eq!(r["score"], serde_json::Value::Null);
    assert_eq!(r["passed"], true);
    assert!(r["environment"]["cpu_count"].as_u64().unwrap() >= 1);
    assert!(r["started_at"].as_str().unwrap().ends_with('Z'));

    // Human output prints the absolute run dir.
    let out = rpr(&["run", "--diff", "HEAD", "--project", root.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().last().unwrap();
    assert!(
        line.starts_with(&format!("run: {}", root.join(".git/reapr/runs/").display())),
        "{text}"
    );
}

/// Exactly one JSON document on stdout, on every path.
fn json_error(out: &Output) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"));
    v["error"].as_str().unwrap().to_owned()
}

#[test]
fn argument_errors_with_json_print_a_json_error() {
    let out = rpr(&["run", "--diff", "HEAD~1", "--json", "--min-score", "150"]);
    assert_eq!(out.status.code(), Some(2));
    let msg = json_error(&out);
    assert!(msg.contains("must be between 0 and 100"), "{msg}");
    assert!(!msg.contains('\u{1b}'), "{msg:?}");
    // clap still renders the error on stderr.
    assert!(String::from_utf8_lossy(&out.stderr).contains("--min-score"));

    let out = rpr(&["run", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(json_error(&out).contains("--diff"));

    // Without --json, stdout stays empty.
    let out = rpr(&["run", "--diff", "HEAD~1", "--min-score", "150"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn base_starting_with_dash_is_rejected() {
    let out_dir = tempfile::tempdir().unwrap();
    let base = format!("--diff=--output={}", out_dir.path().join("x").display());
    let out = rpr(&["run", &base, "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let msg = json_error(&out);
    assert!(msg.contains("must not start with '-'"), "{msg}");
    assert_eq!(std::fs::read_dir(out_dir.path()).unwrap().count(), 0);
}
