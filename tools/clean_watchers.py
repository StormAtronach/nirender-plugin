"""Lists, and with --end ends, the shell tools that log watchers leave behind.

A watcher such as `tail -f log | grep ...`, started from Git Bash in the background, keeps its
`tail.exe` and `grep.exe` running after its shell is stopped (harness trap T58): 66 of them
had piled up between 2026-10-14 and 2026-10-21. This tool finds the processes of Git's shell
tools (tail, grep, sed, cut, sleep from Git's `usr\\bin`) older than the age given. It does not
touch `bash.exe`: the shell of a long run (a night script) is older than an hour and alive.

    python tools/clean_watchers.py            # list them
    python tools/clean_watchers.py --end      # end them
    python tools/clean_watchers.py --minutes 30 --end
"""
import argparse
import json
import subprocess
import sys

TOOLS = ("tail.exe", "grep.exe", "sed.exe", "cut.exe", "sleep.exe")
GIT_TOOLS = "\\git\\usr\\bin\\"


def processes(minutes):
    names = ",".join("'%s'" % name for name in TOOLS)
    script = (
        "$limit = (Get-Date).AddMinutes(-%d); "
        "Get-CimInstance Win32_Process | Where-Object { @(%s) -contains $_.Name -and $_.CreationDate -lt $limit } | "
        "ForEach-Object { [pscustomobject]@{ id = $_.ProcessId; name = $_.Name; path = $_.ExecutablePath; "
        "started = $_.CreationDate.ToString('yyyy-MM-dd HH:mm'); command = $_.CommandLine } } | ConvertTo-Json -Compress"
    ) % (minutes, names)
    out = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", script], capture_output=True, text=True).stdout.strip()
    if not out:
        return []
    found = json.loads(out)
    found = found if isinstance(found, list) else [found]
    return [p for p in found if GIT_TOOLS in (p.get("path") or "").lower()]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--minutes", type=int, default=60, help="only processes older than this (60 without this)")
    ap.add_argument("--end", action="store_true", help="end the processes found; without this the tool only lists them")
    args = ap.parse_args()

    found = processes(args.minutes)
    for p in found:
        print("%6d %-9s %s  %s" % (p["id"], p["name"], p["started"], (p.get("command") or "")[:140]))
    if not found:
        print("no shell tools older than %d minutes" % args.minutes)
        return 0
    if not args.end:
        print("%d processes; --end ends them" % len(found))
        return 0
    ids = ",".join(str(p["id"]) for p in found)
    subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command",
                    "foreach ($i in @(%s)) { try { Stop-Process -Id $i -Force -ErrorAction Stop } catch {} }" % ids])
    left = [p for p in processes(args.minutes) if str(p["id"]) in ids.split(",")]
    print("ended %d processes; %d still there" % (len(found) - len(left), len(left)))
    return 1 if left else 0


if __name__ == "__main__":
    sys.exit(main())
