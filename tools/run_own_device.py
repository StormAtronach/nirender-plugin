"""Runs the game without a Direct3D device: the d3d8.dll of this repository (or of a build of
MGE XE with --mge-build) takes the place of the one of MGE XE in the game folder for the run,
and the game gets the Direct3D 8 of the plugin. The host program draws the only picture, in
the window of the game.

    python tools/run_own_device.py --label owndevice [--backend vulkan]
        [--plugin-dll <akulakhan.dll>] [--log <file.txt>]
    python tools/run_own_device.py --label rel --archive <extracted release archive>  # the release check
    python -I tools/run_own_device.py --self-test    # the tests of the argument handling

The command of a run takes no variable prefix, redirect or chained command, so that one
allow rule of a session covers every run: `--plugin-dll` names the plugin DLL of the run
(the variable `NIRENDER_PLUGIN_DLL` is the fallback, then the release build of this
repository), and `--log` names a file that gets the runner's own output, written to the
console too (the programs the runner starts write to the console only).

The files of the run are in mods of the harness that are on for the run only (session.py):
"Harness - MGE XE" (d3d8.dll, numidium.exe), "Harness - config" (mgeXE.toml of the run) and
"Harness - exe" (--exe). The result is a picture of the host program after each step of the
scenario (host-<step>.png), the text of the host program (host.txt) and the lines of the
plugin about its device. The first lines of progress.log, `[runner]`, and setup.txt say what the
run ran (`harness.write_setup`).
"""
import argparse
import contextlib
import ctypes
import io
import json
import math
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import winreg
from ctypes import wintypes

# The modules of this folder, also under `python -I` (checks.py runs the self-test so).
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lua_checks  # noqa: E402
import session  # noqa: E402
import symbols  # noqa: E402
from common import REPO  # noqa: E402
from make_play_set import PDB_KEEP  # noqa: E402

TAG = "[nirender.owndevice] "
HOST_DIR = os.path.join(REPO, "host")
HOST = os.path.join(HOST_DIR, "target", "x86_64-pc-windows-msvc", "release", "nirender-host.exe")
D3D8_DIR = os.path.join(REPO, "d3d8")
D3D8 = os.path.join(D3D8_DIR, "target", "i686-pc-windows-msvc", "release", "d3d8.dll")
# The mods of the harness for the files of a run.
MGE_MOD = session.harness.RUN_MOD_PREFIX + "MGE XE"
CONFIG_MOD = session.harness.RUN_MOD_PREFIX + "config"
EXE_MOD = session.harness.RUN_MOD_PREFIX + "exe"
# The Lua bridge of mwse-mcp, as the testing instance has it (`--bridge`).
BRIDGE_DIR = os.path.join(session.harness.MO2_ROOT, "mods", "mwseMCP")


class Tee:
    """A stream that writes to the console and to the log file of the run (`--log`)."""

    def __init__(self, console, log):
        self.console = console
        self.log = log

    def write(self, text):
        self.console.write(text)
        self.log.write(text)

    def flush(self):
        self.console.flush()
        self.log.flush()


def plugin_dll_of(argument, environment, default):
    """The plugin DLL of a run and where it came from: `--plugin-dll`, else the variable
    `NIRENDER_PLUGIN_DLL`, else the release build of this repository."""
    if argument:
        return argument, "--plugin-dll"
    if environment.get("NIRENDER_PLUGIN_DLL"):
        return environment["NIRENDER_PLUGIN_DLL"], "NIRENDER_PLUGIN_DLL"
    return default, "the release build of this repository"


# The variables of the run itself, and the option or the place that sets each. `--env` does
# not take them: the runner is an allowed command (decision D6), so the DLL and the programs
# that a run loads come only from its own options, never from a free value.
RUNNER_VARIABLES = {
    "NIRENDER_PLUGIN_DLL": "--plugin-dll",
    "NIRENDER_HOST_EXE": "--mge-build (its numidium.exe), or the host build of this repository without it",
    "NIRENDER_HOST_LOG": "the runner (host.txt in the run folder)",
    "NIRENDER_BACKEND": "--backend",
    "NIRENDER_HOST_ARGS": "--start-frames and --window-shots",
}


def env_of(entries):
    """The variables of `--env NAME=VALUE`, in their order: a later value of a name wins. The
    value is all after the first `=`. A variable of the run itself is refused, with the option
    that sets it."""
    variables = {}
    for entry in entries:
        name, equals, value = entry.partition("=")
        if not equals or not name:
            raise SystemExit("--env takes NAME=VALUE, not %r" % entry)
        if name in RUNNER_VARIABLES:
            raise SystemExit("--env cannot set %s: the run sets it through %s" % (name, RUNNER_VARIABLES[name]))
        variables[name] = value
    return variables


def apply_env(environment, entries):
    """Puts the `--env` variables into `environment` (the runner's, which the game gets), then
    the default box time: a message box of the plugin, the helper or MGE XE closes itself after
    5 s, so that a run never waits for a click, and the box's text is in the log (the
    failure-path design, 5.1). A box time from `--env` or from the runner's environment is
    kept: a test of a box that stays up longer than a limit."""
    environment.update(env_of(entries))
    environment.setdefault("MGE_XE_BOX_SECONDS", "5")


def archive_arguments(args):
    """`--archive`: the run takes the build, the plugin and the MWSE.dll of the extracted release
    archive, and it builds nothing. The mod's Lua is the archive's too (`session.prepare`), and
    the instance's own mod `nirender` is off for the run: under the run's mod, Mod Organizer would
    serve a file that the archive lacks from it, and a faulty archive would pass."""
    if not args.archive:
        return
    if args.plugin_dll or args.mge_build:
        raise SystemExit("--archive names the build and the plugin: no --plugin-dll or --mge-build with it")
    args.mge_build = args.archive
    args.no_build = True
    if "nirender" not in args.disable_mod:
        args.disable_mod.append("nirender")
    args.plugin_dll = os.path.join(args.archive, "Data Files", "MWSE", "lib", "akulakhan.dll")
    mwse = os.path.join(args.archive, "MWSE.dll")
    if os.path.isfile(mwse) and not args.mwse_dll:
        args.mwse_dll = mwse


def read_text(path):
    """The text of a file, or None when it cannot be read."""
    try:
        with open(path, encoding="utf-8") as f:
            return f.read()
    except (OSError, UnicodeDecodeError):
        return None


def run_settings(args):
    """The --set lines of the run: the lines of the command, and the line that --engine-reference adds."""
    return list(args.set) + (["[runtime] disabled = true"] if args.engine_reference else [])


def config_problem(args, root):
    """The fault of the run's mgeXE.toml that the start would stop on, as one line, or None:
    the same edit as the start, on the text in memory (the file of the instance is not
    written)."""
    source = os.path.join(root, "mods", args.config_mod, "Root", "mgeXE.toml")
    text = read_text(source)
    if text is None:
        return "no readable mgeXE.toml at %s (--config-mod %s)" % (source, args.config_mod)
    try:
        session.run_toml_lines(text, run_settings(args), source, wgpu_switch(args))
    except SystemExit as refusal:
        return str(refusal)
    return None


def wgpu_switch(args):
    """The value of renderer_wgpu that the run writes: False for a reference run (or an engine
    reference), True for a run with an MGE XE build, else None."""
    if args.reference or args.engine_reference:
        return False
    return True if args.mge_build else None


# A batch that walks for `NIRENDER_WALK_MINUTES` (the land walks of wgpu-scene-audit), with
# its default: `tonumber(os.getenv("NIRENDER_WALK_MINUTES") or "") or 60`.
WALK_MINUTES_DEFAULT = re.compile(r'os\.getenv\(\s*["\']NIRENDER_WALK_MINUTES["\']\s*\)[^\n]*?\bor\s+(\d+)')
# The seconds of a run before its batch starts and after it ends: the load, the steps before
# the batch, the close. A 4-minute walk ran 314 s from the start to the end (2026-11-03).
WALK_START_SECONDS = 120


def walk_budget_problem(args, read=read_text, environment=os.environ):
    """The fault of a run whose shader batch walks for longer than its `--budget` allows: the
    runner ends the game at the budget, in the walk, and the plugin writes no end report
    (2026-11-03, a 60-minute walk under the default 200 s). The walk's minutes are those of
    `--env NIRENDER_WALK_MINUTES`, else of the runner's environment, else the batch's default.
    None when the budget covers the walk and `WALK_START_SECONDS`, or when the batch does not
    read the variable itself. Not seen: a batch that loads a walk batch from another file, and
    a default on a later line than the `os.getenv` call."""
    if not args.shader_batch:
        return None
    text = read(args.shader_batch)
    default = WALK_MINUTES_DEFAULT.search(text or "")
    if not default:
        return None
    try:
        variables = env_of(args.env)
    except SystemExit:
        return None
    value = variables.get("NIRENDER_WALK_MINUTES", environment.get("NIRENDER_WALK_MINUTES"))
    try:
        minutes = float(value) if value is not None else float(default.group(1))
    except ValueError:
        minutes = None
    if minutes is None or not math.isfinite(minutes) or minutes < 0:
        return "NIRENDER_WALK_MINUTES takes a number of minutes, not %r" % value
    source = "NIRENDER_WALK_MINUTES" if value is not None else "the batch's default"
    needed = int(minutes * 60) + WALK_START_SECONDS
    if args.budget >= needed:
        return None
    return ("--budget %d is shorter than the walk of the shader batch (%g min, %s) and the start (%d s): "
            "give --budget %d or more, or a shorter walk with --env NIRENDER_WALK_MINUTES=N"
            % (args.budget, minutes, source, WALK_START_SECONDS, needed))


def batch_lua_problems(path, text):
    """The findings of the Lua file `path` with `text` that the game will run (`lua_checks`): its
    syntax, and a local function used above its declaration."""
    return lua_checks.problems(path, text, lua_checks.luajit())


def argument_problems(args, exists=os.path.exists, read=read_text, environment=os.environ, lua=None):
    """Every fault of the arguments that shows before the run starts anything: the form of
    each value, the files and folders that the run reads, the faults of its Lua files
    (`lua`, else `batch_lua_problems`), and a budget shorter than the walk of its batch. One
    line for each fault; none for a good command. `--check` runs only this (and the check of
    the save), so that a caller can refuse a bad command before it takes the game slot."""
    lua = lua or batch_lua_problems
    problems = []
    if args.reference_plugin and not (args.reference or args.engine_reference):
        problems.append("--reference-plugin is for a run with --reference or --engine-reference")
    if getattr(args, "plain_case", False) and (args.reference or args.engine_reference or not args.mge_build):
        problems.append("--plain-case needs --mge-build, and not --reference or --engine-reference")
    if args.position is not None:
        parts = args.position.split(",")
        try:
            numbers = [float(part) for part in parts]
        except ValueError:
            numbers = []
        if len(parts) != 3 or len(numbers) != 3:
            problems.append("--position takes x,y,z, three numbers: not %r (a negative x needs --position=-1,2,3)" % args.position)
    if args.hour is not None and not 0 <= args.hour < 24:
        problems.append("--hour takes a number from 0 to 24, not %s" % args.hour)
    if args.weather is not None and not 0 <= args.weather <= 9:
        problems.append("--weather takes a number from 0 to 9, not %s" % args.weather)
    try:
        env_of(args.env)
    except SystemExit as refusal:
        problems.append(str(refusal))
    for entry in args.add_file:
        source, equals, path = entry.partition("=")
        if not equals or not source or not path:
            problems.append("--add-file takes SOURCE=PATH, not %r" % entry)
        elif not exists(source):
            problems.append("--add-file: no file %s" % source)
    for option, path in (("--plugin-dll", args.plugin_dll), ("--shader-batch", args.shader_batch), ("--lua-file", args.lua_file),
                         ("--mwse-dll", args.mwse_dll), ("--exe", args.exe), ("--log", args.log and os.path.dirname(os.path.abspath(args.log)))):
        if path and not exists(path):
            problems.append("%s: no %s" % (option, path))
    # The Lua that the game runs: a fault there ends the run in the game, after its start.
    for option, path in (("--shader-batch", args.shader_batch), ("--lua-file", args.lua_file)):
        if path and exists(path):
            text = read(path)
            if text is None:
                problems.append("%s: %s cannot be read as UTF-8 text" % (option, path))
            else:
                problems.extend("%s %s: %s" % (option, path, finding) for finding in lua(path, text))
    if args.mge_build:
        for name in ("d3d8.dll", "numidium.exe"):
            if not exists(os.path.join(args.mge_build, name)):
                problems.append("--mge-build: no %s in %s" % (name, args.mge_build))
    if args.instance and not exists(args.instance):
        problems.append("--instance: no folder %s" % args.instance)
    walk = walk_budget_problem(args, read, environment) if not args.shader_batch or exists(args.shader_batch) else None
    if walk:
        problems.append(walk)
    return problems


TIMING_STEPS = ("rate", "walk")


def is_timing_run(args):
    """Whether the run measures time, so that the slot check needs the game slot held with
    --timing (the owner's D297): --timing, a step of TIMING_STEPS, or no --steps (all the
    steps, the timing steps among them). A shader batch is a timing run only with --timing:
    most batches take pictures or hold a live session."""
    steps = [name for name in re.split(r"[,\s]+", args.steps or "") if name]
    return bool(args.timing) or not steps or any(name in TIMING_STEPS for name in steps)
WEATHER_IN_LINE = re.compile(r'(?:current|next|from|to) (\d+) "([^"]*)"')
# A change of the weather in a shader batch, with the text of its first argument.
WEATHER_CHANGE = re.compile(r"\b(switchImmediate|switchTransition|changeWeather)\s*\(\s*([^),]*)")


def weather_warning(args, read=read_text):
    """The one line that warns about a run that times something and leaves the weather to the
    game: a storm moves the frame rate (the owner, 2026-11-03). None for any other run. Not a
    refusal. No --steps means all the steps, the timing steps among them. A shader-batch
    run (a --shader-batch, or the step shaders) is a timing run too: its A/B compares frame times.
    None also for a shader batch that changes the weather itself: there the pin of --weather would
    override the batch (`weather_pin_warning`)."""
    if args.weather is not None:
        return None
    steps = [name for name in re.split(r"[,\s]+", args.steps or "") if name]
    if steps and not args.shader_batch and not any(name in TIMING_STEPS + ("shaders",) for name in steps):
        return None
    if args.shader_batch and WEATHER_CHANGE.search(read(args.shader_batch) or ""):
        return None
    return "warning: the run has timing steps and the weather is the game's, which can change in the run and move the frame rate: --weather 0 pins it to clear"


def weather_pin_warning(args, read=read_text):
    """The one line that warns about a run whose --weather pin overrides the shader batch's own
    weather: the mod puts the pinned weather back after each change (2026-11-10: seven W270 starts
    logged weather 0 in every set, though the batch switched to rain and fog). None when nothing
    is pinned, when there is no batch, or when every change in the batch is to the pinned weather
    as a literal number (the harness's boilerplate sets weather 0 again after each move). A change
    to a variable or a table counts as another weather. Not a refusal."""
    if args.weather is None or not args.shader_batch:
        return None
    text = read(args.shader_batch) or ""
    others = sorted({"%s(%s)" % (call, argument.strip()) for call, argument in WEATHER_CHANGE.findall(text)
                     if argument.strip() != str(args.weather)})
    if not others:
        return None
    return ("warning: --weather %d pins the weather and puts it back over the shader batch's own changes (%s): "
            "leave --weather out when the batch sets the weather" % (args.weather, ", ".join(others)))


# The game's registry key (HKLM, the 32-bit view), and the copy of it in the user's VirtualStore.
# The game reads and writes the HKLM key: on 2026-11-10 the VirtualStore had Shadows 0 and HKLM 6,
# and the game's capture had shadow scenes (kind 5), so it read 6 from HKLM; its exit at 07:11:42
# wrote HKLM, not the VirtualStore. The reason: the key gives BUILTIN\Users FullControl (an
# explicit entry, not inherited). The game's process is virtualized, but its open of the real key
# succeeds, so nothing is redirected. [I: not seen] A reinstall that resets this ACL to the
# inherited ReadKey would send the game's writes to the VirtualStore, and the VirtualStore would
# win the reads.
GAME_REGISTRY_KEY = r"SOFTWARE\Bethesda Softworks\Morrowind"
VIRTUAL_STORE_KEY = r"Software\Classes\VirtualStore\MACHINE\SOFTWARE\WOW6432Node\Bethesda Softworks\Morrowind"


def read_registry_dword(root, path, view, name="Shadows"):
    """A REG_DWORD value of a registry key, or None when the key or the value is missing."""
    try:
        with winreg.OpenKey(root, path, 0, winreg.KEY_READ | view) as key:
            value, kind = winreg.QueryValueEx(key, name)
    except OSError:
        return None
    return value if kind == winreg.REG_DWORD else None


def registry_shadows(read=read_registry_dword):
    """The value "Shadows" that the game reads, and where it is: (value, source). The HKLM value
    first; the source names the VirtualStore value too, as a difference of the two shows a stale
    copy. The VirtualStore value only when HKLM has none [I: a virtualized read; not seen]. value is
    None when both are missing. The game writes the value back at its exit
    (`NiDX8Application::writeRegistrySettings` 0x4293A0): one value for every instance and for the
    owner's play. A run reads it and never writes it."""
    machine = read(winreg.HKEY_LOCAL_MACHINE, GAME_REGISTRY_KEY, winreg.KEY_WOW64_32KEY)
    stored = read(winreg.HKEY_CURRENT_USER, VIRTUAL_STORE_KEY, 0)
    if machine is not None:
        return machine, "HKLM; VirtualStore %s" % ("missing" if stored is None else stored)
    return stored, "VirtualStore"


def ini_shadows(ini_bytes):
    """"Number of Shadows" of [General] in the bytes of a Morrowind.ini, as GetPrivateProfileIntA
    reads it: the first such key, the digits at the start of its value, 0 for a negative value and
    when the key is missing. None when the file cannot be read."""
    if ini_bytes is None:
        return None
    section = None
    for line in ini_bytes.decode("latin-1").splitlines():
        line = line.strip()
        if line.startswith("[") and "]" in line:
            section = line[1:line.index("]")].strip().lower()
        elif section == "general" and "=" in line:
            name, value = line.split("=", 1)
            if name.strip().lower() == "number of shadows":
                digits = re.match(r"\s*(-?\d+)", value)
                return max(int(digits.group(1)), 0) if digits else 0
    return 0


def shadow_level_line(registry, ini, source="HKLM"):
    """The line of a run's start that names the game's real-time shadow level, and the level. The
    game takes the smaller of the registry's "Shadows" and the ini's "Number of Shadows"
    (`NiDX8Application::readRegistrySettingsAndGetProcAddress`, 0x428E92-0x428EC1); with the level 0
    the shadow manager adds no caster and the shadow camera draws no scene. The level is None when a
    value is missing."""
    level = None if registry is None or ini is None else min(registry, ini)

    def shown(value):
        return "missing" if value is None else str(value)

    return "shadows: registry Shadows %s (%s), Morrowind.ini Number of Shadows %s: the game's level %s" % (shown(registry), source, shown(ini), shown(level)), level


def shadow_problem(args, level, line):
    """The refusal of a capture run with the game's real-time shadows off: such a capture holds no
    scene of kind 4 or 5, and the gate's stream rule needs a kind 5 scene (2026-11-10: the registry
    had Shadows 0, and the Pelagiad captures had none). None for any other run. No --steps means
    all the steps, the capture among them."""
    steps = [name for name in re.split(r"[,\s]+", args.steps or "") if name]
    if level != 0 or (steps and "capture" not in steps):
        return None
    return ("a capture with the game's real-time shadows off has no shadow scene (kinds 4 and 5): %s. The registry value is the "
            "owner's game setting, shared by every instance: ask the owner; a run does not change it" % line)


FULL_DUMP_SIZE = "0.8 GB"


def full_dump_note(args):
    """The line of `--check` for a run with --full-dump: MWSE's dump at a crash holds the whole
    memory of the game (its virtual memory was 754 MB at an exit crash of 2026-11-03). The dump
    stays in the run folder (D71). None without the option."""
    if not args.full_dump:
        return None
    return "--full-dump: a crash of the run writes a dump of about %s into the run folder" % FULL_DUMP_SIZE


def enable_problems(args, root, game_dir, ini_bytes, exists=os.path.exists):
    """The faults of --enable-mod and --enable-plugin against the instance, and the lines that
    `--check` prints for them: (problems, notes). `root` is the instance, `game_dir` the game's
    folder, `ini_bytes` the profile's Morrowind.ini (None when it cannot be read). A mod must be a
    folder under the instance's mods and not be a mod of the runners or one that --disable-mod
    switches off; a plugin must be a file of such a mod (at its top) or of the game's Data Files, and
    the profile's Morrowind.ini must be there to take its line."""
    problems, notes, folders = [], [], []
    for name in args.enable_mod:
        folder = os.path.join(root, "mods", name)
        if name.startswith(session.harness.RUN_MOD_PREFIX):
            problems.append("--enable-mod %s: a mod of the runners is switched on by the runner itself" % name)
        elif name in args.disable_mod:
            problems.append("--enable-mod %s: the same mod is in --disable-mod" % name)
        elif not exists(folder):
            problems.append("--enable-mod: no mod folder %s" % folder)
        else:
            folders.append(folder)
            notes.append("--enable-mod: %s is on for the run, first in the mod list after the runner's mods, and back after" % name)
    for plugin in args.enable_plugin:
        if not plugin.lower().endswith((".esp", ".esm")) or os.path.basename(plugin) != plugin:
            problems.append("--enable-plugin takes the name of an .esp or .esm file, not %r" % plugin)
            continue
        places = [os.path.join(folder, plugin) for folder in folders] + [os.path.join(game_dir, "Data Files", plugin)]
        if not any(exists(place) for place in places):
            problems.append("--enable-plugin: %s is in no mod of --enable-mod and not in the game's Data Files" % plugin)
        elif ini_bytes is None:
            problems.append("--enable-plugin: the profile has no Morrowind.ini to take a line for %s" % plugin)
        else:
            try:
                number, changed = session.harness.game_file_plan(ini_bytes, plugin)
            except SystemExit as refusal:
                problems.append("--enable-plugin: %s" % refusal)
                continue
            notes.append("--enable-plugin: %s is already active as GameFile%d" % (plugin, number) if changed is None else
                         "--enable-plugin: %s becomes GameFile%d in the profile's Morrowind.ini for the run, and the file is as it was after" % (plugin, number))
    return problems, notes


def read_profile_ini():
    """The bytes of the profile's Morrowind.ini, or None when it cannot be read."""
    try:
        with open(os.path.join(session.harness.PROFILE, "Morrowind.ini"), "rb") as fh:
            return fh.read()
    except OSError:
        return None


def weather_summary(pinned, lines):
    """What the result summary says of the weather: `pinned N, held (k re-pins)` when the mod read the
    pinned weather back after its switch (`weather pinned N: now N ...`), `pinned N, NOT applied (k
    re-pins)` without that line; k counts the
    `weather re-pin` lines that the mod logs each time it undid a change of the game (owndevice.lua);
    or the game's, with the names that the mod's weather lines show, in the order they first appear."""
    if pinned is not None:
        released = ", released at weathers" if any(line.startswith("weather pin released") for line in lines) else ""
        # Held only when the mod read the pinned weather back after its switch ("weather pinned N:
        # now N ..."): the argument alone proves nothing of the game.
        applied = any(line.startswith('weather pinned %d: now %d "' % (pinned, pinned)) for line in lines)
        return "pinned %d, %s (%d re-pins)%s" % (pinned, "held" if applied else "NOT applied", sum(1 for line in lines if line.startswith("weather re-pin")), released)
    names = []
    for line in lines:
        if not line.startswith("weather"):
            continue
        for index, name in WEATHER_IN_LINE.findall(line):
            entry = "%s (%s)" % (name, index)
            if entry not in names:
                names.append(entry)
    return "the game's: " + (", ".join(names) if names else "none logged")


def weather_note(pinned, lines):
    """The text of weather-of-the-run.txt: the summary line, then the mod's weather lines as the game
    logged them (the start, each rate line, each change)."""
    kept = [line for line in lines if line.startswith("weather")]
    return "".join(line + "\n" for line in ["weather: " + weather_summary(pinned, lines)] + kept)


def launch_run(run, spec, args, mods, facts=()):
    """Starts the run (`session.Run.launch`) with the options of the command line. A refusal of
    the renderer by the plugin ends the run at once, with its cause (`session.Run.refusal`), not at
    the end of the budget; not in a --plain-case run, which tests the game after that refusal. `facts` are more lines
    for the record of the run, after the commits."""
    stop = None if getattr(args, "plain_case", False) else run.refusal
    return run.launch(spec, args.budget, TAG, mods=mods, disable=args.disable_mod, facts=[build_commits(args.mge_build), *facts], full_dump=args.full_dump,
                      enable=args.enable_mod, plugins=args.enable_plugin, stop=stop)


def build_commits(mge_build):
    """The line of the record of a run that names the commits of its build, from WHAT.txt of the build
    folder. A folder that does not say them is named as such: no commit is guessed."""
    if not mge_build:
        return "commits: no --mge-build"
    what = os.path.join(mge_build, "WHAT.txt")
    try:
        with open(what, encoding="utf-8", errors="replace") as file:
            found = re.search(r"nirender \S+ (\w+), render-link \S+ (\w+)", file.read())
    except OSError:
        found = None
    if not found:
        return "commits: %s does not say them" % mge_build
    return "commits: nirender %s, render-link %s, from %s" % (found.group(1), found.group(2), what)


class ModuleEntry(ctypes.Structure):
    """MODULEENTRY32W of the Tool Help functions."""

    _fields_ = [("size", wintypes.DWORD), ("module_id", wintypes.DWORD), ("process_id", wintypes.DWORD), ("global_usage", wintypes.DWORD),
                ("process_usage", wintypes.DWORD), ("base", ctypes.c_void_p), ("base_size", wintypes.DWORD), ("handle", wintypes.HMODULE),
                ("name", ctypes.c_wchar * 256), ("path", ctypes.c_wchar * 260)]


def modules_of(pid):
    """The file names of the modules that the process `pid` has loaded, in lower case, the 32-bit
    modules of a 32-bit process too; None when Windows does not give them (the process is gone)."""
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    # TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32. The call can fail with ERROR_BAD_LENGTH while
    # the process loads a module: then again.
    for _ in range(5):
        snapshot = kernel32.CreateToolhelp32Snapshot(0x18, pid)
        if snapshot not in (None, wintypes.HANDLE(-1).value):
            break
        if ctypes.get_last_error() != 24:
            return None
    else:
        return None
    try:
        entry = ModuleEntry()
        entry.size = ctypes.sizeof(ModuleEntry)
        names = []
        more = kernel32.Module32FirstW(wintypes.HANDLE(snapshot), ctypes.byref(entry))
        while more:
            names.append(entry.name.lower())
            more = kernel32.Module32NextW(wintypes.HANDLE(snapshot), ctypes.byref(entry))
        return names
    finally:
        kernel32.CloseHandle(wintypes.HANDLE(snapshot))


class ProcessEntry(ctypes.Structure):
    """PROCESSENTRY32W of the Tool Help functions."""

    _fields_ = [("size", wintypes.DWORD), ("usage", wintypes.DWORD), ("process_id", wintypes.DWORD), ("heap_id", ctypes.c_size_t),
                ("module_id", wintypes.DWORD), ("threads", wintypes.DWORD), ("parent_id", wintypes.DWORD), ("priority", wintypes.LONG),
                ("flags", wintypes.DWORD), ("name", ctypes.c_wchar * 260)]


def pids_of(image):
    """The ids of the processes with this image name. One snapshot of Tool Help, no program
    started: the watch must not disturb the run that it looks at."""
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    snapshot = kernel32.CreateToolhelp32Snapshot(0x2, 0)
    if snapshot in (None, wintypes.HANDLE(-1).value):
        return []
    try:
        entry = ProcessEntry()
        entry.size = ctypes.sizeof(ProcessEntry)
        found = []
        more = kernel32.Process32FirstW(wintypes.HANDLE(snapshot), ctypes.byref(entry))
        while more:
            if entry.name.lower() == image.lower():
                found.append(entry.process_id)
            more = kernel32.Process32NextW(wintypes.HANDLE(snapshot), ctypes.byref(entry))
        return found
    finally:
        kernel32.CloseHandle(wintypes.HANDLE(snapshot))


class PluginWatch:
    """Looks at the modules of every Morrowind.exe every few seconds while a run goes: the proof,
    from Windows, of whether the game loaded akulakhan.dll. `seen` gets each process that had it;
    `checks` counts the looks that Windows answered. With `path`, each look gets a line in that
    file (the run folder keeps the proof)."""

    def __init__(self, seconds=5.0, path=None, image="Morrowind.exe"):
        self.seconds = seconds
        self.seen = set()
        self.checks = 0
        self.path = path
        self.image = image
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.watch, daemon=True)

    def watch(self):
        while not self.stop.is_set():
            for pid in pids_of(self.image):
                names = modules_of(pid)
                if names is None:
                    continue
                self.checks += 1
                found = "akulakhan.dll" in names
                if found:
                    self.seen.add(pid)
                if self.path:
                    with open(self.path, "a", encoding="utf-8") as log:
                        log.write("%s %s %d: %d modules, akulakhan.dll %s\n" % (time.strftime("%H:%M:%S"), self.image, pid, len(names), "LOADED" if found else "not loaded"))
            self.stop.wait(self.seconds)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join()


def plugin_evidence(run_dir, watch_seen, watch_checks):
    """What a run says about the plugin it loaded: the lines to print, and True when any proof
    shows that the game had akulakhan.dll. The proofs: the record of the run (the copy of the DLL
    that the game got), the modules of the game (`PluginWatch`), the lines of main.lua in MWSE.log,
    and the plugin's own lines."""
    lines, loaded = [], False
    try:
        with open(os.path.join(run_dir, "progress.log"), encoding="utf-8", errors="replace") as progress:
            setup = [line.split("] ", 1)[1].strip() for line in progress if line.startswith("[runner] akulakhan.dll")]
    except OSError:
        setup = []
    lines.append("the record of the run: %s" % (setup[0] if setup else "no line about akulakhan.dll"))
    loaded |= bool(setup) and setup[0] != "akulakhan.dll absent"
    if watch_checks:
        lines.append("the modules of the game, %d looks: akulakhan.dll %s" % (watch_checks, "LOADED" if watch_seen else "not loaded"))
    else:
        lines.append("the modules of the game: no look succeeded")
    loaded |= bool(watch_seen)
    try:
        with open(os.path.join(run_dir, "MWSE.log"), encoding="utf-8", errors="replace") as log:
            mwse = [line.strip() for line in log if line.startswith("[nirender] ")]
    except OSError:
        mwse = []
    stand_in = any(line.startswith("[nirender] no plugin:") for line in mwse)
    with_dll = [line for line in mwse if line.startswith(("[nirender] version", "[nirender] ERROR: akulakhan.dll is loaded"))]
    lines.append("MWSE.log: %s" % ("the stand-in table" if stand_in else with_dll[0][:120] if with_dll else "no line of main.lua"))
    loaded |= bool(with_dll)
    plugin = session.common.plugin_lines(run_dir)
    lines.append("the plugin's own lines: %d" % len(plugin))
    loaded |= bool(plugin)
    return lines, loaded


def self_test_plugin_evidence():
    """The proofs of the plugin of a run, with fake run folders; the module list of this process."""
    names = modules_of(os.getpid())
    assert names and "kernel32.dll" in names, names
    assert os.getpid() in pids_of(os.path.basename(sys.executable)), pids_of(os.path.basename(sys.executable))
    # A 32-bit process, as the game: its 32-bit modules are in the list (kernel32.dll of
    # SysWOW64; the 64-bit list of such a process has ntdll.dll and the wow64 DLLs only).
    ping = subprocess.Popen([os.path.join(os.environ["SystemRoot"], "SysWOW64", "PING.EXE"), "-n", "4", "127.0.0.1"], stdout=subprocess.DEVNULL)
    try:
        for _ in range(40):
            names = modules_of(ping.pid) or []
            if "kernel32.dll" in names:
                break
            threading.Event().wait(0.05)
        assert "kernel32.dll" in names and "ping.exe" in names, names
        # The watch looks at that process, logs each look, and finds no plugin in it.
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "plugin-watch.txt")
            with PluginWatch(seconds=0.1, path=path, image="PING.EXE") as watch:
                threading.Event().wait(0.5)
            assert watch.checks >= 2 and not watch.seen, (watch.checks, watch.seen)
            with open(path, encoding="utf-8") as log:
                lines = log.read().splitlines()
            assert len(lines) == watch.checks and "akulakhan.dll not loaded" in lines[0], lines
    finally:
        ping.kill()
        ping.wait()
    with tempfile.TemporaryDirectory() as folder:
        def write(name, text):
            with open(os.path.join(folder, name), "w", encoding="utf-8") as file:
                file.write(text)

        write("progress.log", "[runner] instance x\n[runner] akulakhan.dll absent\n")
        write("MWSE.log", "[nirender] no plugin: akulakhan.dll is not loaded\n")
        lines, loaded = plugin_evidence(folder, set(), 4)
        assert not loaded, lines
        assert "the record of the run: akulakhan.dll absent" in lines and "MWSE.log: the stand-in table" in lines, lines
        # Each proof alone fails the run.
        assert plugin_evidence(folder, {42}, 4)[1]
        write("mgeXE.log", "nirender: install: 313 ports installed\n")
        assert plugin_evidence(folder, set(), 4)[1]
        os.remove(os.path.join(folder, "mgeXE.log"))
        write("MWSE.log", "[nirender] ERROR: akulakhan.dll is loaded in a run whose spec asks for no plugin\n")
        assert plugin_evidence(folder, set(), 4)[1]
        write("MWSE.log", "[nirender] version 0.3.4, 313 ports\n")
        assert plugin_evidence(folder, set(), 4)[1]
        write("MWSE.log", "")
        write("progress.log", "[runner] akulakhan.dll 900150983cd24fb0d6963f7d28e17f72 (mod nirender)\n")
        lines, loaded = plugin_evidence(folder, set(), 0)
        assert loaded and "the modules of the game: no look succeeded" in lines, lines

    # The mods of the instance that hold the plugin, without the run's own mod.
    harness = session.harness
    with tempfile.TemporaryDirectory() as root:
        for name in ("Rel", "Off", session.MOD_NAME):
            os.makedirs(os.path.join(root, "mods", name, "MWSE", "lib"))
            open(os.path.join(root, "mods", name, harness.PLUGIN_FILE), "w").close()
        os.makedirs(os.path.join(root, "mods", "Plain"))
        found = session.plugin_mods(["# list", "+" + session.MOD_NAME, "+Plain", "+Rel", "-Off"], root)
        assert found == ["Rel"], found


def self_test_setup():
    """The record of what a run ran, with a fake instance: each file by its MD5 at the game's folder,
    the plugin from the mod that wins, the plugins in the order of their numbers."""
    harness = session.harness
    with tempfile.TemporaryDirectory() as root:
        game, profile = os.path.join(root, "game"), os.path.join(root, "profiles", "Default")
        plugin = os.path.join("mods", "High", harness.PLUGIN_FILE)
        for folder in (game, profile, os.path.join(root, os.path.dirname(plugin)), os.path.join(root, "mods", "Low", "MWSE", "lib"), os.path.join(root, "mods", "Off", "MWSE", "lib")):
            os.makedirs(folder)

        def write(path, text):
            with open(path, "w", encoding="ascii") as file:
                file.write(text)

        write(os.path.join(game, "MWSE.dll"), "abc")
        write(os.path.join(root, plugin), "abc")
        write(os.path.join(root, "mods", "Low", harness.PLUGIN_FILE), "low")
        write(os.path.join(root, "mods", "Off", harness.PLUGIN_FILE), "off")
        modlist = ["# generated", "-Off", "+High", "+Low"]
        abc = "900150983cd24fb0d6963f7d28e17f72"

        assert harness.winning_file(harness.PLUGIN_FILE, root, modlist, game) == ("mod High", os.path.join(root, plugin))
        assert harness.winning_file(harness.PLUGIN_FILE, root, ["-Off"], game) is None

        ini = "[Game Files]\nGameFile10=Ten.esp\nGameFile2=Two.esp\ngamefile0 = Morrowind.esm \n[Other]\nNotGameFile=x\n"
        assert harness.active_plugins(ini) == ["Morrowind.esm", "Two.esp", "Ten.esp"], harness.active_plugins(ini)
        assert harness.read_active_plugins(profile, game) == ([], None)
        write(os.path.join(game, "Morrowind.ini"), "GameFile0=Game.esm\n")
        assert harness.read_active_plugins(profile, game)[0] == ["Game.esm"]
        write(os.path.join(profile, "Morrowind.ini"), ini)
        plugins, source = harness.read_active_plugins(profile, game)
        assert plugins == ["Morrowind.esm", "Two.esp", "Ten.esp"] and source == os.path.join(profile, "Morrowind.ini")
        assert harness.load_order_md5(plugins) != harness.load_order_md5(list(reversed(plugins)))

        lines = harness.setup_lines(root, profile, game, modlist, plugins, source, ["commits: x"])
        assert lines[0] == "instance %s, profile Default" % root, lines[0]
        assert "MWSE.dll " + abc in lines and "d3d8.dll absent" in lines and "commits: x" in lines, lines
        assert "akulakhan.dll %s (mod High)" % abc in lines, lines
        assert not any(line.startswith(("dinput8.dll", "d3d9.dll")) for line in lines), lines
        write(os.path.join(game, "dinput8.dll"), "abc")
        write(os.path.join(game, "d3d9.dll"), "abc")
        with_optional = harness.setup_lines(root, profile, game, modlist, plugins, source)
        assert "dinput8.dll " + abc in with_optional and "d3d9.dll " + abc in with_optional, with_optional
        assert any(line.startswith("plugins 3, load order MD5 ") for line in lines), lines
        assert harness.setup_lines(root, profile, game, ["-Off"], [], None)[-2:] == ["akulakhan.dll absent", "plugins: no Morrowind.ini names any"]
        text = harness.setup_text(profile, modlist, plugins, source)
        assert "+High\n" in text and "Ten.esp\n" in text and text.index("+High") < text.index("[plugins]") < text.index("Morrowind.esm"), text

        # The commits: only what the folder of the build says.
        assert build_commits(None) == "commits: no --mge-build"
        assert "does not say" in build_commits(root)
        write(os.path.join(root, "WHAT.txt"), "f1 2026-11-02 15:00: the owner's play set, nirender master abc1, render-link feature/wgpu-renderer def2, no pdb; smoke: pending\n")
        assert build_commits(root).startswith("commits: nirender abc1, render-link def2, from "), build_commits(root)


def self_test_archive_lua():
    """A fake archive and a fake run mod in a temporary folder: the archive's file wins, a file
    that the archive's Lua loads and lacks leaves the run's mod (the fault of v0.3.3: main.lua
    without failure.lua), and the harness's scenario stays."""
    with tempfile.TemporaryDirectory() as folder:
        archive = os.path.join(folder, "archive", "Data Files", "MWSE", "mods", "nirender")
        dist = os.path.join(folder, "dist", "MWSE", "mods", "nirender")
        os.makedirs(archive)
        os.makedirs(dist)

        def write(where, name, text):
            with open(os.path.join(where, name), "w", encoding="utf-8") as file:
                file.write(text)

        write(archive, "main.lua", 'dofile("Data Files\\\\MWSE\\\\mods\\\\nirender\\\\failure.lua")\nrequire("nirender.notifier")\nrequire("nirender." .. spec.scenario)\n')
        write(dist, "main.lua", "the repo's")
        write(dist, "failure.lua", "the repo's")
        write(dist, "notifier.lua", "the repo's")
        write(dist, "owndevice.lua", "the scenario")
        shipped, missing = session.stage_archive_lua(os.path.join(folder, "archive"), os.path.join(folder, "dist"))
        assert shipped == ["main.lua"], shipped
        assert missing == ["failure.lua", "notifier.lua"], missing
        assert sorted(os.listdir(dist)) == ["main.lua", "owndevice.lua"], os.listdir(dist)
        assert "failure.lua" in open(os.path.join(dist, "main.lua"), encoding="utf-8").read()

        # With the files there, nothing is missing, and a file loaded through another is reached.
        write(archive, "failure.lua", 'dofile("Data Files/MWSE/mods/nirender/extra.lua")')
        write(archive, "notifier.lua", "")
        write(dist, "extra.lua", "the repo's")
        shipped, missing = session.stage_archive_lua(os.path.join(folder, "archive"), os.path.join(folder, "dist"))
        assert missing == ["extra.lua"], missing
        assert "extra.lua" not in os.listdir(dist)
        write(archive, "extra.lua", "")
        assert session.stage_archive_lua(os.path.join(folder, "archive"), os.path.join(folder, "dist"))[1] == []

        # No main.lua in the archive: refused.
        os.remove(os.path.join(archive, "main.lua"))
        try:
            session.stage_archive_lua(os.path.join(folder, "archive"), os.path.join(folder, "dist"))
        except SystemExit:
            pass
        else:
            raise AssertionError("an archive without main.lua was taken")


def saves_folder(profile, game_dir):
    """The folder of the game's saves in a profile: its own with Mod Organizer's `LocalSaves`,
    else the one of the game."""
    try:
        with open(os.path.join(profile, "settings.ini"), encoding="utf-8", errors="replace") as settings:
            local = any(line.strip().lower() == "localsaves=true" for line in settings)
    except OSError:
        local = False
    return os.path.join(profile, "saves") if local else os.path.join(game_dir, "Saves")


def save_problem(save, folder):
    """Why the run cannot load `save` from `folder`, with the newest saves that are there, or
    None. Without the check the game waits at the main menu for a load that never comes, until the
    budget ends. An empty name starts a new game: no save is needed."""
    if not save or os.path.isfile(os.path.join(folder, save)):
        return None
    try:
        saves = [name for name in os.listdir(folder) if name.lower().endswith(".ess")]
    except OSError:
        saves = []
    saves.sort(key=lambda name: os.path.getmtime(os.path.join(folder, name)), reverse=True)
    return "the save %s is not in %s; the newest there: %s. Name one with --save, or --save \"\" for a new game" % (save, folder, ", ".join(saves[:3]) or "none")


def pngs_from_tgas(folder):
    """Turns the TGA files of a folder into PNG files: `niPixelData:exportTGA` is what the game can write."""
    import picture_diff  # noqa: F401  (it finds the DLLs of Pillow under -I)
    from PIL import Image

    for root, _, names in os.walk(folder):
        for name in names:
            if name.endswith(".tga"):
                path = os.path.join(root, name)
                with Image.open(path) as picture:
                    picture.convert("RGB").save(path[:-4] + ".png")
                os.remove(path)


def self_test_pngs_from_tgas():
    import picture_diff  # noqa: F401  (it finds the DLLs of Pillow under -I)
    from PIL import Image

    with tempfile.TemporaryDirectory() as folder:
        os.mkdir(os.path.join(folder, "eng"))
        path = os.path.join(folder, "eng", "a.tga")
        Image.new("RGBA", (4, 4), (10, 20, 30, 255)).save(path)
        pngs_from_tgas(folder)
        assert not os.path.exists(path)
        with Image.open(path[:-4] + ".png") as picture:
            assert picture.getpixel((0, 0))[:3] == (10, 20, 30)


def self_test_save_problem():
    """A missing save is refused with its folder and the newest saves; a save that is there and a
    new game are not. The folder follows `LocalSaves`."""
    with tempfile.TemporaryDirectory() as root:
        profile, game = os.path.join(root, "profile"), os.path.join(root, "game")
        os.makedirs(os.path.join(profile, "saves"))
        os.makedirs(os.path.join(game, "Saves"))
        settings = os.path.join(profile, "settings.ini")
        assert saves_folder(profile, game) == os.path.join(game, "Saves")
        with open(settings, "w") as f:
            f.write("[General]\nLocalSaves=false\n")
        assert saves_folder(profile, game) == os.path.join(game, "Saves")
        with open(settings, "w") as f:
            f.write("[General]\nLocalSaves=true\n")
        folder = saves_folder(profile, game)
        assert folder == os.path.join(profile, "saves")
        assert save_problem("", folder) is None
        missing = save_problem("quiksave.ess", folder)
        assert "quiksave.ess" in missing and folder in missing and "none" in missing, missing
        for name, age in (("old.ess", 100), ("new.ess", 200), ("note.txt", 300)):
            with open(os.path.join(folder, name), "w"):
                pass
            os.utime(os.path.join(folder, name), (age, age))
        missing = save_problem("quiksave.ess", folder)
        assert "new.ess, old.ess" in missing and "note.txt" not in missing, missing
        assert save_problem("old.ess", folder) is None
        assert save_problem("old.ess", os.path.join(game, "Saves")) is not None
        assert save_problem("x.ess", os.path.join(root, "no such folder")) is not None


def self_test_archive_arguments():
    """`--archive` takes the archive's parts and turns the instance's own mod off, once."""
    args = argparse.Namespace(archive="E:/a", plugin_dll=None, mge_build=None, mwse_dll=None, no_build=False, disable_mod=[])
    archive_arguments(args)
    assert args.mge_build == "E:/a" and args.no_build
    assert args.plugin_dll == os.path.join("E:/a", "Data Files", "MWSE", "lib", "akulakhan.dll"), args.plugin_dll
    assert args.disable_mod == ["nirender"], args.disable_mod
    args = argparse.Namespace(archive="E:/a", plugin_dll=None, mge_build=None, mwse_dll=None, no_build=False, disable_mod=["nirender", "other"])
    archive_arguments(args)
    assert args.disable_mod == ["nirender", "other"], args.disable_mod
    args = argparse.Namespace(archive=None, plugin_dll=None, mge_build=None, mwse_dll=None, no_build=False, disable_mod=[])
    archive_arguments(args)
    assert args.disable_mod == [] and not args.no_build
    try:
        archive_arguments(argparse.Namespace(archive="E:/a", plugin_dll="E:/b.dll", mge_build=None, mwse_dll=None, no_build=False, disable_mod=[]))
    except SystemExit:
        pass
    else:
        raise AssertionError("--archive with --plugin-dll was taken")


def self_test_checks():
    """The checks that refuse a command before anything starts, the staging of a run without the
    plugin, and the exit code of a run that loaded the plugin when it should not have."""
    def namespace(**changes):
        values = dict(reference_plugin=False, reference=False, engine_reference=False, position=None, hour=None, weather=None,
                      env=[], add_file=[], plugin_dll=None, shader_batch=None, lua_file=None, mwse_dll=None, exe=None, log=None,
                      mge_build=None, instance=None, budget=200)
        values.update(changes)
        return argparse.Namespace(**values)

    present = {os.path.join("E:/b", "d3d8.dll"), os.path.join("E:/b", "numidium.exe"), "E:/batch.lua", "E:/a.dll"}
    exists = present.__contains__
    good = namespace(position="-57717,-416069,1198", hour=21.58, weather=0, mge_build="E:/b", shader_batch="E:/batch.lua",
                     plugin_dll="E:/a.dll", env=["NIRENDER_WALK_MINUTES=8"], reference=True, reference_plugin=True)
    # The Lua checks have their own cases below: here a batch of planted text, and no finding.
    batch_text = {"E:/batch.lua": "return {}\n"}.get
    def no_lua(path, text):
        return []
    assert argument_problems(good, exists, batch_text, {}, no_lua) == [], argument_problems(good, exists, batch_text, {}, no_lua)
    bad = [
        namespace(reference_plugin=True),
        namespace(position="57717,-416069"),
        namespace(position="x,1,2"),
        namespace(hour=24.0),
        namespace(weather=10),
        namespace(env=["NOVALUE"]),
        namespace(add_file=["E:/a.dll"]),
        namespace(add_file=["E:/none.dll=Root/x.dll"]),
        namespace(shader_batch="E:/none.lua"),
        namespace(mge_build="E:/c"),
        namespace(plain_case=True),
        namespace(plain_case=True, mge_build="E:/b", engine_reference=True),
    ]
    for args in bad:
        assert argument_problems(args, exists), args
    assert len(argument_problems(namespace(mge_build="E:/c", weather=10), exists)) == 3
    assert argument_problems(namespace(plain_case=True, mge_build="E:/b", env=["MGE_XE_NATIVE_REFUSE_EVENT=3"]), exists) == []

    # The line of the case of a --plain-case run (W462): the fall-back, or the close after a refusal; not the wgpu line.
    fall_back = "!! wgpu renderer: not in use, MWSE refused the event 3. The game runs on its own Direct3D 8"
    assert plain_case_line([">> Using the wgpu renderer: no Direct3D device", fall_back + "\n"]) == fall_back
    close = "!! wgpu renderer: the plugin refused after it changed the game's code (x). MGE XE closes the game with the exit code 12"
    assert plain_case_line([close]) == close
    assert plain_case_line([">> Using the wgpu renderer: no Direct3D device", "-- runtime.disabled: the game runs on its own Direct3D 8"]) is None

    # The close case passes without DONE only with both facts: MGE XE's close line and the game's exit code 12.
    assert plain_case_closed(close, {"reason": "exited", "codes": [12]})
    assert not plain_case_closed(close, {"reason": "exited", "codes": [0]})
    assert not plain_case_closed(fall_back, {"reason": "exited", "codes": [12]})
    assert not plain_case_closed(None, {"reason": "exited", "codes": [12]})

    # A batch that walks: the budget must cover the walk and the start.
    walk = {"E:/batch.lua": 'local MINUTES = tonumber(os.getenv("NIRENDER_WALK_MINUTES") or "") or 60\n'}
    read = walk.get
    quiet = {}
    def walk_problems(**changes):
        return argument_problems(namespace(shader_batch="E:/batch.lua", **changes), exists, read, quiet, no_lua)
    # The case of 2026-11-03: the batch's 60 minutes under the default 200 s.
    problems = walk_problems()
    assert len(problems) == 1 and "--budget 3720 or more" in problems[0] and "the batch's default" in problems[0], problems
    assert walk_problems(budget=3720) == []
    # --env sets the minutes; a later value wins, as in env_of.
    assert walk_problems(env=["NIRENDER_WALK_MINUTES=4"], budget=600) == []
    problems = walk_problems(env=["NIRENDER_WALK_MINUTES=60", "NIRENDER_WALK_MINUTES=4"], budget=359)
    assert len(problems) == 1 and "--budget 360 or more" in problems[0] and "NIRENDER_WALK_MINUTES" in problems[0], problems
    for bad in ("four", "nan", "inf", "-1"):
        problems = walk_problems(env=["NIRENDER_WALK_MINUTES=" + bad], budget=600)
        assert len(problems) == 1 and "takes a number of minutes" in problems[0], (bad, problems)
    # Another default, in single quotes: 15 minutes need 1020 s.
    other = {"E:/batch.lua": "local MINUTES = tonumber(os.getenv('NIRENDER_WALK_MINUTES') or '') or 15\n"}.get
    problems = argument_problems(namespace(shader_batch="E:/batch.lua", budget=1019), exists, other, quiet, no_lua)
    assert len(problems) == 1 and "(15 min, the batch's default)" in problems[0] and "--budget 1020 or more" in problems[0], problems
    assert argument_problems(namespace(shader_batch="E:/batch.lua", budget=1020), exists, other, quiet, no_lua) == []
    # The runner's environment, when --env does not set it.
    assert walk_budget_problem(namespace(shader_batch="E:/batch.lua", budget=600), read, {"NIRENDER_WALK_MINUTES": "4"}) is None
    assert walk_budget_problem(namespace(shader_batch="E:/batch.lua", budget=600), read, quiet)
    # A batch that does not walk, and no batch: no fault.
    assert argument_problems(namespace(shader_batch="E:/batch.lua"), exists, batch_text, quiet, no_lua) == []

    # The Lua checks of the files that the game runs (lua_checks): each finding of --shader-batch
    # and of --lua-file is a fault that names the option and the file, and an unreadable file is one.
    seen = []
    def one_finding(path, text):
        seen.append((path, text))
        return ["the local function f is used on line 1 and declared on line 2"]
    texts = {"E:/batch.lua": "f()\nlocal function f() end\n", "E:/file.lua": "return 1\n"}.get
    both = namespace(shader_batch="E:/batch.lua", lua_file="E:/file.lua")
    problems = argument_problems(both, (present | {"E:/file.lua"}).__contains__, texts, quiet, one_finding)
    assert problems == ["--shader-batch E:/batch.lua: the local function f is used on line 1 and declared on line 2",
                        "--lua-file E:/file.lua: the local function f is used on line 1 and declared on line 2"], problems
    assert seen == [("E:/batch.lua", "f()\nlocal function f() end\n"), ("E:/file.lua", "return 1\n")], seen
    assert argument_problems(namespace(shader_batch="E:/batch.lua"), exists, {}.get, quiet, no_lua) == [
        "--shader-batch: E:/batch.lua cannot be read as UTF-8 text"]
    # A missing file is the existence fault alone; the Lua checks do not run on it.
    seen.clear()
    assert argument_problems(namespace(shader_batch="E:/none.lua"), exists, texts, quiet, one_finding) == ["--shader-batch: no E:/none.lua"]
    assert seen == []
    # The real checks, when this machine has its LuaJIT: the reduced batch of 12ab3fcb (W381) is
    # refused, a syntax fault is refused, and a clean batch passes.
    if os.path.isfile(lua_checks.luajit()):
        def real(text):
            return argument_problems(namespace(shader_batch="E:/batch.lua"), exists, {"E:/batch.lua": text}.get, quiet)
        refused = real(lua_checks.USE_ABOVE)
        assert len(refused) == 1 and refused[0].startswith("--shader-batch E:/batch.lua: the local function setHover is used on line 5"), refused
        assert real("local x = {\nlocal y = 1\n") == [
            "--shader-batch E:/batch.lua: the Lua syntax: batch.lua:2: unexpected symbol near 'local'"], real("local x = {\n")
        assert real("return {}\n") == []
    else:
        print("run_own_device self-test: the real Lua checks are skipped, no LuaJIT at %s" % lua_checks.luajit())
    assert walk_budget_problem(namespace(), read, quiet) is None

    assert run_exit_code(True, True, True) == 2
    assert run_exit_code(True, True, False) == 2
    assert run_exit_code(False, True, True) == 0
    assert run_exit_code(True, False, False) == 1

    with tempfile.TemporaryDirectory() as root:
        assert overwrite_problem(root, "MWSE/lib/akulakhan.dll") is None
        os.makedirs(os.path.join(root, "overwrite", "MWSE", "lib"))
        open(os.path.join(root, "overwrite", "MWSE", "lib", "akulakhan.dll"), "wb").close()
        assert overwrite_problem(root, "MWSE/lib/akulakhan.dll")

    with tempfile.TemporaryDirectory() as folder:
        dll = os.path.join(folder, "akulakhan.dll")
        open(dll, "wb").close()
        open(os.path.join(folder, "akulakhan.pdb"), "wb").close()
        dist = os.path.join(folder, "dist")
        lib = os.path.join(dist, "MWSE", "lib")
        session.stage_mod(dist, dll, plugin=False)
        assert not os.path.exists(os.path.join(lib, "akulakhan.dll")) and not os.path.exists(os.path.join(lib, "akulakhan.pdb"))
        assert os.path.isfile(os.path.join(dist, "MWSE", "mods", "nirender", "main.lua"))
        session.stage_mod(dist, dll, plugin=True)
        assert os.path.isfile(os.path.join(lib, "akulakhan.dll")) and os.path.isfile(os.path.join(lib, "akulakhan.pdb"))
        # A repository whose mod folder has the DLL: the mod of a run without the plugin is refused.
        repo = os.path.join(folder, "repo")
        os.makedirs(os.path.join(repo, "mod", "MWSE", "lib"))
        open(os.path.join(repo, "mod", "MWSE", "lib", "akulakhan.dll"), "wb").close()
        saved, session.REPO = session.REPO, repo
        try:
            session.stage_mod(os.path.join(folder, "dist-repo"), dll, plugin=False)
        except SystemExit as refusal:
            assert "akulakhan.dll" in str(refusal), refusal
        else:
            raise AssertionError("a mod without the plugin was staged with akulakhan.dll")
        finally:
            session.REPO = saved


TEST_TOML = '[runtime]\nrenderer_wgpu = true\nvsync = "default"\n\n[distant_land]\nenabled = true\nfar = 5\n\n[statics] # the statics\nenabled = false\n'


def self_test_run_toml():
    """The lines of the run's mgeXE.toml: the --set forms, a section that the file lacks, and the faults."""
    def lines_of(settings, wgpu=None, text=TEST_TOML):
        return session.run_toml_lines(text, settings, "test.toml", wgpu)

    base = lines_of([])
    assert 'vsync = "immediate"' in base and "renderer_wgpu = true" not in base, base
    # A key in its section is replaced, in that section only (`enabled` is in two sections).
    changed = lines_of(["[statics] enabled = true"])
    assert changed[changed.index("[statics] # the statics") + 1] == "enabled = true", changed
    assert changed[changed.index("[distant_land]") + 1] == "enabled = true" and changed.count("enabled = true") == 2, changed
    # A key that the section lacks goes at the end of the section, before the blank line of the next one.
    added = lines_of(["[distant_land] near = 1"])
    assert added[added.index("far = 5") + 1] == "near = 1" and added[added.index("near = 1") + 1] == "", added
    # A section that the file lacks is made at the end, once, for the keys of one section.
    made = lines_of(["[views] full_tier_max = 4", "[views] other = 2"])
    assert made[-4:] == ["", "[views]", "full_tier_max = 4", "other = 2"], made
    assert made[:len(base)] == base, made
    no_end = lines_of(["[views] full_tier_max = 4"], text=TEST_TOML.rstrip("\n") + "\n\n\n")
    assert no_end[-4:] == ["enabled = false", "", "[views]", "full_tier_max = 4"], no_end
    # A key with no section is replaced where it is.
    kept = lines_of(['vsync = "default"'])
    assert 'vsync = "default"' in kept and 'vsync = "immediate"' not in kept, kept
    assert "far = 9" in lines_of(["far = 9"]) and "far = 5" not in lines_of(["far = 9"])
    # The switch goes first in [runtime], as the value of the mode.
    assert lines_of([], True)[:2] == ["[runtime]", "renderer_wgpu = true"], lines_of([], True)
    assert lines_of([], False)[:2] == ["[runtime]", "renderer_wgpu = false"]
    assert lines_of([], None)[:2] == ["[runtime]", 'vsync = "immediate"']
    # A header with a comment or spaces is the section: no second table of the name. This is the
    # whole file of the run, and it must be a toml.
    for header in ("[statics] # x", "[statics]# x", "  [ statics ]  ", "[statics]"):
        text = TEST_TOML.replace("[statics] # the statics", header)
        edited = lines_of(["[statics] enabled = true", "[statics] near = 2"], text=text)
        assert sum(1 for line in edited if line.strip().startswith("[") and "statics" in line) == 1, (header, edited)
        assert tomllib.loads("\n".join(edited))["statics"] == {"enabled": True, "near": 2}, (header, edited)
    # A line of an array is no header: the key goes after the array, and the file is a toml.
    arrays = TEST_TOML + 'values = [\n  [1, 2]\n]\n'
    edited = lines_of(["[statics] near = 3"], text=arrays)
    assert edited[-1] == "near = 3" and tomllib.loads("\n".join(edited))["statics"]["near"] == 3, edited
    # An edit that makes the file invalid is a fault of the edit; a file that was not a toml is not refused for it.
    try:
        lines_of(["[distant_land] far = ["])
    except SystemExit as refusal:
        assert "is not a toml" in str(refusal) and "test.toml" in str(refusal), refusal
    else:
        raise AssertionError("an edit that makes the file invalid was taken")
    assert lines_of([], text=TEST_TOML + "garbage ((\n")
    # The faults.
    for settings, words in ((["no_such_key = 1"], "give its section, [section] no_such_key = 1"),
                            (["views.full_tier_max = 4"], "give its section, [views] full_tier_max = 4"),
                            (["x = 1.5"], "give its section, [section] x = 1.5"),
                            (["[views full_tier_max = 4"], "a section is written"),
                            (["[views]"], "--set takes key = value"),
                            (["just_a_word"], "--set takes key = value")):
        try:
            lines_of(settings)
        except SystemExit as refusal:
            assert words in str(refusal), (settings, refusal)
        else:
            raise AssertionError("--set %s was taken" % settings)
    try:
        lines_of([], text="[distant_land]\nenabled = true\n")
    except SystemExit as refusal:
        assert "no section [runtime] in test.toml" in str(refusal), refusal
    else:
        raise AssertionError("a toml with no [runtime] was taken")
    # --engine-reference adds its line, and the reference switch is off for it.
    args = argparse.Namespace(set=["a = 1"], engine_reference=True, reference=False, mge_build=None)
    assert run_settings(args) == ["a = 1", "[runtime] disabled = true"] and args.set == ["a = 1"]
    assert wgpu_switch(args) is False
    args = argparse.Namespace(set=[], engine_reference=False, reference=False, mge_build="E:/b")
    assert run_settings(args) == [] and wgpu_switch(args) is True
    args.mge_build = None
    assert wgpu_switch(args) is None


def self_test_check_command():
    """`--check` through main, in a process of its own, against an instance made for the test:
    a fault of the arguments, a missing save and the plugin in the overwrite folder each refuse
    the command, and a good command passes. The checks have their own tests above; this test
    covers their calls in main."""
    with tempfile.TemporaryDirectory() as instance:
        game = os.path.join(instance, "game")
        os.makedirs(os.path.join(game, "Saves"))
        os.makedirs(os.path.join(instance, "profiles", "Default"))
        # The mgeXE.toml of the instance's configuration mod: it has no [views] section.
        config = os.path.join(instance, "mods", "Configuration", "Root")
        os.makedirs(config)
        with open(os.path.join(config, "mgeXE.toml"), "w", encoding="utf-8") as fh:
            fh.write(TEST_TOML)

        def check(*extra):
            command = [sys.executable, "-I", os.path.abspath(__file__), "--check", "--instance", instance, "--game-dir", game,
                       "--save", "a.ess", *extra]
            result = subprocess.run(command, capture_output=True, text=True)
            return result.returncode, result.stdout + result.stderr

        code, text = check()
        assert code != 0 and "a.ess is not in" in text, text
        open(os.path.join(game, "Saves", "a.ess"), "wb").close()
        code, text = check()
        assert code == 0 and "nothing started" in text, text
        assert "--full-dump" not in text, text
        code, text = check("--full-dump")
        assert code == 0 and "--full-dump: a crash of the run writes a dump of about " + FULL_DUMP_SIZE in text, text
        code, text = check("--weather", "10")
        assert code != 0 and "--weather" in text, text
        # --set against the toml of the instance: a section that it lacks is made, and a key with no
        # section that it lacks is a fault, as in the start.
        code, text = check("--set", "[views] full_tier_max = 4")
        assert code == 0, text
        code, text = check("--set", "views.full_tier_max = 4")
        assert code != 0 and "no line with the key views.full_tier_max" in text and "[views] full_tier_max = 4" in text, text
        code, text = check("--set", "[statics] enabled = true")
        assert code == 0, text
        code, text = check("--set", "[distant_land] far = [")
        assert code != 0 and "is not a toml" in text, text
        code, text = check("--set", "no_such_key = 1")
        assert code != 0 and "no line with the key no_such_key" in text and "[section] no_such_key = 1" in text, text
        code, text = check("--reference")
        assert code == 0, text
        plugin = os.path.join(instance, "overwrite", session.harness.PLUGIN_FILE)
        os.makedirs(os.path.dirname(plugin))
        open(plugin, "wb").close()
        code, text = check("--reference")
        assert code != 0 and "overwrite folder" in text, text
        code, text = check("--reference", "--reference-plugin")
        assert code == 0, text
        code, text = check("--reference", "--reference-plugin", "--enable-mod", "Some mod")
        assert code != 0 and "no mod folder" in text, text
        os.makedirs(os.path.join(instance, "mods", "Some mod"))
        open(os.path.join(instance, "mods", "Some mod", "some.esp"), "wb").close()
        code, text = check("--reference", "--reference-plugin", "--enable-mod", "Some mod", "--enable-plugin", "some.esp")
        assert code != 0 and "the profile has no Morrowind.ini" in text, text
        with open(os.path.join(instance, "profiles", "Default", "Morrowind.ini"), "wb") as fh:
            fh.write(b"[Game Files]\r\nGameFile0=Morrowind.esm\r\n")
        # The steps without a capture: the ini has no "Number of Shadows", so the game's level is 0.
        code, text = check("--reference", "--reference-plugin", "--enable-mod", "Some mod", "--enable-plugin", "some.esp", "--steps", "exterior")
        assert code == 0 and "some.esp becomes GameFile1" in text and "Some mod is on for the run" in text, text
        assert "shadows: registry Shadows " in text and "Morrowind.ini Number of Shadows 0" in text, text
        # A capture run with the level 0 is refused. The level is the smaller value, so the ini's 0 is
        # enough; without the registry value on this machine the level is unknown and nothing refuses.
        code, text = check("--steps", "exterior,capture")
        if registry_shadows()[0] is None:
            assert code == 0 and "the game's level missing" in text, text
        else:
            assert code != 0 and "real-time shadows off" in text and "Number of Shadows 0" in text, text


def self_test_shadows():
    """The shadow level of a run's start: the ini's value as GetPrivateProfileIntA reads it, the
    smaller of the two values, and the refusal of a capture run with the level 0."""
    ini = b"[Game Files]\nNumber of Shadows=9\n[General]\nnumber of shadows = 6 ; a comment\nNumber of Shadows=2\n[Other]\n"
    assert ini_shadows(ini) == 6, "the key of [General] only, the first one, the digits at the start"
    assert ini_shadows(b"[General]\nMaximum Shadows Per Object=1\n") == 0, "no key: the game's default 0"
    assert ini_shadows(b"[General]\nNumber of Shadows=x\n") == 0
    assert ini_shadows(None) is None, "no file: unknown, not 0"
    assert ini_shadows(b"[General]\nNumber of Shadows=-1\n") == 0, "GetPrivateProfileIntA gives 0 for a negative value"

    # The registry: HKLM first, the VirtualStore copy only when HKLM has no such value. The case of
    # 2026-11-10: the VirtualStore 0, HKLM 6, and the game's capture had shadow scenes.
    def reader(stored, machine):
        def read(root, path, view):
            if root == winreg.HKEY_CURRENT_USER:
                assert path == VIRTUAL_STORE_KEY and view == 0, (path, view)
                return stored
            assert root == winreg.HKEY_LOCAL_MACHINE and path == GAME_REGISTRY_KEY and view == winreg.KEY_WOW64_32KEY, (root, path, view)
            return machine
        return read

    assert registry_shadows(reader(0, 6)) == (6, "HKLM; VirtualStore 0"), "HKLM wins, and the line shows the VirtualStore value"
    assert registry_shadows(reader(6, 0)) == (0, "HKLM; VirtualStore 6")
    assert registry_shadows(reader(None, 0)) == (0, "HKLM; VirtualStore missing")
    assert registry_shadows(reader(3, None)) == (3, "VirtualStore"), "no HKLM value: the VirtualStore"
    assert registry_shadows(reader(None, None)) == (None, "VirtualStore")

    line, level = shadow_level_line(0, 6, "HKLM; VirtualStore 6")
    assert level == 0 and line == "shadows: registry Shadows 0 (HKLM; VirtualStore 6), Morrowind.ini Number of Shadows 6: the game's level 0", line
    assert shadow_level_line(6, 6)[1] == 6 and shadow_level_line(8, 3)[1] == 3
    assert shadow_level_line(None, 6) == ("shadows: registry Shadows missing (HKLM), Morrowind.ini Number of Shadows 6: the game's level missing", None)
    assert shadow_level_line(6, None)[1] is None

    def namespace(steps):
        return argparse.Namespace(steps=steps)

    # The fault of 2026-11-10: a capture run with the registry's Shadows 0.
    line, level = shadow_level_line(0, 6)
    for steps in ("exterior,capture", "capture", None, ""):
        problem = shadow_problem(namespace(steps), level, line)
        assert problem and "registry Shadows 0 (HKLM), Morrowind.ini Number of Shadows 6" in problem and "ask the owner" in problem, (steps, problem)
    assert shadow_problem(namespace("exterior,shaders"), level, line) is None, "no capture step: no refusal"
    for good in (6, None):
        assert shadow_problem(namespace("exterior,capture"), good, line) is None, good

    value, source = registry_shadows()
    assert (value is None or isinstance(value, int)) and source.startswith(("VirtualStore", "HKLM")), (value, source)


def self_test_weather():
    """The weather line of the summary, and the warning of a timing run that leaves the weather to the game."""
    def namespace(**changes):
        values = dict(weather=None, steps=None, shader_batch=None)
        values.update(changes)
        return argparse.Namespace(**values)

    # A pinned weather: held only with the mod's read-back line of that weather. The planted faults:
    # no read-back (the tour step, or a switch that failed in its pcall), and a read-back of another
    # weather; both are NOT applied, whatever the argument said.
    assert weather_summary(0, []) == "pinned 0, NOT applied (0 re-pins)"
    assert weather_summary(0, ['weather start: current 6 "Ashstorm"']) == "pinned 0, NOT applied (0 re-pins)"
    assert weather_summary(0, ['weather pinned 0: now 6 "Ashstorm"']) == "pinned 0, NOT applied (0 re-pins)"
    assert weather_summary(1, ['weather pinned 1: now 10 "x"']) == "pinned 1, NOT applied (0 re-pins)"
    applied = ['weather pinned 0: now 0 "Clear"']
    assert weather_summary(0, applied) == "pinned 0, held (0 re-pins)"
    repins = applied + ['weather event: changed immediately to 6 "Ashstorm"', 'weather re-pin 1: immediate change to 6 "Ashstorm", back to 0',
              'weather re-pin 2: transition to 2 "Foggy", back to 0', 'rate: 1.0 weather re-pin 3']
    assert weather_summary(0, repins) == "pinned 0, held (2 re-pins)", weather_summary(0, repins)
    # The step weathers releases the pin: the summary says so.
    released = repins + ["weather pin released for the step weathers"]
    assert weather_summary(0, released) == "pinned 0, held (2 re-pins), released at weathers", weather_summary(0, released)
    # The game's weather: the names in order of first appearance, from every kind of line.
    lines = [
        'weather start: current 3 "Overcast"',
        'weather rate: current 3 "Overcast"; next 6 "Rain" transition 0.42',
        'weather event: transition started from 3 "Overcast" to 6 "Rain"',
        'weather event: changed immediately to 9 "Blizzard"',
        'rate: 100.0 frames in a second at "Balmora" 3 "not a weather line"',
    ]
    assert weather_summary(None, lines) == "the game's: Overcast (3), Rain (6), Blizzard (9)", weather_summary(None, lines)
    assert weather_summary(None, ["step exterior: true"]) == "the game's: none logged"
    assert weather_summary(None, ["weather start: not read: x"]) == "the game's: none logged"

    # The timing rule of the slot check (D297): the steps rate and walk, no --steps, or --timing.
    def timing(**changes):
        return is_timing_run(namespace(**{"timing": False, **changes}))
    assert timing() and timing(steps="rate") and timing(steps="exterior, walk") and timing(steps="exterior,rate,capture")
    assert not timing(steps="exterior,capture") and not timing(steps="shaders", shader_batch="E:/batch.lua")
    assert not timing(steps="interior,shaders") and not timing(steps="tour")
    assert timing(steps="shaders", shader_batch="E:/batch.lua", timing=True) and timing(steps="exterior", timing=True)

    # The warning: a run that times and leaves the weather to the game; the steps named decide.
    assert weather_warning(namespace(weather=0)) is None
    assert weather_warning(namespace(weather=0, steps="rate")) is None
    assert "--weather 0" in weather_warning(namespace())
    assert "--weather 0" in weather_warning(namespace(steps="exterior,rate,capture"))
    assert "--weather 0" in weather_warning(namespace(steps="walk"))
    assert weather_warning(namespace(steps="exterior,capture")) is None
    # A shader-batch A/B compares frame times: it warns too, with --shader-batch or with the step shaders.
    assert "--weather 0" in weather_warning(namespace(steps="shaders"))
    assert "--weather 0" in weather_warning(namespace(steps="exterior,shaders"))
    assert "--weather 0" in weather_warning(namespace(steps="exterior", shader_batch="E:/batch.lua"))
    assert weather_warning(namespace(steps="exterior,shaders", weather=0)) is None
    assert weather_warning(namespace(steps="exterior,shaders", shader_batch="E:/batch.lua", weather=0)) is None

    # The pin over the batch's own weather: it warns only for a change to another weather.
    batches = {
        "E:/boilerplate.lua": "wc:switchImmediate(0)\nwc:updateVisuals()\n",
        "E:/rain.lua": "wc:switchImmediate(4)\n",
        "E:/variable.lua": "if x then wc:switchImmediate( hold.weather ) end\n",
        "E:/transition.lua": "wc:switchTransition(2)\ntes3.changeWeather({ id = 6 })\n",
        "E:/none.lua": "return {}\n",
    }
    def pin(path, weather=0):
        return weather_pin_warning(namespace(shader_batch=path, weather=weather), batches.get)
    assert pin("E:/boilerplate.lua") is None
    assert pin("E:/none.lua") is None
    assert pin("E:/rain.lua", weather=None) is None
    assert weather_pin_warning(namespace(weather=0), batches.get) is None
    assert pin("E:/boilerplate.lua", weather=4) and "switchImmediate(0)" in pin("E:/boilerplate.lua", weather=4)
    assert "switchImmediate(4)" in pin("E:/rain.lua") and "leave --weather out" in pin("E:/rain.lua")
    assert "switchImmediate(hold.weather)" in pin("E:/variable.lua"), pin("E:/variable.lua")
    assert "switchTransition(2)" in pin("E:/transition.lua") and "changeWeather({ id = 6 }" in pin("E:/transition.lua"), pin("E:/transition.lua")
    # A missing batch file is the argument check's problem, not this warning's.
    assert pin("E:/missing.lua") is None
    # A batch that sets its own weather, with no --weather: no advice to pin, which would override it.
    assert weather_warning(namespace(steps="exterior,shaders", shader_batch="E:/rain.lua"), batches.get) is None
    assert "--weather 0" in weather_warning(namespace(steps="exterior,shaders", shader_batch="E:/none.lua"), batches.get)

    # The note of the run folder: the summary line, then the weather lines and no others.
    note = weather_note(None, lines)
    assert note.splitlines()[0] == "weather: the game's: Overcast (3), Rain (6), Blizzard (9)", note
    assert note.splitlines()[1:] == lines[:4], note
    assert weather_note(0, lines).splitlines()[0] == "weather: pinned 0, NOT applied (0 re-pins)"
    assert weather_note(0, applied + lines).splitlines()[0] == "weather: pinned 0, held (0 re-pins)"
    assert weather_note(0, repins).splitlines()[0] == "weather: pinned 0, held (2 re-pins)"
    assert weather_note(None, []) == "weather: the game's: none logged\n"


def self_test_mge_log_problems():
    """The log check of the rig tools: the wgpu renderer installs no engine hooks and passes; a
    Direct3D 9 start with none, or with camera-relative rendering off only, fails."""
    problems = session.mge_log_problems
    assert problems("MGE-XE\n>> Using the wgpu renderer: no Direct3D device\nMWSE.dll injected.\n") == []
    assert problems("-- Indexed skinning engine hooks installed\n-- Camera-relative rendering: hooks installed (camera yes).\n") == []
    assert problems("-- Indexed skinning engine hooks installed\n") == []
    none = "MGE XE installed none of its engine hooks"
    assert problems("MGE-XE\nMWSE.dll injected.\n") == [none]
    assert problems("-- Camera-relative rendering off (render.camera_relative); no engine hooks installed.\n") == [none]
    # The plain game (W462): the fall-back and runtime.disabled install nothing by design.
    plain = "!! wgpu renderer: not in use, MWSE's native ABI is missing. The game runs on its own Direct3D 8\n"
    assert problems(plain) == []
    assert problems(">> Using the wgpu renderer: no Direct3D device\n" + plain) == []
    assert problems("-- runtime.disabled: the game runs on its own Direct3D 8\n") == []
    assert problems(">> Using the wgpu renderer: no Direct3D device\nx failed to load/compile\n") == ["MGE XE could not compile 1 shaders"]


def self_test_fallback_guard():
    """The fall-back guard (D328, verdict 314; its probe parts went with W412): MGE XE's fall-back line stops a wgpu run
    and not a reference run; "own device: false" fails a wgpu run with exit code 4, also with DONE."""
    # The line of render-link d3d8/cpp/mge/session.cpp: Session::toPlainGame writes it for each start that cannot use
    # the renderer.
    off = "!! wgpu renderer: not in use, a part is missing: numidium.exe. The game runs on its own Direct3D 8"
    reason = session.fallback_of([">> MGE XE starts", off])
    assert reason == "MGE XE did not use the renderer: " + off, reason
    assert session.fallback_of([">> Using the wgpu renderer: no Direct3D device"]) is None

    # Run.refusal reads the live mgeXE.log: the fall-back stops a wgpu run and keeps its reason; a reference run goes on.
    with tempfile.TemporaryDirectory() as folder:
        saved = (session.MGE_LOGS, session.RESULTS)
        session.RESULTS = folder
        log = os.path.join(folder, "mgeXE.log")
        session.MGE_LOGS = (log,)
        try:
            for wgpu in (False, True):
                run = session.Run("guard-test-%d" % wgpu)
                run.wgpu = wgpu
                run.started = time.time() - 10
                with open(log, "w", encoding="utf-8") as file:
                    file.write("MGE-XE G7\n" + off + "\n")
                reason = run.refusal()
                assert bool(reason) == wgpu and run.fell_back == reason, (wgpu, reason, run.fell_back)
        finally:
            session.MGE_LOGS, session.RESULTS = saved

    # The progress line "own device: false", and the exit code 4 before DONE's 0.
    assert fallback_of_run(False, None, ["own device: false"]) is None, "a reference run has no own device by design"
    assert fallback_of_run(True, None, ["own device: true", "step sh-start: true"]) is None
    late = fallback_of_run(True, None, ["steps: shaders (2)", "own device: false"])
    assert late == "a wgpu run without the renderer: the progress says \"own device: false\"", late
    stopped = fallback_of_run(True, "MGE XE did not use the renderer: x", [])
    assert stopped == "a wgpu run without the renderer: MGE XE did not use the renderer: x", stopped
    assert run_exit_code(False, False, True, ENDED_DONE, True) == EXIT_FELL_BACK == 4
    assert run_exit_code(False, False, False, ENDED_OTHER, True) == 4, "a run that the fall-back line stopped"
    assert run_exit_code(False, False, True, ENDED_DONE) == 0


def self_test_refusal():
    """A refusal line of the plugin stops the run, with the lines of its cause; other lines do not."""
    refusal_of = session.refusal_of
    assert refusal_of(["census: 8 of 348 functions differ from the database", "install: 313 ports installed, 0 failed"]) is None
    # The lines of plugin 862873b on MWSE.dll bf6a3039 (run 20261110-003000), with the end of the refusal line as the
    # plugin writes it since W462.
    old = [
        "census: 0x6a9d84 NiDX8Renderer::deleting_dtor+0x4 [4]: [38, e5, 50, 03] was [88, 00, 00, 00] -> call 0x3bb82c0 in MWSE.dll (PORT LOCKED: it stays game code here)",
        "no detour on NiDX8Renderer::deleting_dtor: entry bytes differ from the database",
        "1 of 313 ports cannot run as Rust: NiDX8Renderer::deleting_dtor",
        "install: the renderer is refused for this session (see the census above); "
        "nothing of the plugin is in the game's code, so the game runs on its own Direct3D 8",
        "own device: the game asked for a Direct3D 8; refused (see above)",
    ]
    reason = refusal_of(old)
    assert reason.startswith("the plugin refused the renderer: census: 0x6a9d84"), reason
    assert "no detour on NiDX8Renderer::deleting_dtor" in reason and reason.endswith("the game runs on its own Direct3D 8"), reason
    # The progress line of main.lua.
    assert "REFUSED: 1 of 313" in refusal_of(["[nirender.owndevice] step sh-start: true", "[nirender] REFUSED: 1 of 313 ports cannot run as Rust"])
    # The live log of MGE XE, as Run.refusal reads it.
    with tempfile.TemporaryDirectory() as folder:
        saved = (session.MGE_LOGS, session.RESULTS)
        session.RESULTS = folder
        log = os.path.join(folder, "mgeXE.log")
        session.MGE_LOGS = (log,)
        try:
            run = session.Run("refusal-test")
            run.started = time.time() - 10
            assert run.refusal() is None
            # The game writes the log as it goes: the cause lines, then the refusal line in two parts.
            text = "MGE-XE\n" + "".join("nirender: %s\n" % line for line in old)
            cut = text.index("install: the renderer") + 10
            with open(log, "w", encoding="utf-8") as file:
                file.write(text[:cut])
            assert run.refusal() is None, "a line without its end stopped the run"
            with open(log, "a", encoding="utf-8") as file:
                file.write(text[cut:])
            assert run.refusal() == reason, run.refusal()
            assert run.refusal() == reason, "a second call lost the lines read before"
        finally:
            session.MGE_LOGS, session.RESULTS = saved


def self_test_ring_problem():
    """A plugin and a helper with different ring marks are refused before the start; the same
    mark, no mark or no helper are not. The helper is the Root file that wins in the mod list."""
    with tempfile.TemporaryDirectory() as root:
        def binary(path, data):
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "wb") as fh:
                fh.write(b"\x00\xc7\x00" + data + b"\x89\x68")
            return path
        plugin = binary(os.path.join(root, "plugin.dll"), b"2RNG")
        same = binary(os.path.join(root, "same.exe"), b"\x3d2RNG\x0f\x85 \x002RNG")
        old = binary(os.path.join(root, "old.exe"), b"\x3d1RNG\x0f\x85 \x001RNG")
        unmarked = binary(os.path.join(root, "unmarked.exe"), b"RNG")
        assert session.ring_marks(old) == {b"1RNG"}
        assert session.ring_problem(plugin, same) is None
        assert session.ring_problem(plugin, unmarked) is None
        assert session.ring_problem(plugin, None) is None and session.ring_problem(None, old) is None
        problem = session.ring_problem(plugin, old)
        assert problem and "2RNG" in problem and "1RNG" in problem and old in problem, problem

        game = os.path.join(root, "game")
        binary(os.path.join(game, "numidium.exe"), b"1RNG")
        binary(os.path.join(root, "mods", "Old", "Root", "numidium.exe"), b"1RNG")
        binary(os.path.join(root, "mods", "New", "Root", "numidium.exe"), b"2RNG")
        winner = session.winning_root_file
        assert winner("numidium.exe", root, ["+New", "+Old"], game) == os.path.join(root, "mods", "New", "Root", "numidium.exe")
        assert winner("numidium.exe", root, ["-New", "+Old"], game) == os.path.join(root, "mods", "Old", "Root", "numidium.exe")
        assert winner("numidium.exe", root, ["-New", "-Old"], game) == os.path.join(game, "numidium.exe")
        binary(os.path.join(root, "overwrite", "Root", "numidium.exe"), b"2RNG")
        assert winner("numidium.exe", root, ["+Old"], game) == os.path.join(root, "overwrite", "Root", "numidium.exe")
        assert winner("other.exe", root, ["+Old"], game) is None

        # The runner's NIRENDER_HOST_EXE (the plugin's own device) wins over the game folder's.
        host = binary(os.path.join(root, "repo", "nirender-host.exe"), b"2RNG")
        helper = session.ring_helper
        assert helper(root, ["+Old"], game, {"NIRENDER_HOST_EXE": host}) == host
        assert helper(root, ["-Old"], game, {}) == os.path.join(root, "overwrite", "Root", "numidium.exe")
        assert helper(root, ["+Old"], game, {"NIRENDER_HOST_EXE": os.path.join(root, "no such.exe")}) == os.path.join(root, "overwrite", "Root", "numidium.exe")

        # A build's files: the binaries always, the rest when the build has them.
        build = os.path.join(root, "build")
        binary(os.path.join(build, "d3d8.dll"), b"")
        binary(os.path.join(build, "Data Files", "mwse", "mods", "MGE XE Options", "main.lua"), b"")
        binary(os.path.join(build, "Data Files", "shaders", "core", "XE Main.fx"), b"")
        files = session.mge_build_files(build)
        assert set(files) == {"Root/d3d8.dll", "Root/numidium.exe", "MWSE/mods/MGE XE Options/main.lua", "shaders/core/XE Main.fx"}, files
        assert files["Root/numidium.exe"] == os.path.join(build, "numidium.exe")

    # Run.launch stops before the start on a ring problem, and restores the instance.
    calls = []

    class Deployment:
        def write_spec(self, *a):
            pass

        def enable_mod(self, name):
            calls.append("enable")

        def use_mod(self, *a):
            pass

        def disable_mod(self, *a):
            pass

        def restore(self):
            calls.append("restore")

    saved = (session.harness.Deployment, session.harness.run_game, session.ring_problem_of_instance, session.RESULTS, session.stage_mwse_config)
    with tempfile.TemporaryDirectory() as results:
        session.harness.Deployment = Deployment
        session.harness.run_game = lambda *a, **k: calls.append("run_game")
        session.ring_problem_of_instance = lambda: "the rings differ"
        session.stage_mwse_config = lambda deployment, settings: None
        session.RESULTS = results
        try:
            run = session.Run("ring-test")
            try:
                run.launch({}, 10, "[t] ")
            except SystemExit as stop:
                assert "the rings differ" in str(stop), stop
            else:
                raise AssertionError("Run.launch started with a ring problem")
            assert calls == ["enable", "restore"], calls
        finally:
            (session.harness.Deployment, session.harness.run_game, session.ring_problem_of_instance, session.RESULTS, session.stage_mwse_config) = saved


def self_test_ending():
    """W372: the three endings of a run and their exit codes. The nulls: DONE gives 0, a hand end 3, and a planted
    abnormal exit or a dump the crash path (1), never 3."""
    by_hand = {"reason": "exited", "codes": [0]}
    assert run_ending(True, {"reason": "done", "codes": []}, [], "a") == ENDED_DONE
    assert run_ending(False, by_hand, [], "a") == ENDED_BY_HAND
    assert run_ending(False, by_hand, [], None) == ENDED_OTHER, "code 0 before the first step: MWSE's check of MGE XE"
    assert run_ending(False, {"reason": "exited", "codes": [0xC0000005]}, [], "a") == ENDED_OTHER, "a planted abnormal exit"
    assert run_ending(False, by_hand, ["MWSE_MiniDump.dmp"], "a") == ENDED_OTHER, "a dump with exit code 0"
    assert "exit code 12" in plugin_refused_line({"reason": "exited", "codes": [12]})
    assert plugin_refused_line(by_hand) is None and plugin_refused_line({"reason": "exited", "codes": [20]}) is None
    assert run_ending(False, {"reason": "exited", "codes": [None]}, [], "a") == ENDED_OTHER, "a code not read"
    assert run_ending(False, {"reason": "exited", "codes": []}, [], "a") == ENDED_OTHER, "no code"
    for reason in ("timeout", "stopped", "never started"):
        assert run_ending(False, {"reason": reason, "codes": []}, [], "a") == ENDED_OTHER, reason
    assert run_ending(False, {}, [], "a") == ENDED_OTHER, "a harness with no LAST_END"
    assert [run_exit_code(False, False, *pair) for pair in ((True, ENDED_DONE), (False, ENDED_BY_HAND), (False, ENDED_OTHER))] == [0, 3, 1]
    assert run_exit_code(True, True, False, ENDED_BY_HAND) == 2, "a reference run that loaded the plugin"
    progress = ["steps: interior,shaders (4)", "going to 'x'", "step interior: true ", "shader batch: start: run true",
                "step sh-start: true ", "shader batch: live: run true held for the bridge"]
    assert last_step(progress) == "sh-live" and last_step(progress[:3]) == "interior" and last_step(progress[:2]) is None
    # A run folder of a live session (20261112-040607-w341b-live) ends with these lines: the live set was the last.
    assert last_step(progress + ["step sh-live: true ", "weather event: changed immediately to 1 \"Cloudy\""]) == "sh-live"
    # report_ending, as main() calls it: the harness's record and the files of a run folder decide; the line names the step.
    class FakeRun:
        def __init__(self, folder):
            self.folder = folder

        def path(self, name):
            return os.path.join(self.folder, name)

    saved = getattr(session.harness, "LAST_END", None)
    with tempfile.TemporaryDirectory() as folder:
        try:
            session.harness.LAST_END = dict(by_hand)
            with contextlib.redirect_stdout(io.StringIO()) as said:
                assert report_ending(FakeRun(folder), False, progress) == ENDED_BY_HAND
            assert said.getvalue() == "ended without DONE after step sh-live, with exit code 0 (a close from outside, or the game's own exit)\n", said.getvalue()
            with contextlib.redirect_stdout(io.StringIO()) as said:
                assert report_ending(FakeRun(folder), False, progress[:2]) == ENDED_OTHER and said.getvalue() == "", "no step started"
            with open(os.path.join(folder, session.harness.DUMPS_FILE), "w", encoding="ascii") as dumps:
                dumps.write("E:/x/Morrowind.exe.1.dmp\n")
            with contextlib.redirect_stdout(io.StringIO()) as said:
                assert report_ending(FakeRun(folder), False, progress) == ENDED_OTHER and said.getvalue() == "", "a WER dump: the crash path"
            os.remove(os.path.join(folder, session.harness.DUMPS_FILE))
            del session.harness.LAST_END
            assert report_ending(FakeRun(folder), False, progress) == ENDED_OTHER, "a harness from before W372"
        finally:
            if saved is not None:
                session.harness.LAST_END = saved


def self_test_full_dump():
    """--full-dump: the setting goes into the run's copy of MWSE.json only with the option, beside
    RunInBackground of a background run; --check names the size."""
    assert session.mwse_run_settings(False, False) == {"RunInBackground": True}
    assert session.mwse_run_settings(False, True) == {"RunInBackground": True, "CreateFullMinidumps": True}
    assert session.mwse_run_settings(True, True) == {"CreateFullMinidumps": True}
    assert session.mwse_run_settings(True, False) == {}
    assert full_dump_note(argparse.Namespace(full_dump=False)) is None
    assert FULL_DUMP_SIZE in full_dump_note(argparse.Namespace(full_dump=True))
    # The harness keeps a run's WER dumps where the data rule looks for them (D94).
    import clean_common
    assert session.harness.RUN_DUMPS == clean_common.KEPT_DUMPS, (session.harness.RUN_DUMPS, clean_common.KEPT_DUMPS)
    assert session.harness.DUMPS_FILE == clean_common.DUMPS_FILE

    # stage_mwse_config writes the copy with the settings over the source's values into the run
    # mod, and the source does not change.
    class Deployment:
        def __init__(self):
            self.created = []

        def create(self, path):
            self.created.append(path)

    saved = (session.harness.SPEC_DIR, session.harness.MO2_ROOT, session.mwse_config_source)
    with tempfile.TemporaryDirectory() as folder:
        source = os.path.join(folder, "mod", "MWSE.json")
        os.makedirs(os.path.dirname(source))
        with open(source, "w", encoding="utf-8") as file:
            json.dump({"LogWarningsWithLuaStack": True, "RunInBackground": False}, file)
        with open(source, "rb") as file:
            before = file.read()
        session.harness.MO2_ROOT = os.path.join(folder, "instance")
        session.mwse_config_source = lambda: source
        try:
            for foreground, expected in ((False, {"RunInBackground": True, "CreateFullMinidumps": True}), (True, {"RunInBackground": False, "CreateFullMinidumps": True})):
                session.harness.SPEC_DIR = os.path.join(folder, "run-%s" % foreground)
                deployment = Deployment()
                session.stage_mwse_config(deployment, session.mwse_run_settings(foreground, True))
                staged = os.path.join(session.harness.SPEC_DIR, "MWSE.json")
                assert deployment.created == [staged], deployment.created
                with open(staged, encoding="utf-8") as file:
                    values = json.load(file)
                assert values == dict({"LogWarningsWithLuaStack": True}, **expected), values
            with open(source, "rb") as file:
                assert file.read() == before, "the source MWSE.json changed"
        finally:
            (session.harness.SPEC_DIR, session.harness.MO2_ROOT, session.mwse_config_source) = saved

    # main's call of launch passes --full-dump.
    class FakeRun:
        def launch(self, *arguments, **options):
            self.options = options
            return True

        def refusal(self):
            return None

    for full_dump in (False, True):
        run = FakeRun()
        launch_run(run, {}, argparse.Namespace(budget=10, disable_mod=[], mge_build=None, full_dump=full_dump, enable_mod=[], enable_plugin=[]), [])
        assert run.options["full_dump"] is full_dump, run.options
    run = FakeRun()
    launch_run(run, {}, argparse.Namespace(budget=10, disable_mod=[], mge_build=None, full_dump=False, enable_mod=["Mirror test"], enable_plugin=["mirrorTest.esp"]), [])
    assert run.options["enable"] == ["Mirror test"] and run.options["plugins"] == ["mirrorTest.esp"], run.options
    # A refusal of the renderer ends the run (self_test_refusal).
    assert run.options["stop"] == run.refusal, run.options


def self_test_enable():
    """--enable-mod and --enable-plugin: the line that game_file_plan adds, the faults, the file as it
    was after restore() (a stopped or crashed run takes the same path), and a launch that fails
    before the start."""
    harness = session.harness
    crlf = b"[Game Files]\r\nGameFile0=Morrowind.esm\r\nGameFile1=Tribunal.esm\r\n[Other]\r\nx=1\r\n"
    number, changed = harness.game_file_plan(crlf, "mirrorTest.esp")
    assert number == 2 and changed == b"[Game Files]\r\nGameFile0=Morrowind.esm\r\nGameFile1=Tribunal.esm\r\nGameFile2=mirrorTest.esp\r\n[Other]\r\nx=1\r\n", changed
    assert harness.game_file_plan(crlf, "TRIBUNAL.ESM") == (1, None)
    lf = b"GameFile3=a.esp\nGameFile10 = b.esp \n[Other]\n"
    number, changed = harness.game_file_plan(lf, "c.esp")
    assert number == 11 and changed == b"GameFile3=a.esp\nGameFile10 = b.esp \nGameFile11=c.esp\n[Other]\n", changed
    # The last GameFile line is the last line of the file, with and without an ending.
    assert harness.game_file_plan(b"GameFile0=a.esm", "b.esp") == (1, b"GameFile0=a.esm\nGameFile1=b.esp")
    assert harness.game_file_plan(b"GameFile0=a.esm\r\n", "b.esp")[1] == b"GameFile0=a.esm\r\nGameFile1=b.esp\r\n"
    for ini in (b"", b"[Other]\r\nx=1\r\n"):
        try:
            harness.game_file_plan(ini, "b.esp")
        except SystemExit as refusal:
            assert "no GameFile line" in str(refusal), refusal
        else:
            raise AssertionError("an ini with no GameFile line was taken")

    saved = (harness.PROFILE, harness.MO2_ROOT)
    with tempfile.TemporaryDirectory() as root:
        profile = os.path.join(root, "profiles", "Default")
        os.makedirs(profile)
        ini = os.path.join(profile, "Morrowind.ini")
        with open(ini, "wb") as fh:
            fh.write(crlf)
        modlist = os.path.join(profile, "modlist.txt")
        modlist_text = b"# This file was automatically generated\r\n+A\r\n-Mirror test\r\n"
        with open(modlist, "wb") as fh:
            fh.write(modlist_text)

        def content(path):
            with open(path, "rb") as fh:
                return fh.read()

        saved_processes = harness.processes
        game_pids = []
        harness.processes = lambda name: [(pid, "Morrowind.exe") for pid in game_pids]
        harness.PROFILE, harness.MO2_ROOT = profile, root
        try:
            deployment = harness.Deployment()
            deployment.enable_mod("Mirror test")
            deployment.enable_plugin("mirrorTest.esp")
            assert b"GameFile2=mirrorTest.esp\r\n" in content(ini)
            assert content(modlist) != modlist_text and content(modlist + ".perfbak") == modlist_text
            assert content(ini + ".perfbak") == crlf
            # While the game of this run goes, the backups are its own: not stale, no restore from outside,
            # and a second run is refused by the file's backup.
            game_pids.append(4242)
            assert harness.stale_backups() == []
            harness.refuse_stale_backups()
            try:
                harness.restore_stale()
            except SystemExit as refusal:
                assert "Morrowind.exe runs" in str(refusal), refusal
            else:
                raise AssertionError("restore_stale ran under a running game")
            try:
                harness.Deployment().enable_plugin("other.esp")
            except SystemExit as refusal:
                assert "stale backup" in str(refusal), refusal
            else:
                raise AssertionError("a second run took the ini while the first one's backup was there")
            game_pids.clear()
            # The testing instance's Mod Organizer can write modlist.txt over a restore.
            harness.processes = lambda name: [(5151, os.path.join(root, "ModOrganizer.exe"))] if name == "ModOrganizer.exe" else []
            try:
                harness.restore_stale()
            except SystemExit as refusal:
                assert "Mod Organizer" in str(refusal), refusal
            else:
                raise AssertionError("restore_stale ran under the instance's Mod Organizer")
            harness.processes = lambda name: [(pid, "Morrowind.exe") for pid in game_pids]
            deployment.restore()
            assert content(ini) == crlf and content(modlist) == modlist_text, "the files are not as they were after restore"
            assert not os.path.exists(ini + ".perfbak") and not os.path.exists(modlist + ".perfbak")
            # A plugin that the file names already changes nothing and leaves no backup.
            harness.Deployment().enable_plugin("Tribunal.esm")
            assert not os.path.exists(ini + ".perfbak")

            # A runner killed hard runs no restore. Every start refuses and names the files and the command;
            # restore_stale puts both back and the start goes.
            killed = harness.Deployment()
            killed.enable_mod("Mirror test")
            killed.enable_plugin("mirrorTest.esp")
            assert [os.path.basename(path) for path in harness.stale_backups()] == ["Morrowind.ini.perfbak", "modlist.txt.perfbak"]
            for start in (harness.Deployment, harness.refuse_stale_backups):
                try:
                    start()
                except SystemExit as refusal:
                    assert "Morrowind.ini.perfbak" in str(refusal) and "modlist.txt.perfbak" in str(refusal) and "--restore-stale" in str(refusal), refusal
                else:
                    raise AssertionError("a start was not refused over the stale backups")
            restored = harness.restore_stale()
            assert [os.path.basename(path) for path in restored] == ["Morrowind.ini", "modlist.txt"], restored
            assert content(ini) == crlf and content(modlist) == modlist_text
            assert harness.stale_backups() == [] and harness.restore_stale() == []
            harness.Deployment()

            # The backup of swap() lies beside the file it saves, outside the profile: a killed run
            # leaves it, and the record in the profile lets every start see it and restore_stale put it back.
            swapped = os.path.join(root, "game", "Morrowind.exe")
            replacement = os.path.join(root, "other.exe")
            os.makedirs(os.path.dirname(swapped), exist_ok=True)
            with open(swapped, "wb") as fh:
                fh.write(b"original")
            with open(replacement, "wb") as fh:
                fh.write(b"replaced")
            killed = harness.Deployment()
            killed.swap(replacement, swapped, "test")
            assert content(swapped) == b"replaced" and content(swapped + ".perfbak") == b"original"
            assert harness.stale_backups() == [swapped + ".perfbak"], harness.stale_backups()
            try:
                harness.Deployment()
            except SystemExit as refusal:
                assert "Morrowind.exe.perfbak" in str(refusal), refusal
            else:
                raise AssertionError("a start was not refused over the backup of swap()")
            assert harness.restore_stale() == [swapped]
            assert content(swapped) == b"original" and not os.path.exists(swapped + ".perfbak")
            assert not os.path.exists(os.path.join(profile, "swap-backups.txt"))
            # A run that restores itself leaves no record.
            clean = harness.Deployment()
            clean.swap(replacement, swapped, "test")
            clean.restore()
            assert content(swapped) == b"original" and not os.path.exists(os.path.join(profile, "swap-backups.txt"))
            harness.Deployment()

            # The line of a backup is written before the copy: a kill between the two leaves a line for a
            # backup that is not there, and no start is refused for it; a kill after the copy leaves both.
            real_copy = harness.shutil.copy2
            seen = []

            def killed_copy(src, dest, *a, **k):
                seen.append(open(os.path.join(profile, "swap-backups.txt"), encoding="utf-8").read())
                raise KeyboardInterrupt("the runner is killed here")

            harness.shutil.copy2 = killed_copy
            try:
                harness.Deployment().swap(replacement, swapped, "test")
            except KeyboardInterrupt:
                pass
            finally:
                harness.shutil.copy2 = real_copy
            assert seen == [swapped + ".perfbak\n"], seen
            assert harness.stale_backups() == [], harness.stale_backups()
            os.remove(os.path.join(profile, "swap-backups.txt"))

            # A file that use_repo_motte makes is named in the record too: a killed run leaves it, every
            # start refuses over it, and restore_stale removes it.
            repo, motte = os.path.join(root, "motte-repo"), os.path.join(root, "motte-mod")
            os.makedirs(os.path.join(repo, "sub"))
            with open(os.path.join(repo, "sub", "new.lua"), "wb") as fh:
                fh.write(b"new")
            saved_motte = (harness.MOTTE_REPO, harness.MOTTE_MOD)
            harness.MOTTE_REPO, harness.MOTTE_MOD = repo, motte
            try:
                harness.Deployment().use_repo_motte()
            finally:
                harness.MOTTE_REPO, harness.MOTTE_MOD = saved_motte
            made = os.path.join(motte, "sub", "new.lua")
            assert os.path.exists(made) and harness.stale_backups() == [made], harness.stale_backups()
            try:
                harness.Deployment()
            except SystemExit as refusal:
                assert "new.lua" in str(refusal) and "--restore-stale" in str(refusal), refusal
            else:
                raise AssertionError("a start was not refused over a file of a killed run")
            assert harness.restore_stale() == [made] and not os.path.exists(made)
            assert not os.path.exists(os.path.join(profile, "swap-backups.txt"))
            # A run that restores itself removes its made files and leaves no record.
            harness.MOTTE_REPO, harness.MOTTE_MOD = repo, motte
            try:
                clean = harness.Deployment()
                clean.use_repo_motte()
                assert os.path.exists(made)
                clean.restore()
            finally:
                harness.MOTTE_REPO, harness.MOTTE_MOD = saved_motte
            assert not os.path.exists(made) and not os.path.exists(os.path.join(profile, "swap-backups.txt"))
            harness.Deployment()
        finally:
            harness.PROFILE, harness.MO2_ROOT = saved
            harness.processes = saved_processes

        # The faults and the notes of --check.
        def namespace(**changes):
            values = dict(enable_mod=[], enable_plugin=[], disable_mod=[])
            values.update(changes)
            return argparse.Namespace(**values)

        game = os.path.join(root, "game")
        os.makedirs(os.path.join(game, "Data Files"))
        os.makedirs(os.path.join(root, "mods", "Mirror test"))
        open(os.path.join(root, "mods", "Mirror test", "mirrorTest.esp"), "wb").close()
        problems, notes = enable_problems(namespace(), root, game, crlf)
        assert problems == [] and notes == [], (problems, notes)
        problems, notes = enable_problems(namespace(enable_mod=["Mirror test"], enable_plugin=["mirrorTest.esp"]), root, game, crlf)
        assert problems == [] and len(notes) == 2 and "first in the mod list" in notes[0] and "GameFile2" in notes[1], (problems, notes)
        problems, notes = enable_problems(namespace(enable_plugin=["Tribunal.esm"]), root, game, crlf)
        assert len(problems) == 1 and "in no mod of --enable-mod" in problems[0], problems
        open(os.path.join(game, "Data Files", "Tribunal.esm"), "wb").close()
        problems, notes = enable_problems(namespace(enable_plugin=["Tribunal.esm"]), root, game, crlf)
        assert problems == [] and "already active as GameFile1" in notes[0], (problems, notes)
        for bad in (namespace(enable_mod=["No such mod"]), namespace(enable_mod=["Harness - nirender"]),
                    namespace(enable_mod=["Mirror test"], disable_mod=["Mirror test"]), namespace(enable_plugin=["x.txt"]),
                    namespace(enable_plugin=["sub/mirrorTest.esp"]), namespace(enable_plugin=["missing.esp"])):
            problems, _ = enable_problems(bad, root, game, crlf)
            assert problems, bad
        problems, _ = enable_problems(namespace(enable_mod=["Mirror test"], enable_plugin=["mirrorTest.esp"]), root, game, None)
        assert len(problems) == 1 and "no Morrowind.ini" in problems[0], problems
        problems, _ = enable_problems(namespace(enable_mod=["Mirror test"], enable_plugin=["mirrorTest.esp"]), root, game, b"[Other]\r\n")
        assert len(problems) == 1 and "no GameFile line" in problems[0], problems

    # Run.launch switches the mod and the plugin on, in this order after the mods to disable, and restores
    # when the start is refused. Each enable puts its mod at the top of the list, so the runner's own
    # mods come after the --enable-mod mods and end above them: a file of --add-file or --mwse-dll wins
    # over the same file in such a mod (D93: a run copy of a mod's main.lua lost to the mod's own).
    calls = []

    class Deployment:
        def write_spec(self, *a):
            pass

        def enable_mod(self, name):
            calls.append("mod " + name)

        def use_mod(self, name, files):
            calls.append("mod " + name)

        def disable_mod(self, name):
            calls.append("off " + name)

        def enable_plugin(self, plugin):
            calls.append("plugin " + plugin)

        def restore(self):
            calls.append("restore")

    saved = (harness.Deployment, harness.run_game, session.ring_problem_of_instance, session.RESULTS, session.stage_mwse_config)
    with tempfile.TemporaryDirectory() as results:
        harness.Deployment = Deployment
        harness.run_game = lambda *a, **k: calls.append("run_game")
        session.ring_problem_of_instance = lambda: "the rings differ"
        session.stage_mwse_config = lambda deployment, settings: None
        session.RESULTS = results
        try:
            try:
                session.Run("enable-test").launch({}, 10, "[t] ", mods=[("Harness - MGE XE", {})], disable=["Old"],
                                                  enable=["Mirror test", "Other mod"], plugins=["mirrorTest.esp"])
            except SystemExit as stop:
                assert "the rings differ" in str(stop), stop
            else:
                raise AssertionError("Run.launch started with a ring problem")
            assert calls == ["off Old", "mod Mirror test", "mod Other mod", "mod " + session.MOD_NAME, "mod Harness - MGE XE",
                             "plugin mirrorTest.esp", "restore"], calls
        finally:
            (harness.Deployment, harness.run_game, session.ring_problem_of_instance, session.RESULTS, session.stage_mwse_config) = saved


def self_test_use_instance():
    """--instance: every path of the harness in the testing instance or its game folder moves to
    the other instance, the crash dumps too. MOTTE_MOD is a mod of the testing instance only."""
    def paths(value):
        return [value] if isinstance(value, str) else [v for v in value if isinstance(v, str)] if isinstance(value, tuple) else []

    names = [name for name in dir(session.harness) if name.isupper() and name not in ("MO2_ROOT", "GAME_DIR", "MOTTE_MOD")]
    saved = {name: getattr(session.harness, name) for name in names + ["MO2_ROOT", "GAME_DIR"]}
    saved_mge = session.MGE_LOGS
    old_roots = (session.harness.MO2_ROOT, session.harness.GAME_DIR)
    held = [name for name in names if any(path.startswith(old_roots) for path in paths(saved[name]))]
    assert "MINIDUMPS" in held and "MWSE_LOGS" in held, held
    instance, game = os.path.join("X:", os.sep, "other"), os.path.join("X:", os.sep, "game")
    try:
        session.use_instance(instance, game)
        for name in held:
            for path in paths(getattr(session.harness, name)):
                assert path.startswith((instance, game)), "%s keeps %s" % (name, path)
        assert session.harness.MINIDUMPS[0] == os.path.join(instance, "overwrite", "Root", "MWSE_MiniDump.dmp")
    finally:
        for name, value in saved.items():
            setattr(session.harness, name, value)
        session.MGE_LOGS = saved_mge


def add_symbols(mge_build, plugin_dll, add=symbols.add):
    """The symbol store (W345): the pdbs of the run's build, beside its binaries or kept by the play set of its name;
    a timing run puts the add off until its game slot is free (D297). The run's plugin can be of another tree than the
    build, so it goes in as a build of its own, "plugin-<md5 8>" (W352): the build's name pins only the build's pdbs.
    A play set's own plugin is in the set's transaction already, and its add is "in the store". A build folder that is
    not a play set f<N> is named with the md5 of its d3d8.dll (symbols.build_name). Returns the lines."""
    folder_name = os.path.basename(os.path.normpath(mge_build))
    binaries = [os.path.join(mge_build, name) for name in ("d3d8.dll", "dinput8.dll", "numidium.exe")]
    lines = [add(symbols.build_name(mge_build), binaries, [os.path.join(PDB_KEEP, folder_name)])]
    if plugin_dll and os.path.isfile(plugin_dll):
        lines.append(add("plugin-" + symbols.md5_of(plugin_dll)[:8], [plugin_dll], [os.path.join(PDB_KEEP, folder_name)]))
    return lines


def self_test_add_symbols():
    """The two adds of a run: the build under its name, the plugin under its md5."""
    with tempfile.TemporaryDirectory() as folder:
        plugin = os.path.join(folder, "akulakhan.dll")
        with open(plugin, "wb") as file:
            file.write(b"abc")
        calls = []

        def add(*call):
            calls.append(call)
            return "symbols: +0"

        build = os.path.join(folder, "f231")
        assert add_symbols(build + os.sep, plugin, add) == ["symbols: +0", "symbols: +0"]
        assert calls[0] == ("f231", [os.path.join(build, name) for name in ("d3d8.dll", "dinput8.dll", "numidium.exe")],
                            [os.path.join(PDB_KEEP, "f231")]), calls[0]
        # A play set's plugin pdb is in the set's pdb folder, not beside its dll (the Reviewer's verdict 204).
        assert calls[1] == ("plugin-90015098", [plugin], [os.path.join(PDB_KEEP, "f231")]), calls[1]
        # A run without the plugin, or with no file at its path, adds the build only.
        calls.clear()
        assert len(add_symbols(build, None, add)) == 1 and len(add_symbols(build, plugin + "x", add)) == 1 and len(calls) == 2
        # A build folder that is not a play set is named with its d3d8.dll's md5; its pdb folder keeps the plain name.
        calls.clear()
        other = os.path.join(folder, "build")
        os.makedirs(other)
        with open(os.path.join(other, "d3d8.dll"), "wb") as file:
            file.write(b"abc")
        add_symbols(other, None, add)
        assert calls[0][0] == "build-90015098" and calls[0][2] == [os.path.join(PDB_KEEP, "build")], calls[0]


def self_test():
    """The tests of the argument handling: no game, no build."""
    self_test_add_symbols()
    self_test_mge_log_problems()
    self_test_refusal()
    self_test_fallback_guard()
    self_test_ring_problem()
    self_test_weather()
    self_test_shadows()
    self_test_checks()
    self_test_run_toml()
    self_test_check_command()
    self_test_enable()
    self_test_setup()
    self_test_save_problem()
    self_test_pngs_from_tgas()
    self_test_archive_lua()
    self_test_archive_arguments()
    self_test_plugin_evidence()
    self_test_full_dump()
    self_test_ending()
    self_test_use_instance()
    build = os.path.join("target", "akulakhan.dll")
    assert plugin_dll_of("E:/a.dll", {"NIRENDER_PLUGIN_DLL": "E:/b.dll"}, build) == ("E:/a.dll", "--plugin-dll")
    assert plugin_dll_of(None, {"NIRENDER_PLUGIN_DLL": "E:/b.dll"}, build) == ("E:/b.dll", "NIRENDER_PLUGIN_DLL")
    assert plugin_dll_of(None, {}, build) == (build, "the release build of this repository")
    assert plugin_dll_of("", {}, build)[0] == build
    assert env_of([]) == {}
    assert env_of(["MGE_XE_BOX_SECONDS=90", "A=1=2", "B=", "A=3"]) == {"MGE_XE_BOX_SECONDS": "90", "A": "3", "B": ""}
    for bad in ["NOVALUE", "=1"] + ["%s=E:/x" % name for name in RUNNER_VARIABLES]:
        try:
            env_of([bad])
        except SystemExit as refusal:
            name = bad.partition("=")[0]
            if name in RUNNER_VARIABLES:
                assert RUNNER_VARIABLES[name] in str(refusal), refusal
        else:
            raise AssertionError("--env %s was taken" % bad)
    # The two names that load code: refused one by one, also after a good entry.
    for name in ("NIRENDER_PLUGIN_DLL", "NIRENDER_HOST_EXE"):
        environment = {}
        try:
            apply_env(environment, ["MGE_XE_BOX_SECONDS=90", "%s=E:/other.bin" % name])
        except SystemExit:
            pass
        else:
            raise AssertionError("--env %s was taken" % name)
        assert name not in environment
    # The box time through the function that main uses: --env wins over the default, a value
    # of the runner's environment is kept, and without either the default is 5 s.
    environment = {}
    apply_env(environment, ["MGE_XE_BOX_SECONDS=90"])
    assert environment["MGE_XE_BOX_SECONDS"] == "90"
    environment = {"MGE_XE_BOX_SECONDS": "20"}
    apply_env(environment, [])
    assert environment["MGE_XE_BOX_SECONDS"] == "20"
    environment = {"MGE_XE_BOX_SECONDS": "20"}
    apply_env(environment, ["MGE_XE_BOX_SECONDS=90"])
    assert environment["MGE_XE_BOX_SECONDS"] == "90"
    environment = {}
    apply_env(environment, ["MGE_XE_WATCHDOG=1"])
    assert environment == {"MGE_XE_WATCHDOG": "1", "MGE_XE_BOX_SECONDS": "5"}
    import io

    # The plugin's lines of a run: from mgeXE.log with the export, else from nirender.log.
    import shutil
    import tempfile

    folder = tempfile.mkdtemp(prefix="nirender-lines-")
    assert session.common.plugin_lines(folder) == []
    with open(os.path.join(folder, "nirender.log"), "w", encoding="utf-8") as own:
        own.write("own device: from the file\n")
    assert session.common.plugin_lines(folder) == ["own device: from the file"]
    with open(os.path.join(folder, "mgeXE.log"), "w", encoding="utf-8", newline="") as mge:
        mge.write("MGE-XE G7\r\nnirender: own device: made\r\nnirender: MISMATCH a call\r\n>> Using the wgpu renderer\r\n")
    assert session.common.plugin_lines(folder) == ["own device: made", "MISMATCH a call"], session.common.plugin_lines(folder)
    with open(os.path.join(folder, "mgeXE.log"), "w", encoding="utf-8") as mge:
        mge.write("MGE-XE G7 without the plugin's lines\n")
    assert session.common.plugin_lines(folder) == ["own device: from the file"]
    shutil.rmtree(folder)
    console, log = io.StringIO(), io.StringIO()
    tee = Tee(console, log)
    print("a line", file=tee)
    tee.flush()
    assert console.getvalue() == "a line\n" and log.getvalue() == "a line\n"
    print("run_own_device self-test: ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", default="owndevice")
    ap.add_argument("--plugin-dll", help="the plugin DLL of the run (else the variable NIRENDER_PLUGIN_DLL, else the release build of this repository)")
    ap.add_argument("--log", help="a file that gets the runner's output, written to the console too; no shell redirect needed")
    ap.add_argument("--self-test", action="store_true", help="the tests of the argument handling: no game, no build")
    ap.add_argument("--check", action="store_true", help="check the command and stop: the form of each value, the files it "
                    "names and the save; nothing starts. Run it before you take the game slot: exit code 0 for a good command")
    ap.add_argument("--save", default="quiksave.ess", help="the save to load; \"\" starts a new game instead (the films of the game play)")
    ap.add_argument("--interior", default="Balmora, Guild of Mages")
    ap.add_argument("--exterior", default="Balmora")
    ap.add_argument("--backend", default="vulkan")
    ap.add_argument("--start-frames", default="5,100,300,600,900,1040", help="the frames of the start to take a picture of")
    ap.add_argument("--mge-build", help="a folder with a build of MGE XE that has the switch runtime.renderer_wgpu (its bin/MSVC-Release): "
                    "the run uses its d3d8.dll and its numidium.exe with the switch on, and not the d3d8.dll and the host program of this repository")
    ap.add_argument("--config-mod", default="Configuration", help="the mod of the testing instance with mgeXE.toml")
    ap.add_argument("--reference", action="store_true", help="run the same scenario with MGE XE on Direct3D 9, for its frame rate: the "
                    "MGE XE of the instance, or of --mge-build")
    ap.add_argument("--engine-reference", action="store_true", help="run the same scenario on the engine's own renderer: `runtime.disabled` in "
                    "the run's mgeXE.toml, so the game has the system's Direct3D 8 and no MGE XE draws. The pictures are eng-<step>.png: the world with no interface, at 2048 x 1024. "
                    "MGE XE of W462 and later installs nothing then; MWSE and the Code Patch apply their own: not the game of 2002")
    ap.add_argument("--plain-case", action="store_true", help="a run of a case where MGE XE cannot use the renderer (W462): the build of "
                    "--mge-build with the switch renderer_wgpu on, and a switch of --env that makes the case. The game runs on its own Direct3D 8, "
                    "and the scenario runs as with --engine-reference (nirender/noplugin.lua, eng-<step>.png). MGE XE's fall-back line and the "
                    "plugin's refusal do not stop the run; the run fails with exit code 4 when mgeXE.log has neither the fall-back line nor "
                    "the close after a refusal. A run that ends in that close (the line and the game's exit code 12) has no DONE and exits with 0. "
                    "Without --plugin-dll the run has no plugin, as a reference run")
    ap.add_argument("--reference-plugin", action="store_true", help="with --reference or --engine-reference: keep akulakhan.dll in the run, its ports "
                    "installed in mode off and its recorder on the device, as every reference run before 2026-11-03. Without this a reference "
                    "run has no plugin: the run's mod has no DLL, the mods of the instance with one are off, the scenario runs with "
                    "nirender/noplugin.lua, and the run fails when the game loaded the DLL. Only to repeat an old reference run as it was, "
                    "or to test the plugin's install on Direct3D 9")
    ap.add_argument("--instance", help="the folder of another Mod Organizer instance to run in, and not the testing instance")
    ap.add_argument("--game-dir", help="with --instance: the game folder of that instance")
    ap.add_argument("--profile", default="Default", help="with --instance: its profile")
    ap.add_argument("--position", help="x,y,z in the exterior cell: the player goes there after the cell loads (the "
                    "coordinates of the console). A value that starts with a minus needs the form with =, "
                    "--position=-57717,-416069,1198: without it the value reads as an option")
    ap.add_argument("--hour", type=float, default=None, help="the hour of the day in the exterior (12 without this)")
    ap.add_argument("--weather", type=int, default=None, help="the weather in the exterior, 0 clear to 9 blizzard (the game's without this)")
    ap.add_argument("--disable-mod", action="append", default=[], help="a mod of the profile switched off in its modlist.txt for the run, and back after; again for more mods")
    ap.add_argument("--enable-mod", action="append", default=[], metavar="NAME",
                    help="a mod of the instance (a folder under its mods) switched on, first in the mod list after the runner's mods, for the "
                    "run, and back after; again for more mods. The files of the runner's mods (--add-file, --mwse-dll) win over it. It "
                    "brings the files of the mod, not its plugins: see --enable-plugin")
    ap.add_argument("--enable-plugin", action="append", default=[], metavar="FILE.esp",
                    help="a plugin made active for the run: one GameFile line more in the profile's Morrowind.ini, and the file as it was after; "
                    "again for more plugins. The file must be in a mod of --enable-mod or in the game's Data Files")
    ap.add_argument("--find-water", action="store_true", help="after the cell loads, put the player under the water at the "
                    "nearest spot where the land is deep enough (a test of the view from under the water)")
    ap.add_argument("--find-dry", action="store_true", help="after the interior cell loads, put the player on the nearest floor above "
                    "the water level of the cell, looking back at the entry (a test of the water of an interior)")
    ap.add_argument("--yaw", type=float, default=None, help="the heading of the player in degrees, 0 north; "
                    "a negative heading as --yaw=-129 is the safe form")
    ap.add_argument("--lua-file", help="a file of Lua that the game runs a little after the player arrives at the interior and at the "
                    "exterior: for a state that a mod makes from Lua (the variables of a post shader)")
    ap.add_argument("--shader-batch", help="a file of Lua that gives a list of sets of post shaders, { name = \"lens\", { \"Lens Effects\", "
                    "variable = value } }: the step `shaders` takes one picture for each set (sh-<name>), with one start of the game")
    ap.add_argument("--set", action="append", default=[], help="a line of mgeXE.toml for the run, key = value: the line with "
                    "that key is replaced (for example \"expanded_light_limit = true\"); with a section first, "
                    "\"[distant_land] key = value\", a key the file does not have goes at the end of that section, and a "
                    "section the file does not have is added at the end of the run's copy (the instance's file is not changed); "
                    "--check tests each line against the file")
    ap.add_argument("--tour-cells", help="the cells of a test level for the step tour, with semicolons (the NIF test level: "
                    "\"@nifTest_1;@nifTest_2;@nifTest_3\")")
    ap.add_argument("--tour", default="@nif_", help="the start of the ids of the objects of the tour")
    ap.add_argument("--tour-extra", action="store_true", help="the test NIFs of the flag support (tools/make_test_nifs.py) placed "
                    "in the first cell of the tour, as stops")
    ap.add_argument("--tour-seconds", type=float, default=1.5, help="the time at each object of the tour")
    ap.add_argument("--tour-dual", action="store_true", help="at each stop of the tour, two frames in mode gpu and two in mode off, "
                    "with the time of the game held between them (tools/validate_tour.py compares the pair)")
    ap.add_argument("--steps", help="the steps of the scenario to do, with commas (for example exterior,rate,capture); "
                    "all of them without this")
    ap.add_argument("--capture-menu", type=int, default=0, help="frames of a capture file that starts at the main menu and holds the load of the "
                    "save (menu.nircap): the loading screen is a screen polygon")
    ap.add_argument("--budget", type=int, default=200)
    ap.add_argument("--timing", action="store_true", help="the run measures time (a shader batch that times frames): the slot check "
                    "refuses it unless this session holds the game slot with slot.py take game|both ... --timing (D297). The steps rate "
                    "and walk, and a run with no --steps, are timing runs without this")
    ap.add_argument("--full-dump", action="store_true", help="MWSE writes a full-memory dump at a crash (its setting CreateFullMinidumps, set only in "
                    "the run's copy of MWSE.json): about " + FULL_DUMP_SIZE + " in the run folder, so that the objects and the heap of a crash can be read")
    ap.add_argument("--settle", type=float, default=None, help="the seconds to wait at the interior and at the exterior before the picture (6 without this)")
    ap.add_argument("--keep-fade", action="store_true", help="keep the fade-in of the distant statics and the grass (the tunable statics.fade); "
                    "without this the mod sets it to 0 at the start, so that a picture does not depend on the moment of an arrival")
    ap.add_argument("--keep-ai", action="store_true", help="keep the AI of the actors on; without this the mod turns it off after the load, so "
                    "that two pictures of a place show the same thing (a measurement of the game's cost needs the AI on)")
    ap.add_argument("--port-times", action="store_true", help="the mod reports the port times at each step (it turns the timing of every live port call on: not for a profile)")
    ap.add_argument("--overlay", action="store_true", help="turn the debug overlay of the renderer on at the exterior step (it is on the picture of the "
                    "window only, not in the picture files)")
    ap.add_argument("--exe", help="a Morrowind.exe to put in the game folder for the run, and the one of the game back after: "
                    "for example the exe with other options of the Morrowind Code Patch")
    ap.add_argument("--early-picture", action="store_true", help="the test mod asks for a picture (mge.saveScreenshot) when its main.lua "
                    "runs, before the game has a renderer: the file early.png of the run must come with the first frames")
    ap.add_argument("--after-load-pictures", action="store_true", help="the test mod takes a picture at the event loaded of the save, then one every "
                    "0.1 s (after-load-NN.png): the first frames after a game load")
    ap.add_argument("--window-shots", action="store_true", help="each picture of the host program also as the window shows it, "
                    "<name>-window.png: with the status text of MGE XE and the debug overlay (wgpu only)")
    ap.add_argument("--bridge", action="store_true", help="the Lua bridge of mwse-mcp (the mod mwseMCP of the testing instance) for this run only, "
                    "in the mod of the run's mgeXE.toml: live work in another instance without a change to its mods")
    ap.add_argument("--add-file", action="append", default=[], metavar="SOURCE=PATH",
                    help="a file for the run's mod at PATH (from the Data Files folder, or Root/... for the game folder), for this run only: "
                    "a test texture or mesh; the run's mod wins over the instance's mods")
    ap.add_argument("--mwse-dll", help="another MWSE.dll for this run only (as --add-file <it>=Root/MWSE.dll): an A/B of an MWSE build "
                    "without a change to the instance")
    ap.add_argument("--env", action="append", default=[], metavar="NAME=VALUE",
                    help="a variable for the game's process (the plugin, the helper and MGE XE read it), for this run only; "
                    "again for more. It wins over the runner's default box time (MGE_XE_BOX_SECONDS=5). The variables "
                    "of the run itself are refused, with the option that sets each: " + ", ".join(sorted(RUNNER_VARIABLES)))
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--archive", help="the release check: a folder with an extracted release archive. The run takes its build, its plugin, its "
                    "MWSE.dll and its Lua files; a Lua file that the archive loads and lacks is not in the run, as in a player's game")
    args = ap.parse_args()
    archive_arguments(args)
    if args.self_test:
        self_test()
        return 0
    problems = argument_problems(args)
    if problems:
        raise SystemExit("run_own_device: the command has faults; nothing started:\n  " + "\n  ".join(problems))
    if args.check:
        if args.instance:
            session.use_instance(args.instance, args.game_dir, args.profile)
        problem = save_problem(args.save, saves_folder(session.harness.PROFILE, session.harness.GAME_DIR))
        no_plugin_run = ((args.reference or args.engine_reference) and not args.reference_plugin) or (args.plain_case and not args.plugin_dll)
        if not problem and no_plugin_run:
            problem = overwrite_problem(session.harness.MO2_ROOT, session.harness.PLUGIN_FILE)
        session.harness.refuse_stale_backups()
        enable_faults, enable_notes = enable_problems(args, session.harness.MO2_ROOT, session.harness.GAME_DIR, read_profile_ini())
        if not problem and enable_faults:
            problem = "the command has faults; nothing started:\n  " + "\n  ".join(enable_faults)
        if not problem:
            problem = config_problem(args, session.harness.MO2_ROOT)
        registry, source = registry_shadows()
        shadows, level = shadow_level_line(registry, ini_shadows(read_profile_ini()), source)
        if not problem:
            problem = shadow_problem(args, level, shadows)
        if problem:
            raise SystemExit("run_own_device: " + problem)
        print("run_own_device: the arguments are good; nothing started (--check)")
        print("run_own_device: " + shadows)
        for note in enable_notes:
            print("run_own_device: " + note)
        if weather_warning(args):
            print("run_own_device: " + weather_warning(args))
        if weather_pin_warning(args):
            print("run_own_device: " + weather_pin_warning(args))
        if full_dump_note(args):
            print("run_own_device: " + full_dump_note(args))
        if is_timing_run(args):
            print("run_own_device: a timing run: the game slot must be held with --timing (D297)")
        return 0
    if args.engine_reference:
        args.reference = True
    # A reference run has no code of the plugin in the game, unless --reference-plugin. A --plain-case run has the
    # plugin only when --plugin-dll names one (the cases of the plugin).
    no_plugin = (args.reference and not args.reference_plugin) or (args.plain_case and not args.plugin_dll)
    # The game is started from this process: its variables go into the game's environment.
    apply_env(os.environ, args.env)
    if args.log:
        # The runner's own lines go to the console and to the file; the file is the one a
        # shell redirect wrote before.
        log = open(args.log, "w", encoding="utf-8", buffering=1)
        sys.stdout = Tee(sys.stdout, log)
        sys.stderr = Tee(sys.stderr, log)
    session.DLL, source = plugin_dll_of(args.plugin_dll, os.environ, session.DLL)
    if no_plugin:
        print("plugin DLL: none, a reference run without the plugin (--reference-plugin keeps it)", flush=True)
    else:
        print("plugin DLL: %s (%s)" % (session.DLL, source), flush=True)
    if weather_warning(args):
        print("run_own_device: " + weather_warning(args), flush=True)
    if weather_pin_warning(args):
        print("run_own_device: " + weather_pin_warning(args), flush=True)
    extra_files = [entry.split("=", 1) for entry in args.add_file]
    if args.mwse_dll:
        extra_files.append((args.mwse_dll, "Root/MWSE.dll"))

    if args.instance:
        session.use_instance(args.instance, args.game_dir, args.profile)
    problem = save_problem(args.save, saves_folder(session.harness.PROFILE, session.harness.GAME_DIR))
    if problem:
        raise SystemExit("run_own_device: " + problem)
    session.harness.refuse_stale_backups()
    enable_faults, enable_notes = enable_problems(args, session.harness.MO2_ROOT, session.harness.GAME_DIR, read_profile_ini())
    if enable_faults:
        raise SystemExit("run_own_device: the command has faults; nothing started:\n  " + "\n  ".join(enable_faults))
    for note in enable_notes:
        print("run_own_device: " + note, flush=True)
    registry, source = registry_shadows()
    shadows, level = shadow_level_line(registry, ini_shadows(read_profile_ini()), source)
    print("run_own_device: " + shadows, flush=True)
    problem = shadow_problem(args, level, shadows)
    if problem:
        raise SystemExit("run_own_device: " + problem)
    if not args.no_build:
        cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
        for folder in (HOST_DIR, D3D8_DIR):
            subprocess.run([cargo, "build", "--release", "--quiet"], cwd=folder, check=True)
    problem = no_plugin and overwrite_problem(session.harness.MO2_ROOT, session.harness.PLUGIN_FILE)
    if problem:
        raise SystemExit("run_own_device: " + problem)
    session.prepare(build=not args.no_build, extra_files=extra_files, archive=args.archive, plugin=not no_plugin,
                    timing=is_timing_run(args))
    if no_plugin:
        for name in session.plugin_mods(session.harness.read_modlist()[0]):
            if name not in args.disable_mod:
                args.disable_mod.append(name)
                print("  mod of the instance with akulakhan.dll, off for the run: %s" % name, flush=True)
    run = session.Run(args.label)
    run.wgpu = not args.reference and not args.plain_case
    # Without a save (--save ""), the scenario starts a new game: the films of the game play.
    spec = run.spec("owndevice", save=args.save or None, interior=args.interior, exterior=args.exterior, dir=run.dir.replace("\\", "/"), steps=args.steps,
                    position=args.position, yaw=args.yaw, findWater=args.find_water, findDry=args.find_dry, hour=args.hour, weather=args.weather,
                    lua=open(args.lua_file, encoding="utf-8").read() if args.lua_file else None,
                    shaderBatch=open(args.shader_batch, encoding="utf-8").read() if args.shader_batch else None,
                    tourCells=args.tour_cells, tour=args.tour, tourSeconds=args.tour_seconds, tourDual=args.tour_dual or None, tourExtra=args.tour_extra or None, captureMenu=args.capture_menu, settleSeconds=args.settle, portTimes=args.port_times or None,
                    overlay=args.overlay or None, earlyPicture=run.path("early.png").replace("\\", "/") if args.early_picture else None,
                    afterLoadPictures=args.after_load_pictures or None, keepFade=args.keep_fade or None, keepAI=args.keep_ai or None,
                    engineReference=(args.engine_reference or args.plain_case) or None, noPlugin=no_plugin or None)
    # The plugin starts the host program. It reads these from the environment of the game.
    if not args.mge_build:
        os.environ["NIRENDER_HOST_EXE"] = HOST
    os.environ["NIRENDER_HOST_LOG"] = run.path("host.txt")
    os.environ["NIRENDER_BACKEND"] = args.backend
    # Pictures of the start of the game, before the scenario can ask for one.
    # Pictures of the start of the game, before the scenario can ask for one, and of a frame of
    # a film (the 120th: a film starts from black): a film holds the main loop, so the scenario
    # cannot ask for it.
    os.environ["NIRENDER_HOST_ARGS"] = '--shots-at %s --shots-prefix "%s" --shot-film "%s"' % (args.start_frames, run.path("start"), run.path("host-film-frame.png"))
    if args.window_shots:
        os.environ["NIRENDER_HOST_ARGS"] += " --window-shots"
    # The mgeXE.toml of the run: no frame limit (vsync off), so that the frame rates of the two
    # modes compare; with the switch for the wgpu renderer when the run is with it.
    toml = os.path.join(session.harness.MO2_ROOT, "mods", args.config_mod, "Root", "mgeXE.toml")
    text = open(toml, encoding="utf-8").read()
    lines = session.run_toml_lines(text, run_settings(args), toml, wgpu_switch(args))
    switched = run.path("mgeXE.toml")
    open(switched, "w", encoding="utf-8").write("".join(line + chr(10) for line in lines))
    # The files of the run are in mods of the harness, on for the run over the mods of the
    # instance; Root Builder copies their Root folders into the game folder.
    mods = [(MGE_MOD, {"Root/d3d8.dll": D3D8}), (CONFIG_MOD, {"Root/mgeXE.toml": switched})]
    if args.mge_build:
        # MGE XE with its switch for the wgpu renderer on: its own d3d8.dll gives Direct3D to
        # the plugin, and its helper is the renderer. With --reference the same build with the
        # switch off: Direct3D 9 with the d3d8.dll and the helper of that build (the helper as
        # the IPC host of distant land).
        mods[0] = (MGE_MOD, session.mge_build_files(args.mge_build))
        for line in add_symbols(args.mge_build, None if no_plugin else session.DLL):
            print(line, flush=True)
    elif args.reference:
        # MGE XE of the instance on Direct3D 9.
        mods.pop(0)
    if args.exe:
        mods.append((EXE_MOD, {"Root/Morrowind.exe": args.exe}))
    if args.bridge:
        bridge = os.path.join(BRIDGE_DIR, "MWSE", "mods", "mwseMCP")
        files = next(files for name, files in mods if name == CONFIG_MOD)
        for name in os.listdir(bridge):
            files["MWSE/mods/mwseMCP/" + name] = os.path.join(bridge, name)
    with PluginWatch(path=run.path("plugin-watch.txt")) as watch:
        done = launch_run(run, spec, args, mods, facts=[shadows])

    print()
    progress = run.progress_lines(TAG)
    for line in progress:
        if line.startswith(("own device", "step", "after", "steps done", "ERROR", "save loaded", "rate", "zoom")):
            print("game: " + line[:200])
    print("weather: " + weather_summary(args.weather, progress))
    with open(run.path("weather-of-the-run.txt"), "w", encoding="utf-8") as note:
        note.write(weather_note(args.weather, progress))
    if os.path.isfile(run.path("host.txt")):
        print("\nhost:")
        print("  " + open(run.path("host.txt"), errors="replace").read().strip().replace("\n", "\n  ")[-3000:])
    else:
        print("\nhost: no text; the host program did not start")
    # The plugin's lines: in mgeXE.log with MGE XE's d3d8.dll. Without its export the plugin
    # writes into nirender.log of the game folder until Lua names the log of the run.
    lines = run.plugin_lines()
    game_log = os.path.join(session.harness.GAME_DIR, "nirender.log")
    if os.path.isfile(game_log) and os.path.getmtime(game_log) > run.started:
        lines = open(game_log, encoding="utf-8", errors="replace").read().splitlines() + lines
    for line in lines:
        if line.startswith(("own device", "stream:", "capture: end", "own host", "install:", "link:")):
            print("plugin: " + line.strip()[:240])
    if args.engine_reference or args.plain_case:
        pngs_from_tgas(run.dir)
    # Which plugin the game loaded, from each proof. A run without the plugin that loaded it
    # is not a reference: it fails.
    evidence, loaded = plugin_evidence(run.dir, watch.seen, watch.checks)
    print("\nplugin of the run (%s):" % ("none asked" if no_plugin else "akulakhan.dll asked"))
    for line in evidence:
        print("  " + line)
    with open(run.path("plugin-of-the-run.txt"), "w", encoding="utf-8") as proof:
        proof.write("plugin of the run (%s):\n" % ("none asked" if no_plugin else "akulakhan.dll asked"))
        proof.write("".join("  %s\n" % line for line in evidence))
    failed = no_plugin and loaded
    if failed:
        print("ERROR: the game loaded akulakhan.dll in a reference run without the plugin: the run is no reference")
    fell_back = fallback_of_run(run.wgpu, run.fell_back, progress)
    if fell_back:
        print("ERROR: " + fell_back)
    closed = False
    if args.plain_case:
        mge_log = run.path("mgeXE.log")
        case = plain_case_line(open(mge_log, encoding="utf-8", errors="replace").read().splitlines() if os.path.isfile(mge_log) else [])
        closed = plain_case_closed(case, getattr(session.harness, "LAST_END", {}))
        if case:
            print("plain case: " + case)
        else:
            fell_back = "a --plain-case run without MGE XE's line of the case in mgeXE.log: the renderer was in use"
            print("ERROR: " + fell_back)
    ending = report_ending(run, done, progress)
    print("pictures in " + run.dir)
    # The size of the run and of every run kept (the owner, 2026-10-18: the results folder
    # had grown to hundreds of GB unnoticed). The runner deletes nothing; the line says when
    # it is time to.
    print("size: this run %s; all runs in %s %s" % (folder_size(run.dir), os.path.dirname(run.dir), folder_size(os.path.dirname(run.dir))))
    print_disk_limits()
    if closed:
        print("plain case: the close after the refusal, as designed; the run passes without DONE")
    return run_exit_code(no_plugin, loaded, done or closed, ending, bool(fell_back))


# The endings of a run (W372): DONE came; the game closed by itself without DONE, after a step started, with exit code
# 0 and no dump, which is a close from outside (taskkill without /F, a close by the owner or the Coordinator) or the
# game's own exit (a mod's os.exit(0)): the runner cannot tell them apart; or any other end, the crash path among
# them. A code 0 before the first step stays on the old path: MWSE's check of MGE XE exits with 0 at the start.
ENDED_DONE, ENDED_BY_HAND, ENDED_OTHER = "done", "by hand", "other"
EXIT_ENDED_BY_HAND = 3
# A wgpu run without the renderer: MGE XE fell back to the plain game (the fall-back guard, D328, verdict 314; W462).
# A --plain-case run without MGE XE's fall-back line fails with it too.
EXIT_FELL_BACK = 4
# The line of the nirender mod (owndevice.lua) in a run whose game has no device of the plugin.
NO_OWN_DEVICE = "own device: false"
STEP_LINE = re.compile(r"^step (\S+?):")


def fallback_of_run(wgpu, stopped, progress):
    """Why a wgpu run failed without the renderer, or None. `stopped` is the fall-back line that stopped the run
    (`session.Run.fell_back`); without it, the progress line "own device: false" shows a fall-back that the line did
    not show. A reference run and a --plain-case run (`wgpu` False) have no own device by design."""
    if not wgpu:
        return None
    if stopped:
        return "a wgpu run without the renderer: " + stopped
    if any(line.startswith(NO_OWN_DEVICE) for line in progress):
        return "a wgpu run without the renderer: the progress says \"%s\"" % NO_OWN_DEVICE
    return None


# The lines of MGE XE (render-link d3d8/cpp/mge/session.cpp) that a --plain-case run must show: the fall-back to the
# plain game, or the close after the plugin's refusal with its detours in (the exit code 12, GAME_EXIT_PLUGIN_REFUSED).
PLAIN_CASE_MARKS = ("!! wgpu renderer: not in use,", "!! wgpu renderer: the plugin refused after it changed the game's code")


def plain_case_line(lines):
    """MGE XE's line of the case of a --plain-case run (`PLAIN_CASE_MARKS`), stripped, or None."""
    return next((line.strip() for line in lines if any(mark in line for mark in PLAIN_CASE_MARKS)), None)


def plain_case_closed(case, end):
    """True when a --plain-case run ended in the close after the plugin's refusal: MGE XE's close line (`case`) and
    the game's exit code 12 in the harness's `end`. The game closes before DONE; the run passes."""
    return bool(case) and PLAIN_CASE_MARKS[1] in case and plugin_refused_line(end) is not None
SHADER_SET_LINE = re.compile(r"^shader batch: (\S+?): ")


def run_ending(done, end, crash_files, step):
    """The ending of a run, from the harness's LAST_END, the dumps of the run folder and the last step that started
    (`last_step`). A hand end needs the game's own close ("exited") with every exit code read and 0, no dump, and a
    step that started: an abnormal code, a code not read or a dump is the crash path, and an exit before the first
    step stays 1 (verdict 235)."""
    if done:
        return ENDED_DONE
    codes = end.get("codes") or []
    if step and end.get("reason") == "exited" and codes and all(code == 0 for code in codes) and not crash_files:
        return ENDED_BY_HAND
    return ENDED_OTHER


# The game's exit code when MGE XE closes it after the plugin refused with its detours in (W462; src/stream_format.rs
# EXIT_PLUGIN_REFUSED).
GAME_EXIT_PLUGIN_REFUSED = 12


def plugin_refused_line(end):
    """The line for a game that MGE XE closed because the plugin refused after it changed the game's code, or None."""
    if end.get("reason") == "exited" and GAME_EXIT_PLUGIN_REFUSED in (end.get("codes") or []):
        return ("the game closed with exit code %d: the plugin refused the renderer after it changed the game's code, "
                "and MGE XE closed the game (see the \"box:\" line of mgeXE.log)" % GAME_EXIT_PLUGIN_REFUSED)
    return None


def report_ending(run, done, progress):
    """The ending of `run` from the harness's LAST_END (absent in a harness from before W372: the crash path, as before)
    and the dumps that the harness moved into the run folder; prints the line of a hand end or of the plugin's refusal
    close. Returns the ending."""
    dumps = [path for path in (run.path("MWSE_MiniDump.dmp"), run.path(session.harness.DUMPS_FILE)) if os.path.isfile(path)]
    step = last_step(progress)
    end = getattr(session.harness, "LAST_END", {})
    ending = run_ending(done, end, dumps, step)
    if ending == ENDED_BY_HAND:
        print("ended without DONE after step %s, with exit code 0 (a close from outside, or the game's own exit)" % step)
    refused = plugin_refused_line(end)
    if refused:
        print(refused)
    return ending


def last_step(progress):
    """The last step that the progress lines show started: a "step <name>:" line, or the start of a set of a shader
    batch ("shader batch: <set>: ...", the step sh-<set>). None before the first."""
    found = None
    for line in progress:
        step = STEP_LINE.match(line)
        shader_set = SHADER_SET_LINE.match(line)
        if step:
            found = step.group(1)
        elif shader_set:
            found = "sh-" + shader_set.group(1)
    return found


def run_exit_code(no_plugin, loaded, done, ending=ENDED_OTHER, fell_back=False):
    """2 for a run without the plugin that loaded it (no reference), 4 (EXIT_FELL_BACK) for a wgpu
    run that MGE XE took to the plain game, also with DONE, else 0 when the steps were done in the
    budget, 3 for a run ended by hand (W372), 1 for any other end. The 4 of check_mwse_sites.py and
    deploy_rest.py (a missing MWSE feature) is another tool's code: nothing reads both."""
    if no_plugin and loaded:
        return 2
    if fell_back:
        return EXIT_FELL_BACK
    if done:
        return 0
    return EXIT_ENDED_BY_HAND if ending == ENDED_BY_HAND else 1


def overwrite_problem(root, plugin_file):
    """The overwrite folder of the Mod Organizer instance at `root` serves its files over every
    mod: with the plugin there, a run without the plugin cannot keep it out."""
    if os.path.isfile(os.path.join(root, "overwrite", plugin_file)):
        return "the overwrite folder of the instance has %s: a run without the plugin cannot keep it out" % plugin_file
    return None


def print_disk_limits():
    """The limits of the data rule (D71), from the slot tool beside the harness. A fault here never stops a run."""
    try:
        import slot

        for line in slot.runner_disk_lines():
            print(line)
    except Exception as error:  # noqa: BLE001 - only a line of a report
        print("the limits of the disks are not known: %s" % error)


def folder_size(path):
    """The size of a folder as text, counted file by file."""
    total = 0
    for root, _, files in os.walk(path):
        for name in files:
            try:
                total += os.path.getsize(os.path.join(root, name))
            except OSError:
                pass
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if total < 1024 or unit == "TB":
            return "%.1f %s" % (total, unit)
        total /= 1024.0
    return "?"


if __name__ == "__main__":
    sys.exit(main())
