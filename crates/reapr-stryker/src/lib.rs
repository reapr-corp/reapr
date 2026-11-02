//! StrykerJS adapter for Reapr.
//!
//! Runs the user's own Stryker config and overrides exactly two things: `mutate` (the diff's changed
//! ranges, intersected with the config's own `mutate`) and a `json` reporter written to the run dir.

mod schema;

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;

use reapr_core::resources::{children_usage, usage_since};
use reapr_core::{
    AdapterError, CacheKey, CoverageMap, Detection, FileRange, Mutant, MutationEngineAdapter,
    MutationScope, RunContext, RunOutcome, Shard,
};
use serde::Deserialize;
use tokio::process::Command;

pub use schema::{ScopedReport, parse_report};

const ENGINE: &str = "stryker";
const PLAN_JS: &str = include_str!("plan.mjs");
const NPX: &str = if cfg!(windows) { "npx.cmd" } else { "npx" };

/// Stryker's own lookup order (`SUPPORTED_CONFIG_FILE_NAMES` in
/// `@stryker-mutator/core/dist/src/config/config-file-formats.js`).
const CONFIG_FILES: [&str; 16] = [
    "stryker.conf.json",
    "stryker.conf.js",
    "stryker.conf.mjs",
    "stryker.conf.cjs",
    "stryker.config.json",
    "stryker.config.js",
    "stryker.config.mjs",
    "stryker.config.cjs",
    ".stryker.conf.json",
    ".stryker.conf.js",
    ".stryker.conf.mjs",
    ".stryker.conf.cjs",
    ".stryker.config.json",
    ".stryker.config.js",
    ".stryker.config.mjs",
    ".stryker.config.cjs",
];

#[derive(Debug, Default, Clone, Copy)]
pub struct StrykerAdapter;

impl StrykerAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Whether `file_name` is one of Stryker's own config file names.
    pub fn owns_config(file_name: &str) -> bool {
        CONFIG_FILES.contains(&file_name)
    }

    /// `version` of the `@stryker-mutator/core` that Node would resolve from `project_dir` (nearest
    /// `node_modules`, walking up). `None` when it is not installed or unreadable.
    pub async fn engine_version(&self, project_dir: &Path) -> Option<String> {
        for dir in project_dir.ancestors() {
            let pkg = dir.join("node_modules/@stryker-mutator/core/package.json");
            if let Ok(text) = tokio::fs::read_to_string(&pkg).await {
                let v: serde_json::Value = serde_json::from_str(&text).ok()?;
                return v.get("version")?.as_str().map(str::to_owned);
            }
        }
        None
    }
}

fn unsupported(operation: &'static str) -> AdapterError {
    AdapterError::Unsupported {
        engine: ENGINE,
        operation,
    }
}

fn io_err(context: impl std::fmt::Display) -> impl FnOnce(io::Error) -> AdapterError {
    move |source| AdapterError::Io {
        context: context.to_string(),
        source,
    }
}

fn absolute(path: &Path) -> Result<PathBuf, AdapterError> {
    std::path::absolute(path).map_err(io_err(format!("resolving {}", path.display())))
}

#[derive(Deserialize, Debug, PartialEq)]
struct Plan {
    /// Glob-escaped `file:a-b` patterns for Stryker.
    mutate: Vec<String>,
    /// The same selection as `mutate`, unescaped: the scope the report is filtered to.
    ranges: Vec<FileRange>,
    /// The user's config has a numeric `thresholds.break`.
    thresholds_break: bool,
    warnings: Vec<String>,
}

/// Whether a finished Stryker run failed its own gate. Stryker exits 1 for a score under a numeric
/// `thresholds.break`; every other non-zero exit is an error of the run, even with a report.
fn engine_gate_failed(
    code: Option<i32>,
    thresholds_break: bool,
    command: &str,
    report: &Path,
) -> Result<bool, AdapterError> {
    match code {
        Some(0) => Ok(false),
        Some(1) if thresholds_break => Ok(true),
        _ => Err(AdapterError::Process {
            command: command.to_owned(),
            code,
            detail: format!(
                "Stryker failed (not a thresholds.break failure); the report it wrote is at {}",
                report.display()
            ),
        }),
    }
}

/// Runs `plan.mjs`: the changed ranges that the user's own `mutate` patterns select.
async fn plan(
    project_dir: &Path,
    run_dir: &Path,
    config: &Path,
    ranges: &[FileRange],
) -> Result<Plan, AdapterError> {
    let script = run_dir.join("plan.mjs");
    let input = run_dir.join("plan-input.json");
    let input_json = serde_json::to_vec(&serde_json::json!({ "config": config, "ranges": ranges }))
        .map_err(|e| AdapterError::Parse {
            context: "plan input".into(),
            detail: e.to_string(),
        })?;
    tokio::fs::write(&script, PLAN_JS)
        .await
        .map_err(io_err(format!("writing {}", script.display())))?;
    tokio::fs::write(&input, input_json)
        .await
        .map_err(io_err(format!("writing {}", input.display())))?;

    let command = format!("node {} {}", script.display(), input.display());
    let out = Command::new("node")
        .arg(&script)
        .arg(&input)
        .current_dir(project_dir)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .await
        .map_err(io_err(format!("running `{command}`")))?;
    if !out.status.success() {
        return Err(AdapterError::Process {
            command,
            code: out.status.code(),
            detail: "planning the Stryker run failed, see stderr above".into(),
        });
    }
    serde_json::from_slice(&out.stdout).map_err(|e| AdapterError::Parse {
        context: format!("output of `{command}`"),
        detail: e.to_string(),
    })
}

/// The config Stryker runs: the user's config plus `mutate` and the json reporter, nothing else.
fn wrapper_source(config: &Path, mutate: &[String], report: &Path) -> Result<String, AdapterError> {
    let config_lit = js_lit("config path", config)?;
    let mutate_lit = js_lit("mutate", mutate)?;
    let report_lit = js_lit("report path", report)?;
    let is_json = config
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let attrs = if is_json {
        ", { with: { type: 'json' } }"
    } else {
        ""
    };
    Ok(format!(
        "// Generated by reapr: your Stryker config with only `mutate` and the json reporter overridden.\n\
         import {{ pathToFileURL }} from 'node:url';\n\
         \n\
         const {{ default: base }} = await import(pathToFileURL({config_lit}).href{attrs});\n\
         \n\
         export default {{\n  \
           ...base,\n  \
           mutate: {mutate_lit},\n  \
           reporters: [...new Set([...(base.reporters ?? ['clear-text', 'progress', 'html']), 'json'])],\n  \
           jsonReporter: {{ ...base.jsonReporter, fileName: {report_lit} }},\n\
         }};\n"
    ))
}

/// A JSON literal is a valid JS literal.
fn js_lit<T: serde::Serialize + ?Sized>(what: &str, value: &T) -> Result<String, AdapterError> {
    serde_json::to_string(value).map_err(|e| AdapterError::Parse {
        context: format!("{what} for the Stryker wrapper"),
        detail: e.to_string(),
    })
}

impl MutationEngineAdapter for StrykerAdapter {
    fn id(&self) -> &'static str {
        ENGINE
    }

    async fn detect(&self, project_dir: &Path) -> Result<Option<Detection>, AdapterError> {
        for name in CONFIG_FILES {
            let path = project_dir.join(name);
            if tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_file()) {
                return Ok(Some(Detection {
                    engine: ENGINE,
                    config: Some(path),
                    reason: format!("found {name}"),
                }));
            }
        }
        let pkg_path = project_dir.join("package.json");
        let text = match tokio::fs::read_to_string(&pkg_path).await {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_err(format!("reading {}", pkg_path.display()))(e)),
        };
        let pkg: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| AdapterError::Parse {
                context: pkg_path.display().to_string(),
                detail: e.to_string(),
            })?;
        for dep in ["@stryker-mutator/core", "vitest", "jest"] {
            for section in ["dependencies", "devDependencies"] {
                if pkg.get(section).and_then(|s| s.get(dep)).is_some() {
                    return Ok(Some(Detection {
                        engine: ENGINE,
                        config: None,
                        reason: format!(
                            "package.json {section} lists {dep}, but there is no Stryker config file"
                        ),
                    }));
                }
            }
        }
        Ok(None)
    }

    async fn list_mutants(
        &self,
        _ctx: &RunContext,
        _scope: &MutationScope,
    ) -> Result<Vec<Mutant>, AdapterError> {
        Err(unsupported("list_mutants"))
    }

    async fn coverage(
        &self,
        _ctx: &RunContext,
        _scope: &MutationScope,
    ) -> Result<CoverageMap, AdapterError> {
        Err(unsupported("coverage"))
    }

    async fn run(&self, ctx: &RunContext, shard: &Shard) -> Result<RunOutcome, AdapterError> {
        let project_dir = absolute(&ctx.project_dir)?;
        let config = match &ctx.config {
            None => {
                return Err(AdapterError::ConfigMissing {
                    engine: ENGINE,
                    hint: format!(
                        "no Stryker config file in {}; create one with `npx stryker init` or pass --config <path>",
                        project_dir.display()
                    ),
                });
            }
            Some(config) => absolute(config)?,
        };
        if !tokio::fs::metadata(&config)
            .await
            .is_ok_and(|m| m.is_file())
        {
            return Err(AdapterError::ConfigMissing {
                engine: ENGINE,
                hint: format!("{} does not exist", config.display()),
            });
        }
        let nothing = |warnings| RunOutcome {
            report: None,
            engine_gate_failed: false,
            warnings,
            resources: None,
        };
        if shard.scope.ranges.is_empty() {
            return Ok(nothing(vec![]));
        }

        let run_dir = absolute(&ctx.run_dir)?;
        let plan = plan(&project_dir, &run_dir, &config, &shard.scope.ranges).await?;
        if plan.mutate.is_empty() {
            return Ok(nothing(plan.warnings));
        }

        let report_path = run_dir.join("mutation.json");
        let wrapper = run_dir.join("stryker.wrapper.mjs");
        tokio::fs::write(
            &wrapper,
            wrapper_source(&config, &plan.mutate, &report_path)?,
        )
        .await
        .map_err(io_err(format!("writing {}", wrapper.display())))?;

        let command = format!("{NPX} --no-install stryker run {}", wrapper.display());
        let started = Instant::now();
        // Read right before spawn and right after wait: the delta is Stryker's process tree (the plan
        // step's node was already waited for). The peak RSS cannot be scoped that way and may be the
        // plan step's, though Stryker's is far larger in practice.
        let before = children_usage();
        // Stryker's stdout goes to our stderr: our stdout belongs to the report.
        let status = Command::new(NPX)
            .args(["--no-install", "stryker", "run"])
            .arg(&wrapper)
            .current_dir(&project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(io::stderr()))
            .stderr(Stdio::inherit())
            .status()
            .await
            .map_err(io_err(format!("running `{command}`")))?;
        let resources = before.zip(children_usage()).map(|(b, a)| usage_since(b, a));
        let duration = started.elapsed();

        let json = match tokio::fs::read_to_string(&report_path).await {
            Ok(json) => json,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(AdapterError::Process {
                    command,
                    code: status.code(),
                    detail: format!("Stryker wrote no report to {}", report_path.display()),
                });
            }
            Err(e) => return Err(io_err(format!("reading {}", report_path.display()))(e)),
        };
        let gate_failed =
            engine_gate_failed(status.code(), plan.thresholds_break, &command, &report_path)?;
        // Stryker's report is not the diff's: with `incremental` it appends old results from outside
        // the mutate ranges (incremental-differ.js, "old mutants that didn't run this time around").
        let scoped = parse_report(&json, ENGINE, &project_dir, duration, &plan.ranges)?;
        let mut warnings = plan.warnings;
        if scoped.out_of_scope > 0 {
            warnings.push(format!(
                "stryker report carried {} results from outside the diff scope (incremental); excluded from the diff score",
                scoped.out_of_scope
            ));
        }
        Ok(RunOutcome {
            report: Some(scoped.report),
            engine_gate_failed: gate_failed,
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

    fn node_available() -> bool {
        let ok = std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        if !ok {
            eprintln!("node not on PATH, skipping");
        }
        ok
    }

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn range(path: &str, start: u32, end: u32) -> FileRange {
        FileRange {
            path: path.into(),
            start,
            end,
        }
    }

    // ---- detect ----

    #[tokio::test]
    async fn detects_every_config_name() {
        for name in CONFIG_FILES {
            let dir = tempfile::tempdir().unwrap();
            write(&dir.path().join(name), "{}");
            let d = StrykerAdapter.detect(dir.path()).await.unwrap().unwrap();
            assert!(StrykerAdapter::owns_config(name));
            assert_eq!(d.engine, "stryker");
            assert_eq!(d.config, Some(dir.path().join(name)), "{name}");
        }
    }

    #[test]
    fn owns_only_stryker_config_names() {
        for name in ["mutants.toml", "stryker.config.ts", "vitest.config.mjs"] {
            assert!(!StrykerAdapter::owns_config(name), "{name}");
        }
    }

    #[tokio::test]
    async fn detect_follows_stryker_lookup_order() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(".stryker.config.mjs"), "");
        write(&dir.path().join("stryker.config.mjs"), "");
        write(&dir.path().join("stryker.conf.js"), "");
        let d = StrykerAdapter.detect(dir.path()).await.unwrap().unwrap();
        assert_eq!(d.config, Some(dir.path().join("stryker.conf.js")));
    }

    #[tokio::test]
    async fn detect_package_json_without_config() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("package.json"),
            r#"{ "devDependencies": { "vitest": "^4.1.5" } }"#,
        );
        let d = StrykerAdapter.detect(dir.path()).await.unwrap().unwrap();
        assert_eq!(d.config, None);
        assert!(d.reason.contains("vitest"), "{}", d.reason);

        write(
            &dir.path().join("package.json"),
            r#"{ "dependencies": { "react": "1" } }"#,
        );
        assert_eq!(StrykerAdapter.detect(dir.path()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn detect_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(StrykerAdapter.detect(dir.path()).await.unwrap(), None);
    }

    // ---- wrapper ----

    #[test]
    fn wrapper_escapes_literals_and_overrides_only_mutate_and_json_reporter() {
        let src = wrapper_source(
            Path::new(r#"/p/we"ird\dir/stryker.config.json"#),
            &["src/a.ts:1-2".into(), r#"src/q"uote.ts:3-3"#.into()],
            Path::new(r"/p/.reapr/runs/1-2/mutation.json"),
        )
        .unwrap();
        assert!(src.contains(r#"pathToFileURL("/p/we\"ird\\dir/stryker.config.json").href, { with: { type: 'json' } })"#), "{src}");
        assert!(
            src.contains(r#"mutate: ["src/a.ts:1-2","src/q\"uote.ts:3-3"],"#),
            "{src}"
        );
        assert!(
            src.contains(r#"fileName: "/p/.reapr/runs/1-2/mutation.json" }"#),
            "{src}"
        );
        assert!(src.contains("...base,"));

        let mjs = wrapper_source(Path::new("/p/stryker.config.mjs"), &[], Path::new("/r")).unwrap();
        assert!(
            mjs.contains(r#"pathToFileURL("/p/stryker.config.mjs").href)"#),
            "{mjs}"
        );
        assert!(!mjs.contains("with:"));
    }

    #[test]
    fn wrapper_evaluates_to_user_config_plus_overrides() {
        if !node_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        // Node rejects `\` in ESM paths (ERR_INVALID_MODULE_SPECIFIER), for Stryker too.
        let cfg_dir = dir.path().join(r#"we"ird dir"#);
        let config = cfg_dir.join("stryker.config.mjs");
        write(
            &config,
            "export default { testRunner: 'vitest', disableBail: false, concurrency: 3, \
             thresholds: { break: 80 }, reporters: ['html', 'json'], jsonReporter: { fileName: 'x.json' } };",
        );
        let wrapper = dir.path().join("stryker.wrapper.mjs");
        let report = dir.path().join("mutation.json");
        fs::write(
            &wrapper,
            wrapper_source(&config, &["src/a.ts:1-2".into()], &report).unwrap(),
        )
        .unwrap();
        let out = std::process::Command::new("node")
            .args([
                "--input-type=module",
                "-e",
                "import { pathToFileURL } from 'node:url'; \
                 const m = await import(pathToFileURL(process.argv[1]).href); \
                 console.log(JSON.stringify(m.default));",
            ])
            .arg(&wrapper)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "testRunner": "vitest", "disableBail": false, "concurrency": 3,
                "thresholds": { "break": 80 },
                "reporters": ["html", "json"],
                "jsonReporter": { "fileName": report },
                "mutate": ["src/a.ts:1-2"],
            })
        );
    }

    // ---- plan.mjs ----

    const DEFAULT_MUTATE: &str = r#"{ "properties": { "mutate": { "default": [
        "{src,lib}/**/!(*.+(s|S)pec|*.+(t|T)est).+(cjs|mjs|js|ts|mts|cts|jsx|tsx|html|vue|svelte)",
        "!{src,lib}/**/__tests__/**/*.+(cjs|mjs|js|ts|mts|cts|jsx|tsx|html|vue|svelte)"
    ] } } }"#;

    /// A project with a fake installed Stryker (schema only) and vitest 4.1.5.
    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let core = dir.path().join("node_modules/@stryker-mutator/core");
        write(
            &core.join("package.json"),
            r#"{ "name": "@stryker-mutator/core", "version": "9.6.1" }"#,
        );
        write(&core.join("schema/stryker-schema.json"), DEFAULT_MUTATE);
        write(
            &dir.path().join("node_modules/vitest/package.json"),
            r#"{ "name": "vitest", "version": "4.1.5", "exports": { "./package.json": "./package.json" } }"#,
        );
        dir
    }

    /// Copies `name` and its dependencies, resolved as siblings the way both pnpm's store and npm's
    /// flat layout place them, from `from` into `to/node_modules`.
    fn copy_package(name: &str, from: &Path, to: &Path) {
        let src = from.join(name).canonicalize().unwrap();
        let dest = to.join("node_modules").join(name);
        if dest.exists() {
            return;
        }
        copy_dir(&src, &dest);
        let pkg: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(src.join("package.json")).unwrap()).unwrap();
        if let Some(deps) = pkg["dependencies"].as_object() {
            for dep in deps.keys() {
                copy_package(dep, src.parent().unwrap(), to);
            }
        }
    }

    fn copy_dir(src: &Path, dest: &Path) {
        fs::create_dir_all(dest).unwrap();
        for entry in fs::read_dir(src).unwrap() {
            let path = entry.unwrap().path();
            let target = dest.join(path.file_name().unwrap());
            if path.is_dir() {
                copy_dir(&path, &target);
            } else {
                fs::copy(&path, &target).unwrap();
            }
        }
    }

    /// A real minimatch to copy (read-only sources): the dependencies of the `@stryker-mutator/core`
    /// install named by `REAPR_TEST_STRYKER_CORE`, else the copy bundled with the global npm.
    fn minimatch_source() -> Option<PathBuf> {
        let core = std::env::var_os("REAPR_TEST_STRYKER_CORE").map(PathBuf::from);
        if let Some(Ok(core)) = core.map(|c| c.canonicalize()) {
            // pnpm: <store>/node_modules/@stryker-mutator/core; its deps sit in <store>/node_modules.
            return core.parent()?.parent().map(Path::to_path_buf);
        }
        let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
        let out = std::process::Command::new(npm)
            .args(["root", "-g"])
            .output()
            .ok()?;
        let dir =
            PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("npm/node_modules");
        dir.join("minimatch").exists().then_some(dir)
    }

    /// [`project`] plus Stryker's real minimatch; `None` (skip) without node or a minimatch to copy.
    fn project_with_minimatch() -> Option<tempfile::TempDir> {
        if !node_available() {
            return None;
        }
        let Some(source) = minimatch_source() else {
            eprintln!("no minimatch to copy, skipping");
            return None;
        };
        let dir = project();
        copy_package("minimatch", &source, dir.path());
        Some(dir)
    }

    async fn run_plan(dir: &Path, config: &str, ranges: &[FileRange]) -> Plan {
        let run_dir = dir.join(".reapr/runs/1");
        fs::create_dir_all(&run_dir).unwrap();
        plan(dir, &run_dir, &dir.join(config), ranges)
            .await
            .unwrap()
    }

    const BAIL_WARNING: &str = "vitest >= 4 with bail can report killed mutants as survived (observed: 98.0 vs 87.8 on the same code); consider disableBail: true";

    #[tokio::test]
    async fn plan_intersects_with_mjs_mutate_in_stryker_order() {
        let Some(dir) = project_with_minimatch() else {
            return;
        };
        write(
            &dir.path().join("stryker.config.mjs"),
            "console.log('config side effect'); process.stdout.write('raw\\n');\
             export default { testRunner: 'vitest', mutate: [\
               'src/**/*.ts', '!src/**/*.spec.ts', '!src/generated/**', 'src/generated/keep.ts'] };",
        );
        let ranges = [
            range("src/a.ts", 1, 2),
            range("src/a.spec.ts", 1, 9),
            range("src/generated/x.ts", 1, 1),
            range("src/generated/keep.ts", 3, 3),
            range("other/b.ts", 1, 1),
            range("src/app/[id]/page.ts", 5, 6),
            range("src/.hidden/h.ts", 1, 1),
        ];
        let p = run_plan(dir.path(), "stryker.config.mjs", &ranges).await;
        assert_eq!(
            p.mutate,
            [
                "src/a.ts:1-2",
                "src/generated/keep.ts:3-3",
                "src/app/[[]id[]]/page.ts:5-6"
            ]
        );
        // The unescaped twin of `mutate`, used to filter the report.
        assert_eq!(
            p.ranges,
            [
                range("src/a.ts", 1, 2),
                range("src/generated/keep.ts", 3, 3),
                range("src/app/[id]/page.ts", 5, 6)
            ]
        );
        assert_eq!(p.warnings, [BAIL_WARNING]);
    }

    #[tokio::test]
    async fn plan_reads_json_config_and_respects_disable_bail() {
        let Some(dir) = project_with_minimatch() else {
            return;
        };
        write(
            &dir.path().join("stryker.config.json"),
            r#"{ "testRunner": "vitest", "disableBail": true, "mutate": ["lib/**/*.js"] }"#,
        );
        let ranges = [range("lib/x/a.js", 4, 8), range("src/a.ts", 1, 1)];
        let p = run_plan(dir.path(), "stryker.config.json", &ranges).await;
        assert_eq!(
            p,
            Plan {
                mutate: vec!["lib/x/a.js:4-8".into()],
                ranges: vec![range("lib/x/a.js", 4, 8)],
                thresholds_break: false,
                warnings: vec![]
            }
        );
    }

    #[tokio::test]
    async fn plan_uses_installed_stryker_default_mutate() {
        let Some(dir) = project_with_minimatch() else {
            return;
        };
        write(
            &dir.path().join("stryker.config.mjs"),
            "export default { testRunner: 'vitest' };",
        );
        let ranges = [
            range("src/a.ts", 1, 1),
            range("src/a.spec.ts", 1, 1),
            range("src/__tests__/t.ts", 1, 1),
            range("lib/b.mjs", 2, 3),
            range("test/c.ts", 1, 1),
        ];
        let p = run_plan(dir.path(), "stryker.config.mjs", &ranges).await;
        assert_eq!(p.mutate, ["src/a.ts:1-1", "lib/b.mjs:2-3"]);
        assert_eq!(p.warnings, [BAIL_WARNING]);
    }

    /// Stryker's minimatch is case-sensitive on every OS; `path.matchesGlob` is not on macOS and
    /// Windows. Also reports a numeric `thresholds.break`.
    #[tokio::test]
    async fn plan_matches_case_sensitively_and_reports_thresholds_break() {
        let Some(dir) = project_with_minimatch() else {
            return;
        };
        write(
            &dir.path().join("stryker.config.mjs"),
            "export default { thresholds: { break: 80 }, mutate: ['src/**/*.ts', '!src/**/*.test.ts'] };",
        );
        let ranges = [
            range("src/Foo.Test.ts", 1, 1),
            range("src/foo.test.ts", 1, 1),
            range("SRC/bar.ts", 1, 1),
        ];
        let p = run_plan(dir.path(), "stryker.config.mjs", &ranges).await;
        assert_eq!(p.mutate, ["src/Foo.Test.ts:1-1"]);
        assert!(p.thresholds_break);

        write(
            &dir.path().join("stryker.config.mjs"),
            "export default { thresholds: { break: null } };",
        );
        let p = run_plan(dir.path(), "stryker.config.mjs", &ranges).await;
        assert!(!p.thresholds_break);
    }

    #[tokio::test]
    async fn plan_without_strykers_minimatch_fails() {
        if !node_available() {
            return;
        }
        let dir = project();
        write(
            &dir.path().join("stryker.config.mjs"),
            "export default { mutate: ['src/**'] };",
        );
        let run_dir = dir.path().join("run");
        fs::create_dir_all(&run_dir).unwrap();
        let err = plan(
            dir.path(),
            &run_dir,
            &dir.path().join("stryker.config.mjs"),
            &[range("src/a.ts", 1, 1)],
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AdapterError::Process { .. }), "{err:?}");
    }

    #[test]
    fn engine_gate_only_for_exit_1_with_numeric_break() {
        let gate = |code, brk| {
            engine_gate_failed(
                code,
                brk,
                "npx stryker run w",
                Path::new("/r/mutation.json"),
            )
        };
        assert!(!gate(Some(0), false).unwrap());
        assert!(!gate(Some(0), true).unwrap());
        assert!(gate(Some(1), true).unwrap());
        for (code, brk) in [
            (Some(1), false),
            (Some(2), true),
            (Some(137), true),
            (None, true),
        ] {
            match gate(code, brk) {
                Err(AdapterError::Process {
                    code: c, detail, ..
                }) => {
                    assert_eq!(c, code);
                    assert!(detail.contains("/r/mutation.json"), "{detail}");
                }
                other => panic!("{code:?} {brk}: expected Process, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn plan_failure_is_process_error() {
        let Some(dir) = project_with_minimatch() else {
            return;
        };
        write(
            &dir.path().join("stryker.config.mjs"),
            "export const notDefault = {};",
        );
        let run_dir = dir.path().join("run");
        fs::create_dir_all(&run_dir).unwrap();
        let err = plan(
            dir.path(),
            &run_dir,
            &dir.path().join("stryker.config.mjs"),
            &[range("src/a.ts", 1, 1)],
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, AdapterError::Process { code: Some(1), .. }),
            "{err:?}"
        );
    }

    // ---- run ----

    fn ctx(dir: &Path, config: Option<&str>) -> RunContext {
        RunContext {
            project_dir: dir.into(),
            config: config.map(|c| dir.join(c)),
            run_dir: dir.join(".reapr/runs/1"),
        }
    }

    fn shard(ranges: Vec<FileRange>) -> Shard {
        Shard {
            index: 0,
            total: 1,
            scope: MutationScope { ranges },
        }
    }

    #[tokio::test]
    async fn run_without_config_is_config_missing() {
        let dir = tempfile::tempdir().unwrap();
        for config in [None, Some("stryker.config.mjs")] {
            let err = StrykerAdapter
                .run(
                    &ctx(dir.path(), config),
                    &shard(vec![range("src/a.ts", 1, 1)]),
                )
                .await
                .unwrap_err();
            match err {
                AdapterError::ConfigMissing { hint, .. } if config.is_none() => {
                    assert!(hint.contains("npx stryker init"), "{hint}")
                }
                AdapterError::ConfigMissing { .. } => {}
                other => panic!("expected ConfigMissing, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn run_with_nothing_in_scope_does_not_start_stryker() {
        let Some(dir) = project_with_minimatch() else {
            return;
        };
        write(
            &dir.path().join("stryker.config.mjs"),
            "export default { testRunner: 'vitest', mutate: ['src/**'] };",
        );
        fs::create_dir_all(dir.path().join(".reapr/runs/1")).unwrap();
        let out = StrykerAdapter
            .run(
                &ctx(dir.path(), Some("stryker.config.mjs")),
                &shard(vec![range("docs/x.md", 1, 1)]),
            )
            .await
            .unwrap();
        assert_eq!(out.report, None);
        assert!(!out.engine_gate_failed);
        assert_eq!(out.warnings, [BAIL_WARNING]);
        assert!(
            !dir.path()
                .join(".reapr/runs/1/stryker.wrapper.mjs")
                .exists()
        );
    }

    /// [`project_with_minimatch`] with `mutate: ['src/**']` and a fake `stryker` bin, which
    /// `npx --no-install` runs from the project's `node_modules/.bin`. `script` is its sh body; the
    /// wrapper path is `$2`, so the report path is `"$(dirname "$2")/mutation.json"`.
    #[cfg(unix)]
    fn project_with_fake_stryker(script: &str) -> Option<tempfile::TempDir> {
        use std::os::unix::fs::PermissionsExt;
        let dir = project_with_minimatch()?;
        write(
            &dir.path().join("stryker.config.mjs"),
            "export default { mutate: ['src/**'] };",
        );
        write(&dir.path().join("src/a.ts"), "export const a = 1;\n");
        let bin = dir.path().join("node_modules/.bin/stryker");
        write(&bin, &format!("#!/bin/sh\n{script}\n"));
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(dir.path().join(".reapr/runs/1")).unwrap();
        Some(dir)
    }

    /// A v2 report for `src/a.ts` with one mutant per line in `lines`, written by the fake stryker.
    #[cfg(unix)]
    fn write_report_script(lines: &[u32]) -> String {
        let mutants: Vec<String> = lines
            .iter()
            .map(|l| {
                format!(
                    r#"{{ "id": "{l}", "mutatorName": "M", "status": "Killed",
                        "location": {{ "start": {{ "line": {l}, "column": 1 }}, "end": {{ "line": {l}, "column": 2 }} }} }}"#
                )
            })
            .collect();
        format!(
            "cat > \"$(dirname \"$2\")/mutation.json\" <<'EOF'\n{{ \"files\": {{ \"src/a.ts\": {{ \"mutants\": [{}] }} }} }}\nEOF",
            mutants.join(", ")
        )
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_parses_the_report_stryker_wrote() {
        let Some(dir) = project_with_fake_stryker(&write_report_script(&[1])) else {
            return;
        };
        let out = StrykerAdapter
            .run(
                &ctx(dir.path(), Some("stryker.config.mjs")),
                &shard(vec![range("src/a.ts", 1, 1)]),
            )
            .await
            .unwrap();
        let report = out.report.unwrap();
        assert_eq!(report.results.len(), 1);
        assert_eq!(report.results[0].mutant.file, Path::new("src/a.ts"));
        assert!(!out.engine_gate_failed);
        assert_eq!(out.warnings, Vec::<String>::new());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_excludes_results_outside_the_scope_with_a_warning() {
        // Line 5 is what Stryker's `incremental` carries over from an earlier run.
        let Some(dir) = project_with_fake_stryker(&write_report_script(&[1, 5])) else {
            return;
        };
        let out = StrykerAdapter
            .run(
                &ctx(dir.path(), Some("stryker.config.mjs")),
                &shard(vec![range("src/a.ts", 1, 1)]),
            )
            .await
            .unwrap();
        assert_eq!(out.report.unwrap().results.len(), 1);
        // The count is the contract (docs/design/phase-1.md), not the wording.
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert!(
            out.warnings[0].contains("carried 1 results from outside the diff scope"),
            "{}",
            out.warnings[0]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_without_a_report_is_a_process_error() {
        let Some(dir) = project_with_fake_stryker("exit 0") else {
            return;
        };
        let err = StrykerAdapter
            .run(
                &ctx(dir.path(), Some("stryker.config.mjs")),
                &shard(vec![range("src/a.ts", 1, 1)]),
            )
            .await
            .unwrap_err();
        match err {
            AdapterError::Process { code, detail, .. } => {
                assert_eq!(code, Some(0));
                assert!(detail.contains("wrote no report"), "{detail}");
            }
            other => panic!("expected Process, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn engine_version_from_nearest_node_modules() {
        let dir = project();
        let pkg = dir.path().join("packages/app");
        fs::create_dir_all(&pkg).unwrap();
        assert_eq!(
            StrykerAdapter.engine_version(&pkg).await.as_deref(),
            Some("9.6.1")
        );
        let none = tempfile::tempdir().unwrap();
        assert_eq!(StrykerAdapter.engine_version(none.path()).await, None);
    }

    // ---- conformance kit ----

    #[tokio::test]
    async fn conformance_empty_scope_reports_none() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("stryker.config.mjs"), "export default {};");
        let c = ctx(dir.path(), Some("stryker.config.mjs"));
        fs::create_dir_all(&c.run_dir).unwrap();
        reapr_core::testkit::empty_scope_reports_none(&StrykerAdapter, &c).await;
    }

    #[test]
    fn conformance_status_mapping() {
        use reapr_core::MutantStatus::*;
        let parse = |status: &str| {
            let json = format!(
                r#"{{ "files": {{ "a.ts": {{ "mutants": [{{ "id": "1", "mutatorName": "M", "status": "{status}",
                    "location": {{ "start": {{ "line": 1, "column": 1 }}, "end": {{ "line": 1, "column": 2 }} }} }}] }} }} }}"#
            );
            let r = parse_report(
                &json,
                ENGINE,
                Path::new("/p"),
                std::time::Duration::ZERO,
                &[range("a.ts", 1, 1)],
            )?;
            Ok(r.report.results[0].status)
        };
        reapr_core::testkit::status_mapping(
            parse,
            &[
                ("Killed", Killed),
                ("Survived", Survived),
                ("NoCoverage", NoCoverage),
                ("Timeout", Timeout),
                ("CompileError", CompileError),
                ("RuntimeError", RuntimeError),
                ("Ignored", Ignored),
                ("Pending", Pending),
            ],
        );
    }

    #[tokio::test]
    async fn conformance_unsupported_operations() {
        use reapr_core::testkit::Operation::*;
        let dir = tempfile::tempdir().unwrap();
        reapr_core::testkit::unsupported(
            &StrykerAdapter,
            &ctx(dir.path(), None),
            &[ListMutants, Coverage, CacheKey],
        )
        .await;
        assert_eq!(StrykerAdapter::new().id(), "stryker");
    }
}
