//! `rpr`: mutation testing scoped to your diff.

mod output;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
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
    /// Project directory (engine cwd).
    #[arg(long, value_name = "DIR", default_value = ".")]
    project: PathBuf,
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
    let args = match Cli::try_parse() {
        Ok(Cli {
            command: Command::Run(args),
        }) => args,
        Err(e) => return clap_exit(&e),
    };
    let json = args.json;
    match run(args).await {
        Ok(summary) => {
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
    }
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

async fn run(args: RunArgs) -> Result<Summary> {
    let started_at = record::rfc3339(SystemTime::now());
    // The real path, as the engine sees its cwd: Stryker reports `projectRoot: process.cwd()`, so a
    // `..` or a symlink here would put every mutant outside the scope. `dunce` avoids Windows' `\\?\`.
    let project_dir = dunce::canonicalize(&args.project)
        .with_context(|| format!("invalid project dir {}", args.project.display()))?;

    let (engine, config) = match args.config {
        Some(path) => {
            let path = std::path::absolute(&path)
                .with_context(|| format!("invalid config path {}", path.display()))?;
            if !path.is_file() {
                bail!("config file not found: {}", path.display());
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
    Ok(summary)
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
    bail!(
        "no supported mutation engine detected in {}",
        project_dir.display()
    )
}
