//! `rpr`: mutation testing scoped to your diff.

mod output;
mod telemetry;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use reapr_core::{
    AdapterError, Detection, MutationEngineAdapter, MutationScope, RunContext, RunOutcome, Shard,
    diff, record, store,
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
    /// Mutate the lines changed since <base> with the project's own engine config.
    Run(RunArgs),
    /// Show or change anonymous usage telemetry (see TELEMETRY.md).
    Telemetry {
        #[command(subcommand)]
        action: Option<telemetry::Action>,
    },
}

#[derive(clap::Args)]
struct RunArgs {
    /// Base ref to diff against (`<base>...HEAD`), e.g. origin/main.
    #[arg(long = "diff", value_name = "BASE", value_parser = parse_base)]
    base: String,
    /// Print a single JSON document on stdout instead of the human summary.
    #[arg(long)]
    json: bool,
    /// Fail (exit 1) when the mutation score is below this percentage.
    #[arg(long, value_name = "N", value_parser = parse_min_score)]
    min_score: Option<f64>,
    /// Engine config file; skips config detection.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Project directory (engine cwd). Default: the current directory.
    #[arg(long, value_name = "DIR")]
    project: Option<PathBuf>,
}

impl RunArgs {
    /// The names of the optional flags given, for telemetry (names only, never values).
    fn flag_names(&self) -> Vec<&'static str> {
        [
            ("json", self.json),
            ("min_score", self.min_score.is_some()),
            ("config", self.config.is_some()),
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

/// Static dispatch over the supported engines (the adapter trait is not dyn-compatible).
enum Engine {
    Stryker(StrykerAdapter),
}

impl Engine {
    fn all() -> [Engine; 1] {
        [Engine::Stryker(StrykerAdapter::new())]
    }

    fn id(&self) -> &'static str {
        match self {
            Engine::Stryker(a) => a.id(),
        }
    }

    async fn detect(&self, project_dir: &Path) -> Result<Option<Detection>, AdapterError> {
        match self {
            Engine::Stryker(a) => a.detect(project_dir).await,
        }
    }

    async fn run(&self, ctx: &RunContext, shard: &Shard) -> Result<RunOutcome, AdapterError> {
        match self {
            Engine::Stryker(a) => a.run(ctx, shard).await,
        }
    }

    async fn version(&self, project_dir: &Path) -> Option<String> {
        match self {
            Engine::Stryker(a) => a.engine_version(project_dir).await,
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
    telemetry::report(&props).await;
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

    let (engine, config) = match args.config {
        Some(path) => {
            let path =
                std::path::absolute(&path).map_err(|e| CliError::ConfigPath(path.clone(), e))?;
            if !path.is_file() {
                return Err(CliError::ConfigNotFound(path).into());
            }
            (Engine::Stryker(StrykerAdapter::new()), Some(path))
        }
        None => detect(&project_dir).await?,
    };

    let ranges = diff::changed_ranges(&project_dir, &args.base)
        .await
        .with_context(|| format!("failed to compute the diff vs {}", args.base))?;

    let git_dir = store::git_dir(&project_dir)
        .await
        .with_context(|| format!("failed to find the git dir of {}", project_dir.display()))?;
    let (run_id, run_dir) = store::new_run_dir(&git_dir)
        .with_context(|| format!("failed to create a run dir in {}", git_dir.display()))?;
    // Before the engine runs: the facts describe the commit that was tested.
    let (git, git_warnings) = record::git_facts(&project_dir, &args.base).await;

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

async fn detect(project_dir: &Path) -> Result<(Engine, Option<PathBuf>)> {
    for engine in Engine::all() {
        let found = engine
            .detect(project_dir)
            .await
            .with_context(|| format!("{} detection failed", engine.id()))?;
        if let Some(d) = found {
            return Ok((engine, d.config));
        }
    }
    Err(CliError::NoEngine(project_dir.to_owned()).into())
}
