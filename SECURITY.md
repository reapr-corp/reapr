# Security policy

## Reporting a vulnerability

Report vulnerabilities privately through a GitHub security advisory:
<https://github.com/reapr-corp/reapr/security/advisories/new>

Do not open a public issue. Include the Reapr version (`rpr --version`), your OS, and steps to reproduce.
We will acknowledge the report in the advisory and keep you informed there until it is resolved.

## Supported versions

Reapr is pre-release. There is no published release yet; fixes go to `main`. Once releases exist, only the
latest release is supported.

## What Reapr executes

Reapr runs code from the repository it is pointed at. `rpr run`:

- imports the project's engine config in Node (`plan.mjs` and the generated wrapper load it with `import`, so
  any code in a JavaScript config runs);
- runs the project's installed engine (`npx --no-install stryker run`), which runs the project's test suite;
- runs `git` in the project.

Treat `rpr run` like running the project's tests: only run it on code you trust, and in CI use the same
isolation you use for untrusted pull requests. A config or test that misbehaves under Reapr would misbehave
under the engine alone; reports of that kind are not Reapr vulnerabilities unless Reapr makes it worse.

## Data Reapr keeps

Every run writes files under `<git dir>/reapr/runs/<run_id>/`: the engine's report, Reapr's report and
`run.json`. `run.json` includes commit SHAs, author email addresses and commit trailers from `<base>...HEAD`,
the runner name, OS and CPU count. These are local files. Reapr makes no network requests and sends nothing
anywhere. They are not committed by git, but CI artifact uploads or caches that include `.git` will carry them.
