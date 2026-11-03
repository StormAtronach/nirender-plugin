"""One run of the game with the nirender mod, through the testing instance.

`prepare()` builds the plugin and puts it into the mod "Harness - nirender" of the testing
instance. `Run` holds the folder of a run and starts the game with a run spec. The spec goes to
the scenario of the mod (nirender/test.lua, perf.lua or video.lua). For the run, that mod and the
other mods of the run ("Harness - MGE XE" and so on) go on at the top of the mod list, over the
mods of the instance (its own "nirender" is the release); after the run the mod list is as it
was. The mods of the instance and the game folder are not written: Root Builder copies the Root
folders of the run's mods into the game folder at the start and takes them out at the end.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import time

import common  # noqa: E402
from common import REPO, local, local_file_hint, sibling

HARNESS_DIR = local("harness", sibling("moreFPS", "docs", "scripts", "test-harness-2026-10-03"))
if not os.path.isfile(os.path.join(HARNESS_DIR, "harness.py")):
    raise SystemExit("session.py: no harness.py in %s. Set the key \"harness\" in %s." % (HARNESS_DIR, local_file_hint()))
sys.path.insert(0, HARNESS_DIR)
import harness  # noqa: E402

MOD_NAME = harness.RUN_MOD_PREFIX + "nirender"
# The plugin of the run: the build of the repo, or another DLL for one arm of an A/B
# (`NIRENDER_PLUGIN_DLL`; the "before" of an A/B is built from a commit).
DLL = os.environ.get("NIRENDER_PLUGIN_DLL") or os.path.join(REPO, "target", "i686-pc-windows-msvc", "release", "akulakhan.dll")
DIST = os.path.join(REPO, "target", "dist", "nirender")
RESULTS = os.path.join(REPO, "results")
SPEC_NAME = "nirender_test"
# MGE XE writes its log into the game folder. Mod Organizer moves it to overwrite at its exit.
MGE_LOGS = (os.path.join(harness.GAME_DIR, "mgeXE.log"), os.path.join(harness.MO2_ROOT, "overwrite", "Root", "mgeXE.log"))


def use_instance(instance, game_dir=None, profile="Default"):
    """Points the harness at another Mod Organizer instance for this process: its folder, its
    game folder and its profile. The harness has the places of the testing instance built in;
    they become those of this one. A runner calls this before it touches the instance."""
    global MGE_LOGS
    harness.MO2_ROOT = instance
    harness.MO2_EXE = os.path.join(instance, "ModOrganizer.exe")
    harness.PROFILE = os.path.join(instance, "profiles", profile)
    harness.MOD_DIR = os.path.join(instance, "mods", harness.MOD_NAME)
    harness.SPEC_DIR = os.path.join(harness.MOD_DIR, "MWSE", "config")
    harness.GAME_DIR = game_dir or harness.GAME_DIR
    harness.MWSE_LOGS = (os.path.join(instance, "overwrite", "Root", "MWSE.log"), os.path.join(harness.GAME_DIR, "MWSE.log"))
    harness.WARNINGS = (os.path.join(instance, "overwrite", "Root", "Warnings.txt"), os.path.join(harness.GAME_DIR, "Warnings.txt"))
    harness.ROOT_BUILDER_DATA = os.path.join(instance, "plugins", "data", "RootBuilder")
    MGE_LOGS = (os.path.join(harness.GAME_DIR, "mgeXE.log"), os.path.join(instance, "overwrite", "Root", "mgeXE.log"))


def mwse_config_source():
    """The MWSE.json that the game reads in this instance: the one in overwrite, else the one of
    the enabled mod with the highest priority that has it, else None."""
    rel = os.path.join("MWSE", "config", "MWSE.json")
    candidates = [os.path.join(harness.MO2_ROOT, "overwrite", rel)]
    lines, _ = harness.read_modlist()
    candidates += [os.path.join(harness.MO2_ROOT, "mods", line[1:], rel) for line in lines if line.startswith("+")]
    return next((path for path in candidates if os.path.isfile(path)), None)


def stage_background_config(deployment):
    """MWSE's setting RunInBackground on for the run (the owner, 2026-10-20: MWSE has the setting,
    the mod patches nothing). A copy of the instance's MWSE.json with the key true goes into the
    run mod, over the instance's copy. MWSE writes the file back at its start, into the same
    place; restore() removes it. A copy in overwrite would win over the run mod's: the runner
    stops and says so, the owner moves it."""
    path = os.path.join(harness.SPEC_DIR, "MWSE.json")
    if os.path.exists(path):
        raise SystemExit("refusing to overwrite existing %s: a run is going, or one left it behind" % path)
    source = mwse_config_source()
    if source and source.startswith(os.path.join(harness.MO2_ROOT, "overwrite")):
        raise SystemExit("%s is in overwrite and would win over the run's copy: move it into a mod" % source)
    values = json.load(open(source, encoding="utf-8")) if source else {}
    values["RunInBackground"] = True
    os.makedirs(harness.SPEC_DIR, exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(values, fh, indent=2)
    deployment.create(path)
    print("  MWSE.json with RunInBackground = true <- %s" % (source or "the defaults (no MWSE.json in the instance)"))


def loaded_lua(text):
    """The files of the nirender mod that a Lua text loads, as names under the mod's folder: a
    dofile of `mods\\\\nirender\\\\<name>.lua` or `mods/nirender/<name>.lua`, and a require or include
    of `nirender.<name>`. A name built at run time (`"nirender." .. spec.scenario`, the run spec
    of a harness run) is not a file of an install, and it is not found. xtask's `loaded_lua` of
    MGE XE has the same rule for the build of an archive."""
    names = [match.replace("\\\\", "/").replace("\\", "/").lower() + ".lua" for match in re.findall(r"mods[\\/]+nirender[\\/]+([\w.\\/-]+?)\.lua", text, re.I)]
    names += [match.replace(".", "/").lower() + ".lua" for match in re.findall(r"""(?:require|include)\s*\(?\s*["']nirender\.([\w.]+)["']""", text, re.I)]
    return names


def stage_archive_lua(archive, dist):
    """The mod's Lua files of a run on a release archive (an extracted folder): the archive's files
    take the place of the repo's, and a file that the archive's Lua loads and the archive lacks
    leaves the run's mod, so that the run fails as a player's game does (v0.3.3: the harness's
    copies of two files hid that the archive had only main.lua). The other files of the repo's
    mod stay: the scenario that a run spec starts, which a player's game never loads.

    Returns (the archive's Lua files, the loaded files that it lacks)."""
    source = os.path.join(archive, "Data Files", "MWSE", "mods", "nirender")
    target = os.path.join(dist, "MWSE", "mods", "nirender")
    shipped = {name.lower(): name for name in os.listdir(source)} if os.path.isdir(source) else {}
    if "main.lua" not in shipped:
        raise SystemExit("the archive at %s has no Data Files/MWSE/mods/nirender/main.lua" % archive)
    for name in shipped.values():
        shutil.copy2(os.path.join(source, name), os.path.join(target, name))
    # What the archive's main.lua reaches, through the files that the archive has.
    reached, missing, queue = set(), set(), ["main.lua"]
    while queue:
        name = queue.pop()
        if name in reached:
            continue
        reached.add(name)
        if name in shipped:
            queue += loaded_lua(open(os.path.join(source, shipped[name]), encoding="utf-8", errors="replace").read())
        else:
            missing.add(name)
    for name in missing:
        place = os.path.join(target, *name.split("/"))
        if os.path.isfile(place):
            os.remove(place)
    return sorted(shipped), sorted(missing)


def prepare(build=True, features=(), extra_files=(), archive=None, plugin=True):
    """Builds the plugin, runs the offline tests and installs the mod into the testing instance.

    `features` are cargo features of the build: ("rig",) for a run of the comparison rig.
    With `plugin` False the mod gets the Lua files only, no akulakhan.dll: a reference run
    without the plugin (`plugin_mods` names the mods of the instance to switch off too).
    `extra_files` are (source, path in the mod) pairs that go into the run's mod for this run
    only: test files, or another MWSE.dll as `Root/MWSE.dll`. The run's mod wins over the mods
    of the instance; the next install takes them out. `archive` is an extracted release archive:
    the mod's Lua of the run is the archive's ([`stage_archive_lua`])."""
    if build:
        env = dict(os.environ)
        env["PATH"] = os.path.join(os.path.expanduser("~"), ".cargo", "bin") + os.pathsep + env["PATH"]
        with_features = ["--features", ",".join(features)] if features else []
        for command in (["cargo", "test", "--quiet"] + with_features, ["cargo", "build", "--release", "--quiet"] + with_features):
            print("> " + " ".join(command), flush=True)
            if subprocess.run(command, cwd=REPO, env=env).returncode != 0:
                raise SystemExit("build failed")
    stage_mod(DIST, DLL, plugin, archive)
    for source, target in extra_files:
        if not os.path.isfile(source):
            raise SystemExit("no file %s for %s" % (source, target))
        destination = os.path.join(DIST, *target.replace("\\", "/").split("/"))
        os.makedirs(os.path.dirname(destination), exist_ok=True)
        shutil.copy2(source, destination)
        print("  extra file: %s <- %s" % (target, source))
    # The harness installs its own mod by default. These names make it install this one.
    harness.MOD_SRC = DIST
    harness.MOD_NAME = MOD_NAME
    harness.MOD_DIR = os.path.join(harness.MO2_ROOT, "mods", MOD_NAME)
    harness.SPEC_DIR = os.path.join(harness.MOD_DIR, "MWSE", "config")
    harness.refuse_if_busy()
    # The mod stays off between runs: it would take the place of the release in a game by hand.
    for change in harness.install_mod(enable=False):
        print("  %s: %s" % (MOD_NAME, change))


def stage_mod(dist, dll, plugin, archive=None):
    """The mod as it is installed, in `dist`: the Lua files and, with `plugin`, the DLL. The
    symbol file goes beside the DLL when the build has one (the owner, 2026-10-16): a profile
    of the run then resolves the plugin's symbols without a search path. This is the Harness
    mod of a run, never an install or an archive, where no pdb goes. Without `plugin` the mod
    has no file of the plugin: a run without the plugin fails if it has one."""
    if os.path.isdir(dist):
        shutil.rmtree(dist)
    shutil.copytree(os.path.join(REPO, "mod"), dist)
    lib = os.path.join(dist, "MWSE", "lib")
    if plugin:
        os.makedirs(lib, exist_ok=True)
        shutil.copy2(dll, os.path.join(lib, "akulakhan.dll"))
    if archive:
        shipped, missing = stage_archive_lua(archive, dist)
        print("archive Lua: %s; loaded by it and not in the archive (the run lacks them, as a player's game): %s" % (", ".join(shipped), ", ".join(missing) or "none"), flush=True)
    pdb = os.path.splitext(dll)[0] + ".pdb"
    if plugin and os.path.isfile(pdb):
        shutil.copy2(pdb, os.path.join(lib, "akulakhan.pdb"))
    if not plugin:
        found = [name for name in ("akulakhan.dll", "akulakhan.pdb") if os.path.isfile(os.path.join(lib, name))]
        if found:
            raise SystemExit("the mod of a run without the plugin has %s" % ", ".join(found))


def plugin_mods(modlist, root=None):
    """The enabled mods of a mod list, other than the run's own mod, that hold akulakhan.dll: a
    run without the plugin switches them off. `root` is the Mod Organizer instance."""
    root = root or harness.MO2_ROOT
    names = [line[1:] for line in modlist if line.startswith("+") and line[1:] != MOD_NAME]
    return [name for name in names if os.path.isfile(os.path.join(root, "mods", name, harness.PLUGIN_FILE))]


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

    def launch(self, spec, timeout, tag, mods=(), disable=(), facts=()):
        """Starts the game with `spec` and follows it until the scenario says DONE.

        `mods` are the other mods of the run, as (name, {path in the mod: source file}); the
        names start with harness.RUN_MOD_PREFIX. `disable` are mods of the instance that are off
        for the run. The mod list is as it was after the run. `facts` are more lines for the record
        of what the run ran (`harness.write_setup`).

        A run is a background run unless NIRENDER_FOREGROUND=1 (the owner, 2026-10-20): the
        runner stages MWSE.json with RunInBackground = true (stage_background_config) and does
        not bring the game window forward. A run with real clicks or OBS window capture needs
        the foreground.

        Returns True when the scenario finished within `timeout` seconds."""
        foreground = os.environ.get("NIRENDER_FOREGROUND", "0") == "1"
        spec = dict(spec, background=not foreground)
        deployment = harness.Deployment()
        self.started = time.time()
        print("launching (budget %ds, results in %s)" % (timeout, self.dir), flush=True)
        try:
            deployment.write_spec(SPEC_NAME, spec)
            if not foreground:
                stage_background_config(deployment)
            deployment.enable_mod(MOD_NAME)
            for name, files in mods:
                deployment.use_mod(name, files)
            for name in disable:
                deployment.disable_mod(name)
            problem = ring_problem_of_instance()
            if problem:
                raise SystemExit(problem)
            done = harness.run_game(self.progress, timeout, tag, deployment, self.started, foreground=foreground, facts=facts)
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

    def plugin_lines(self):
        """The plugin's lines of this run (`common.plugin_lines`)."""
        return common.plugin_lines(self.dir)

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
        return mge_log_problems(open(path, encoding="utf-8", errors="replace").read())


# The ring mark of the stream (`RING_MAGIC` in src/stream_format.rs, "2RNG" in memory): the
# plugin writes it at the head of the ring and the helper asserts it. A plugin and a helper with
# different marks crash the helper at the start (host/src/live.rs, "not a ring of the plugin").
RING_MARK = re.compile(rb"[0-9]RNG")


def ring_marks(path):
    """The ring marks in the bytes of a plugin or a helper program: one in each build so far."""
    with open(path, "rb") as fh:
        return set(RING_MARK.findall(fh.read()))


def ring_problem(plugin, helper):
    """Why `plugin` and `helper` cannot share a ring, or None. Paths may be None (no plugin or no
    helper in the run). A binary with no readable mark is not refused: the mark may have a new
    form."""
    if not plugin or not helper:
        return None
    plugin_marks, helper_marks = ring_marks(plugin), ring_marks(helper)
    if not plugin_marks or not helper_marks or plugin_marks & helper_marks:
        return None
    return ("the plugin %s has the ring mark %s and the helper %s has %s: the helper would crash at the start. "
            "Use a helper built with the plugin (--mge-build of the same build, or switch on the mod with the matching numidium.exe)"
            % (plugin, b", ".join(sorted(plugin_marks)).decode(), helper, b", ".join(sorted(helper_marks)).decode()))


def winning_root_file(name, root, modlist, game_dir):
    """The path of the program file `name` in the game folder at the start (Root Builder): the
    overwrite's Root, then the enabled mods' Root by priority, then the game folder. None if
    none has it."""
    places = [os.path.join(root, "overwrite", "Root", name)]
    places += [os.path.join(root, "mods", line[1:], "Root", name) for line in modlist if line.startswith("+")]
    places.append(os.path.join(game_dir, name))
    return next((path for path in places if os.path.isfile(path)), None)


def ring_problem_of_instance():
    """The ring check of the instance as the run starts it: the plugin and the helper that win in
    the mod list as it is now."""
    modlist = harness.read_modlist()[0]
    plugin = harness.winning_file(harness.PLUGIN_FILE, harness.MO2_ROOT, modlist, harness.GAME_DIR)
    helper = winning_root_file("numidium.exe", harness.MO2_ROOT, modlist, harness.GAME_DIR)
    return ring_problem(plugin and plugin[1], helper)


# The line of MGE XE (d3d8/cpp/main.cpp) when the wgpu renderer draws: MGE XE then runs as a
# proxy only, with no Direct3D device and none of its engine hooks.
WGPU_LINE = ">> Using the wgpu renderer"
# The line of a late fall-back to Direct3D 9 (d3d8/cpp/main.cpp, at FakeDirect3DCreate): MGE XE
# then needs its engine hooks again.
WGPU_FALLBACK_LINE = "wgpu renderer: not in use"


def mge_log_problems(text):
    """What the text of an mgeXE.log says went wrong. An empty list is good.

    On Direct3D 9, MGE XE must install engine hooks. "no engine hooks installed" (camera-relative
    rendering off) does not count as one. On the wgpu renderer it installs none by design; after a
    late fall-back to Direct3D 9 it must install them again."""
    problems = []
    hooks = [line for line in text.splitlines() if "hooks installed" in line and "no engine hooks installed" not in line]
    wgpu = WGPU_LINE in text and WGPU_FALLBACK_LINE not in text
    if not hooks and not wgpu:
        problems.append("MGE XE installed none of its engine hooks")
    failed = text.count("failed to load/compile")
    if failed:
        problems.append("MGE XE could not compile %d shaders" % failed)
    problems += [line.strip()[:160] for line in text.splitlines() if "Another mod" in line]
    return problems
