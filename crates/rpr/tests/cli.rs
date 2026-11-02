use std::process::{Command, Output};

fn rpr(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rpr"))
        .args(args)
        // Test runs never send telemetry.
        .env("DO_NOT_TRACK", "1")
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
    for flag in [
        "--diff",
        "--json",
        "--min-score",
        "--config",
        "--project",
        "--engine",
        "cargo-mutants",
    ] {
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
/// A git repo with one commit holding `files`.
fn repo(files: &[&str]) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap();
    for f in files {
        std::fs::write(root.join(f), "{}").unwrap();
    }
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "one"]);
    (tmp, root)
}

#[test]
fn several_engines_need_engine_flag() {
    let (_tmp, root) = repo(&["Cargo.toml", "stryker.config.json"]);
    let project = root.to_str().unwrap();
    let out = rpr(&["run", "--diff", "HEAD", "--project", project, "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let msg = json_error(&out);
    assert!(msg.contains("several mutation engines detected"), "{msg}");
    assert!(msg.contains("stryker (found stryker.config.json)"), "{msg}");
    assert!(msg.contains("cargo-mutants (found Cargo.toml"), "{msg}");
    assert!(msg.contains("--engine <stryker|cargo-mutants>"), "{msg}");

    // `--engine`, or a config file name that belongs to one engine, settles it.
    let config = root.join("stryker.config.json");
    let mutants_toml = root.join("mutants.toml");
    std::fs::write(&mutants_toml, "").unwrap();
    for (args, engine) in [
        (vec!["--engine", "cargo-mutants"], "cargo-mutants"),
        (vec!["--engine", "stryker"], "stryker"),
        (vec!["--config", config.to_str().unwrap()], "stryker"),
        (
            vec!["--config", mutants_toml.to_str().unwrap()],
            "cargo-mutants",
        ),
    ] {
        let mut all = vec!["run", "--diff", "HEAD", "--project", project, "--json"];
        all.extend(&args);
        let out = rpr(&all);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["engine"], engine, "{args:?}");
    }

    // A config name no engine owns falls back to detection, which is ambiguous here.
    let custom = root.join("custom.json");
    std::fs::write(&custom, "{}").unwrap();
    let out = rpr(&[
        "run",
        "--diff",
        "HEAD",
        "--project",
        project,
        "--config",
        custom.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--engine"));
}

#[test]
fn cargo_project_detects_cargo_mutants() {
    let (_tmp, root) = repo(&["Cargo.toml"]);
    let out = rpr(&[
        "run",
        "--diff",
        "HEAD",
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
    assert_eq!(v["engine"], "cargo-mutants");
    assert_eq!(v["score"], serde_json::Value::Null);
}

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

/// `rpr` with a private telemetry state dir and no inherited opt-out or CI variables.
fn rpr_with_telemetry(args: &[&str], config_home: &std::path::Path, debug: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rpr"));
    cmd.args(args).env("XDG_CONFIG_HOME", config_home);
    for k in [
        "DO_NOT_TRACK",
        "RPR_TELEMETRY",
        "RPR_TELEMETRY_DEBUG",
        "GITHUB_ACTIONS",
        "GITLAB_CI",
        "CIRCLECI",
        "BUILDKITE",
        "JENKINS_URL",
        "CI",
    ] {
        cmd.env_remove(k);
    }
    if debug {
        cmd.env("RPR_TELEMETRY_DEBUG", "1");
    }
    cmd.output().unwrap()
}

#[test]
fn telemetry_debug_prints_the_payload_on_stderr_only() {
    let tmp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap().join("repo");
    std::fs::create_dir(&root).unwrap();
    let config_home = tmp.path().join("config");
    std::fs::write(root.join("stryker.config.json"), "{}").unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "one"]);
    let project = root.to_str().unwrap();

    let args = ["run", "--diff", "HEAD", "--project", project, "--json"];
    let out = rpr_with_telemetry(&args, &config_home, true);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let json = stderr
        .lines()
        .find(|l| l.starts_with('{'))
        .unwrap_or_else(|| panic!("no payload in {stderr}"));
    let p: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(p["event"], "run_completed");
    let props = &p["properties"];
    assert_eq!(props["outcome"], "nothing_to_mutate");
    assert_eq!(props["flags"], serde_json::json!(["json", "project"]));
    assert_eq!(props["engine"], "stryker");
    assert_eq!(props["ranges_bucket"], "0");
    assert_eq!(props["$process_person_profile"], false);
    assert!(!json.contains(project) && !json.contains("repo"), "{json}");
    // Without a project key nothing is persisted, even in debug mode.
    if p["api_key"] == "" {
        assert!(!config_home.exists());
    }

    // stdout is the same single --json document as without telemetry.
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["run_id"].is_string() && v["score"].is_null());
    let plain = rpr_with_telemetry(&args, &config_home, false);
    let w: serde_json::Value = serde_json::from_slice(&plain.stdout).unwrap();
    let keys = |v: &serde_json::Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&v), keys(&w));
    assert!(!String::from_utf8_lossy(&plain.stderr).contains("run_completed"));

    // Errors after argument parsing send an error kind, never the message.
    let args = ["run", "--diff", "HEAD", "--project", "/no/such/dir"];
    let out = rpr_with_telemetry(&args, &config_home, true);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let json = stderr.lines().find(|l| l.starts_with('{')).unwrap();
    let p: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(p["properties"]["outcome"], "error");
    assert_eq!(p["properties"]["error_kind"], "args");
    assert!(!json.contains("/no/such"), "{json}");
}

#[test]
fn telemetry_command_saves_the_setting() {
    let tmp = tempfile::tempdir().unwrap();
    let out = rpr_with_telemetry(&["telemetry"], tmp.path(), false);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("telemetry: enabled (default)\n"), "{text}");

    let out = rpr_with_telemetry(&["telemetry", "disable"], tmp.path(), false);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.starts_with("telemetry: disabled (setting)\n"),
        "{text}"
    );
    let state = tmp.path().join("reapr/telemetry.json");
    let s: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    assert_eq!(s["enabled"], false);
    assert_eq!(s["install_id"].as_str().unwrap().len(), 36);

    let out = rpr_with_telemetry(&["telemetry", "enable"], tmp.path(), false);
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("telemetry: enabled (setting)\n"));
}
