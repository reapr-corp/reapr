//! Runs the real cargo-mutants on the fixture crate (`tests/fixtures/crate`). Ignored by default: it
//! needs cargo-mutants installed and takes 10-30 s. CI runs it in the `mutation` job:
//! `cargo test -p reapr-cargo-mutants -- --ignored`.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use reapr_cargo_mutants::{CargoMutantsAdapter, parse_outcomes};
use reapr_core::{MutationEngineAdapter, MutationScope, RunContext, Shard, diff};

fn sh(dir: &Path, program: &str, args: &[&str]) -> String {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let mut all = vec!["-c", "user.name=t", "-c", "user.email=t@example.com"];
    all.extend(args);
    sh(dir, "git", &all)
}

/// The fixture crate in a git repo: a base commit with `add` only, then the fixture's `lib.rs`.
fn fixture_repo() -> tempfile::TempDir {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crate");
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::create_dir_all(d.join(".cargo")).unwrap();
    for f in ["Cargo.toml", ".cargo/mutants.toml"] {
        std::fs::copy(src.join(f), d.join(f)).unwrap();
    }
    let lib = std::fs::read_to_string(src.join("src/lib.rs")).unwrap();
    let base: String = lib.lines().take(3).map(|l| format!("{l}\n")).collect();
    std::fs::write(d.join("src/lib.rs"), base).unwrap();
    git(d, &["init", "-q", "-b", "main"]);
    git(d, &["add", "-A"]);
    git(d, &["commit", "-qm", "base"]);
    std::fs::write(d.join("src/lib.rs"), lib).unwrap();
    git(d, &["commit", "-qam", "change"]);
    dir
}

#[tokio::test]
#[ignore = "needs cargo-mutants; run with --ignored"]
async fn real_run_matches_cargo_mutants_on_the_same_diff() {
    let repo = fixture_repo();
    let dir = repo.path();
    let ranges = diff::changed_ranges(dir, "HEAD~1").await.unwrap();
    assert!(!ranges.is_empty());
    let run_dir = dir.join(".git/reapr-run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let ctx = RunContext {
        project_dir: dir.into(),
        config: None,
        run_dir: run_dir.clone(),
    };
    let scope = MutationScope { ranges };

    let listed = CargoMutantsAdapter::new()
        .list_mutants(&ctx, &scope)
        .await
        .unwrap();
    let shard = Shard {
        index: 0,
        total: 1,
        scope,
    };
    let outcome = CargoMutantsAdapter::new().run(&ctx, &shard).await.unwrap();
    let report = outcome.report.unwrap();

    // The fixture's own `.cargo/mutants.toml` (minimum_test_timeout = 2) was used: the two infinite
    // loops time out quickly. cargo-mutants exited 3 (timeouts): not an error, not a gate.
    let c = report.counts();
    assert_eq!(
        (c.killed, c.survived, c.timeout, c.compile_error),
        (8, 6, 2, 1)
    );
    assert!(!outcome.engine_gate_failed);
    assert_eq!(outcome.warnings, Vec::<String>::new());
    assert!(outcome.resources.is_some_and(|r| r.cpu_user_ms > 0));
    assert!(run_dir.join("mutants.out/outcomes.json").is_file());

    let mut listed_ids: Vec<_> = listed.iter().map(|m| m.id.as_str()).collect();
    let mut run_ids: Vec<_> = report
        .results
        .iter()
        .map(|r| r.mutant.id.as_str())
        .collect();
    listed_ids.sort_unstable();
    run_ids.sort_unstable();
    assert_eq!(
        listed_ids, run_ids,
        "list_mutants and run select the same mutants"
    );

    // cargo-mutants' own run on git's diff of the same change.
    let native_dir = dir.join(".git/native");
    std::fs::create_dir_all(&native_dir).unwrap();
    let native_diff = native_dir.join("git.diff");
    std::fs::write(&native_diff, git(dir, &["diff", "HEAD~1...HEAD"])).unwrap();
    let status = Command::new("cargo")
        .args(["mutants", "--in-diff"])
        .arg(&native_diff)
        .arg("--output")
        .arg(&native_dir)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(3), "timeouts");
    let native = parse_outcomes(
        &std::fs::read_to_string(native_dir.join("mutants.out/outcomes.json")).unwrap(),
        "cargo-mutants",
        Path::new(""),
        Duration::ZERO,
    )
    .unwrap();
    reapr_core::testkit::assert_parity(&report, &native);
}
