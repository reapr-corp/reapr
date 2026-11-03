#!/usr/bin/env python3
"""OSS pilot: full StrykerJS run (A) vs `rpr run --diff` (C) on the heads of merged pull requests.

usage: pilot.py --repo DIR --gh OWNER/NAME --pr N [--pr N ...] --rpr BIN --out DIR
                [--c-reps 3] [--a-reps 1] [--cpus N] [--install "npm ci"] [--project packages/x]
                [--a-report reports/mutation/mutation.json] [--a-args "..."] [--timeout-min 60]

For each PR: resolve its merge commit with `gh`, check out the PR head (merge^2, or the squash commit
itself), install deps, then run A (`npx --no-install stryker run`, the repo's config as is) and C
(`rpr run --diff <merge^1> --json`). Nothing in the repo is edited; the harness only deletes Stryker's
own report and temp dir before A so a stale report is never read.

Writes OUT/<pr>/<A|C>-<rep>.json per run, OUT/<pr>/A-<rep>.mutation.json, OUT/summary.json and
OUT/summary.md. Parity applies Stryker's containment rule (see schema.rs `in_scope`) to both raw
reports with the ranges from C's JSON, keyed by file, mutator, location and replacement.
"""
import argparse, json, os, shlex, shutil, signal, statistics, subprocess, sys, time
from pathlib import Path

END_COLUMN = 1 << 53
DETECTED = {"Killed", "Timeout"}
UNDETECTED = {"Survived", "NoCoverage"}


def sh(cmd, cwd, **kw):
    return subprocess.run(cmd, cwd=cwd, check=True, text=True, capture_output=True, **kw).stdout.strip()


def timed(cmd, cwd, timeout_s, env, log):
    """Spawn cmd, wait4 its tree; stdout/stderr to log.{stdout,stderr}. exit None on timeout."""
    meta = {"cmd": cmd, "loadavg_before": os.getloadavg(), "start": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    t0 = time.monotonic()
    out = open(f"{log}.stdout", "w+")
    err = open(f"{log}.stderr", "w")
    p = subprocess.Popen(cmd, cwd=cwd, stdin=subprocess.DEVNULL, stdout=out, stderr=err, env=env,
                         start_new_session=True)
    code = None
    try:
        while True:
            pid, status, ru = os.wait4(p.pid, os.WNOHANG)
            if pid:
                code = os.waitstatus_to_exitcode(status)
                break
            if time.monotonic() - t0 > timeout_s:
                os.killpg(p.pid, 9)
                _, status, ru = os.wait4(p.pid, 0)
                break
            time.sleep(0.5)
    except BaseException:  # Ctrl-C / SIGTERM: the run has its own session, so take its workers down too
        os.killpg(p.pid, 9)
        raise
    meta.update(wall_ms=int((time.monotonic() - t0) * 1000), exit=code, timed_out=code is None,
                cpu_user_ms=int(ru.ru_utime * 1000), cpu_sys_ms=int(ru.ru_stime * 1000),
                max_rss_kb=ru.ru_maxrss, loadavg_after=os.getloadavg())
    out.seek(0)
    stdout = out.read()
    out.close(), err.close()
    return code, stdout, meta


def mutants(report_path, repo):
    """{key: status} from a Stryker v2 report, file keys relative to repo (the project dir, as rpr's ranges)."""
    rep = json.loads(Path(report_path).read_text())
    root = Path(rep.get("projectRoot") or repo).resolve()
    res = {}
    for key, f in rep["files"].items():
        rel = str((root / key).resolve().relative_to(repo.resolve()))
        for m in f["mutants"]:
            s, e = m["location"]["start"], m["location"]["end"]
            k = (rel, m["mutatorName"], s["line"], s["column"], e["line"], e["column"], m.get("replacement"))
            res[k] = m["status"]
    return res


def in_scope(k, ranges):
    file, _, sl, sc, el, ec, _ = k
    return any(r["path"] == file and (sl, sc) >= (r["start"], 1) and (el, ec) <= (r["end"], END_COLUMN) for r in ranges)


def score(statuses):
    d = sum(s in DETECTED for s in statuses)
    u = sum(s in UNDETECTED for s in statuses)
    return round(100 * d / (d + u), 2) if d + u else None


def resolve_pr(gh, pr, repo):
    sha = sh(["gh", "pr", "view", str(pr), "-R", gh, "--json", "mergeCommit", "-q", ".mergeCommit.oid"], repo)
    parents = sh(["git", "rev-list", "--parents", "-n1", sha], repo).split()[1:]
    return {"merge": sha, "base": parents[0], "head": parents[1] if len(parents) > 1 else sha}


def main():
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", type=Path, required=True)
    ap.add_argument("--gh", required=True, help="OWNER/NAME, to resolve PR merge commits")
    ap.add_argument("--pr", type=int, action="append", required=True)
    ap.add_argument("--rpr", default=os.environ.get("REAPR_BIN", "rpr"))
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--a-reps", type=int, default=1)
    ap.add_argument("--c-reps", type=int, default=3)
    ap.add_argument("--cpus", type=int, help="pin every run to CPUs 0..N-1 (taskset), e.g. 4 for a 4-vCPU runner")
    ap.add_argument("--install", default="npm ci", help="shell command, run at the repo root (may chain a build)")
    ap.add_argument("--project", default=".", help="package dir (relative to --repo) where A and C run, for monorepos")
    ap.add_argument("--a-report", default="reports/mutation/mutation.json", help="where the config's json reporter writes")
    ap.add_argument("--a-args", default="", help="extra args for A, e.g. '--reporters clear-text,json' if the config has no json reporter")
    ap.add_argument("--timeout-min", type=float, default=60)
    a = ap.parse_args()
    repo = a.repo.resolve()
    proj = repo / a.project
    env = {**os.environ, "DO_NOT_TRACK": "1", "RPR_TELEMETRY": "0"}
    pin = ["taskset", "-c", f"0-{a.cpus - 1}"] if a.cpus else []
    rpr = str(Path(a.rpr).resolve()) if os.sep in a.rpr else a.rpr
    sh(["git", "fetch", "-q", "origin"], repo)
    summary = {"repo": a.gh, "project": a.project, "install": a.install, "a_args": a.a_args, "node": sh(["node", "--version"], repo), "rpr": sh([rpr, "--version"], repo),
               "cpu_count": os.cpu_count(), "pinned_cpus": a.cpus, "prs": []}
    for pr in a.pr:
        c = resolve_pr(a.gh, pr, repo)
        d = a.out / str(pr)
        d.mkdir(parents=True, exist_ok=True)
        sh(["git", "checkout", "-q", "--detach", "-f", c["head"]], repo)
        subprocess.run(a.install, shell=True, cwd=repo, check=True, capture_output=True)
        c["stryker"] = sh(["node", "-p", "require('@stryker-mutator/core/package.json').version"], proj)
        runs = {"A": [], "C": []}
        for rep in range(1, a.a_reps + 1):
            shutil.rmtree(proj / ".stryker-tmp", ignore_errors=True)
            (proj / a.a_report).unlink(missing_ok=True)
            code, _, meta = timed(pin + ["npx", "--no-install", "stryker", "run", *shlex.split(a.a_args)],
                                  proj, a.timeout_min * 60, env, d / f"A-{rep}")
            rp = proj / a.a_report
            if rp.exists():
                shutil.copy(rp, d / f"A-{rep}.mutation.json")
                st = list(mutants(rp, proj).values())
                meta.update(mutants=len(st), score=score(st))
            (d / f"A-{rep}.json").write_text(json.dumps(meta, indent=1))
            runs["A"].append(meta)
            print(f"#{pr} A-{rep}: exit={code} wall={meta['wall_ms']/1000:.0f}s load={meta['loadavg_before'][0]:.1f}", flush=True)
        for rep in range(1, a.c_reps + 1):
            code, out, meta = timed(pin + [rpr, "run", "--diff", c["base"], "--json"], proj, a.timeout_min * 60, env, d / f"C-{rep}")
            try:
                j = json.loads(out)
            except ValueError:
                j = {"error": out[-500:]}
            meta["rpr"] = j
            if "counts" in j:
                meta.update(mutants=sum(j["counts"].values()), score=j["score"], engine_ms=j["duration_ms"])
            (d / f"C-{rep}.json").write_text(json.dumps(meta, indent=1))
            runs["C"].append(meta)
            print(f"#{pr} C-{rep}: exit={code} wall={meta['wall_ms']/1000:.1f}s mutants={meta.get('mutants')} "
                  f"load={meta['loadavg_before'][0]:.1f}", flush=True)
        # Parity: A's and C's raw Stryker reports, both filtered with the same ranges.
        par = None
        last_c = next((r["rpr"] for r in reversed(runs["C"]) if "ranges" in r["rpr"]), None)
        if last_c and (d / "A-1.mutation.json").exists():
            ranges = last_c["ranges"]
            am = {k: v for k, v in mutants(d / "A-1.mutation.json", proj).items() if in_scope(k, ranges)}
            cm = {k: v for k, v in mutants(Path(last_c["run_dir"]) / "mutation.json", proj).items() if in_scope(k, ranges)}
            diff = [{"key": list(k), "A": am.get(k), "C": cm.get(k)} for k in sorted(am.keys() | cm.keys(), key=str)
                    if am.get(k) != cm.get(k)]
            par = {"a_in_diff": len(am), "c_in_diff": len(cm), "same": sum(am[k] == cm.get(k) for k in am),
                   "mismatches": diff, "a_diff_score": score(am.values()), "c_diff_score": score(cm.values())}
        med = lambda rs, k: statistics.median(r[k] for r in rs) if rs else None
        c.update(pr=pr, runs=runs, parity=par,
                 a_wall_ms=med(runs["A"], "wall_ms"), c_wall_ms=med(runs["C"], "wall_ms"),
                 a_cpu_ms=med(runs["A"], "cpu_user_ms") + med(runs["A"], "cpu_sys_ms"),
                 c_cpu_ms=statistics.median(r["cpu_user_ms"] + r["cpu_sys_ms"] for r in runs["C"]))
        c["speedup_wall"] = round(c["a_wall_ms"] / c["c_wall_ms"], 1)
        c["speedup_cpu"] = round(c["a_cpu_ms"] / c["c_cpu_ms"], 1)
        summary["prs"].append(c)
        (a.out / "summary.json").write_text(json.dumps(summary, indent=1))
    rows = ["| PR | Stryker | A wall | A CPU | A mutants | A score | C wall (median of n) | C mutants | C score | "
            "speedup wall / CPU | parity |", "|---|---|---|---|---|---|---|---|---|---|---|"]
    for c in summary["prs"]:
        A, C, p = c["runs"]["A"][0], c["runs"]["C"], c["parity"]
        par = f"{p['same']}/{p['a_in_diff']} same, {len(p['mismatches'])} differ" if p else "n/a"
        rows.append(f"| #{c['pr']} | {c['stryker']} | {c['a_wall_ms']/1000:.0f} s | {c['a_cpu_ms']/1000:.0f} s | "
                    f"{A.get('mutants')} | {A.get('score')} | {c['c_wall_ms']/1000:.1f} s (n={len(C)}) | "
                    f"{C[-1].get('mutants')} | {C[-1].get('score')} | {c['speedup_wall']}x / {c['speedup_cpu']}x | {par} |")
    (a.out / "summary.md").write_text("\n".join(rows) + "\n")
    print("\n".join(rows))


def self_test():
    r = [{"path": "src/a.ts", "start": 5, "end": 6}]
    k = lambda f, sl, sc, el, ec: (f, "M", sl, sc, el, ec, None)
    assert in_scope(k("src/a.ts", 5, 1, 6, 999), r)       # whole range
    assert not in_scope(k("src/a.ts", 4, 9, 5, 3), r)     # starts before the range
    assert not in_scope(k("src/a.ts", 6, 1, 7, 1), r)     # ends after it
    assert not in_scope(k("src/b.ts", 5, 1, 5, 2), r)     # other file
    assert score(["Killed", "Timeout", "Survived", "NoCoverage", "Ignored"]) == 50.0 and score(["Ignored"]) is None
    print("self-test ok")


if __name__ == "__main__":
    self_test() if sys.argv[1:] == ["--self-test"] else main()
