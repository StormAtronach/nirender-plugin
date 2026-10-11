"""One run of the game with the nirender mod, through the testing instance.

`prepare()` builds and installs the mod. `Run` holds the folder of a run and starts the game
with a run spec. The spec goes to the scenario of the mod (nirender/test.lua, perf.lua or
video.lua). The testing instance is put back after each run. The mod "nirender" stays
installed: it does nothing without a run spec and with mode "off".
"""
import os
import shutil
import subprocess
import sys
import time

from common import REPO

HARNESS_DIR = r"C:\Users\ivan_\source\repos\moreFPS\docs\scripts\test-harness-2026-10-03"
sys.path.insert(0, HARNESS_DIR)
import harness  # noqa: E402

MOD_NAME = "nirender"
DLL = os.path.join(REPO, "target", "i686-pc-windows-msvc", "release", "nirender.dll")
DIST = os.path.join(REPO, "target", "dist", MOD_NAME)
RESULTS = os.path.join(REPO, "results")
SPEC_NAME = "nirender_test"
# MGE XE writes its log into the game folder. Mod Organizer moves it to overwrite at its exit.
MGE_LOGS = (os.path.join(harness.GAME_DIR, "mgeXE.log"), os.path.join(harness.MO2_ROOT, "overwrite", "Root", "mgeXE.log"))


def prepare(build=True):
    """Builds the plugin, runs the offline tests and installs the mod into the testing instance."""
    if build:
        env = dict(os.environ)
        env["PATH"] = os.path.join(os.path.expanduser("~"), ".cargo", "bin") + os.pathsep + env["PATH"]
        for command in (["cargo", "test", "--quiet"], ["cargo", "build", "--release", "--quiet"]):
            print("> " + " ".join(command), flush=True)
            if subprocess.run(command, cwd=REPO, env=env).returncode != 0:
                raise SystemExit("build failed")
    # The mod as it is installed: the Lua files and the DLL. The pdb is never installed.
    if os.path.isdir(DIST):
        shutil.rmtree(DIST)
    shutil.copytree(os.path.join(REPO, "mod"), DIST)
    os.makedirs(os.path.join(DIST, "MWSE", "lib"), exist_ok=True)
    shutil.copy2(DLL, os.path.join(DIST, "MWSE", "lib", "nirender.dll"))
    # The harness installs its own mod by default. These names make it install this one.
    harness.MOD_SRC = DIST
    harness.MOD_NAME = MOD_NAME
    harness.MOD_DIR = os.path.join(harness.MO2_ROOT, "mods", MOD_NAME)
    harness.SPEC_DIR = os.path.join(harness.MOD_DIR, "MWSE", "config")
    harness.refuse_if_busy()
    for change in harness.install_mod():
        print("  nirender mod: %s" % change)


class Run:
    """The folder and the files of one run of the game."""

    def __init__(self, label):
        self.dir = os.path.join(RESULTS, time.strftime("%Y%m%d-%H%M%S") + "-" + label)
        os.makedirs(self.dir)
        self.progress = self.path("progress.log")
        self.report = self.path("report.tsv")
        self.log = self.path("nirender.log")
        self.started = None

    def path(self, name):
        return os.path.join(self.dir, name)

    def spec(self, scenario, **values):
        """The run spec: the files of the run, the scenario, and the values of the scenario."""
        spec = {
            "scenario": scenario,
            "progressFile": self.progress.replace("\\", "/"),
            "reportFile": self.report.replace("\\", "/"),
            "logFile": self.log.replace("\\", "/"),
        }
        spec.update(values)
        return spec

    def launch(self, spec, timeout, tag, swaps=()):
        """Starts the game with `spec` and follows it until the scenario says DONE.

        `swaps` are files to put in place for the run, as (source, destination, label). The
        files that were there come back after the run.

        Returns True when the scenario finished within `timeout` seconds."""
        deployment = harness.Deployment()
        self.started = time.time()
        print("launching (budget %ds, results in %s)" % (timeout, self.dir), flush=True)
        try:
            deployment.write_spec(SPEC_NAME, spec)
            for source, destination, label in swaps:
                deployment.swap(source, destination, label)
            done = harness.run_game(self.progress, timeout, tag, deployment, self.started)
        finally:
            # run_game also restores. This restore is for a failure before run_game gets to
            # that point.
            deployment.restore()
        harness.copy_logs(self.dir, self.started)
        for path in MGE_LOGS:
            if os.path.isfile(path) and os.path.getmtime(path) > self.started:
                shutil.copy2(path, self.path("mgeXE.log"))
                break
        return done

    def progress_lines(self, tag):
        if not os.path.isfile(self.progress):
            return []
        text = open(self.progress, encoding="utf-8", errors="replace").read()
        return [line.replace(tag, "").strip() for line in text.splitlines()]

    def mge_problems(self):
        """What the log of MGE XE says went wrong in this run. An empty list is good.

        The plugin must not change what MGE XE does. One version of the plugin put a wrapper
        on the IDirect3D8 of MGE XE. MGE XE then installed none of its hooks and compiled none
        of its shaders, and the tests of the plugin still passed."""
        path = self.path("mgeXE.log")
        if not os.path.isfile(path):
            return ["no fresh mgeXE.log: MGE XE did not run, or its log is in another place"]
        text = open(path, encoding="utf-8", errors="replace").read()
        problems = []
        if "hooks installed" not in text:
            problems.append("MGE XE installed none of its engine hooks")
        failed = text.count("failed to load/compile")
        if failed:
            problems.append("MGE XE could not compile %d shaders" % failed)
        problems += [line.strip()[:160] for line in text.splitlines() if "Another mod" in line]
        return problems
