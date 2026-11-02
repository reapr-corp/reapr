//! `rpr`: mutation testing scoped to your diff.

mod output;
mod telemetry;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use reapr_cargo_mutants::CargoMutantsAdapter;
use reapr_core::{
    AdapterError, Detection, FileRange, MutationEngineAdapter, MutationScope, RunContext,
    RunOutcome, Shard, diff, record, store,
};
use reapr_stryker::StrykerAdapter;

use output::{RunRecord, Summary};

#[derive(Parser)]
#[command(name = "rpr", version, about = "Mutation testing scoped to your diff.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Mutate the lines changed since <base> (or, with --all, every tracked file) with the project's
    /// own engine config.
    Run(RunArgs),
    /// Show or change anonymous usage telemetry (see TELEMETRY.md).
    Telemetry {
        #[command(subcommand)]
        action: Option<telemetry::Action>,
    },
}

#[derive(clap::Args)]
// Exactly one scope: `--diff <BASE>` or `--all`.
#[command(group = clap::ArgGroup::new("scope").required(true).args(["base", "all"]))]
struct RunArgs {
    /// Base ref to diff against (`<base>...HEAD`), e.g. origin/main.
    #[arg(long = "diff", value_name = "BASE", value_parser = parse_base)]
    base: Option<String>,
    /// Mutate every git-tracked file under the project dir instead of a diff (a whole-project audit).
    #[arg(long)]
    all: bool,
    /// Print a single JSON document on stdout instead of the human summary.
    #[arg(long)]
    json: bool,
    /// Fail (exit 1) when the mutation score is below this percentage.
    #[arg(long, value_name = "N", value_parser = parse_min_score)]
    min_score: Option<f64>,
    /// Engine config file; skips config detection. Its file name picks the engine when it is one the
    /// engine itself looks for (stryker.config.mjs, mutants.toml, ...).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Mutation engine; needed when several are detected in the project.
    #[arg(long, value_enum, value_name = "ENGINE")]
    engine: Option<EngineId>,
    /// Project directory (engine cwd). Default: the current directory.
    #[arg(long, value_name = "DIR")]
    project: Option<PathBuf>,
}

impl RunArgs {
    /// The names of the optional flags given, for telemetry (names only, never values).
    fn flag_names(&self) -> Vec<&'static str> {
        [
            ("all", self.all),
            ("json", self.json),
            ("min_score", self.min_score.is_some()),
            ("config", self.config.is_some()),
            ("engine", self.engine.is_some()),
            ("project", self.project.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, used)| used.then_some(name))
        .collect()
    }
}

/// The CLI's own errors, typed so telemetry can tell their kind without reading the message.
#[derive(thiserror::Error, Debug)]
pub enum CliError {
    #[error("invalid project dir {}", .0.display())]
    Project(PathBuf, #[source] std::io::Error),
    #[error("invalid config path {}", .0.display())]
    ConfigPath(PathBuf, #[source] std::io::Error),
    #[error("config file not found: {}", .0.display())]
    ConfigNotFound(PathBuf),
    #[error("no supported mutation engine detected in {}", .0.display())]
    NoEngine(PathBuf),
    #[error("several mutation engines detected in {}: {}; choose one with --engine <stryker|cargo-mutants>", .0.display(), .1)]
    AmbiguousEngine(PathBuf, String),
}

/// Git would read a leading `-` as an option (`--output=<path>` writes a file).
fn parse_base(s: &str) -> Result<String, String> {
    if s.starts_with('-') {
        Err(format!("must not start with '-', got `{s}`"))
    } else {
        Ok(s.to_owned())
    }
}

fn parse_min_score(s: &str) -> Result<f64, String> {
    let n: f64 = s.parse().map_err(|_| format!("`{s}` is not a number"))?;
    if (0.0..=100.0).contains(&n) {
        Ok(n)
    } else {
        Err(format!("must be between 0 and 100, got {s}"))
    }
}

/// `--engine` values.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum EngineId {
    Stryker,
    CargoMutants,
}

/// Static dispatch over the supported engines (the adapter trait is not dyn-compatible, ADR 0001).
enum Engine {
    Stryker(StrykerAdapter),
    CargoMutants(CargoMutantsAdapter),
}

impl Engine {
    /// Every engine, in detection order.
    fn all() -> [Engine; 2] {
        [EngineId::Stryker, EngineId::CargoMutants].map(Engine::of)
    }

    fn of(id: EngineId) -> Engine {
        match id {
            EngineId::Stryker => Engine::Stryker(StrykerAdapter::new()),
            EngineId::CargoMutants => Engine::CargoMutants(CargoMutantsAdapter::new()),
        }
    }

    /// The engine whose own config file name `config` has, if exactly one has it.
    fn for_config(config: &Path) -> Option<Engine> {
        let name = config.file_name()?.to_str()?;
        let mut owners = [
            (EngineId::Stryker, StrykerAdapter::owns_config(name)),
            (
                EngineId::CargoMutants,
                CargoMutantsAdapter::owns_config(name),
            ),
        ]
        .into_iter()
        .filter_map(|(id, owns)| owns.then_some(id));
        match (owners.next(), owners.next()) {
            (Some(id), None) => Some(Engine::of(id)),
            _ => None,
        }
    }

    fn id(&self) -> &'static str {
        match self {
            Engine::Stryker(a) => a.id(),
            Engine::CargoMutants(a) => a.id(),
        }
    }

    async fn detect(&self, project_dir: &Path) -> Result<Option<Detection>, AdapterError> {
        match self {
            Engine::Stryker(a) => a.detect(project_dir).await,
            Engine::CargoMutants(a) => a.detect(project_dir).await,
        }
    }

    async fn run(&self, ctx: &RunContext, shard: &Shard) -> Result<RunOutcome, AdapterError> {
        match self {
            Engine::Stryker(a) => a.run(ctx, shard).await,
            Engine::CargoMutants(a) => a.run(ctx, shard).await,
        }
    }

    async fn version(&self, project_dir: &Path) -> Option<String> {
        match self {
            Engine::Stryker(a) => a.engine_version(project_dir).await,
            Engine::CargoMutants(a) => a.engine_version(project_dir).await,
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let started = Instant::now();
    let args = match Cli::try_parse() {
        Ok(Cli {
            command: Command::Run(args),
        }) => args,
        Ok(Cli {
            command: Command::Telemetry { action },
        }) => return telemetry::command(action.unwrap_or(telemetry::Action::Status)),
        Err(e) => return clap_exit(&e),
    };
    let json = args.json;
    let flags = args.flag_names();
    let project = args.project.clone().unwrap_or_else(|| PathBuf::from("."));
    let result = run(args).await;
    let code = match &result {
        Ok((summary, _)) => {
            for w in &summary.warnings {
                eprintln!("warning: {w}");
            }
            if json {
                println!("{}", summary.to_json());
            } else {
                print!("{}", summary.render_human());
            }
            match summary.failure() {
                None => ExitCode::SUCCESS,
                Some(why) => {
                    eprintln!("failed: {why}");
                    ExitCode::from(1)
                }
            }
        }
        Err(e) => {
            let msg = format!("{e:#}");
            eprintln!("error: {msg}");
            if json {
                println!("{}", serde_json::json!({ "error": msg }));
            }
            ExitCode::from(2)
        }
    };
    // After the output is printed: telemetry never delays or changes it.
    let run = result.as_ref().map(|(s, v)| (s, v.as_deref()));
    let props = telemetry::Properties::new(&telemetry::os_env, flags, run, started.elapsed());
    telemetry::report(&props, &project).await;
    code
}

/// clap's own rendering and exit code (0 for help and version, 2 for errors), plus the JSON error
/// document on stdout when `--json` was asked for.
fn clap_exit(e: &clap::Error) -> ExitCode {
    let wants_json = std::env::args_os().any(|a| a == "--json");
    if e.use_stderr() && wants_json {
        // `StyledStr`'s Display is plain text, without ANSI styling.
        let msg = e.render().to_string();
        let msg = msg.trim().strip_prefix("error: ").unwrap_or(msg.trim());
        println!("{}", serde_json::json!({ "error": msg }));
    }
    // Printing can only fail on a closed stream; the exit code still tells.
    let _ = e.print();
    ExitCode::from(u8::try_from(e.exit_code()).unwrap_or(2))
}

/// The summary and the engine version.
async fn run(args: RunArgs) -> Result<(Summary, Option<String>)> {
    let started_at = record::rfc3339(SystemTime::now());
    // The real path, as the engine sees its cwd: Stryker reports `projectRoot: process.cwd()`, so a
    // `..` or a symlink here would put every mutant outside the scope. `dunce` avoids Windows' `\\?\`.
    let project = args.project.unwrap_or_else(|| PathBuf::from("."));
    let project_dir =
        dunce::canonicalize(&project).map_err(|e| CliError::Project(project.clone(), e))?;

    let config = match args.config {
        Some(path) => {
            let path =
                std::path::absolute(&path).map_err(|e| CliError::ConfigPath(path.clone(), e))?;
            if !path.is_file() {
                return Err(CliError::ConfigNotFound(path).into());
            }
            Some(path)
        }
        None => None,
    };
    let (engine, config) = select_engine(&project_dir, args.engine, config).await?;

    let ranges = match &args.base {
        Some(base) => diff::changed_ranges(&project_dir, base)
            .await
            .with_context(|| format!("failed to compute the diff vs {base}"))?,
        None => all_ranges(&project_dir)
            .await
            .context("failed to list the git-tracked files")?,
    };

    let git_dir = store::git_dir(&project_dir)
        .await
        .with_context(|| format!("failed to find the git dir of {}", project_dir.display()))?;
    let (run_id, run_dir) = store::new_run_dir(&git_dir)
        .with_context(|| format!("failed to create a run dir in {}", git_dir.display()))?;
    // Before the engine runs: the facts describe the commit that was tested.
    // `--all` has no base: HEAD gives the tested commit and no commit list.
    let (git, git_warnings) =
        record::git_facts(&project_dir, args.base.as_deref().unwrap_or("HEAD")).await;

    // Empty diff scope: nothing to mutate, the engine is not started.
    let mut outcome = if ranges.is_empty() {
        RunOutcome {
            report: None,
            engine_gate_failed: false,
            warnings: vec![],
            resources: None,
        }
    } else {
        let ctx = RunContext {
            project_dir: project_dir.clone(),
            config,
            run_dir: run_dir.clone(),
        };
        let shard = Shard {
            index: 0,
            total: 1,
            scope: MutationScope {
                ranges: ranges.clone(),
            },
        };
        engine
            .run(&ctx, &shard)
            .await
            .with_context(|| format!("{} run failed", engine.id()))?
    };
    outcome.warnings.extend(git_warnings);

    if let Some(report) = &outcome.report {
        store::write_report(&run_dir, report)
            .with_context(|| format!("failed to write the report to {}", run_dir.display()))?;
    }

    let summary = Summary::new(
        run_id,
        run_dir,
        engine.id(),
        args.base,
        ranges,
        outcome,
        args.min_score,
    );
    let engine_version = engine.version(&project_dir).await;
    let environment = record::Environment::detect();
    let record = RunRecord::new(
        &summary,
        &git,
        engine_version.as_deref(),
        &started_at,
        &environment,
    );
    store::write_json(&summary.run_dir, "run.json", &record).with_context(|| {
        format!(
            "failed to write the run record to {}",
            summary.run_dir.display()
        )
    })?;
    Ok((summary, engine_version))
}

/// `--all`: every git-tracked file under `project_dir` (`git ls-files`, paths relative to it), each as
/// one range over all its lines, so each adapter intersects it with its own config as it does a diff.
/// Binary files (a NUL in the first 8000 bytes, git's own test), empty files and tracked files that
/// are gone from the work tree are skipped.
async fn all_ranges(project_dir: &Path) -> Result<Vec<FileRange>> {
    let out = tokio::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(project_dir)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .context("failed to run `git ls-files`")?;
    if !out.status.success() {
        anyhow::bail!(
            "`git ls-files` failed (exit code {:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let mut ranges = Vec::new();
    for rel in String::from_utf8_lossy(&out.stdout).split_terminator('\0') {
        let path = project_dir.join(rel);
        // Submodules (directories) and deleted files are not mutable files.
        if !tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_file()) {
            continue;
        }
        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?;
        if bytes[..bytes.len().min(8000)].contains(&0) {
            continue;
        }
        // `str::lines` counting: a last line without `\n` still counts.
        let lines = bytes.iter().filter(|&&b| b == b'\n').count()
            + usize::from(bytes.last().is_some_and(|&b| b != b'\n'));
        if lines > 0 {
            ranges.push(FileRange {
                path: rel.into(),
                start: 1,
                end: u32::try_from(lines).unwrap_or(u32::MAX),
            });
        }
    }
    // The core's order (by path, then start), as `changed_ranges` returns. A file with a merge
    // conflict is listed once per stage.
    ranges.sort_by(|a, b| a.path.cmp(&b.path));
    ranges.dedup_by(|a, b| a.path == b.path);
    Ok(ranges)
}

/// The engine and its config: `--engine`, else the engine `--config`'s file name belongs to, else the
/// one engine detected in the project. An explicit `--config` always wins over a detected one.
async fn select_engine(
    project_dir: &Path,
    id: Option<EngineId>,
    config: Option<PathBuf>,
) -> Result<(Engine, Option<PathBuf>)> {
    let engine = id
        .map(Engine::of)
        .or_else(|| config.as_deref().and_then(Engine::for_config));
    if let Some(engine) = engine {
        if config.is_some() {
            return Ok((engine, config));
        }
        let found = detection(&engine, project_dir).await?;
        return Ok((engine, found.and_then(|d| d.config)));
    }
    let mut found = Vec::new();
    for engine in Engine::all() {
        if let Some(d) = detection(&engine, project_dir).await? {
            found.push((engine, d));
        }
    }
    if found.len() > 1 {
        let list = found
            .iter()
            .map(|(e, d)| format!("{} ({})", e.id(), d.reason))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(CliError::AmbiguousEngine(project_dir.to_owned(), list).into());
    }
    match found.pop() {
        Some((engine, d)) => Ok((engine, config.or(d.config))),
        None => Err(CliError::NoEngine(project_dir.to_owned()).into()),
    }
}

async fn detection(engine: &Engine, project_dir: &Path) -> Result<Option<Detection>> {
    engine
        .detect(project_dir)
        .await
        .with_context(|| format!("{} detection failed", engine.id()))
}
