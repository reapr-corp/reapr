#!/usr/bin/env python3
"""OSS benchmark suite, driven by .github/workflows/oss-benchmark.yml. Repos are defined in repos.json.

  suite.py repos --select all|id,id                 -> GITHUB_OUTPUT line `repos=[{id,node}...]`
  suite.py scan --repo-id ID --n 10 --min-survivor-prs 3 --out scan.json
  suite.py plan --scans DIR --reps 3 --scenarios A,B,C -> GITHUB_OUTPUT lines a=, seed=, b=, c=
  suite.py run --repo-id ID --pr N --base SHA --head SHA --scenario A|seed|B|C --rep K --out r.json [--seed FILE]
  suite.py aggregate --results DIR --scans DIR [--jobs jobs.json] --out DIR

Scenarios: A `npx stryker run` as configured (plus `json` when the config lacks it); seed = A on the PR base with
the B flags, kept for its incremental file; B = A on the head with the B flags and the seed's incremental file;
C = `rpr run --diff <base> --json`. Every run job clones the target, checks out the commit, installs (timed
apart), and times only the scenario command. Result JSON holds no local paths.
"""
import argparse, json, os, re, shlex, shutil, statistics, subprocess, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from pilot import in_scope, mutants, score, sh, timed  # noqa: E402

HERE = Path(__file__).resolve().parent
REPOS = {r["id"]: r for r in json.loads((HERE / "repos.json").read_text())}
UNDETECTED = {"Survived", "NoCoverage"}
ENV = {**os.environ, "DO_NOT_TRACK": "1", "RPR_TELEMETRY": "0"}


def out(key, value):  # one GITHUB_OUTPUT line (stdout when run by hand)
    line = f"{key}={json.dumps(value, separators=(',', ':'))}"
    path = os.environ.get("GITHUB_OUTPUT")
    if path:
        with open(path, "a") as f:
            f.write(line + "\n")
    print(line[:300])


def clone(r, work):
    if not (work / ".git").exists():
        sh(["git", "clone", "-q", "--filter=blob:none", f"https://github.com/{r['gh']}", str(work)], ".")
    return work


def checkout(work, sha):
    sh(["git", "checkout", "-q", "--detach", "-f", sha], work)
    sh(["git", "clean", "-qfdx", "-e", "node_modules"], work)


def install(r, work):
    t0 = time.monotonic()
    p = subprocess.run(r["install"], shell=True, cwd=work, env=ENV, capture_output=True, text=True)
    if p.returncode:
        raise RuntimeError(f"install failed ({p.returncode}): {(p.stdout + p.stderr)[-2000:]}")
    return int((time.monotonic() - t0) * 1000)


def diff_ranges(work, base, project):
    """rpr's scope (crates/reapr-core/src/diff.rs): same git flags, added/changed lines, merged, inside project."""
    d = sh(["git", "-c", "core.quotePath=false", "diff", "--unified=0", "--no-color", "--no-ext-diff", "--find-renames",
            "--inter-hunk-context=0", "--no-textconv", "--src-prefix=a/", "--dst-prefix=b/", "--end-of-options",
            f"{base}...HEAD", "--"], work)
    ranges, path = [], None
    for line in d.splitlines():
        if line.startswith("diff --git "):
            path = None
        elif line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else None
        elif line.startswith("@@ -") and path:
            m = re.match(r"@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@", line)
            start, count = int(m[1]), int(m[2] or 1)
            if count:
                ranges.append([path, start, start + count - 1])
    prefix = "" if project in (".", "") else project.rstrip("/") + "/"
    merged = []
    for p, s, e in sorted(x for x in ranges if x[0].startswith(prefix)):
        p = p[len(prefix):]
        if merged and merged[-1]["path"] == p and s <= merged[-1]["end"] + 1:
            merged[-1]["end"] = max(merged[-1]["end"], e)
        else:
            merged.append({"path": p, "start": s, "end": e})
    return merged


def flat(ms, ranges):
    return sorted([*k, v] for k, v in ms.items() if in_scope(k, ranges))


def stryker_version(proj):
    return sh(["node", "-p", "require('@stryker-mutator/core/package.json').version"], proj)


def cmd_repos(a):
    ids = list(REPOS) if a.select.strip() in ("", "all") else [s.strip() for s in a.select.split(",") if s.strip()]
    bad = [i for i in ids if i not in REPOS]
    if bad:
        sys.exit(f"unknown repo ids: {bad}; known: {list(REPOS)}")
    out("repos", [{"id": i, "node": REPOS[i]["node"]} for i in ids])


def run_c(proj, base, log, timeout_min):
    code, stdout, meta = timed(["rpr", "run", "--diff", base, "--json"], proj, timeout_min * 60, ENV, log)
    try:
        j = json.loads(stdout)
    except ValueError:
        j = {"error": stdout[-1000:]}
    return code, j, meta


def cmd_scan(a):
    r = REPOS[a.repo_id]
    work = clone(r, Path(a.work))
    inc, exc = re.compile(r["mutate_re"]), re.compile(r["exclude_re"])
    prs = json.loads(sh(["gh", "pr", "list", "-R", r["gh"], "--base", r["branch"], "--state", "merged", "--limit",
                         str(a.limit), "--json", "number,title,mergedAt,mergeCommit,files"], work))
    prs.sort(key=lambda p: p["mergedAt"], reverse=True)
    cands = [p for p in prs if p.get("mergeCommit") and any(inc.search(f["path"]) and not exc.search(f["path"])
                                                             for f in p["files"])]
    proj = work / r["project"]
    scanned, t0 = [], time.monotonic()
    Path(a.out).parent.mkdir(parents=True, exist_ok=True)
    logs = Path(a.out).parent / f"scan-{a.repo_id}-logs"
    logs.mkdir(parents=True, exist_ok=True)

    def enough():
        ok = [s for s in scanned if s["valid"]]
        surv = sum(s["c_survivors"] > 0 for s in ok)
        return len(ok) >= a.n and surv >= a.min_survivor_prs

    for p in cands:
        if enough() or time.monotonic() - t0 > a.budget_min * 60:
            break
        merge = p["mergeCommit"]["oid"]
        s = {"pr": p["number"], "title": p["title"], "merged_at": p["mergedAt"], "merge": merge, "valid": False}
        scanned.append(s)
        try:
            parents = sh(["git", "rev-list", "--parents", "-n1", merge], work).split()[1:]
            s.update(base=parents[0], head=parents[1] if len(parents) > 1 else merge, merge_kind=
                     "merge" if len(parents) > 1 else "squash")
            cfg = (Path(r["project"]) / r["config"]).as_posix()
            if subprocess.run(["git", "cat-file", "-e", f"{s['head']}:{cfg}"], cwd=work, capture_output=True).returncode:
                s["skip"] = "no Stryker config at head"
                continue
            checkout(work, s["head"])
            install(r, work)
            code, j, meta = run_c(proj, s["base"], logs / str(p["number"]), a.timeout_min)
            s.update(c_exit=code, c_wall_ms=meta["wall_ms"])
            if "counts" not in j:
                s["skip"] = f"rpr error: {str(j.get('error'))[:300]}"
                continue
            if j.get("resources") is None:
                s["skip"] = "nothing to mutate (no changed file selected by the config's mutate)"
                continue
            s.update(valid=True, c_mutants=sum(j["counts"].values()), c_survivors=len(j["survivors"]),
                     c_score=j["score"])
        except Exception as e:  # noqa: BLE001 - a broken PR head is recorded, not fatal
            s["skip"] = f"{type(e).__name__}: {str(e)[:500]}"
        print(f"scan #{p['number']}:", s.get("skip") or f"mutants={s.get('c_mutants')} survivors={s.get('c_survivors')}",
              flush=True)
    ok = [s for s in scanned if s["valid"]]
    selected = ok[:a.n]
    need = a.min_survivor_prs - sum(s["c_survivors"] > 0 for s in selected)
    extra = [s for s in ok[a.n:] if s["c_survivors"] > 0][:max(0, need)]
    for s in selected:
        s["why"] = "last N"
    for s in extra:
        s["why"] = "older PR added for in-diff survivors"
    res = {"repo": a.repo_id, "gh": r["gh"], "branch": r["branch"], "n": a.n, "min_survivor_prs": a.min_survivor_prs,
           "candidates_total": len(cands), "scanned": scanned, "selected": selected + extra,
           "scan_minutes": round((time.monotonic() - t0) / 60, 1)}
    Path(a.out).write_text(json.dumps(res, indent=1))
    print(f"selected {len(res['selected'])} PRs: {[s['pr'] for s in res['selected']]}")


def cmd_plan(a):
    scen = {s.strip() for s in a.scenarios.split(",")}
    m = {"a": [], "seed": [], "b": [], "c": []}
    for f in sorted(Path(a.scans).glob("**/scan-*.json")):
        sc = json.loads(f.read_text())
        r = REPOS[sc["repo"]]
        for s in sc["selected"]:
            e = {"repo": sc["repo"], "node": r["node"], "pr": s["pr"], "base": s["base"], "head": s["head"]}
            for rep in range(1, a.reps + 1):
                if "A" in scen:
                    m["a"].append({**e, "rep": rep})
                if "C" in scen:
                    m["c"].append({**e, "rep": rep})
                if "B" in scen and r["b"]:
                    m["b"].append({**e, "rep": rep})
            if "B" in scen and r["b"]:
                m["seed"].append(e)
    for k, v in m.items():
        if len(v) > 256:
            sys.exit(f"matrix {k} has {len(v)} jobs; GitHub allows 256. Lower prs or reps.")
        out(k, v)


def cmd_run(a):
    r = REPOS[a.repo_id]
    work = clone(r, Path(a.work))
    proj = work / r["project"]
    o = Path(a.out)
    o.parent.mkdir(parents=True, exist_ok=True)
    res = {"repo": a.repo_id, "pr": a.pr, "scenario": a.scenario, "rep": a.rep, "base": a.base, "head": a.head,
           "node": sh(["node", "--version"], "."), "runner": {k: os.environ.get(k) for k in
                                                               ("RUNNER_NAME", "ImageOS", "ImageVersion", "RUNNER_ARCH")},
           "cpu_count": os.cpu_count(), "ok": False}
    try:
        checkout(work, a.base if a.scenario == "seed" else a.head)
        res["install_ms"] = install(r, work)
        res["stryker"] = stryker_version(proj)
        if a.scenario == "C":
            res["rpr_version"] = sh(["rpr", "--version"], ".")
            code, j, meta = run_c(proj, a.base, o.with_suffix(""), a.timeout_min)
            run_dir = j.pop("run_dir", None)
            res.update(meta, rpr=j)
            if "counts" in j:
                res["in_diff"] = flat(mutants(Path(run_dir) / "mutation.json", proj), j["ranges"]) if j.get("resources") else []
                res.update(mutants=sum(j["counts"].values()), score=j["score"], ok=code in (0, 1))
        else:
            shutil.rmtree(proj / ".stryker-tmp", ignore_errors=True)
            report = proj / a.a_report
            report.unlink(missing_ok=True)
            extra = shlex.split(r["a_args"]) + (shlex.split(r["b"]["args"]) if a.scenario in ("seed", "B") else [])
            if a.scenario == "B":
                dst = proj / r["b"]["file"]
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy(a.seed, dst)
            code, stdout, meta = timed(["npx", "--no-install", "stryker", "run", *extra], proj, a.timeout_min * 60,
                                       ENV, o.with_suffix(""))
            res.update(meta, incremental_log=[l.strip()[:300] for l in stdout.splitlines() if "ncremental" in l][:8])
            if report.exists():
                ms = mutants(report, proj)
                res["ranges"] = diff_ranges(work, a.base, r["project"]) if a.scenario != "seed" else None
                res.update(mutants=len(ms), score=score(ms.values()),
                           in_diff=flat(ms, res["ranges"]) if res["ranges"] is not None else None,
                           ok=code in (0, 1) and not meta["timed_out"])
            if a.scenario == "seed":
                shutil.copy(proj / r["b"]["file"], o.parent / "incremental.json")
    except Exception as e:  # noqa: BLE001 - recorded in the result, the job still uploads it
        res["error"] = f"{type(e).__name__}: {str(e)[-1500:]}"
    res["cmd"] = ["rpr" if c.endswith("/rpr") else c for c in res.get("cmd", [])]
    if not res["ok"]:  # enough to see why, without uploading whole logs
        for ext in ("stdout", "stderr"):
            f = o.with_suffix(f".{ext}")
            res[f"{ext}_tail"] = f.read_text(errors="replace")[-3000:] if f.exists() else None
    o.write_text(json.dumps(res, indent=1))
    print(json.dumps({k: res.get(k) for k in ("repo", "pr", "scenario", "rep", "ok", "exit", "timed_out", "wall_ms",
                                              "mutants", "score", "error")}))
    if a.scenario == "seed" and not (o.parent / "incremental.json").exists():
        sys.exit(1)


def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def verdicts(run):
    return {tuple(x[:7]): x[7] for x in run.get("in_diff") or []}


def cmd_aggregate(a):
    runs = [json.loads(f.read_text()) for f in Path(a.results).glob("**/r-*.json")]
    scans = {json.loads(f.read_text())["repo"]: json.loads(f.read_text()) for f in Path(a.scans).glob("**/scan-*.json")}
    groups = {}
    for r in runs:
        groups.setdefault((r["repo"], r["pr"]), {}).setdefault(r["scenario"], []).append(r)
    rows = []
    for (repo, pr), g in sorted(groups.items()):
        ok = {s: sorted([r for r in rs if r.get("ok")], key=lambda r: r["rep"]) for s, rs in g.items()}
        bad = {s: [{"rep": r.get("rep"), "exit": r.get("exit"), "timed_out": r.get("timed_out"), "error": r.get("error")}
                   for r in rs if not r.get("ok")] for s, rs in g.items()}
        row = {"repo": repo, "pr": pr, "failures": {s: v for s, v in bad.items() if v}}
        for s in ("A", "B", "C"):
            rs = ok.get(s, [])
            row[s] = {"n": len(rs), "wall_ms": med([r["wall_ms"] for r in rs]),
                      "cpu_ms": med([r["cpu_user_ms"] + r["cpu_sys_ms"] for r in rs]),
                      "mutants": rs[0].get("mutants") if rs else None, "score": rs[0].get("score") if rs else None,
                      "stable": len({json.dumps(sorted(verdicts(r).items(), key=str)) for r in rs}) <= 1}
        for s in ("A", "B"):
            if row[s]["wall_ms"] and row["C"]["wall_ms"]:
                row[f"speedup_{s}_wall"] = round(row[s]["wall_ms"] / row["C"]["wall_ms"], 2)
                row[f"speedup_{s}_cpu"] = round(row[s]["cpu_ms"] / row["C"]["cpu_ms"], 2)
        if ok.get("C"):
            c = ok["C"][0]
            ranges = c["rpr"]["ranges"]
            cv = verdicts(c)
            for s in ("A", "B"):
                if not ok.get(s):
                    continue
                x = ok[s][0]
                xv = {k: v for k, v in verdicts(x).items() if in_scope(k, ranges)}
                mism = [{"key": list(k), s: xv.get(k), "C": cv.get(k)} for k in sorted(xv.keys() | cv.keys(), key=str)
                        if xv.get(k) != cv.get(k)]
                surv = [k for k, v in xv.items() if v in UNDETECTED]
                row[f"parity_{s}"] = {"total": len(xv), "c_total": len(cv), "same": sum(xv[k] == cv.get(k) for k in xv),
                                      "survivors": len(surv), "survivors_same": sum(xv[k] == cv.get(k) for k in surv),
                                      "ranges_match": x.get("ranges") == ranges, "mismatches": mism[:50]}
        rows.append(row)
    repos = {}
    for row in rows:
        repos.setdefault(row["repo"], []).append(row)
    per_repo = {}
    for repo, rs in repos.items():
        par = [r["parity_A"] for r in rs if "parity_A" in r]
        per_repo[repo] = {"prs": len(rs), "median_speedup_wall": med([r.get("speedup_A_wall") for r in rs]),
                          "median_speedup_cpu": med([r.get("speedup_A_cpu") for r in rs]),
                          "median_speedup_B_wall": med([r.get("speedup_B_wall") for r in rs]),
                          "parity_same": sum(p["same"] for p in par), "parity_total": sum(p["total"] for p in par),
                          "survivors_same": sum(p["survivors_same"] for p in par),
                          "survivors_total": sum(p["survivors"] for p in par),
                          "prs_with_survivors": sum(p["survivors"] > 0 for p in par),
                          "failed_runs": sum(len(v) for r in rs for v in r["failures"].values())}
    allpar = [r["parity_A"] for r in rows if "parity_A" in r]
    overall = {"prs": len(rows), "median_speedup_wall": med([r.get("speedup_A_wall") for r in rows]),
               "median_speedup_cpu": med([r.get("speedup_A_cpu") for r in rows]),
               "speedup_wall_range": [min((r["speedup_A_wall"] for r in rows if "speedup_A_wall" in r), default=None),
                                      max((r["speedup_A_wall"] for r in rows if "speedup_A_wall" in r), default=None)],
               "parity_same": sum(p["same"] for p in allpar), "parity_total": sum(p["total"] for p in allpar),
               "survivors_same": sum(p["survivors_same"] for p in allpar),
               "survivors_total": sum(p["survivors"] for p in allpar),
               "runs": len(runs), "failed_runs": sum(not r.get("ok") for r in runs)}
    if a.jobs and Path(a.jobs).exists():
        jobs = json.loads(Path(a.jobs).read_text())
        mins = {}
        for j in jobs:
            if j.get("started_at") and j.get("completed_at"):
                t = (time.mktime(time.strptime(j["completed_at"], "%Y-%m-%dT%H:%M:%SZ"))
                     - time.mktime(time.strptime(j["started_at"], "%Y-%m-%dT%H:%M:%SZ"))) / 60
                kind = j["name"].split(" ")[0]
                mins[kind] = mins.get(kind, 0) + t
        overall["runner_minutes"] = {k: round(v, 1) for k, v in sorted(mins.items())}
        overall["runner_minutes_total"] = round(sum(mins.values()), 1)
    meta = {"node": sorted({r.get("node") for r in runs if r.get("node")}),
            "rpr": sorted({r.get("rpr_version") for r in runs if r.get("rpr_version")}),
            "images": sorted({f"{r['runner'].get('ImageOS')} {r['runner'].get('ImageVersion')}" for r in runs}),
            "cpu_count": sorted({r.get("cpu_count") for r in runs})}
    summary = {"overall": overall, "per_repo": per_repo, "meta": meta, "rows": rows,
               "scans": {k: {x: v[x] for x in ("candidates_total", "scan_minutes", "selected")} | {
                   "skipped": [{"pr": s["pr"], "skip": s.get("skip")} for s in v["scanned"] if not s["valid"]]}
                   for k, v in scans.items()}}
    o = Path(a.out)
    o.mkdir(parents=True, exist_ok=True)
    (o / "summary.json").write_text(json.dumps(summary, indent=1))
    (o / "summary.md").write_text(markdown(summary))
    print(markdown(summary))


def fmt_s(ms):
    return "n/a" if ms is None else (f"{ms / 1000:.1f} s" if ms < 120_000 else f"{ms / 60000:.1f} min")


def x(v):
    return "n/a" if v is None else f"{v:g}×"


def markdown(s):
    o, m = s["overall"], s["meta"]
    L = ["# OSS benchmark: full StrykerJS (A) vs Stryker incremental (B) vs rpr (C)", "",
         f"Runners: {', '.join(m['images'])}, {m['cpu_count']} CPUs; Node {', '.join(m['node'])}; rpr {', '.join(m['rpr'])}.",
         "", f"**Median wall speedup A/C over {o['prs']} PRs: {x(o['median_speedup_wall'])}** "
             f"(range {x(o['speedup_wall_range'][0])} to {x(o['speedup_wall_range'][1])}); CPU {x(o['median_speedup_cpu'])}. "
             f"Verdict parity on the diff: {o['parity_same']}/{o['parity_total']} mutants, survivors "
             f"{o['survivors_same']}/{o['survivors_total']}. Failed or timed-out runs: {o['failed_runs']} of {o['runs']}.", ""]
    if "runner_minutes_total" in o:
        L += [f"Runner minutes: {o['runner_minutes_total']} ({', '.join(f'{k} {v}' for k, v in o['runner_minutes'].items())}).", ""]
    L += ["| Repo | PRs | Median speedup wall / CPU | B/C wall | Parity (same/total) | Survivors (same/total, PRs) | Failed runs |",
          "|---|---|---|---|---|---|---|"]
    for k, v in s["per_repo"].items():
        L.append(f"| {k} | {v['prs']} | {x(v['median_speedup_wall'])} / {x(v['median_speedup_cpu'])} | "
                 f"{x(v['median_speedup_B_wall'])} | {v['parity_same']}/{v['parity_total']} | "
                 f"{v['survivors_same']}/{v['survivors_total']} ({v['prs_with_survivors']} PRs) | {v['failed_runs']} |")
    L += ["", "| Repo | PR | A wall (n) | A CPU | A mutants / score | B wall (n) | C wall (n) | C mutants / score | "
              "Speedup wall / CPU | Parity A=C (survivors) | Failures |", "|---|---|---|---|---|---|---|---|---|---|---|"]
    for r in s["rows"]:
        A, B, C, p = r["A"], r["B"], r["C"], r.get("parity_A")
        par = (f"{p['same']}/{p['total']} ({p['survivors_same']}/{p['survivors']})"
               + ("" if p["ranges_match"] else " ranges differ")) if p else "n/a"
        fails = "; ".join(f"{k}: {len(v)}" for k, v in r["failures"].items()) or "-"
        L.append(f"| {r['repo']} | #{r['pr']} | {fmt_s(A['wall_ms'])} ({A['n']}) | {fmt_s(A['cpu_ms'])} | "
                 f"{A['mutants']} / {A['score']} | {fmt_s(B['wall_ms'])} ({B['n']}) | {fmt_s(C['wall_ms'])} ({C['n']}) | "
                 f"{C['mutants']} / {C['score']} | {x(r.get('speedup_A_wall'))} / {x(r.get('speedup_A_cpu'))} | {par} | {fails} |")
    return "\n".join(L) + "\n"


def cmd_selftest(_):
    """Diff parser on a throwaway repo, plan matrix shape, and aggregate on synthetic results."""
    import tempfile
    from pilot import self_test
    self_test()
    with tempfile.TemporaryDirectory() as t:
        w = Path(t)
        g = lambda *c: sh(["git", "-c", "user.email=t@t", "-c", "user.name=t", *c], w)
        g("init", "-q", "-b", "main")
        (w / "pkg").mkdir()
        (w / "pkg/a.ts").write_text("1\n2\n3\n4\n5\n")
        (w / "b.ts").write_text("x\n")
        g("add", ".")
        g("commit", "-qm", "base")
        (w / "pkg/a.ts").write_text("1\nTWO\n3\n4\n5\nsix\nseven\n")  # line 2 changed, 6-7 added
        (w / "b.ts").write_text("y\n")                                      # outside the project
        g("commit", "-qam", "head")
        assert diff_ranges(w, "HEAD~1", "pkg") == [{"path": "a.ts", "start": 2, "end": 2},
                                                   {"path": "a.ts", "start": 6, "end": 7}], diff_ranges(w, "HEAD~1", "pkg")
        assert [r["path"] for r in diff_ranges(w, "HEAD~1", ".")] == ["b.ts", "pkg/a.ts", "pkg/a.ts"]
        scans, res = w / "scans", w / "res"
        scans.mkdir(), res.mkdir()
        (scans / "scan-brazilian-utils.json").write_text(json.dumps({"repo": "brazilian-utils", "candidates_total": 1,
            "scan_minutes": 1, "scanned": [], "selected": [{"pr": 1, "base": "b", "head": "h"}]}))
        os.environ.pop("GITHUB_OUTPUT", None)
        cmd_plan(argparse.Namespace(scans=str(scans), reps=2, scenarios="A,B,C"))
        k = ["src/a.ts", "M", 2, 1, 2, 5, "x"]
        rng = [{"path": "src/a.ts", "start": 2, "end": 2}]
        base = {"repo": "brazilian-utils", "pr": 1, "ok": True, "cpu_sys_ms": 0, "mutants": 1, "score": 0.0,
                "ranges": rng, "runner": {}, "cpu_count": 4}
        for sc, wall, st in (("A", 600_000, "Survived"), ("C", 20_000, "Survived")):
            for rep in (1, 2):
                (res / f"r-{sc}-{rep}.json").write_text(json.dumps({**base, "scenario": sc, "rep": rep, "wall_ms": wall,
                    "cpu_user_ms": wall, "in_diff": [[*k, st]], "rpr": {"ranges": rng}}))
        cmd_aggregate(argparse.Namespace(results=str(res), scans=str(scans), jobs=None, out=str(w / "out")))
        r = json.loads((w / "out/summary.json").read_text())["rows"][0]
        assert r["speedup_A_wall"] == 30 and r["parity_A"]["survivors_same"] == 1 and r["A"]["stable"], r
    print("suite self-test ok")


def main():
    ap = argparse.ArgumentParser()
    sp = ap.add_subparsers(dest="cmd", required=True)
    p = sp.add_parser("repos")
    p.add_argument("--select", default="all")
    p = sp.add_parser("scan")
    p.add_argument("--repo-id", required=True)
    p.add_argument("--n", type=int, default=10)
    p.add_argument("--min-survivor-prs", type=int, default=3)
    p.add_argument("--limit", type=int, default=300, help="merged PRs to list")
    p.add_argument("--budget-min", type=float, default=150)
    p.add_argument("--timeout-min", type=float, default=60)
    p.add_argument("--work", default="target-repo")
    p.add_argument("--out", required=True)
    p = sp.add_parser("plan")
    p.add_argument("--scans", required=True)
    p.add_argument("--reps", type=int, default=3)
    p.add_argument("--scenarios", default="A,B,C")
    p = sp.add_parser("run")
    for k in ("--repo-id", "--base", "--head", "--scenario", "--out"):
        p.add_argument(k, required=True)
    p.add_argument("--pr", type=int, required=True)
    p.add_argument("--rep", type=int, default=1)
    p.add_argument("--seed")
    p.add_argument("--a-report", default="reports/mutation/mutation.json")
    p.add_argument("--timeout-min", type=float, default=60)
    p.add_argument("--work", default="target-repo")
    sp.add_parser("selftest")
    p = sp.add_parser("aggregate")
    p.add_argument("--results", required=True)
    p.add_argument("--scans", required=True)
    p.add_argument("--jobs")
    p.add_argument("--out", required=True)
    a = ap.parse_args()
    {"repos": cmd_repos, "scan": cmd_scan, "plan": cmd_plan, "run": cmd_run, "aggregate": cmd_aggregate,
     "selftest": cmd_selftest}[a.cmd](a)


if __name__ == "__main__":
    main()
