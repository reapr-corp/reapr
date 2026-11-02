#!/usr/bin/env python3
"""Phase durations from run.py logs. usage: phases.py RAW_DIR  (reads <variant>-<rep>.log/.json)

Markers are Stryker 9.6.1 log messages (sources under @stryker-mutator/core/dist/src, see README).
Times are when run.py *received* the line. Stryker also stamps each line with its own HH:MM:SS
clock (1 s resolution); `lag` is receipt minus that stamp, so lag > 1 s means the line was
emitted earlier than received (Stryker's event loop was busy and flushed output late).
"""
import json, re, statistics, sys, time
from collections import defaultdict
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
LOGLINE = re.compile(r"(\d\d):(\d\d):(\d\d) \(\d+\) (TRACE|DEBUG|INFO|WARN|ERROR|FATAL) ")
MARKERS = [  # (name, substring); first occurrence wins
    ("npx_up", "npm warn "),
    ("found", "INFO ProjectReader Found "),
    ("pool", "INFO ConcurrencyTokenProvider Creating "),
    ("sandbox", "DEBUG Sandbox Creating a sandbox for files in"),
    ("symlink", "DEBUG Sandbox Start symlink node_modules"),
    ("runner", "Started ChildProcessTestRunnerWorker"),
    ("dry_start", "INFO DryRunExecutor Starting initial test run"),
    ("dry_ok", "INFO DryRunExecutor Initial test run succeeded"),
    ("done", "INFO MutationTestExecutor Done in"),
    ("rmtmp", "DEBUG TemporaryDirectory Deleting stryker temp directory"),
]
PHASES = [  # (label, from, to)
    ("P0 start->first Stryker line", "start", "first"),
    ("  P0a start->npx up (npm warn line)", "start", "npx_up"),
    ("P1 ->ProjectReader Found", "first", "found"),
    ("P2 ->Starting initial test run", "found", "dry_start"),
    ("  P2a instrument+preprocess (->Creating N process(es))", "found", "pool"),
    ("  P2b checker init (->Creating a sandbox)", "pool", "sandbox"),
    ("  P2c sandbox copy (->Start symlink)", "sandbox", "symlink"),
    ("  P2d symlink+reporters (->runner worker started)", "symlink", "runner"),
    ("  P2e runner startup (->Starting initial test run)", "runner", "dry_start"),
    ("  P2bcde checker+sandbox+runner (info-level split)", "pool", "dry_start"),
    ("P3 dry run (->Initial test run succeeded)", "dry_start", "dry_ok"),
    ("P4 mutation testing (->Done in)", "dry_ok", "done"),
    ("P5 ->process exit", "done", "exit"),
    ("  P5a reporters/dispose (->Deleting temp dir)", "done", "rmtmp"),
    ("  P5b temp dir delete+exit", "rmtmp", "exit"),
    ("TOTAL wall", "start", "exit"),
]


def parse(log):
    t, lag = {}, {}
    for raw in open(log, errors="replace"):
        ms, stream, rest = (raw.rstrip("\n").split(" ", 2) + [""])[:3]
        ms = int(ms)
        line = ANSI.sub("", rest)
        if stream == "meta":
            t["start" if line.startswith("START") else "exit"] = ms
            continue
        m = LOGLINE.search(line)
        if m and "first" not in t:
            t["first"] = ms
        for name, sub in MARKERS:
            if name not in t and sub in line:
                t[name] = ms
                if m:
                    lt = time.localtime(ms / 1000)
                    stamp = time.mktime((*lt[:3], int(m[1]), int(m[2]), int(m[3]), 0, 0, -1))
                    lag[name] = ms - int(stamp * 1000)
    return t, lag


def fmt(xs):
    if not xs:
        return "n/a"
    return f"{statistics.median(xs)/1000:6.1f} ({min(xs)/1000:.1f}-{max(xs)/1000:.1f})"


QUIET_LOAD = 4.0  # runs starting with 1-min loadavg above this are "noisy": kept in raw, out of medians


def main():
    raw = Path(sys.argv[1])
    runs = defaultdict(list)
    for log in sorted(raw.glob("*-*.log")):
        m = re.fullmatch(r"([A-Z][0-9]?)-([0-9]+)", log.stem)
        if not m:
            continue
        variant = m[1]
        t, lag = parse(log)
        meta = json.loads(log.with_suffix(".json").read_text())
        reapr = None
        if variant == "A":
            d = json.loads(log.with_suffix(".stdout").read_text())
            reapr = {"duration_ms": d["duration_ms"], **d["resources"], "score": d["score"], "counts": d["counts"]}
        runs[variant].append({"rep": log.stem, "t": t, "lag": lag, "meta": meta, "reapr": reapr,
                              "quiet": meta["loadavg_before"][0] <= QUIET_LOAD})
    out = {}
    print(f"# per-run (load = 1-min loadavg just before spawn; quiet = load <= {QUIET_LOAD})")
    print("| run | load | quiet | wall s | exit | top-3 CPU before |")
    print("|---|---|---|---|---|---|")
    for v, rs in sorted(runs.items()):
        for r in sorted(rs, key=lambda r: int(r["rep"].rsplit("-", 1)[1])):
            mt = r["meta"]
            print(f"| {r['rep']} | {mt['loadavg_before'][0]:.1f} | {'y' if r['quiet'] else 'n'} | "
                  f"{mt['wall_ms']/1000:.1f} | {mt['exit']} | {'; '.join(mt.get('top3_before', ['(not recorded)']))} |")
    for v, rs in sorted(runs.items()):
        quiet = [r for r in rs if r["quiet"]]
        noisy = [r for r in rs if not r["quiet"]]
        use, tag = (quiet, "quiet runs") if quiet else (rs, "ALL runs, MEASURED UNDER LOAD")
        print(f"\n## {v}  (median (min-max) s over {tag}: n={len(use)} of {len(rs)})")
        rows = {}
        for label, a, b in PHASES:
            xs = [r["t"][b] - r["t"][a] for r in use if a in r["t"] and b in r["t"]]
            rows[label] = xs
            if xs:
                print(f"| {label:55} | {fmt(xs)} |")
        cpu = [r["meta"]["cpu_user_ms"] + r["meta"]["cpu_sys_ms"] for r in use]
        print(f"| {'CPU user+sys (wait4 rusage, whole tree)':55} | {fmt(cpu)} |")
        print(f"| {'max RSS MB (largest single process)':55} | {statistics.median(r['meta']['max_rss_kb'] for r in use)/1024:.0f} |")
        print(f"| {'loadavg(1m) before run':55} | {' '.join(f'{r['meta']['loadavg_before'][0]:.1f}' for r in use)} |")
        if quiet and noisy:
            print(f"noisy-vs-quiet wall: noisy {fmt([r['meta']['wall_ms'] for r in noisy])} (n={len(noisy)}) "
                  f"vs quiet {fmt([r['meta']['wall_ms'] for r in quiet])} (n={len(quiet)})")
        big = sorted({(n, l) for r in use for n, l in r["lag"].items() if l > 1100}, key=lambda x: -x[1])
        if big:
            print(f"lag>1.1s (receipt later than Stryker's own stamp): {big[:6]}")
        if v == "A":
            for k in ("duration_ms", "cpu_user_ms", "cpu_sys_ms", "max_rss_kb"):
                print(f"reapr json {k}: {[r['reapr'][k] for r in use]}")
            print("reapr score/counts:", {(r["reapr"]["score"], json.dumps(r["reapr"]["counts"], sort_keys=True)) for r in rs})
        out[v] = {"phases_ms": rows, "cpu_ms": cpu, "runs": rs}
    (raw.parent / "summary.json").write_text(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
