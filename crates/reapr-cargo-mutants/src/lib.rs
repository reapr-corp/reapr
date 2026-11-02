//! cargo-mutants adapter for Reapr.
//!
//! Runs `cargo mutants` with the project's own configuration (`.cargo/mutants.toml`, read by
//! cargo-mutants itself) and adds exactly two things: `--in-diff` (the diff scope) and `--output` (the
//! run dir). `--workspace` is added when the project dir is the workspace root, so the diff alone decides
//! which packages are mutated.

mod outcomes;

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;

use reapr_core::resources::{children_usage, usage_since};
use reapr_core::{
    AdapterError, CacheKey, CoverageMap, Detection, FileRange, Mutant, MutationEngineAdapter,
    MutationScope, RunContext, RunOutcome, Shard,
};
use tokio::process::Command;

pub use outcomes::{parse_list, parse_outcomes};

const ENGINE: &str = "cargo-mutants";
const INSTALL_HINT: &str = "install it with `cargo install --locked cargo-mutants`";

/// The cargo-mutants versions this adapter is verified against: each has real fixtures in
/// `tests/fixtures/<version>/` (docs/adr/0002-engine-version-support.md). Others run with a warning.
pub const SUPPORTED_VERSIONS: VersionRange = VersionRange {
    min: (26, 2, 0),
    max: (28, 0, 0),
};

/// `>=min, <max`, compared on `major.minor.patch` (a pre-release counts as its release).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionRange {
    pub min: (u64, u64, u64),
    pub max: (u64, u64, u64),
}

impl VersionRange {
    /// `false` also for a version that is not `major.minor.patch`.
    pub fn contains(&self, version: &str) -> bool {
        parse_version(version).is_some_and(|v| self.min <= v && v < self.max)
    }
}

impl fmt::Display for VersionRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (a, b, c) = self.min;
        let (x, y, z) = self.max;
        write!(f, ">={a}.{b}.{c}, <{x}.{y}.{z}")
    }
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

/// The warning for a version outside [`SUPPORTED_VERSIONS`]; `None` inside it.
fn version_warning(version: &str) -> Option<String> {
    (!SUPPORTED_VERSIONS.contains(version)).then(|| {
        format!(
            "{ENGINE} {version} is outside the supported range {SUPPORTED_VERSIONS}; its results may be misread"
        )
    })
}

#[derive(Debug, Clone)]
pub struct CargoMutantsAdapter {
    /// The `cargo` program; a fake in tests.
    cargo: OsString,
}

impl Default for CargoMutantsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CargoMutantsAdapter {
    pub fn new() -> Self {
        Self {
            cargo: "cargo".into(),
        }
    }

    /// Whether `file_name` is cargo-mutants' config file name (`.cargo/mutants.toml`).
    pub fn owns_config(file_name: &str) -> bool {
        file_name == "mutants.toml"
    }

    /// The version `cargo mutants --version` prints from `project_dir`; `None` when it is not installed.
    pub async fn engine_version(&self, project_dir: &Path) -> Option<String> {
        let out = Command::new(&self.cargo)
            .args(["mutants", "--version"])
            .current_dir(project_dir)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8(out.stdout).ok()?;
        text.trim()
            .strip_prefix("cargo-mutants ")
            .map(|v| v.trim().to_owned())
    }
}

fn unsupported(operation: &'static str) -> AdapterError {
    AdapterError::Unsupported {
        engine: ENGINE,
        operation,
    }
}

fn io_err(context: impl fmt::Display) -> impl FnOnce(io::Error) -> AdapterError {
    move |source| AdapterError::Io {
        context: context.to_string(),
        source,
    }
}

fn detection_reason(version: Option<&str>) -> String {
    match version {
        Some(v) => format!("found Cargo.toml; cargo-mutants {v} is installed"),
        None => format!("found Cargo.toml, but cargo-mutants is not installed; {INSTALL_HINT}"),
    }
}

/// The cargo workspace root that cargo-mutants resolves from `project_dir` (it runs the same
/// `cargo locate-project --workspace`), as a real path.
async fn workspace_root(cargo: &OsString, project_dir: &Path) -> Result<PathBuf, AdapterError> {
    let command = "cargo locate-project --workspace --message-format plain";
    let out = Command::new(cargo)
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .current_dir(project_dir)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(io_err(format!("running `{command}`")))?;
    if !out.status.success() {
        return Err(AdapterError::Process {
            command: command.into(),
            code: out.status.code(),
            detail: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        });
    }
    let manifest = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let root = manifest.parent().ok_or_else(|| AdapterError::Parse {
        context: format!("output of `{command}`"),
        detail: format!("{} has no parent dir", manifest.display()),
    })?;
    dunce::canonicalize(root).map_err(io_err(format!("resolving {}", root.display())))
}

/// Where the project sits in its workspace: cargo-mutants paths are relative to the workspace root,
/// Reapr's to the project dir.
struct Layout {
    project_dir: PathBuf,
    /// The project dir relative to the workspace root; empty when they are the same dir.
    prefix: PathBuf,
}

async fn layout(cargo: &OsString, ctx: &RunContext) -> Result<Layout, AdapterError> {
    let project_dir = dunce::canonicalize(&ctx.project_dir)
        .map_err(io_err(format!("resolving {}", ctx.project_dir.display())))?;
    let root = workspace_root(cargo, &project_dir).await?;
    let prefix = project_dir
        .strip_prefix(&root)
        .map_err(|_| AdapterError::Parse {
            context: "cargo workspace".into(),
            detail: format!(
                "{} is not inside its workspace root {}",
                project_dir.display(),
                root.display()
            ),
        })?
        .to_path_buf();
    Ok(Layout {
        project_dir,
        prefix,
    })
}

/// The Rust files of the scope: cargo-mutants ignores every other file in a diff.
fn rust_ranges(scope: &MutationScope) -> Vec<&FileRange> {
    scope
        .ranges
        .iter()
        .filter(|r| r.path.extension().is_some_and(|e| e == "rs"))
        .collect()
}

/// A unified diff that marks exactly `ranges` as changed, for `--in-diff`.
///
/// The trait hands the adapter ranges, not the git base, so the diff is synthesized: one insertion-only
/// hunk per range (`@@ -<start-1>,0 +<start>,<len> @@`) whose `+` lines are the file's current lines.
/// cargo-mutants checks that a diff's new text matches the tree (exit 5 otherwise) and treats inserted
/// lines as the changed ones (`affected_lines` in `in_diff.rs`), so this selects the mutants that
/// intersect the ranges by cargo-mutants' own rule. Paths are relative to the workspace root. Lines past
/// the end of a file and files that no longer exist are skipped.
async fn scope_diff(
    project_dir: &Path,
    prefix: &Path,
    ranges: &[&FileRange],
) -> Result<String, AdapterError> {
    let mut diff = String::new();
    // The core sorts ranges by path, so each file's ranges are consecutive.
    for file_ranges in ranges.chunk_by(|a, b| a.path == b.path) {
        let rel = &file_ranges[0].path;
        let file = project_dir.join(rel);
        let text = match tokio::fs::read_to_string(&file).await {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io_err(format!("reading {}", file.display()))(e)),
        };
        // `str::lines`, as cargo-mutants splits the source it compares the diff with.
        let lines: Vec<&str> = text.lines().collect();
        let mut hunks = String::new();
        for r in file_ranges {
            let start = r.start.max(1) as usize;
            let end = (r.end as usize).min(lines.len());
            if start > end {
                continue;
            }
            hunks.push_str(&format!(
                "@@ -{},0 +{start},{} @@\n",
                start - 1,
                end - start + 1
            ));
            for line in &lines[start - 1..end] {
                hunks.push('+');
                hunks.push_str(line);
                hunks.push('\n');
            }
        }
        if !hunks.is_empty() {
            let path = prefix.join(rel).to_string_lossy().replace('\\', "/");
            diff.push_str(&format!("--- a/{path}\n+++ b/{path}\n{hunks}"));
        }
    }
    Ok(diff)
}

/// Writes the scope diff to the run dir; `None` when the scope has no line cargo-mutants could mutate.
async fn write_scope_diff(
    layout: &Layout,
    run_dir: &Path,
    ranges: &[&FileRange],
) -> Result<Option<PathBuf>, AdapterError> {
    let diff = scope_diff(&layout.project_dir, &layout.prefix, ranges).await?;
    if diff.is_empty() {
        return Ok(None);
    }
    let path = run_dir.join("scope.diff");
    tokio::fs::write(&path, diff)
        .await
        .map_err(io_err(format!("writing {}", path.display())))?;
    Ok(Some(path))
}

/// The arguments after `cargo mutants`: the scope, the user's explicit config, and `--workspace` at the
/// workspace root. Nothing else: everything else comes from the project's own config.
fn scope_args(layout: &Layout, diff: &Path, config: Option<&Path>) -> Vec<String> {
    let mut args = vec![
        "mutants".to_owned(),
        "--in-diff".to_owned(),
        diff.display().to_string(),
    ];
    if layout.prefix.as_os_str().is_empty() {
        args.push("--workspace".into());
    }
    if let Some(config) = config {
        args.push("--config".into());
        args.push(config.display().to_string());
    }
    args
}

/// cargo-mutants' exit codes (`exit_code.rs`): 0 all caught, 2 missed mutants, 3 timeouts — a finished
/// run, never a gate, since cargo-mutants has no threshold. 4: the tests fail without mutations.
/// Anything else (1 usage, 5/6 diff rejected, 70 internal) is a failed run.
fn check_exit(code: Option<i32>, command: &str, out_dir: &Path) -> Result<(), AdapterError> {
    let detail = match code {
        Some(0 | 2 | 3) => return Ok(()),
        Some(4) => format!(
            "the tests fail in the unmutated tree, so no mutant was tested; see {}",
            out_dir.join("log/baseline.log").display()
        ),
        _ => "cargo-mutants failed, see its output above".into(),
    };
    Err(AdapterError::Process {
        command: command.to_owned(),
        code,
        detail,
    })
}

fn not_installed(project_dir: &Path) -> AdapterError {
    AdapterError::Process {
        command: "cargo mutants --version".into(),
        code: None,
        detail: format!(
            "cargo-mutants is not installed for {}; {INSTALL_HINT}",
            project_dir.display()
        ),
    }
}

impl MutationEngineAdapter for CargoMutantsAdapter {
    fn id(&self) -> &'static str {
        ENGINE
    }

    /// A `Cargo.toml` in the project dir. cargo-mutants needs no config file: it reads
    /// `.cargo/mutants.toml` itself when there is one, so `config` stays `None`.
    async fn detect(&self, project_dir: &Path) -> Result<Option<Detection>, AdapterError> {
        if !tokio::fs::metadata(project_dir.join("Cargo.toml"))
            .await
            .is_ok_and(|m| m.is_file())
        {
            return Ok(None);
        }
        let version = self.engine_version(project_dir).await;
        Ok(Some(Detection {
            engine: ENGINE,
            config: None,
            reason: detection_reason(version.as_deref()),
        }))
    }

    async fn list_mutants(
        &self,
        ctx: &RunContext,
        scope: &MutationScope,
    ) -> Result<Vec<Mutant>, AdapterError> {
        let ranges = rust_ranges(scope);
        if ranges.is_empty() {
            return Ok(vec![]);
        }
        let layout = layout(&self.cargo, ctx).await?;
        let run_dir = std::path::absolute(&ctx.run_dir)
            .map_err(io_err(format!("resolving {}", ctx.run_dir.display())))?;
        let Some(diff) = write_scope_diff(&layout, &run_dir, &ranges).await? else {
            return Ok(vec![]);
        };
        let mut args = scope_args(&layout, &diff, ctx.config.as_deref());
        args.extend(["--list".into(), "--json".into()]);
        let command = format!("cargo {}", args.join(" "));
        let out = Command::new(&self.cargo)
            .args(&args)
            .current_dir(&layout.project_dir)
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .output()
            .await
            .map_err(io_err(format!("running `{command}`")))?;
        if !out.status.success() {
            return Err(AdapterError::Process {
                command,
                code: out.status.code(),
                detail: "listing mutants failed, see stderr above".into(),
            });
        }
        parse_list(&String::from_utf8_lossy(&out.stdout), &layout.prefix)
    }

    async fn coverage(
        &self,
        _ctx: &RunContext,
        _scope: &MutationScope,
    ) -> Result<CoverageMap, AdapterError> {
        Err(unsupported("coverage"))
    }

    async fn run(&self, ctx: &RunContext, shard: &Shard) -> Result<RunOutcome, AdapterError> {
        let nothing = |warnings, resources| RunOutcome {
            report: None,
            engine_gate_failed: false,
            warnings,
            resources,
        };
        let ranges = rust_ranges(&shard.scope);
        if ranges.is_empty() {
            return Ok(nothing(vec![], None));
        }
        let layout = layout(&self.cargo, ctx).await?;
        let version = self
            .engine_version(&layout.project_dir)
            .await
            .ok_or_else(|| not_installed(&layout.project_dir))?;
        let warnings: Vec<String> = version_warning(&version).into_iter().collect();

        let run_dir = std::path::absolute(&ctx.run_dir)
            .map_err(io_err(format!("resolving {}", ctx.run_dir.display())))?;
        let Some(diff) = write_scope_diff(&layout, &run_dir, &ranges).await? else {
            return Ok(nothing(warnings, None));
        };
        let mut args = scope_args(&layout, &diff, ctx.config.as_deref());
        args.extend(["--output".into(), run_dir.display().to_string()]);
        if shard.total > 1 {
            // cargo-mutants shards are `k/n` with 0 <= k < n, as Reapr's.
            args.extend(["--shard".into(), format!("{}/{}", shard.index, shard.total)]);
        }
        let command = format!("cargo {}", args.join(" "));
        let out_dir = run_dir.join("mutants.out");

        let started = Instant::now();
        let before = children_usage();
        // cargo-mutants' stdout goes to our stderr: our stdout belongs to the report.
        let status = Command::new(&self.cargo)
            .args(&args)
            .current_dir(&layout.project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(io::stderr()))
            .stderr(Stdio::inherit())
            .status()
            .await
            .map_err(io_err(format!("running `{command}`")))?;
        let resources = before.zip(children_usage()).map(|(b, a)| usage_since(b, a));
        let duration = started.elapsed();

        check_exit(status.code(), &command, &out_dir)?;
        let outcomes_path = out_dir.join("outcomes.json");
        let json = match tokio::fs::read_to_string(&outcomes_path).await {
            Ok(json) => json,
            // Exit 0 without outcomes: no mutant in the diff, cargo-mutants stopped before testing.
            Err(e) if e.kind() == io::ErrorKind::NotFound && status.code() == Some(0) => {
                return Ok(nothing(warnings, resources));
            }
            Err(e) => return Err(io_err(format!("reading {}", outcomes_path.display()))(e)),
        };
        let report = parse_outcomes(&json, ENGINE, &layout.prefix, duration)?;
        Ok(RunOutcome {
            report: Some(report),
            // cargo-mutants has no threshold of its own: missed mutants are results, not a gate.
            engine_gate_failed: false,
            warnings,
            resources,
        })
    }

    async fn cache_key(
        &self,
        _ctx: &RunContext,
        _mutant: &Mutant,
    ) -> Result<CacheKey, AdapterError> {
        Err(unsupported("cache_key"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn range(path: &str, start: u32, end: u32) -> FileRange {
        FileRange {
            path: path.into(),
            start,
            end,
        }
    }

    fn ctx(dir: &Path) -> RunContext {
        RunContext {
            project_dir: dir.into(),
            config: None,
            run_dir: dir.join("run"),
        }
    }

    #[test]
    fn version_range() {
        assert_eq!(SUPPORTED_VERSIONS.to_string(), ">=26.2.0, <28.0.0");
        for ok in ["26.2.0", "27.0.0", "27.1.0", "27.9.3", "27.1.0-pre1"] {
            assert!(SUPPORTED_VERSIONS.contains(ok), "{ok}");
            assert_eq!(version_warning(ok), None);
        }
        for out in [
            "26.1.9", "25.3.1", "28.0.0", "100.0.0", "27.1", "27.1.0.1", "x", "",
        ] {
            assert!(!SUPPORTED_VERSIONS.contains(out), "{out}");
        }
        assert_eq!(
            version_warning("28.0.0").as_deref(),
            Some(
                "cargo-mutants 28.0.0 is outside the supported range >=26.2.0, <28.0.0; its results may be misread"
            )
        );
    }

    #[test]
    fn exit_codes() {
        let out = Path::new("/r/mutants.out");
        for ok in [0, 2, 3] {
            assert!(check_exit(Some(ok), "c", out).is_ok(), "{ok}");
        }
        for bad in [Some(1), Some(4), Some(5), Some(6), Some(70), None] {
            match check_exit(bad, "c", out) {
                Err(AdapterError::Process { code, detail, .. }) => {
                    assert_eq!(code, bad);
                    if bad == Some(4) {
                        assert!(
                            detail.contains("/r/mutants.out/log/baseline.log"),
                            "{detail}"
                        );
                    }
                }
                other => panic!("{bad:?}: expected Process, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn scope_diff_marks_exactly_the_ranges() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/a.rs"), "l1\nl2\r\nl3\nl4\n").unwrap();
        fs::write(dir.path().join("src/b.rs"), "b1").unwrap();
        fs::write(dir.path().join("src/c.rs"), "c1\n").unwrap();
        let ranges = [
            range("src/a.rs", 2, 3),
            range("src/a.rs", 4, 9),
            range("src/b.rs", 1, 1),
            range("src/b.rs", 5, 6),
            range("src/c.rs", 3, 4),
            range("src/gone.rs", 1, 1),
        ];
        let refs: Vec<_> = ranges.iter().collect();
        let diff = scope_diff(dir.path(), Path::new("crates/x"), &refs)
            .await
            .unwrap();
        assert_eq!(
            diff,
            "--- a/crates/x/src/a.rs\n+++ b/crates/x/src/a.rs\n\
             @@ -1,0 +2,2 @@\n+l2\n+l3\n\
             @@ -3,0 +4,1 @@\n+l4\n\
             --- a/crates/x/src/b.rs\n+++ b/crates/x/src/b.rs\n\
             @@ -0,0 +1,1 @@\n+b1\n"
        );
    }

    #[test]
    fn only_rust_files_are_in_scope() {
        let scope = MutationScope {
            ranges: vec![
                range("src/a.rs", 1, 1),
                range("README.md", 1, 1),
                range("rs", 1, 1),
            ],
        };
        let paths: Vec<_> = rust_ranges(&scope).iter().map(|r| r.path.clone()).collect();
        assert_eq!(paths, [PathBuf::from("src/a.rs")]);
    }

    #[test]
    fn args_add_only_scope_output_and_workspace() {
        let at_root = Layout {
            project_dir: "/w".into(),
            prefix: "".into(),
        };
        assert_eq!(
            scope_args(&at_root, Path::new("/r/scope.diff"), None),
            ["mutants", "--in-diff", "/r/scope.diff", "--workspace"]
        );
        let member = Layout {
            project_dir: "/w/crates/a".into(),
            prefix: "crates/a".into(),
        };
        assert_eq!(
            scope_args(
                &member,
                Path::new("/r/scope.diff"),
                Some(Path::new("/c/mutants.toml"))
            ),
            [
                "mutants",
                "--in-diff",
                "/r/scope.diff",
                "--config",
                "/c/mutants.toml"
            ]
        );
    }

    #[test]
    fn owns_mutants_toml_only() {
        assert!(CargoMutantsAdapter::owns_config("mutants.toml"));
        for name in ["Cargo.toml", "stryker.config.json", "mutants.json"] {
            assert!(!CargoMutantsAdapter::owns_config(name), "{name}");
        }
    }

    #[test]
    fn detection_reasons() {
        assert_eq!(
            detection_reason(Some("27.1.0")),
            "found Cargo.toml; cargo-mutants 27.1.0 is installed"
        );
        assert!(detection_reason(None).contains("cargo install --locked cargo-mutants"));
    }

    #[tokio::test]
    async fn detects_cargo_toml_only() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            CargoMutantsAdapter::new().detect(dir.path()).await.unwrap(),
            None
        );
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let d = CargoMutantsAdapter::new()
            .detect(dir.path())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(d.engine, "cargo-mutants");
        assert_eq!(d.config, None);
        assert!(d.reason.starts_with("found Cargo.toml"), "{}", d.reason);
    }

    // ---- conformance kit ----

    #[tokio::test]
    async fn conformance_empty_scope_reports_none() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        fs::create_dir_all(&c.run_dir).unwrap();
        reapr_core::testkit::empty_scope_reports_none(&CargoMutantsAdapter::new(), &c).await;
    }

    #[tokio::test]
    async fn conformance_unsupported_operations() {
        use reapr_core::testkit::Operation::*;
        let dir = tempfile::tempdir().unwrap();
        reapr_core::testkit::unsupported(
            &CargoMutantsAdapter::new(),
            &ctx(dir.path()),
            &[Coverage, CacheKey],
        )
        .await;
        assert_eq!(CargoMutantsAdapter::new().id(), "cargo-mutants");
    }

    /// A scope with no Rust file starts nothing and lists nothing.
    #[tokio::test]
    async fn non_rust_scope_is_nothing_to_mutate() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let scope = MutationScope {
            ranges: vec![range("README.md", 1, 3)],
        };
        let shard = Shard {
            index: 0,
            total: 1,
            scope: scope.clone(),
        };
        let out = CargoMutantsAdapter::new().run(&c, &shard).await.unwrap();
        assert_eq!((out.report, out.resources), (None, None));
        assert_eq!(
            CargoMutantsAdapter::new()
                .list_mutants(&c, &scope)
                .await
                .unwrap(),
            vec![]
        );
        assert!(!c.run_dir.exists());
    }

    // ---- run and list against a fake `cargo` ----

    /// Records its argv and the `--in-diff` file, then answers from files next to it: `version`
    /// (`cargo mutants --version`), `root` (workspace root for `locate-project`, default: cwd),
    /// `list.json`, `outcomes.json` (copied to `<--output>/mutants.out/`) and `exit`.
    #[cfg(unix)]
    const FAKE_CARGO: &str = r#"#!/bin/sh
d=$(dirname "$0")
printf '%s\n' "$*" >> "$d/argv"
if [ "$1" = locate-project ]; then
  if [ -f "$d/root" ]; then echo "$(cat "$d/root")/Cargo.toml"; else echo "$PWD/Cargo.toml"; fi
  exit 0
fi
if [ "$2" = --version ]; then
  [ -f "$d/version" ] || exit 101
  cat "$d/version"; exit 0
fi
prev=; out=
for a in "$@"; do
  [ "$prev" = --output ] && out=$a
  [ "$prev" = --in-diff ] && cp "$a" "$d/seen.diff"
  prev=$a
done
case " $* " in *" --list "*) cat "$d/list.json"; exit 0;; esac
if [ -f "$d/outcomes.json" ]; then mkdir -p "$out/mutants.out"; cp "$d/outcomes.json" "$out/mutants.out/"; fi
exit "$(cat "$d/exit")"
"#;

    #[cfg(unix)]
    struct Fake {
        _tmp: tempfile::TempDir,
        bin: PathBuf,
        project: PathBuf,
        ctx: RunContext,
        adapter: CargoMutantsAdapter,
    }

    #[cfg(unix)]
    impl Fake {
        /// A project at `<tmp>/ws/<member>` (member "" = the workspace root), with `src/lib.rs` from
        /// the fixture crate, and a fake cargo reporting `cargo-mutants 27.1.0`.
        fn new(member: &str) -> Fake {
            use std::os::unix::fs::PermissionsExt;
            let tmp = tempfile::tempdir().unwrap();
            let root = dunce::canonicalize(tmp.path()).unwrap();
            let bin = root.join("bin");
            fs::create_dir_all(&bin).unwrap();
            let cargo = bin.join("cargo");
            fs::write(&cargo, FAKE_CARGO).unwrap();
            fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
            let ws = root.join("ws");
            let project = ws.join(member);
            fs::create_dir_all(project.join("src")).unwrap();
            let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
            fs::copy(
                fixtures.join("crate/src/lib.rs"),
                project.join("src/lib.rs"),
            )
            .unwrap();
            fs::copy(fixtures.join("27.1.0/list.json"), bin.join("list.json")).unwrap();
            fs::write(bin.join("root"), ws.to_str().unwrap()).unwrap();
            fs::write(bin.join("version"), "cargo-mutants 27.1.0\n").unwrap();
            let run_dir = root.join("run");
            fs::create_dir_all(&run_dir).unwrap();
            Fake {
                ctx: RunContext {
                    project_dir: project.clone(),
                    config: None,
                    run_dir,
                },
                adapter: CargoMutantsAdapter {
                    cargo: cargo.into(),
                },
                bin,
                project,
                _tmp: tmp,
            }
        }

        /// The fixture run's `outcomes.json`, with paths under `prefix`, and the exit code.
        fn outcomes(&self, prefix: &str, exit: i32) {
            let json = fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/27.1.0/outcomes.json"),
            )
            .unwrap()
            .replace("\"src/lib.rs\"", &format!("\"{prefix}src/lib.rs\""));
            fs::write(self.bin.join("outcomes.json"), json).unwrap();
            self.exit(exit);
        }

        fn exit(&self, code: i32) {
            fs::write(self.bin.join("exit"), code.to_string()).unwrap();
        }

        fn read(&self, file: &str) -> String {
            fs::read_to_string(self.bin.join(file)).unwrap_or_default()
        }

        async fn run(&self, total: u32) -> Result<RunOutcome, AdapterError> {
            let shard = Shard {
                index: total - 1,
                total,
                scope: MutationScope {
                    ranges: vec![range("src/lib.rs", 7, 7), range("src/lib.rs", 18, 19)],
                },
            };
            self.adapter.run(&self.ctx, &shard).await
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_at_the_workspace_root() {
        let f = Fake::new("");
        f.outcomes("", 3);
        let out = f.run(1).await.unwrap();
        let c = out.report.unwrap().counts();
        assert_eq!(
            (c.killed, c.survived, c.timeout, c.compile_error),
            (8, 6, 2, 1)
        );
        assert!(!out.engine_gate_failed, "exit 3 is a result, not a gate");
        assert_eq!(out.warnings, Vec::<String>::new());
        assert!(out.resources.is_some());
        let run_dir = f.ctx.run_dir.display();
        let argv = f.read("argv");
        assert!(
            argv.contains(&format!(
                "mutants --in-diff {run_dir}/scope.diff --workspace --output {run_dir}\n"
            )),
            "{argv}"
        );
        assert_eq!(
            f.read("seen.diff"),
            "--- a/src/lib.rs\n+++ b/src/lib.rs\n\
             @@ -6,0 +7,1 @@\n+    n % 2 == 0\n\
             @@ -17,0 +18,2 @@\n+    while n > 0 {\n+        n -= 1;\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_in_a_member_crate_with_shards_and_config() {
        let mut f = Fake::new("crates/a");
        f.ctx.config = Some(f.project.join("mutants.toml"));
        f.outcomes("crates/a/", 2);
        let out = f.run(2).await.unwrap();
        let report = out.report.unwrap();
        assert!(
            report
                .results
                .iter()
                .all(|r| r.mutant.file == Path::new("src/lib.rs"))
        );
        let argv = f.read("argv");
        let argv = argv
            .lines()
            .find(|l| l.starts_with("mutants --in-diff"))
            .unwrap();
        assert!(!argv.contains("--workspace"), "{argv}");
        assert!(argv.contains(" --config "), "{argv}");
        assert!(argv.contains(" --shard 1/2"), "{argv}");
        assert!(
            f.read("seen.diff")
                .starts_with("--- a/crates/a/src/lib.rs\n+++ b/crates/a/src/lib.rs\n")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_warns_outside_the_version_range() {
        let f = Fake::new("");
        f.outcomes("", 0);
        fs::write(f.bin.join("version"), "cargo-mutants 28.0.0\n").unwrap();
        let out = f.run(1).await.unwrap();
        assert_eq!(out.warnings, [version_warning("28.0.0").unwrap()]);
        assert_eq!(
            f.adapter.engine_version(&f.project).await.as_deref(),
            Some("28.0.0")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_errors() {
        let f = Fake::new("");
        // Not installed.
        fs::remove_file(f.bin.join("version")).unwrap();
        match f.run(1).await {
            Err(AdapterError::Process { detail, .. }) => {
                assert!(
                    detail.contains("cargo install --locked cargo-mutants"),
                    "{detail}"
                )
            }
            other => panic!("expected Process, got {other:?}"),
        }
        assert_eq!(f.adapter.engine_version(&f.project).await, None);
        fs::write(f.bin.join("version"), "cargo-mutants 27.1.0\n").unwrap();

        // Exit 0 and no outcomes: no mutant in the diff.
        f.exit(0);
        let out = f.run(1).await.unwrap();
        assert_eq!(out.report, None);
        // A finished run must leave outcomes.
        f.exit(2);
        assert!(matches!(f.run(1).await, Err(AdapterError::Io { .. })));
        // Baseline failure.
        f.outcomes("", 4);
        match f.run(1).await {
            Err(AdapterError::Process {
                code: Some(4),
                detail,
                ..
            }) => {
                assert!(detail.contains("log/baseline.log"), "{detail}")
            }
            other => panic!("expected Process, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn list_mutants_parses_the_listing() {
        let f = Fake::new("");
        let scope = MutationScope {
            ranges: vec![range("src/lib.rs", 7, 7)],
        };
        let list = f.adapter.list_mutants(&f.ctx, &scope).await.unwrap();
        assert_eq!(list.len(), 12);
        assert!(
            f.read("argv").contains("--workspace --list --json"),
            "{}",
            f.read("argv")
        );
        fs::write(f.bin.join("list.json"), "").unwrap();
        assert_eq!(
            f.adapter.list_mutants(&f.ctx, &scope).await.unwrap(),
            vec![]
        );
    }

    #[tokio::test]
    async fn unreadable_scope_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/dir.rs")).unwrap();
        let r = range("src/dir.rs", 1, 1);
        assert!(matches!(
            scope_diff(dir.path(), Path::new(""), &[&r]).await,
            Err(AdapterError::Io { .. })
        ));
    }
}
