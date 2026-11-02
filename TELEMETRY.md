# Telemetry

`rpr` sends one anonymous usage event per `rpr run`. It is on by default and easy to turn off.

## Why

To learn how Reapr is used in practice: which outcomes are common, how long runs take, how large diffs are, which
warnings and errors people hit, and on which platforms and CI systems. That decides what gets fixed and built
next. The event holds counts, durations and fixed categories only, never your code or anything that names you,
your project or your machine.

## How it works

- One event, `run_completed`, per `rpr run`, also when the run ends in an error (after the arguments were parsed).
  `rpr --help`, `rpr --version`, invalid arguments and `rpr telemetry` send nothing.
- It is sent only after the output is printed, so it never delays or changes it. Nothing happens while the engine
  runs. stdout is untouched: with `--json` the JSON document is still the only thing on stdout.
- It is sent by running `curl` (a `POST` with the JSON on curl's stdin). `rpr` waits at most 1.5 seconds, then
  kills curl. Network errors are ignored. If `curl` is not on `PATH`, nothing is sent.
- The first time telemetry is active on a machine, `rpr` prints this notice once, on stderr:

  ```
  rpr collects anonymous usage data (no code, paths or names) to improve the tool. Disable: rpr telemetry disable or DO_NOT_TRACK=1. Details: https://github.com/reapr-corp/reapr/blob/main/TELEMETRY.md
  ```

- In CI (any of `GITHUB_ACTIONS`, `GITLAB_CI`, `CIRCLECI`, `BUILDKITE`, `JENKINS_URL` or `CI` is set), nothing is
  written to disk: each run uses a fresh random id and prints the notice (one stderr line).

Builds without a project key (every build before the key is set in the source) send nothing, print no notice and
write no state file.

## Where it goes

[PostHog](https://posthog.com) US cloud (`https://us.i.posthog.com/i/v0/e/`). Events are sent with
`$process_person_profile: false`, so PostHog creates no person profile. Retention: PostHog's default. The
project API key in the binary is public by design: it can only write events, not read them.

## What is sent

The event, exactly as sent:

```json
{
  "api_key": "<the project key>",
  "event": "run_completed",
  "distinct_id": "d211dd4c-b6f3-4622-b367-f3643505abc0",
  "timestamp": "2026-11-02T17:20:00Z",
  "properties": { "...": "below" }
}
```

`distinct_id` is the install id: a random UUID v4 created on the first run and kept in the state file (in CI, a
new one per run). It is not derived from anything about you or the machine. `timestamp` is RFC 3339 UTC.

| Property | Example | Meaning |
|---|---|---|
| `rpr_version` | `"0.1.0"` | The `rpr` version. |
| `os`, `arch` | `"linux"`, `"x86_64"` | Rust's `std::env::consts`; not the CI's `RUNNER_OS`. |
| `ci` | `true` | Whether a CI variable listed above is set. |
| `ci_provider` | `"github_actions"` | `github_actions`, `gitlab`, `circleci`, `buildkite`, `jenkins`, `other` (only `CI` is set) or `null` (not CI). From which variables are set, never their values. |
| `runner` | `"github_hosted"` | `local` when not in CI; `github_hosted` or `self_hosted` from GitHub's `RUNNER_ENVIRONMENT`; else `null`. |
| `cpu_count` | `4` | Available parallelism. |
| `flags` | `["json", "min_score"]` | Names of the optional flags given: `json`, `min_score`, `config`, `engine`, `project`. Never their values. `--diff` is always given and is not listed. |
| `engine` | `"stryker"` | Engine id: `stryker` or `cargo-mutants`; `null` on errors. |
| `engine_version` | `"9.6.1"` | The engine's version; `null` when unknown, on errors, or when it has any character other than letters, digits, `.`, `+` or `-`. |
| `outcome` | `"passed"` | `passed`, `min_score_failed`, `engine_gate_failed`, `nothing_to_mutate` or `error`. |
| `error_kind` | `"git"` | On errors: `config_missing`, `no_engine`, `ambiguous_engine` (several engines detected, no `--engine`), `git`, `engine_process`, `parse`, `io`, `args` (invalid `--project` or `--config` path) or `other`. A fixed kind, never the error message. `null` otherwise. |
| `warning_kinds` | `["vitest_bail"]` | Kinds of the warnings printed: `vitest_bail`, `incremental_carried_over`, `git_metadata`, `engine_version_unsupported` (the engine version is outside the adapter's supported range), `other`. Never their text. |
| `duration_ms_total` | `90123` | Wall time of `rpr`, from start to the printed output. |
| `duration_ms_engine` | `83456` | Engine wall time (`duration_ms` in the JSON output); `null` on errors. |
| `mutants_in_scope` | `16` | Mutants in the diff scope (sum of `counts`). |
| `counts` | `{"killed": 14, "survived": 2, ...}` | Mutants per status, as in the JSON output. |
| `score` | `87.5` | Mutation score, rounded to one decimal; `null` when there is none. |
| `survivors` | `2` | Number of surviving mutants. Never their files, lines or code. |
| `ranges_bucket`, `files_bucket` | `"6-20"`, `"2-5"` | Changed ranges and files in the diff, as a bucket: `0`, `1`, `2-5`, `6-20`, `21-100`, `101+`. |
| `carried_over_excluded` | `"0"` | Results an incremental report carried from outside the diff and Reapr excluded, bucketed the same way. |
| `engine_cpu_ms` | `280000` | CPU time (user + system) of the engine's process tree; `null` where it is not measured. |
| `engine_max_rss_kb` | `812000` | Peak RSS of the engine's largest process, in KiB; `null` where it is not measured. |
| `$process_person_profile` | `false` | Tells PostHog not to create a person profile. |

On errors, the run-result properties (`engine` through `engine_max_rss_kb`, except `outcome`, `error_kind` and
`duration_ms_total`) are `null` or empty. There is no `configs` property: `rpr` runs one config per call (the
GitHub Action's `configs` input calls it once per config).

## What is never sent

Code, mutants, file paths, file or repository names, URLs, branch names or the `--diff` base, commit shas, emails,
commit trailers, git config, the contents of your engine config, error or warning messages (only their kind),
usernames, hostnames, runner names, and the values of environment variables. The event has no IP address field;
PostHog, like any server, sees the IP address the request comes from.
A unit test builds an event from a run full of paths, emails and shas and fails if any value contains `/`, `\`,
`@` or a 40-character hex sha.

## How to turn it off

In precedence order:

1. `DO_NOT_TRACK` set to any non-empty value other than `0` turns it off ([consoledonottrack.com](https://consoledonottrack.com)).
2. `RPR_TELEMETRY=0` (or `false`, `off`, `no`, any case) turns it off; `RPR_TELEMETRY=1` (or `true`, `on`, `yes`)
   turns it on. Other values are ignored.
3. `rpr telemetry disable` or `rpr telemetry enable` saves the setting.
4. Otherwise it is on.

`rpr telemetry` (or `rpr telemetry status`) prints whether it is on and why, the install id and the state file
path. It never sends anything.

In GitHub Actions, set it for the job:

```yaml
env:
  DO_NOT_TRACK: 1
```

## See the payload

`RPR_TELEMETRY_DEBUG=1` prints the exact JSON that would be sent on stderr, on one line, and sends nothing:

```sh
RPR_TELEMETRY_DEBUG=1 rpr run --diff origin/main --json > /dev/null
```

## State file

`${XDG_CONFIG_HOME:-$HOME/.config}/reapr/telemetry.json`:

```json
{
  "install_id": "d211dd4c-b6f3-4622-b367-f3643505abc0",
  "enabled": null,
  "notice_shown": true
}
```

`enabled` is `null` until you run `rpr telemetry enable` or `disable`. Delete the file to get a new install id.
If the file cannot be read or written, `rpr` behaves as if it were missing and never fails because of it.
