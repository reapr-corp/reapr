#!/usr/bin/env python3
"""One timed run: restore incremental state, spawn CMD in the target repo, timestamp every output line.

usage: TARGET=<repo> [INCREMENTAL=<state file rel. to TARGET>] run.py OUT_PREFIX [--sample-sandbox] -- CMD...
writes OUT_PREFIX.log  ("<epoch_ms> <out|err> <line>" per line, plus START/EXIT markers)
       OUT_PREFIX.json (start/end ms, exit code, rusage of the whole waited-for tree, loadavg)
       OUT_PREFIX.stdout (raw stdout, only kept verbatim for Reapr's --json document)
"""
import json, os, shutil, subprocess, sys, threading, time
from pathlib import Path

TARGET = Path(os.environ["TARGET"])
BACKUP = Path(__file__).resolve().parent / "backup"
STATE = [("incremental.json", os.environ.get("INCREMENTAL", "reports/stryker-incremental.json"))]


def ms():
    return int(time.time() * 1000)


def count_tree(root):
    """Regular files and bytes under root, not following symlinks (node_modules is symlinked)."""
    files = size = links = 0
    for dirpath, dirnames, filenames in os.walk(root):
        for n in filenames:
            p = os.path.join(dirpath, n)
            st = os.lstat(p)
            if os.path.islink(p):
                links += 1
            else:
                files += 1
                size += st.st_size
        links += sum(os.path.islink(os.path.join(dirpath, d)) for d in dirnames)
    return {"files": files, "bytes": size, "symlinks": links}


def main():
    out = Path(sys.argv[1])
    rest = sys.argv[2:]
    sample = "--sample-sandbox" in rest
    cmd = rest[rest.index("--") + 1:]
    for name, rel in STATE:
        shutil.copy2(BACKUP / name, TARGET / rel)
    tmp = TARGET / ".stryker-tmp"
    before_sb = set(os.listdir(tmp)) if tmp.exists() else set()
    lock = threading.Lock()
    log = open(f"{out}.log", "w")
    stdout_raw = open(f"{out}.stdout", "w")
    # pcpu and comm only: full args would record other projects' paths into the results.
    top = subprocess.run(["ps", "-eo", "pcpu,comm", "--sort=-pcpu"], capture_output=True, text=True).stdout
    meta = {"cmd": cmd, "loadavg_before": os.getloadavg(),
            "top3_before": [" ".join(l.split()) for l in top.splitlines()[1:]
                            if l.split()[1] != "ps"][:3]}

    def emit(stream, line):
        with lock:
            log.write(f"{ms()} {stream} {line}")
            if not line.endswith("\n"):
                log.write("\n")
            if sample and "sandbox" not in meta and "Starting initial test run" in line:
                new = [d for d in os.listdir(tmp) if d not in before_sb]
                meta["sandbox"] = {d: count_tree(tmp / d) for d in new}
                meta["sandbox_counted_ms"] = ms()

    def pump(pipe, stream):
        for line in iter(pipe.readline, ""):
            if stream == "out":
                stdout_raw.write(line)
            emit(stream, line)

    meta["start_ms"] = ms()
    log.write(f"{meta['start_ms']} meta START\n")
    p = subprocess.Popen(cmd, cwd=TARGET, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, text=True, errors="replace")
    threads = [threading.Thread(target=pump, args=(p.stdout, "out")),
               threading.Thread(target=pump, args=(p.stderr, "err"))]
    for t in threads:
        t.start()
    _, status, ru = os.wait4(p.pid, 0)
    meta["end_ms"] = ms()
    p.returncode = os.waitstatus_to_exitcode(status)
    for t in threads:
        t.join()
    log.write(f"{meta['end_ms']} meta EXIT {p.returncode}\n")
    meta.update(exit=p.returncode, wall_ms=meta["end_ms"] - meta["start_ms"],
                cpu_user_ms=int(ru.ru_utime * 1000), cpu_sys_ms=int(ru.ru_stime * 1000),
                max_rss_kb=ru.ru_maxrss, loadavg_after=os.getloadavg())
    log.close()
    stdout_raw.close()
    Path(f"{out}.json").write_text(json.dumps(meta, indent=1))
    print(f"{out.name}: exit={p.returncode} wall={meta['wall_ms']}ms load={meta['loadavg_before'][0]:.1f}")


if __name__ == "__main__":
    main()
