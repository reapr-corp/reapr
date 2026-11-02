//! Anonymous, opt-out usage telemetry: one `run_completed` event per `rpr run`, sent to PostHog after
//! the output is printed. What is sent, what never is, and how to turn it off: TELEMETRY.md.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::time::{Duration, Instant, SystemTime};

use reapr_core::diff::DiffError;
use reapr_core::{AdapterError, StatusCounts, record};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::CliError;
use crate::output::Summary;

/// PostHog project API key (Reapr org, project 705587). Public and write-only by design: it can only
/// ingest events. If it were empty, telemetry would be a silent no-op (no notice, no state file, nothing
/// sent); `RPR_TELEMETRY_DEBUG=1` prints the payload that would be sent.
pub const POSTHOG_PROJECT_KEY: &str = "phc_oLXpAoTpY2pSPHPhnTzcKParQPjgipUz5WUrRdojynyP";
const CAPTURE_URL: &str = "https://us.i.posthog.com/i/v0/e/";
/// The whole send (curl's own `-m 1` included) is cut off after this.
const SEND_TIMEOUT: Duration = Duration::from_millis(1500);
const NOTICE: &str = "rpr collects anonymous usage data (no code, paths or names) to improve the tool. \
Disable: rpr telemetry disable or DO_NOT_TRACK=1. \
Details: https://github.com/reapr-corp/reapr/blob/main/TELEMETRY.md";

/// Reads an environment variable. A parameter, so tests never touch the process environment.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

pub fn os_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

#[derive(clap::Subcommand, Clone, Copy, Debug)]
pub enum Action {
    /// Print whether telemetry is on and why, the install id and the state file (the default).
    Status,
    /// Turn telemetry on (saved in the state file).
    Enable,
    /// Turn telemetry off (saved in the state file).
    Disable,
}

/// Why telemetry is on or off, in precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    DoNotTrack,
    RprTelemetry,
    Setting,
    Default,
}

impl Why {
    fn label(self) -> &'static str {
        match self {
            Why::DoNotTrack => "env: DO_NOT_TRACK",
            Why::RprTelemetry => "env: RPR_TELEMETRY",
            Why::Setting => "setting",
            Why::Default => "default",
        }
    }
}

/// `DO_NOT_TRACK` (any non-empty value but `0`) > `RPR_TELEMETRY` > the saved setting > on.
pub fn decide(env: Env, setting: Option<bool>) -> (bool, Why) {
    if env("DO_NOT_TRACK").is_some_and(|v| !v.is_empty() && v != "0") {
        return (false, Why::DoNotTrack);
    }
    match env("RPR_TELEMETRY")
        .map(|v| v.to_ascii_lowercase())
        .as_deref()
    {
        Some("0" | "false" | "off" | "no") => return (false, Why::RprTelemetry),
        Some("1" | "true" | "on" | "yes") => return (true, Why::RprTelemetry),
        _ => {}
    }
    match setting {
        Some(on) => (on, Why::Setting),
        None => (true, Why::Default),
    }
}

/// `telemetry.json`. Unknown or missing fields read as their defaults.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(default)]
pub struct State {
    pub install_id: String,
    pub enabled: Option<bool>,
    pub notice_shown: bool,
}

/// `${XDG_CONFIG_HOME:-$HOME/.config}/reapr/telemetry.json`; `None` without either.
pub fn state_path(env: Env) -> Option<PathBuf> {
    let set = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = set("XDG_CONFIG_HOME")
        .filter(|p| p.is_absolute())
        .or_else(|| set("HOME").map(|h| h.join(".config")))?;
    Some(base.join("reapr").join("telemetry.json"))
}

/// `None` when the file is missing or unreadable.
fn load(path: &Path) -> Option<State> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn save(path: &Path, state: &State) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(state)?)
}

/// Random UUID v4.
fn new_uuid() -> String {
    let mut b = [0u8; 16];
    let urandom = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    if urandom.is_err() {
        // ponytail: std's randomly keyed hasher where /dev/urandom is missing; getrandom if a non-Linux
        // target needs better.
        use std::hash::BuildHasher;
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        for (i, chunk) in b.chunks_mut(8).enumerate() {
            let h = std::hash::RandomState::new().hash_one((nanos, i, std::process::id()));
            chunk.copy_from_slice(&h.to_le_bytes());
        }
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// Only an id we could have written is ever sent: a hand-edited `install_id` is replaced.
fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_digit() || ('a'..='f').contains(&c),
        })
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    MinScoreFailed,
    EngineGateFailed,
    NothingToMutate,
    Error,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    ConfigMissing,
    NoEngine,
    AmbiguousEngine,
    Git,
    EngineProcess,
    Parse,
    Io,
    Args,
    Other,
}

/// The kind of a run error, from the first typed error in its chain. Never its message.
pub fn error_kind(e: &anyhow::Error) -> ErrorKind {
    for cause in e.chain() {
        if let Some(c) = cause.downcast_ref::<CliError>() {
            return match c {
                CliError::Project(..) | CliError::ConfigPath(..) => ErrorKind::Args,
                CliError::ConfigNotFound(_) => ErrorKind::ConfigMissing,
                CliError::NoEngine(_) => ErrorKind::NoEngine,
                CliError::AmbiguousEngine(..) => ErrorKind::AmbiguousEngine,
            };
        }
        if let Some(a) = cause.downcast_ref::<AdapterError>() {
            return match a {
                AdapterError::ConfigMissing { .. } => ErrorKind::ConfigMissing,
                AdapterError::Process { .. } => ErrorKind::EngineProcess,
                AdapterError::Parse { .. } => ErrorKind::Parse,
                AdapterError::Io { .. } => ErrorKind::Io,
                AdapterError::Unsupported { .. } => ErrorKind::Other,
            };
        }
        if cause.is::<DiffError>() {
            return ErrorKind::Git;
        }
        if cause.is::<std::io::Error>() {
            return ErrorKind::Io;
        }
    }
    ErrorKind::Other
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    VitestBail,
    IncrementalCarriedOver,
    GitMetadata,
    EngineVersionUnsupported,
    Other,
}

// ponytail: matched on the adapter's warning text; typed warnings once there are more than a handful.
const CARRIED_PREFIX: &str = "stryker report carried ";

fn warning_kind(w: &str) -> WarningKind {
    if w.starts_with("vitest >= 4 with bail") {
        WarningKind::VitestBail
    } else if w.starts_with(CARRIED_PREFIX) {
        WarningKind::IncrementalCarriedOver
    } else if w.starts_with("git metadata:") {
        WarningKind::GitMetadata
    } else if w.contains(" is outside the supported range ") {
        WarningKind::EngineVersionUnsupported
    } else {
        WarningKind::Other
    }
}

/// Results excluded from the diff score because an incremental report carried them over.
fn carried_over(warnings: &[String]) -> usize {
    warnings
        .iter()
        .filter_map(|w| {
            w.strip_prefix(CARRIED_PREFIX)?
                .split(' ')
                .next()?
                .parse::<usize>()
                .ok()
        })
        .sum()
}

pub fn bucket(n: usize) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2..=5 => "2-5",
        6..=20 => "6-20",
        21..=100 => "21-100",
        _ => "101+",
    }
}

fn ci_provider(env: Env) -> Option<&'static str> {
    [
        ("GITHUB_ACTIONS", "github_actions"),
        ("GITLAB_CI", "gitlab"),
        ("CIRCLECI", "circleci"),
        ("BUILDKITE", "buildkite"),
        ("JENKINS_URL", "jenkins"),
        ("CI", "other"),
    ]
    .into_iter()
    .find(|(k, _)| env(k).is_some_and(|v| !v.is_empty()))
    .map(|(_, provider)| provider)
}

/// How the run was started: `local` outside CI; GitHub's `GITHUB_EVENT_NAME` (an allow-listed value, else
/// `other`); GitLab's `CI_PIPELINE_SOURCE`, mapped to the same names; `None` on other CI providers.
fn trigger(env: Env, ci_provider: Option<&str>) -> Option<&'static str> {
    const GITHUB: [&str; 6] = [
        "pull_request",
        "pull_request_target",
        "push",
        "schedule",
        "workflow_dispatch",
        "merge_group",
    ];
    match ci_provider {
        None => Some("local"),
        Some("github_actions") => {
            let event = env("GITHUB_EVENT_NAME");
            Some(
                GITHUB
                    .into_iter()
                    .find(|e| event.as_deref() == Some(e))
                    .unwrap_or("other"),
            )
        }
        Some("gitlab") => Some(match env("CI_PIPELINE_SOURCE").as_deref() {
            Some("merge_request_event") => "pull_request",
            Some("push") => "push",
            Some("schedule") => "schedule",
            Some("web") => "manual",
            _ => "other",
        }),
        Some(_) => None,
    }
}

/// The coding agent running `rpr`, from which variables are set (never their values). Each marker is
/// one the agent documents (or sets in its official source) for the commands it spawns; see TELEMETRY.md.
fn agent(env: Env) -> Option<&'static str> {
    [
        ("CLAUDECODE", "claude_code"),
        ("CURSOR_AGENT", "cursor"),
        ("CODEX_THREAD_ID", "codex"),
        ("CODEX_SANDBOX", "codex"),
        ("CODEX_SANDBOX_NETWORK_DISABLED", "codex"),
        ("GEMINI_CLI", "gemini_cli"),
    ]
    .into_iter()
    .find(|(k, _)| env(k).is_some_and(|v| !v.is_empty()))
    .map(|(_, agent)| agent)
}

/// Mixed into [`project_id`] so the hash is specific to Reapr telemetry. Never change it: every project
/// would get a new id.
const PROJECT_SALT: &str = "reapr-telemetry-v1:";
/// `git rev-list` walks the whole history; on a very large repo it is cut off and the id is `null`.
const GIT_TIMEOUT: Duration = Duration::from_secs(1);

/// The fingerprint of `git rev-list --max-parents=0 HEAD` output: hex SHA-256 of the salt plus the
/// root commit shas, sorted and joined with `,`, truncated to 32 hex chars.
fn fingerprint(roots: &str) -> Option<String> {
    let mut shas: Vec<&str> = roots.split_whitespace().collect();
    if shas.is_empty() {
        return None;
    }
    shas.sort_unstable();
    let digest = Sha256::digest(format!("{PROJECT_SALT}{}", shas.join(",")));
    Some(digest.iter().take(16).map(|b| format!("{b:02x}")).collect())
}

/// `git <args>` in `dir`: its stdout, or `None` on any failure or after [`GIT_TIMEOUT`].
fn git_stdout(dir: &Path, args: &[&str]) -> Option<String> {
    let mut child = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + GIT_TIMEOUT;
    // ponytail: polls; output past the pipe buffer (~1500 root commits) blocks git until the timeout.
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout?.read_to_string(&mut out).ok()?;
    Some(out)
}

/// Anonymous project fingerprint (see [`fingerprint`]); `None` outside a git repo, in a shallow clone
/// (its root is the shallow boundary, a different commit on every run) or when git fails.
pub fn project_id(dir: &Path) -> Option<String> {
    if git_stdout(dir, &["rev-parse", "--is-shallow-repository"])?.trim() != "false" {
        return None;
    }
    fingerprint(&git_stdout(dir, &["rev-list", "--max-parents=0", "HEAD"])?)
}

/// Versions are free text in a package.json: anything but `[0-9A-Za-z.+-]` drops them.
fn safe_version(v: &str) -> Option<String> {
    (!v.is_empty()
        && v.len() <= 64
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-')))
    .then(|| v.to_owned())
}

/// The event's properties. Every value is a number, a bool, a fixed enum or a bucket, except the two
/// versions (filtered by [`safe_version`]) and `project_id` (a truncated hash, [`project_id`]).
#[derive(Serialize, Debug, Clone)]
pub struct Properties {
    rpr_version: &'static str,
    os: &'static str,
    arch: &'static str,
    ci: bool,
    ci_provider: Option<&'static str>,
    runner: Option<&'static str>,
    trigger: Option<&'static str>,
    via_action: bool,
    agent: Option<&'static str>,
    first_run: Option<bool>,
    build: &'static str,
    project_id: Option<String>,
    cpu_count: Option<usize>,
    flags: Vec<&'static str>,
    engine: Option<String>,
    engine_version: Option<String>,
    outcome: Outcome,
    error_kind: Option<ErrorKind>,
    warning_kinds: BTreeSet<WarningKind>,
    duration_ms_total: u64,
    duration_ms_engine: Option<u64>,
    mutants_in_scope: Option<u32>,
    counts: Option<StatusCounts>,
    score: Option<f64>,
    survivors: Option<usize>,
    ranges_bucket: Option<&'static str>,
    files_bucket: Option<&'static str>,
    carried_over_excluded: Option<&'static str>,
    engine_cpu_ms: Option<u64>,
    engine_max_rss_kb: Option<u64>,
    #[serde(rename = "$process_person_profile")]
    process_person_profile: bool,
}

impl Properties {
    /// `run`: the summary and the engine version, or the run's error.
    pub fn new(
        env: Env,
        flags: Vec<&'static str>,
        run: Result<(&Summary, Option<&str>), &anyhow::Error>,
        total: Duration,
    ) -> Self {
        let ci_provider = ci_provider(env);
        let runner = match (ci_provider, env("RUNNER_ENVIRONMENT").as_deref()) {
            (None, _) => Some("local"),
            (Some(_), Some("github-hosted")) => Some("github_hosted"),
            (Some(_), Some("self-hosted")) => Some("self_hosted"),
            _ => None,
        };
        let mut p = Properties {
            rpr_version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            ci: ci_provider.is_some(),
            ci_provider,
            runner,
            trigger: trigger(env, ci_provider),
            via_action: env("RPR_VIA_ACTION").as_deref() == Some("1"),
            agent: agent(env),
            // Set by `prepare`, which knows whether the state file was created and the run is sent.
            first_run: None,
            build: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            project_id: None,
            cpu_count: std::thread::available_parallelism().ok().map(|n| n.get()),
            flags,
            engine: None,
            engine_version: None,
            outcome: Outcome::Error,
            error_kind: None,
            warning_kinds: BTreeSet::new(),
            duration_ms_total: u64::try_from(total.as_millis()).unwrap_or(u64::MAX),
            duration_ms_engine: None,
            mutants_in_scope: None,
            counts: None,
            score: None,
            survivors: None,
            ranges_bucket: None,
            files_bucket: None,
            carried_over_excluded: None,
            engine_cpu_ms: None,
            engine_max_rss_kb: None,
            process_person_profile: false,
        };
        match run {
            Err(e) => p.error_kind = Some(error_kind(e)),
            Ok((s, engine_version)) => {
                let c = s.counts;
                let files: BTreeSet<&Path> = s.ranges.iter().map(|r| r.path.as_path()).collect();
                p.engine = safe_version(&s.engine);
                p.engine_version = engine_version.and_then(safe_version);
                p.outcome = if s.engine_gate_failed {
                    Outcome::EngineGateFailed
                } else if !s.passed {
                    Outcome::MinScoreFailed
                } else if s.nothing_to_mutate {
                    Outcome::NothingToMutate
                } else {
                    Outcome::Passed
                };
                p.warning_kinds = s.warnings.iter().map(|w| warning_kind(w)).collect();
                p.duration_ms_engine = Some(s.duration_ms);
                p.mutants_in_scope = Some(
                    c.killed
                        + c.survived
                        + c.no_coverage
                        + c.timeout
                        + c.compile_error
                        + c.runtime_error
                        + c.ignored
                        + c.pending,
                );
                p.counts = Some(c);
                p.score = s.score.map(|x| (x * 10.0).round() / 10.0);
                p.survivors = Some(s.survivors.len());
                p.ranges_bucket = Some(bucket(s.ranges.len()));
                p.files_bucket = Some(bucket(files.len()));
                p.carried_over_excluded = Some(bucket(carried_over(&s.warnings)));
                p.engine_cpu_ms = s.resources.map(|r| r.cpu_user_ms + r.cpu_sys_ms);
                p.engine_max_rss_kb = s.resources.map(|r| r.max_rss_kb);
            }
        }
        p
    }
}

#[derive(Serialize)]
struct Payload<'a> {
    api_key: &'a str,
    event: &'static str,
    distinct_id: &'a str,
    timestamp: String,
    properties: &'a Properties,
}

/// What to do once the state file is settled.
#[derive(Debug, Default, PartialEq, Eq)]
struct Prepared {
    notice: bool,
    /// `RPR_TELEMETRY_DEBUG=1`: printed, never sent.
    print: Option<String>,
    send: Option<String>,
}

/// Decides, updates the state file, and builds the payload. Spawns nothing but `project_id`, called only
/// when the event is built.
fn prepare(
    key: &str,
    env: Env,
    props: &Properties,
    project_id: impl FnOnce() -> Option<String>,
) -> Prepared {
    let debug = env("RPR_TELEMETRY_DEBUG").as_deref() == Some("1");
    if key.is_empty() && !debug {
        return Prepared::default();
    }
    // Nothing persists in CI (a fresh id and the notice on every run), nor without a key.
    let path = (!props.ci && !key.is_empty())
        .then(|| state_path(env))
        .flatten();
    let stored = path.as_deref().and_then(load);
    let mut state = stored.clone().unwrap_or_default();
    if !decide(env, state.enabled).0 {
        return Prepared::default();
    }
    if !is_uuid(&state.install_id) {
        state.install_id = new_uuid();
    }
    let notice = !key.is_empty() && !state.notice_shown;
    state.notice_shown = true;
    // Unwritable: the next run shows the notice again (and `first_run` is false), nothing else changes.
    let saved = match &path {
        Some(path) if stored.as_ref() != Some(&state) => save(path, &state).is_ok(),
        _ => false,
    };
    let props = &Properties {
        first_run: path.is_some().then_some(stored.is_none() && saved),
        project_id: project_id(),
        ..props.clone()
    };
    let payload = serde_json::to_string(&Payload {
        api_key: key,
        event: "run_completed",
        distinct_id: &state.install_id,
        timestamp: record::rfc3339(SystemTime::now()),
        properties: props,
    })
    .unwrap_or_default();
    let (print, send) = if debug {
        (Some(payload), None)
    } else {
        (None, Some(payload))
    };
    Prepared {
        notice,
        print,
        send,
    }
}

/// Shows the notice if due, then prints (debug) or sends the event. Silent on every failure.
/// `project`: the `--project` dir, for [`project_id`].
pub async fn report(props: &Properties, project: &Path) {
    let p = prepare(POSTHOG_PROJECT_KEY, &os_env, props, || project_id(project));
    if p.notice {
        eprintln!("{NOTICE}");
    }
    if let Some(json) = p.print {
        eprintln!("rpr telemetry (RPR_TELEMETRY_DEBUG=1, not sent):\n{json}");
    }
    if let Some(json) = p.send {
        send(&json).await;
    }
}

/// POSTs through `curl`, killed after [`SEND_TIMEOUT`]. No curl on PATH: nothing happens.
async fn send(json: &str) {
    let Ok(mut child) = tokio::process::Command::new("curl")
        .args(["-sS", "-m", "1", "-X", "POST"])
        .args([
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
        ])
        .arg(CAPTURE_URL)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    else {
        return;
    };
    let stdin = child.stdin.take();
    let finished = tokio::time::timeout(SEND_TIMEOUT, async {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(json.as_bytes()).await;
            // Dropped here: EOF, so curl sends.
        }
        child.wait().await
    })
    .await;
    if finished.is_err() {
        let _ = child.kill().await;
    }
}

/// `rpr telemetry [status|enable|disable]`. Never sends anything.
pub fn command(action: Action) -> ExitCode {
    let env: Env = &os_env;
    let path = state_path(env);
    let enabled = match action {
        Action::Status => None,
        Action::Enable => Some(true),
        Action::Disable => Some(false),
    };
    if let Some(on) = enabled {
        let Some(path) = &path else {
            eprintln!("error: no config dir to save the setting in (set HOME or XDG_CONFIG_HOME)");
            return ExitCode::from(2);
        };
        let mut state = load(path).unwrap_or_default();
        if !is_uuid(&state.install_id) {
            state.install_id = new_uuid();
        }
        state.enabled = Some(on);
        state.notice_shown = true;
        if let Err(e) = save(path, &state) {
            eprintln!("error: failed to write {}: {e}", path.display());
            return ExitCode::from(2);
        }
    }
    print!("{}", status(env, POSTHOG_PROJECT_KEY, path.as_deref()));
    ExitCode::SUCCESS
}

fn status(env: Env, key: &str, path: Option<&Path>) -> String {
    let state = path.and_then(load);
    let (on, why) = decide(env, state.as_ref().and_then(|s| s.enabled));
    let id = match &state {
        _ if ci_provider(env).is_some() => "a new one per run (CI)".to_owned(),
        Some(s) if is_uuid(&s.install_id) => s.install_id.clone(),
        _ => "none yet (created on the first run)".to_owned(),
    };
    let path = path.map_or_else(
        || "none (set HOME or XDG_CONFIG_HOME)".to_owned(),
        |p| p.display().to_string(),
    );
    let mut out = format!(
        "telemetry: {} ({})\ninstall id: {id}\nstate file: {path}\n",
        if on { "enabled" } else { "disabled" },
        why.label()
    );
    if key.is_empty() {
        out.push_str("note: this build has no telemetry key; nothing is ever sent\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use reapr_core::{
        FileRange, Mutant, MutantResult, MutantStatus, MutationReport, Position, Resources,
        RunOutcome,
    };

    const KEY: &str = "phc_test";

    fn env_of(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| vars.get(k).cloned()
    }

    #[test]
    fn opt_out_precedence() {
        let d = |vars: &[(&str, &str)], setting| decide(&env_of(vars), setting);
        assert_eq!(d(&[], None), (true, Why::Default));
        assert_eq!(d(&[], Some(false)), (false, Why::Setting));
        assert_eq!(d(&[], Some(true)), (true, Why::Setting));
        for off in ["0", "false", "OFF", "No"] {
            assert_eq!(
                d(&[("RPR_TELEMETRY", off)], Some(true)),
                (false, Why::RprTelemetry)
            );
        }
        for on in ["1", "TRUE", "on", "yes"] {
            assert_eq!(
                d(&[("RPR_TELEMETRY", on)], Some(false)),
                (true, Why::RprTelemetry)
            );
        }
        // Unknown values fall through to the setting.
        assert_eq!(
            d(&[("RPR_TELEMETRY", "maybe")], Some(false)),
            (false, Why::Setting)
        );
        // DO_NOT_TRACK beats everything; empty and `0` do not count.
        assert_eq!(
            d(&[("DO_NOT_TRACK", "1"), ("RPR_TELEMETRY", "1")], Some(true)),
            (false, Why::DoNotTrack)
        );
        assert_eq!(
            d(&[("DO_NOT_TRACK", "true")], None),
            (false, Why::DoNotTrack)
        );
        assert_eq!(d(&[("DO_NOT_TRACK", "0")], None), (true, Why::Default));
        assert_eq!(d(&[("DO_NOT_TRACK", "")], None), (true, Why::Default));
    }

    #[test]
    fn state_path_follows_xdg() {
        let p = |vars: &[(&str, &str)]| state_path(&env_of(vars));
        assert_eq!(
            p(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")]),
            Some(PathBuf::from("/x/reapr/telemetry.json"))
        );
        assert_eq!(
            p(&[("XDG_CONFIG_HOME", "rel"), ("HOME", "/h")]),
            Some(PathBuf::from("/h/.config/reapr/telemetry.json"))
        );
        assert_eq!(p(&[("XDG_CONFIG_HOME", "")]), None);
    }

    fn props(ci: bool) -> Properties {
        let vars: &[(&str, &str)] = if ci {
            &[("GITHUB_ACTIONS", "true")]
        } else {
            &[]
        };
        let e = anyhow::anyhow!("x");
        Properties::new(&env_of(vars), vec![], Err(&e), Duration::ZERO)
    }

    /// `prepare` with no project id.
    fn prep(key: &str, env: Env, props: &Properties) -> Prepared {
        prepare(key, env, props, || None)
    }

    fn payload(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn local_runs_persist_the_id_and_show_the_notice_once() {
        let tmp = tempfile::tempdir().unwrap();
        let xdg = tmp.path().to_str().unwrap();
        let env = env_of(&[("XDG_CONFIG_HOME", xdg)]);
        let file = tmp.path().join("reapr/telemetry.json");

        let first = prep(KEY, &env, &props(false));
        assert!(first.notice);
        assert_eq!(first.print, None);
        let p1 = payload(first.send.as_deref().unwrap());
        let state = load(&file).unwrap();
        assert!(is_uuid(&state.install_id));
        assert!(state.notice_shown);
        assert_eq!(state.enabled, None);
        assert_eq!(p1["distinct_id"], state.install_id.as_str());
        assert_eq!(p1["api_key"], KEY);
        assert_eq!(p1["event"], "run_completed");
        assert_eq!(p1["properties"]["$process_person_profile"], false);
        assert_eq!(p1["properties"]["first_run"], true);

        let second = prep(KEY, &env, &props(false));
        assert!(!second.notice);
        let p2 = payload(&second.send.unwrap());
        assert_eq!(p2["distinct_id"], p1["distinct_id"]);
        assert_eq!(p2["properties"]["first_run"], false);

        // A hand-edited id is never sent.
        let mut edited = state.clone();
        edited.install_id = "me@example.com".into();
        save(&file, &edited).unwrap();
        let third = prep(KEY, &env, &props(false));
        let id = payload(&third.send.unwrap())["distinct_id"].clone();
        assert!(is_uuid(id.as_str().unwrap()), "{id}");

        // `rpr telemetry disable`: nothing is sent, the file is not touched.
        save(
            &file,
            &State {
                enabled: Some(false),
                ..state.clone()
            },
        )
        .unwrap();
        assert_eq!(prep(KEY, &env, &props(false)), Prepared::default());
    }

    #[test]
    fn ci_runs_persist_nothing_and_show_the_notice_every_run() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_of(&[("XDG_CONFIG_HOME", tmp.path().to_str().unwrap())]);
        let a = prep(KEY, &env, &props(true));
        let b = prep(KEY, &env, &props(true));
        assert!(a.notice && b.notice);
        assert_eq!(
            payload(a.send.as_deref().unwrap())["properties"]["first_run"],
            serde_json::Value::Null
        );
        assert_ne!(
            payload(&a.send.unwrap())["distinct_id"],
            payload(&b.send.unwrap())["distinct_id"]
        );
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn unwritable_state_still_sends() {
        let tmp = tempfile::tempdir().unwrap();
        let blocker = tmp.path().join("file");
        std::fs::write(&blocker, "").unwrap();
        let env = env_of(&[("XDG_CONFIG_HOME", blocker.to_str().unwrap())]);
        let p = prep(KEY, &env, &props(false));
        assert!(p.notice);
        assert_eq!(payload(&p.send.unwrap())["properties"]["first_run"], false);
        let env = env_of(&[]);
        assert!(prep(KEY, &env, &props(false)).send.is_some());
    }

    #[test]
    fn opted_out_does_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let xdg = tmp.path().to_str().unwrap();
        for (k, v) in [("DO_NOT_TRACK", "1"), ("RPR_TELEMETRY", "off")] {
            let env = env_of(&[
                ("XDG_CONFIG_HOME", xdg),
                (k, v),
                ("RPR_TELEMETRY_DEBUG", "1"),
            ]);
            let never = || -> Option<String> { panic!("git must not run when opted out") };
            assert_eq!(
                prepare(KEY, &env, &props(false), never),
                Prepared::default()
            );
        }
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn debug_prints_and_does_not_send() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_of(&[
            ("XDG_CONFIG_HOME", tmp.path().to_str().unwrap()),
            ("RPR_TELEMETRY_DEBUG", "1"),
        ]);
        let p = prep(KEY, &env, &props(false));
        assert_eq!(p.send, None, "only `send` spawns curl");
        assert_eq!(payload(&p.print.unwrap())["event"], "run_completed");
    }

    #[test]
    fn empty_key_is_a_silent_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let xdg = tmp.path().to_str().unwrap();
        let env = env_of(&[("XDG_CONFIG_HOME", xdg)]);
        assert_eq!(prep("", &env, &props(false)), Prepared::default());
        // Debug still shows what would be sent; no notice, no state file.
        let env = env_of(&[("XDG_CONFIG_HOME", xdg), ("RPR_TELEMETRY_DEBUG", "1")]);
        let p = prep("", &env, &props(false));
        assert!(!p.notice && p.send.is_none());
        assert_eq!(payload(&p.print.unwrap())["api_key"], "");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn buckets() {
        let cases = [
            (0, "0"),
            (1, "1"),
            (2, "2-5"),
            (5, "2-5"),
            (6, "6-20"),
            (20, "6-20"),
            (21, "21-100"),
            (100, "21-100"),
            (101, "101+"),
            (usize::MAX, "101+"),
        ];
        for (n, b) in cases {
            assert_eq!(bucket(n), b, "{n}");
        }
    }

    #[test]
    fn error_kinds() {
        use anyhow::Context;
        let io = || std::io::Error::other("x");
        let ctx = |e: anyhow::Error| e.context("/home/me/repo failed");
        let cases: Vec<(anyhow::Error, ErrorKind)> = vec![
            (CliError::Project("/p".into(), io()).into(), ErrorKind::Args),
            (
                CliError::ConfigPath("/c".into(), io()).into(),
                ErrorKind::Args,
            ),
            (
                CliError::ConfigNotFound("/c".into()).into(),
                ErrorKind::ConfigMissing,
            ),
            (CliError::NoEngine("/p".into()).into(), ErrorKind::NoEngine),
            (
                CliError::AmbiguousEngine("/p".into(), "stryker, cargo-mutants".into()).into(),
                ErrorKind::AmbiguousEngine,
            ),
            (
                AdapterError::ConfigMissing {
                    engine: "stryker",
                    hint: "h".into(),
                }
                .into(),
                ErrorKind::ConfigMissing,
            ),
            (
                AdapterError::Process {
                    command: "c".into(),
                    code: Some(1),
                    detail: "d".into(),
                }
                .into(),
                ErrorKind::EngineProcess,
            ),
            (
                AdapterError::Parse {
                    context: "c".into(),
                    detail: "d".into(),
                }
                .into(),
                ErrorKind::Parse,
            ),
            (
                AdapterError::Io {
                    context: "c".into(),
                    source: io(),
                }
                .into(),
                ErrorKind::Io,
            ),
            (
                AdapterError::Unsupported {
                    engine: "stryker",
                    operation: "o",
                }
                .into(),
                ErrorKind::Other,
            ),
            (
                DiffError::Git {
                    command: "git".into(),
                    code: Some(128),
                    stderr: "s".into(),
                }
                .into(),
                ErrorKind::Git,
            ),
            (
                DiffError::Io {
                    command: "git".into(),
                    source: io(),
                }
                .into(),
                ErrorKind::Git,
            ),
            (io().into(), ErrorKind::Io),
            (anyhow::anyhow!("free text"), ErrorKind::Other),
        ];
        for (e, kind) in cases {
            let wrapped = ctx(anyhow::anyhow!("{e}"));
            assert_eq!(
                error_kind(&wrapped),
                ErrorKind::Other,
                "messages are never read"
            );
            let label = format!("{e:#}");
            assert_eq!(error_kind(&ctx(e)), kind, "{label}");
        }
        // Through `with_context`, as the CLI wraps engine errors.
        let r: Result<(), AdapterError> = Err(AdapterError::Parse {
            context: "c".into(),
            detail: "d".into(),
        });
        assert_eq!(
            error_kind(&r.context("stryker run failed").unwrap_err()),
            ErrorKind::Parse
        );
    }

    #[test]
    fn warning_kinds_and_carried_over() {
        let w = vec![
            "vitest >= 4 with bail can report killed mutants as survived (observed: 98.0 vs 87.8 on the same code); consider disableBail: true".to_owned(),
            "stryker report carried 7 results from outside the diff scope (incremental); excluded from the diff score".to_owned(),
            "git metadata: `git rev-parse HEAD` failed (exit code Some(128)): fatal".to_owned(),
            "cargo-mutants 28.0.0 is outside the supported range >=26.2.0, <28.0.0; its results may be misread".to_owned(),
            "something new at /home/me/repo".to_owned(),
        ];
        let kinds: Vec<_> = w.iter().map(|w| warning_kind(w)).collect();
        assert_eq!(
            kinds,
            [
                WarningKind::VitestBail,
                WarningKind::IncrementalCarriedOver,
                WarningKind::GitMetadata,
                WarningKind::EngineVersionUnsupported,
                WarningKind::Other
            ]
        );
        assert_eq!(carried_over(&w), 7);
        assert_eq!(carried_over(&[]), 0);
    }

    #[test]
    fn versions_are_filtered() {
        assert_eq!(safe_version("9.6.1").as_deref(), Some("9.6.1"));
        assert_eq!(
            safe_version("1.0.0-rc.1+b2").as_deref(),
            Some("1.0.0-rc.1+b2")
        );
        for bad in ["", "file:../stryker", "git+https://x/y", "a@b", "C:\\x"] {
            assert_eq!(safe_version(bad), None, "{bad}");
        }
    }

    fn mutant(file: &str, line: u32, status: MutantStatus) -> MutantResult {
        MutantResult {
            mutant: Mutant {
                id: format!("{file}:{line}"),
                file: file.into(),
                start: Position { line, column: 1 },
                end: Position { line, column: 2 },
                mutator: "ConditionalExpression".into(),
                replacement: Some("me@example.com".into()),
            },
            status,
            killed_by: vec!["test/a.test.ts".into()],
            status_reason: Some("expected /home/me to equal C:\\Users\\me".into()),
        }
    }

    /// A summary full of paths, emails, shas and free text; none of it may reach the payload.
    fn realistic_summary(gate: bool, min_score: Option<f64>) -> Summary {
        let sha = "7ae48700c070e1eeaa460868d5e063fe77160a01";
        Summary::new(
            "1793640000-4242".into(),
            "/home/me/secret-repo/.git/reapr/runs/1793640000-4242".into(),
            "stryker",
            format!("origin/feature/me@example.com-{sha}"),
            vec![
                FileRange { path: "packages/secret/src/a.ts".into(), start: 1, end: 3 },
                FileRange { path: "packages/secret/src/a.ts".into(), start: 9, end: 9 },
                FileRange { path: "C:\\work\\b.ts".into(), start: 5, end: 6 },
            ],
            RunOutcome {
                report: Some(MutationReport {
                    engine: "stryker".into(),
                    results: vec![
                        mutant("packages/secret/src/a.ts", 1, MutantStatus::Killed),
                        mutant("packages/secret/src/a.ts", 2, MutantStatus::Survived),
                        mutant("C:\\work\\b.ts", 5, MutantStatus::Timeout),
                    ],
                    duration: Duration::from_millis(83_456),
                }),
                engine_gate_failed: gate,
                warnings: vec![
                    format!("git metadata: `git merge-base {sha} HEAD` failed: fatal: /home/me"),
                    "stryker report carried 12 results from outside the diff scope (incremental); excluded from the diff score".into(),
                    "custom warning about /home/me/secret-repo and me@example.com".into(),
                ],
                resources: Some(Resources { cpu_user_ms: 250_000, cpu_sys_ms: 30_000, max_rss_kb: 812_000 }),
            },
            min_score,
        )
    }

    fn strings(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            serde_json::Value::Object(o) => o.iter().for_each(|(k, x)| {
                out.push(k.clone());
                strings(x, out);
            }),
            _ => {}
        }
    }

    /// 32 hex chars or more: a sha, or a truncated hash of one.
    fn has_sha(s: &str) -> bool {
        s.as_bytes()
            .windows(32)
            .any(|w| w.iter().all(u8::is_ascii_hexdigit))
    }

    #[test]
    fn payload_holds_no_paths_names_emails_or_shas() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_of(&[
            ("XDG_CONFIG_HOME", tmp.path().to_str().unwrap()),
            ("GITHUB_ACTIONS", "true"),
            ("RUNNER_ENVIRONMENT", "github-hosted"),
            ("RUNNER_NAME", "me@runner-host"),
            ("GITHUB_REPOSITORY", "me/secret-repo"),
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("RPR_VIA_ACTION", "1"),
            ("CLAUDECODE", "1"),
            ("HOME", "/home/me"),
        ]);
        let s = realistic_summary(false, Some(50.0));
        let flags = vec!["json", "min_score", "config", "project"];
        let errors = [
            anyhow::Error::from(CliError::NoEngine("/home/me/secret-repo".into())),
            AdapterError::Process {
                command: "npx stryker run /home/me/x".into(),
                code: Some(2),
                detail: "me@example.com".into(),
            }
            .into(),
        ];
        let mut runs: Vec<Result<(&Summary, Option<&str>), &anyhow::Error>> = vec![
            Ok((&s, Some("9.6.1"))),
            Ok((&s, Some("file:/home/me/stryker"))),
        ];
        runs.extend(errors.iter().map(Err));
        for run in runs {
            let props = Properties::new(&env, flags.clone(), run, Duration::from_millis(90_123));
            let id = || fingerprint("7ae48700c070e1eeaa460868d5e063fe77160a01");
            let json = prepare(KEY, &env, &props, id).send.unwrap();
            let mut v = payload(&json);
            // The one exception: `project_id` is a hex hash by design (salted SHA-256 of the root
            // commits, truncated; TELEMETRY.md). It must be exactly that, and nothing else may be.
            let project_id = v["properties"]["project_id"].take();
            let project_id = project_id.as_str().unwrap();
            assert!(
                project_id.len() == 32
                    && project_id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{project_id}"
            );
            let mut all = vec![];
            strings(&v, &mut all);
            for s in &all {
                assert!(
                    !s.contains(['/', '\\', '@']) && !has_sha(s),
                    "{s:?} in {json}"
                );
            }
        }

        let props = Properties::new(
            &env,
            flags,
            Ok((&s, Some("9.6.1"))),
            Duration::from_millis(90_123),
        );
        let v = serde_json::to_value(&props).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "rpr_version": env!("CARGO_PKG_VERSION"),
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "ci": true,
                "ci_provider": "github_actions",
                "runner": "github_hosted",
                "trigger": "pull_request",
                "via_action": true,
                "agent": "claude_code",
                "first_run": null,
                "build": "debug",
                "project_id": null,
                "cpu_count": std::thread::available_parallelism().ok().map(|n| n.get()),
                "flags": ["json", "min_score", "config", "project"],
                "engine": "stryker",
                "engine_version": "9.6.1",
                "outcome": "passed",
                "error_kind": null,
                "warning_kinds": ["incremental_carried_over", "git_metadata", "other"],
                "duration_ms_total": 90123,
                "duration_ms_engine": 83456,
                "mutants_in_scope": 3,
                "counts": {
                    "killed": 1, "survived": 1, "no_coverage": 0, "timeout": 1,
                    "compile_error": 0, "runtime_error": 0, "ignored": 0, "pending": 0,
                },
                "score": 66.7,
                "survivors": 1,
                "ranges_bucket": "2-5",
                "files_bucket": "2-5",
                "carried_over_excluded": "6-20",
                "engine_cpu_ms": 280000,
                "engine_max_rss_kb": 812000,
                "$process_person_profile": false,
            })
        );
    }

    #[test]
    fn outcomes() {
        let env = env_of(&[]);
        let o = |s: &Summary| Properties::new(&env, vec![], Ok((s, None)), Duration::ZERO).outcome;
        assert_eq!(o(&realistic_summary(false, None)), Outcome::Passed);
        assert_eq!(
            o(&realistic_summary(false, Some(90.0))),
            Outcome::MinScoreFailed
        );
        assert_eq!(
            o(&realistic_summary(true, Some(90.0))),
            Outcome::EngineGateFailed
        );
        let empty = Summary::new(
            "r".into(),
            "/r".into(),
            "stryker",
            "main".into(),
            vec![],
            RunOutcome {
                report: None,
                engine_gate_failed: false,
                warnings: vec![],
                resources: None,
            },
            Some(90.0),
        );
        let p = Properties::new(&env, vec![], Ok((&empty, None)), Duration::ZERO);
        assert_eq!(p.outcome, Outcome::NothingToMutate);
        assert_eq!((p.runner, p.ci), (Some("local"), false));
        assert_eq!(p.score, None);
        let e = anyhow::Error::from(CliError::NoEngine("/p".into()));
        let p = Properties::new(&env, vec![], Err(&e), Duration::ZERO);
        assert_eq!(
            (p.outcome, p.error_kind),
            (Outcome::Error, Some(ErrorKind::NoEngine))
        );
        assert_eq!(p.counts, None);
    }

    #[test]
    fn ci_providers_from_presence_only() {
        let p = |vars: &[(&str, &str)]| ci_provider(&env_of(vars));
        assert_eq!(p(&[]), None);
        assert_eq!(p(&[("GITLAB_CI", "true")]), Some("gitlab"));
        assert_eq!(p(&[("CIRCLECI", "true")]), Some("circleci"));
        assert_eq!(p(&[("BUILDKITE", "true")]), Some("buildkite"));
        assert_eq!(
            p(&[("JENKINS_URL", "https://ci.example.com/")]),
            Some("jenkins")
        );
        assert_eq!(p(&[("CI", "woodpecker")]), Some("other"));
        assert_eq!(p(&[("CI", "")]), None);
    }

    #[test]
    fn status_text() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("reapr/telemetry.json");
        let env = env_of(&[]);
        let text = status(&env, "", Some(&file));
        assert!(
            text.starts_with("telemetry: enabled (default)\ninstall id: none yet"),
            "{text}"
        );
        assert!(text.contains("nothing is ever sent"));
        let id = new_uuid();
        save(
            &file,
            &State {
                install_id: id.clone(),
                enabled: Some(false),
                notice_shown: true,
            },
        )
        .unwrap();
        let text = status(&env, KEY, Some(&file));
        assert!(
            text.starts_with(&format!(
                "telemetry: disabled (setting)\ninstall id: {id}\n"
            )),
            "{text}"
        );
        assert!(!text.contains("nothing is ever sent"));
        let env = env_of(&[("RPR_TELEMETRY", "yes")]);
        assert!(status(&env, KEY, None).starts_with("telemetry: enabled (env: RPR_TELEMETRY)"));
    }

    #[test]
    fn uuids() {
        let a = new_uuid();
        assert!(is_uuid(&a), "{a}");
        assert_eq!(&a[14..15], "4");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"), "{a}");
        assert_ne!(a, new_uuid());
        assert!(!is_uuid("7AE48700-C070-41EE-AA46-0868D5E063FE"));
        assert!(!is_uuid("not-a-uuid"));
    }

    #[test]
    fn triggers() {
        let t = |vars: &[(&str, &str)]| {
            let env = env_of(vars);
            trigger(&env, ci_provider(&env))
        };
        assert_eq!(t(&[]), Some("local"));
        let gha = |event: &str| t(&[("GITHUB_ACTIONS", "true"), ("GITHUB_EVENT_NAME", event)]);
        for e in [
            "pull_request",
            "pull_request_target",
            "push",
            "schedule",
            "workflow_dispatch",
            "merge_group",
        ] {
            assert_eq!(gha(e), Some(e));
        }
        assert_eq!(gha("issue_comment"), Some("other"));
        assert_eq!(t(&[("GITHUB_ACTIONS", "true")]), Some("other"));
        let gitlab = |source: &str| t(&[("GITLAB_CI", "true"), ("CI_PIPELINE_SOURCE", source)]);
        assert_eq!(gitlab("merge_request_event"), Some("pull_request"));
        assert_eq!(gitlab("push"), Some("push"));
        assert_eq!(gitlab("schedule"), Some("schedule"));
        assert_eq!(gitlab("web"), Some("manual"));
        assert_eq!(gitlab("api"), Some("other"));
        assert_eq!(
            t(&[("CIRCLECI", "true"), ("GITHUB_EVENT_NAME", "push")]),
            None
        );
    }

    #[test]
    fn agents_from_presence_only() {
        let a = |vars: &[(&str, &str)]| agent(&env_of(vars));
        assert_eq!(a(&[]), None);
        assert_eq!(a(&[("CLAUDECODE", "1")]), Some("claude_code"));
        assert_eq!(a(&[("CURSOR_AGENT", "1")]), Some("cursor"));
        assert_eq!(a(&[("CODEX_THREAD_ID", "t")]), Some("codex"));
        assert_eq!(a(&[("CODEX_SANDBOX", "seatbelt")]), Some("codex"));
        assert_eq!(a(&[("CODEX_SANDBOX_NETWORK_DISABLED", "1")]), Some("codex"));
        assert_eq!(a(&[("GEMINI_CLI", "1")]), Some("gemini_cli"));
        // Not markers: a user's own config dir, an empty value.
        assert_eq!(
            a(&[("CODEX_HOME", "/home/me/.codex"), ("CLAUDECODE", "")]),
            None
        );
    }

    #[test]
    fn project_id_recipe() {
        // printf 'reapr-telemetry-v1:aaa,bbb' | sha256sum | cut -c1-32
        let expected = "d0a7e0b2f7b6dcd3b223cb4eee2ee90e";
        assert_eq!(fingerprint("bbb\naaa\n").as_deref(), Some(expected));
        assert_eq!(fingerprint("aaa\nbbb").as_deref(), Some(expected));
        assert_eq!(fingerprint("\n"), None);

        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(project_id(tmp.path()), None, "not a git repo");
        assert_eq!(project_id(&tmp.path().join("missing")), None);
        let repo = tmp.path().join("repo");
        let git = |dir: &Path, args: &[&str]| {
            let out = std::process::Command::new("git")
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@example.com",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap()
        };
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        assert_eq!(project_id(&repo), None, "no commit yet");
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "one"]);
        let root = git(&repo, &["rev-parse", "HEAD"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "two"]);
        let id = project_id(&repo).unwrap();
        assert_eq!(Some(id.clone()), fingerprint(&root));
        // A shallow clone's root is its boundary commit, not the project's: no id.
        let url = format!("file://{}", repo.display());
        git(
            tmp.path(),
            &["clone", "-q", "--depth", "1", &url, "shallow"],
        );
        assert_eq!(project_id(&tmp.path().join("shallow")), None);
    }
}
