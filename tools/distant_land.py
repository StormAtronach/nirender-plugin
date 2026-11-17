"""The distant land of a run: one Mod Organizer mod for each key (W521 step 4, Part 2).

MGE XE reads and writes the game's `Data Files\\distantland`, and Mod Organizer serves it from the mods (render-link
RENDERER.md, "Install with Mod Organizer 2"). So each distant land is a mod of the instance, `Distant land <key>`, with
the tool's marker file, and a run switches its own mod on and every other marked mod off. A build of another format
has another mod, so it never rebuilds the distant land of another build (trap T112).

The key has three parts:
- the format: 8 hex of the format text of the set's helper (`numidium.exe --distant-land-key`). A set whose manifest
  has `own-<render-link 8 hex>` (a helper from before the option) keeps that name, and shares no distant land;
- the settings: 8 hex of the identity that the generator makes of the run's generation settings. For an `own-` set,
  8 hex of the md5 of the run's --set lines of the [generation...] and [distant_land] sections, without
  automatic_rebuild; "base" when there are none;
- the content (rulings Q1): only for a run that lets the generator run with its own files. 8 hex of the blake2 of
  the runner's own list of such files (`run_own_device.load_changes`), with the md5 of each added file's source.
  A guarded run (`run_own_device.rebuild_guard`) has no content part: it reads the mod of its format and settings
  and writes nothing there.

Mod Organizer puts a file that the generator makes new into overwrite/distantland. So after each run that may write,
the files there move into the run's mod (the end move), and a start that finds files there moves them into the mod
of the run file's key, else into the resting mod (the start move). The run file `distant-land-run.txt` in the
profile folder holds the key of the run that has not ended. A guarded run moves nothing: a file in overwrite after it
is a fault, and so is a changed state file in its mod.

    python -I tools/distant_land.py --self-test
"""
import argparse
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

MOD_PREFIX = "Distant land "
MARKER = "nirender-distant-land.json"
RUN_FILE = "distant-land-run.txt"
DATA = "distantland"
VERSION_FILE = "version"
STATE_FILE = "generation_state.bin"
KEY_OPTION = "--distant-land-key"
OWN = "own-"
# The md5 of a state file that is there and cannot be read.
NOT_READABLE = "not readable"
# A section of mgeXE.toml whose lines are generation settings for the settings part of an `own-` key.
GENERATION_SECTION = re.compile(r"\s*\[\s*(generation(\.[^\]]*)?|distant_land)\s*\]")
REBUILD_KEY = re.compile(r"\s*(\[[^\]]*\]\s*)?automatic_rebuild\s*=")


class Problem(Exception):
    """A distant land that a run cannot use, or a state of the instance that a person must look at."""


class Key:
    """The key of a distant land: format, settings and content parts, and the format's text."""

    def __init__(self, form, form_text, settings, content=None):
        self.form, self.form_text, self.settings, self.content = form, form_text, settings, content

    @property
    def text(self):
        return "-".join(part for part in (self.form, self.settings, self.content) if part)

    @property
    def mod_name(self):
        return MOD_PREFIX + self.text

    def __eq__(self, other):
        return isinstance(other, Key) and (self.text, self.form_text) == (other.text, other.form_text)

    def __repr__(self):
        return "Key(%s)" % self.text


def helper_key(numidium, toml, run=subprocess.run):
    """(format 8 hex, format text, settings 8 hex) from `numidium.exe --distant-land-key --config <toml>`."""
    result = run([numidium, KEY_OPTION, "--config", toml], capture_output=True, text=True, timeout=60)
    lines = dict(line.split(" ", 1) for line in result.stdout.splitlines() if " " in line)
    if result.returncode != 0 or "format" not in lines or "settings" not in lines:
        raise Problem("%s %s --config %s gave no key (exit code %s): %s"
                      % (numidium, KEY_OPTION, toml, result.returncode, (result.stderr or result.stdout).strip()[-300:]))
    form, _, text = lines["format"].partition(" ")
    return form, text, lines["settings"].strip()


def own_settings(set_lines):
    """The settings part of an `own-` key: 8 hex of the md5 of the --set lines of the generation and distant land
    sections, sorted, without automatic_rebuild; "base" with none."""
    chosen = sorted(line.strip() for line in set_lines if GENERATION_SECTION.match(line) and not REBUILD_KEY.match(line.split("]", 1)[-1]))
    return hashlib.md5("\n".join(chosen).encode("utf-8")).hexdigest()[:8] if chosen else "base"


def md5_of(path):
    digest = hashlib.md5()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def content_part(changes, sources):
    """The content part (rulings Q1.3): 8 hex of the blake2 of the runner's list of load changes, sorted, each
    "--add-file <path>" with the md5 of its source (`sources`: {path in the mod: source file}). None with no change."""
    if not changes:
        return None
    entries = []
    for change in changes:
        option, _, path = change.partition(" ")
        entries.append("%s %s" % (change, md5_of(sources[path])) if option == "--add-file" else change)
    return hashlib.blake2b("\n".join(sorted(entries)).encode("utf-8"), digest_size=4).hexdigest()


def key_of(set_folder, manifest, toml, set_lines, content=None, run=subprocess.run):
    """The key of a run on the set at `set_folder` (its `manifest`) with the mgeXE.toml `toml`, made from the run's
    `set_lines`, and the content part or None."""
    form = manifest.get("distant_land_format") or OWN + manifest["commits"]["render-link"]["commit"][:8]
    if form.startswith(OWN):
        return Key(form, form, own_settings(set_lines), content)
    got, text, settings = helper_key(os.path.join(set_folder, "numidium.exe"), toml, run)
    if got != form or text != manifest.get("distant_land_format_text", text):
        raise Problem("the set's helper gives the format %s, its manifest names %s" % (got, form))
    return Key(form, text, settings, content)


def mods_folder(instance):
    return os.path.join(instance, "mods")


def overwrite_folder(instance):
    return os.path.join(instance, "overwrite", DATA)


def read_marker(folder):
    """The marker of the mod folder `folder`, or None when it has none or it does not read."""
    try:
        with open(os.path.join(folder, MARKER), encoding="utf-8") as file:
            marker = json.load(file)
    except (OSError, ValueError):
        return None
    return marker if isinstance(marker, dict) else None


def marked_mods(instance):
    """{mod name: marker} of the mods of `instance` that are the tool's: a real folder named MOD_PREFIX + key with a
    marker that names that key and this instance. Any other folder is never touched."""
    found = {}
    root = mods_folder(instance)
    for name in sorted(os.listdir(root)) if os.path.isdir(root) else ():
        folder = os.path.join(root, name)
        if not name.lower().startswith(MOD_PREFIX.lower()) or os.path.islink(folder) or not os.path.isdir(folder):
            continue
        marker = read_marker(folder)
        if marker and MOD_PREFIX + str(marker.get("key")) == name and os.path.normcase(os.path.abspath(str(marker.get("instance")))) == os.path.normcase(os.path.abspath(instance)):
            found[name] = marker
    return found


def ensure_mod(instance, key, commit, now=None):
    """The folder of the key's mod, made with its marker when it is not there. Returns (folder, made)."""
    folder = os.path.join(mods_folder(instance), key.mod_name)
    if os.path.isdir(folder):
        if key.mod_name not in marked_mods(instance):
            raise Problem("%s is there, but it is not a mod of this tool (no marker for this key and instance): it is not used" % folder)
        return folder, False
    os.makedirs(folder)
    marker = {"key": key.text, "format": key.form_text, "instance": os.path.abspath(instance),
              "made": (now or datetime.datetime.now().astimezone()).isoformat(timespec="seconds"), "tool": commit}
    with open(os.path.join(folder, MARKER), "w", encoding="utf-8") as file:
        json.dump(marker, file, indent=1)
    return folder, True


def has_data(folder):
    """Whether the mod at `folder` holds a distant land (its version file)."""
    return os.path.isfile(os.path.join(folder, DATA, VERSION_FILE))


def overwrite_files(instance):
    """The files of overwrite/distantland, relative and sorted."""
    root = overwrite_folder(instance)
    found = []
    for folder, _, names in os.walk(root):
        found += [os.path.relpath(os.path.join(folder, name), root).replace("\\", "/") for name in names]
    return sorted(found)


def move_overwrite(instance, mod):
    """Moves each file of overwrite/distantland into the mod folder `mod` (a file of the same path there is
    replaced), and removes the empty folders under overwrite/distantland. Returns the number of files."""
    root = overwrite_folder(instance)
    files = overwrite_files(instance)
    for relative in files:
        target = os.path.join(mod, DATA, relative)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        os.replace(os.path.join(root, relative), target)
    for folder, _, _ in sorted(os.walk(root), key=lambda entry: -len(entry[0])):
        if folder != root and not os.listdir(folder):
            os.rmdir(folder)
    return len(files)


def state_md5(mod):
    """The md5 of the generator's state file in the mod folder `mod`, None when it has none, NOT_READABLE."""
    path = os.path.join(mod, DATA, STATE_FILE)
    if not os.path.isfile(path):
        return None
    try:
        return md5_of(path)
    except OSError:
        return NOT_READABLE


def run_file(profile):
    return os.path.join(profile, RUN_FILE)


def write_run_file(profile, key, instance, read_only, commit, now=None):
    """The run file (rulings Q3): written before the mods are switched and before the game."""
    data = {"key": key.text, "mod": key.mod_name, "read_only": bool(read_only), "instance": os.path.abspath(instance),
            "time": (now or datetime.datetime.now().astimezone()).isoformat(timespec="seconds"), "tool": commit}
    with open(run_file(profile), "w", encoding="utf-8") as file:
        json.dump(data, file, indent=1)


def read_run_file(profile):
    try:
        with open(run_file(profile), encoding="utf-8") as file:
            return json.load(file)
    except FileNotFoundError:
        return None
    except (OSError, ValueError) as error:
        raise Problem("%s does not read (%s): look at it, then remove it" % (run_file(profile), error))


def resting_mod(instance, modlist):
    """The marked mod that is on in the profile's mod list (`modlist`: its lines), or None. More than one: a Problem."""
    marked = marked_mods(instance)
    on = [line[1:] for line in modlist if line.startswith("+") and line[1:] in marked]
    if len(on) > 1:
        raise Problem("more than one distant land mod is on in the mod list: %s" % ", ".join(on))
    return on[0] if on else None


def start(instance, profile, modlist):
    """The start move: what overwrite/distantland holds goes into the mod of the run file's key, else into the resting
    mod; then the run file goes. A read-only run file with files in overwrite, or files with no mod to take them,
    stop the start (a Problem with the files' names). Returns the lines to print."""
    files = overwrite_files(instance)
    left = read_run_file(profile)
    lines = []
    if files:
        named = ", ".join(files[:6]) + (" and %d more" % (len(files) - 6) if len(files) > 6 else "")
        if left and left.get("read_only"):
            raise Problem("overwrite/distantland of %s holds %d files after the read-only run of %s (%s), which did not end: "
                          "they were made from that run's files; remove them by hand (%s)" % (instance, len(files), left.get("mod"), left.get("time"), named))
        name = left.get("mod") if left else resting_mod(instance, modlist)
        if not name or name not in marked_mods(instance):
            raise Problem("overwrite/distantland of %s holds %d files, and no distant land mod takes them (%s)" % (instance, len(files), named))
        moved = move_overwrite(instance, os.path.join(mods_folder(instance), name))
        lines.append("distant land: %d files of overwrite moved into %s (%s)" % (moved, name, "the run that did not end" if left else "the resting mod"))
    if left:
        os.remove(run_file(profile))
        lines.append("distant land: the run file of %s (%s) removed" % (left.get("mod"), left.get("time")))
    return lines


def switch_plan(instance, modlist, own):
    """The marked mods to switch off for a run on the mod `own`: every marked mod that is on, except `own`."""
    return [line[1:] for line in modlist if line.startswith("+") and line[1:] in marked_mods(instance) and line[1:] != own]


def end(instance, profile, mod, read_only, before, move=True):
    """The run's end, after the game and Mod Organizer are gone and before the mod list is put back (C1, C2).
    A read-only run: overwrite/distantland must be empty and the state md5 of `mod` as `before`; nothing moves.
    A run that may write: the files of overwrite move into `mod`. Then the run file goes, unless the read-only check
    failed (the next start then names the files). Returns (lines, fault)."""
    files = overwrite_files(instance)
    after = state_md5(mod)
    if read_only:
        faults = []
        if files:
            faults.append("overwrite/distantland holds %d files after a read-only run: %s" % (len(files), ", ".join(files[:6])))
        if NOT_READABLE in (before, after):
            faults.append("the state file of %s could not be read before or after the run" % os.path.basename(mod))
        elif before != after:
            faults.append("the state file of %s changed (%s -> %s)" % (os.path.basename(mod), (before or "none")[:8], (after or "none")[:8]))
        if faults:
            return ["the run CHANGED THE DISTANT LAND: %s. The files stay where they are" % "; ".join(faults)], True
        os.remove(run_file(profile))
        return ["distant land: read-only, as before the run (%s)" % (before or "none")[:8]], False
    moved = move_overwrite(instance, mod) if move else 0
    os.remove(run_file(profile))
    return ["distant land: %d files of overwrite moved into %s; state %s -> %s" % (moved, os.path.basename(mod), (before or "none")[:8], (after or "none")[:8])], False


def set_modlist(instance, profile, own):
    """Puts the mod `own` on in the profile's modlist.txt and every other marked mod off; a line for `own` goes in under
    the comment lines when the list has none. Keeps the file's line ending. Returns the changes, as texts."""
    path = os.path.join(profile, "modlist.txt")
    with open(path, "rb") as file:
        data = file.read()
    ending = "\r\n" if b"\r\n" in data else "\n"
    marked = marked_mods(instance)
    lines, changes, found = [], [], False
    for line in data.decode("utf-8").splitlines():
        name = line[1:] if line[:1] in ("+", "-") else None
        if name == own:
            found = True
            if line != "+" + own:
                changes.append("on: " + own)
            lines.append("+" + own)
        elif name in marked and line.startswith("+"):
            changes.append("off: " + name)
            lines.append("-" + name)
        else:
            lines.append(line)
    if not found:
        at = next((index for index, line in enumerate(lines) if not line.startswith("#")), len(lines))
        lines.insert(at, "+" + own)
        changes.append("added, on: " + own)
    if changes:
        with open(path, "wb") as file:
            file.write((ending.join(lines) + ending).encode("utf-8"))
    return changes


def rest(instance, profile, key, commit, apply=True):
    """The resting distant land of an install after a deploy (deploy_rest.py): the key's mod (made with its marker when
    new) on in the mod list, every other marked mod off. Returns the lines; with `apply` False, what it would do."""
    folder = os.path.join(mods_folder(instance), key.mod_name)
    state = "with its distant land" if has_data(folder) else "new or empty: the first start of the game generates it"
    if not apply:
        return ["distant land of %s: would put %s on (%s)" % (instance, key.mod_name, state)]
    ensure_mod(instance, key, commit)
    changes = set_modlist(instance, profile, key.mod_name)
    return ["distant land of %s: %s on (%s); mod list: %s" % (instance, key.mod_name, state, "; ".join(changes) or "as it was")]


def adopt(instance, profile, key, commit, apply=False):
    """The one-time move (design 3.3): the distant land in overwrite/distantland of `instance` goes into the mod of `key`,
    which goes on in the mod list. A dry run by default. A Problem when overwrite is empty, when the key's mod holds a
    distant land already, or when a run file says that a run did not end."""
    if read_run_file(profile):
        raise Problem("%s is there: a run did not end; a start of the runner acts on it first" % run_file(profile))
    files = overwrite_files(instance)
    if not files:
        raise Problem("nothing to adopt: %s is empty" % overwrite_folder(instance))
    folder = os.path.join(mods_folder(instance), key.mod_name)
    if has_data(folder):
        raise Problem("%s holds a distant land already: nothing is moved" % folder)
    size = sum(os.path.getsize(os.path.join(overwrite_folder(instance), *name.split("/"))) for name in files)
    lines = ["adopt: %d files, %d bytes, from %s into %s" % (len(files), size, overwrite_folder(instance), os.path.join(folder, DATA))]
    if not apply:
        return lines + ["adopt: a dry run; nothing is moved (--apply moves the files and puts the mod on)"]
    ensure_mod(instance, key, commit)
    moved = move_overwrite(instance, folder)
    changes = set_modlist(instance, profile, key.mod_name)
    return lines + ["adopt: %d files moved; mod list: %s" % (moved, "; ".join(changes) or "as it was")]


def tool_commit():
    """The commit of this tool's tree, for the marker and the run file; "unknown" when git does not answer."""
    try:
        return subprocess.run(["git", "-C", HERE, "rev-parse", "HEAD"], capture_output=True, text=True, timeout=30).stdout.strip() or "unknown"
    except (OSError, subprocess.SubprocessError):
        return "unknown"


def generation_cause(mod):
    """The generator's own cause of its last generation in the mod folder `mod` (`rebuild_cause` of its
    generation_report.toml), or None."""
    try:
        with open(os.path.join(mod, DATA, "generation_report.toml"), encoding="utf-8", errors="replace") as file:
            found = re.search(r'^rebuild_cause\s*=\s*"(.*)"\s*$', file.read(), re.M)
    except OSError:
        return None
    return found.group(1) if found else None


def archive_key(archive, toml, set_lines, content=None, run=subprocess.run):
    """The key of a run on an unpacked release archive, which has no manifest: its helper's key when the helper has the
    option (the same mod as a set of that format), else a format of that folder's own. An old helper does not know
    the option and ends at once in its IPC host mode, which writes its log into the current folder: the call runs in a
    temporary folder."""
    with tempfile.TemporaryDirectory(prefix="distant-land-key-") as temp:
        try:
            form, text, settings = helper_key(os.path.join(archive, "numidium.exe"), toml,
                                              lambda command, **kinds: run(command, cwd=temp, **kinds))
            return Key(form, text, settings, content)
        except (Problem, OSError, subprocess.SubprocessError):
            pass
    form = "archive-" + hashlib.md5(os.path.normcase(os.path.abspath(archive)).encode("utf-8")).hexdigest()[:8]
    return Key(form, form, own_settings(set_lines), content)


class RunLand:
    """The distant land of one run on the instance at `instance` (profile folder `profile`): `prepare` before the
    game, `switch` for the mod list of the run, and `end` as the harness's step before the restore (C1, C2).
    `read_only` is the runner's rebuild guard: the run reads its mod and writes nothing (rulings Q1)."""

    def __init__(self, instance, profile, key, read_only, commit):
        self.instance, self.profile, self.key, self.read_only, self.commit = instance, profile, key, read_only, commit
        self.mod = os.path.join(mods_folder(instance), key.mod_name)
        self.before = None
        self.lines, self.fault, self.ended = [], False, False

    def prepare(self, modlist, timing=False):
        """The start move, the mod (made with its marker when new), the refusal of a timing run on a mod with no
        distant land, the run file, and the state before the game. Returns the lines for the run's head; a
        Problem stops the run before the game."""
        lines = start(self.instance, self.profile, modlist)
        _, made = ensure_mod(self.instance, self.key, self.commit)
        ready = has_data(self.mod)
        if timing and not ready:
            raise Problem("%s has no distant land yet, and a timing run does not make it: run a plain run of this set first" % self.key.mod_name)
        write_run_file(self.profile, self.key, self.instance, self.read_only, self.commit)
        self.before = state_md5(self.mod)
        state = "made now, empty" if made else "with its distant land" if ready else "with no distant land yet"
        lines.append("distant land: the mod %s (%s)" % (self.key.mod_name, state))
        if self.read_only:
            lines.append("distant land: the run changes what the game loads; the distant land is the mod %s, made without "
                         "the run's files, and the run writes nothing there" % self.key.mod_name)
        return lines

    def switch(self, modlist):
        """(the marked mods to switch off for the run, the run's own mod to switch on)."""
        return switch_plan(self.instance, modlist, self.key.mod_name), self.key.mod_name

    def end(self):
        """The step before the restore: the check of a read-only run, or the move of a run that may write."""
        self.lines, self.fault = end(self.instance, self.profile, self.mod, self.read_only, self.before)
        if self.fault and generation_cause(self.mod):
            self.lines.append("the generator's cause: %s" % generation_cause(self.mod))
        self.ended = True

    def result(self):
        """(the lines for the run's end, whether it is a fault). A run whose step did not run is a fault: its run file
        stays, and the next start acts on it."""
        if not self.ended:
            return ["distant land: not checked, the step before the restore did not run; the run file %s stays for the next start"
                    % run_file(self.profile)], True
        return self.lines, self.fault


def self_test():
    import contextlib
    import io
    import tempfile

    with tempfile.TemporaryDirectory() as temp:
        instance = os.path.join(temp, "Instance")
        profile = os.path.join(instance, "profiles", "Default")
        os.makedirs(profile)
        os.makedirs(os.path.join(mods_folder(instance), "Distant Lights OAAB patched and improved"))
        os.makedirs(os.path.join(mods_folder(instance), "Distant land byhand"))

        def plant(relative, text="x"):
            path = os.path.join(overwrite_folder(instance), relative)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as file:
                file.write(text)

        # The key of an old helper is own-, with no helper call; its settings part follows the generation lines only.
        manifest = {"distant_land_format": "own-5296d863", "commits": {"render-link": {"commit": "5296d863aaaa", "clean": True}}}
        never = lambda *a, **k: 1 / 0  # noqa: E731
        plain = key_of("S", manifest, "t.toml", ["[views] full_tier_max = 4", "[distant_land] automatic_rebuild = false"], run=never)
        assert plain.text == "own-5296d863-base" and plain.mod_name == "Distant land own-5296d863-base", plain
        # A set made before the manifest named its format: own- with the render-link commit of its manifest.
        older = {"commits": manifest["commits"]}
        assert key_of("S", older, "t.toml", [], run=never) == plain
        tuned = key_of("S", manifest, "t.toml", ["[distant_land] far = 5"], run=never)
        assert tuned.settings != "base" and tuned.text != plain.text
        # The null of the old helper: a manifest with a format asks the helper, and takes its two lines.
        helper = lambda command, **k: subprocess.CompletedProcess(command, 0, "format abcd1234 A=1;B=2\nsettings 99887766\n", "")  # noqa: E731
        new = key_of("S", {"distant_land_format": "abcd1234", "distant_land_format_text": "A=1;B=2", "commits": {}}, "t.toml", [], run=helper)
        assert new.text == "abcd1234-99887766" and new.form_text == "A=1;B=2", new
        try:
            key_of("S", {"distant_land_format": "ffff0000", "commits": {}}, "t.toml", [], run=helper)
            raise AssertionError("a helper of another format was taken")
        except Problem:
            pass

        # The content part: none for no change; the source's bytes count.
        source = os.path.join(temp, "a.nif")
        with open(source, "w") as file:
            file.write("one")
        assert content_part([], {}) is None
        first = content_part(["--add-file meshes/a.nif", "--enable-mod Trees"], {"meshes/a.nif": source})
        assert first == content_part(["--enable-mod Trees", "--add-file meshes/a.nif"], {"meshes/a.nif": source})
        with open(source, "w") as file:
            file.write("two")
        assert content_part(["--add-file meshes/a.nif", "--enable-mod Trees"], {"meshes/a.nif": source}) != first

        # T-N1a: the switch touches only marked mods; the folder named by hand and the "Distant Lights" mod stay.
        mod, made = ensure_mod(instance, plain, "c0ffee")
        assert made and ensure_mod(instance, plain, "c0ffee") == (mod, False)
        other, _ = ensure_mod(instance, tuned, "c0ffee")
        modlist = ["+" + plain.mod_name, "+Distant Lights OAAB patched and improved", "+Distant land byhand", "-" + tuned.mod_name]
        assert sorted(marked_mods(instance)) == sorted([plain.mod_name, tuned.mod_name])
        assert switch_plan(instance, modlist, tuned.mod_name) == [plain.mod_name]
        assert resting_mod(instance, modlist) == plain.mod_name
        try:
            ensure_mod(instance, Key("byhand", "byhand", ""), "c0ffee")
            raise AssertionError("a folder with no marker was taken")
        except Problem:
            pass
        # The null of T-N1a: with a marker, the folder named by hand is the tool's and goes off.
        with open(os.path.join(mods_folder(instance), "Distant land byhand", MARKER), "w", encoding="utf-8") as file:
            json.dump({"key": "byhand", "instance": instance}, file)
        assert switch_plan(instance, modlist, tuned.mod_name) == [plain.mod_name, "Distant land byhand"]
        os.remove(os.path.join(mods_folder(instance), "Distant land byhand", MARKER))

        # T-N1b and C1 (T-N3a): a run that may write. A new file and a file of the same path go into the run's mod;
        # overwrite ends empty, and the run file is gone. The null: the end with the move left out leaves both.
        os.makedirs(os.path.join(other, DATA))
        with open(os.path.join(other, DATA, VERSION_FILE), "w") as file:
            file.write("old")
        for move in (False, True):
            plant("version", "new")
            plant("statics/usage.data")
            write_run_file(profile, tuned, instance, False, "c0ffee")
            lines, fault = end(instance, profile, other, False, None, move=move)
            assert not fault
            if move:
                assert overwrite_files(instance) == [] and not os.path.exists(run_file(profile)), lines
                assert open(os.path.join(other, DATA, VERSION_FILE)).read() == "new" and has_data(other)
                assert os.path.isfile(os.path.join(other, DATA, "statics", "usage.data"))
            else:
                assert overwrite_files(instance) == ["statics/usage.data", "version"]
        assert os.path.isdir(overwrite_folder(instance)) and os.listdir(overwrite_folder(instance)) == []

        # C2 (T-N3b): a read-only run with a file in overwrite fails, and the file stays; the run file stays too.
        # The null: overwrite empty and the state md5 as before passes.
        write_run_file(profile, plain, instance, True, "c0ffee")
        lines, fault = end(instance, profile, mod, True, state_md5(mod))
        assert not fault and not os.path.exists(run_file(profile)), lines
        write_run_file(profile, plain, instance, True, "c0ffee")
        plant("lights.data")
        lines, fault = end(instance, profile, mod, True, state_md5(mod))
        assert fault and "lights.data" in lines[0] and overwrite_files(instance) == ["lights.data"] and os.path.exists(run_file(profile)), lines

        # T-N3c: a start after that read-only run stops and names the file. The null: with overwrite empty, it starts
        # and removes the run file.
        try:
            start(instance, profile, modlist)
            raise AssertionError("a start took the files of a read-only run")
        except Problem as problem:
            assert "lights.data" in str(problem)
        os.remove(os.path.join(overwrite_folder(instance), "lights.data"))
        assert start(instance, profile, modlist) and not os.path.exists(run_file(profile))

        # The start move: with no run file, files go into the resting mod; with a run file that may write, into its
        # mod. Files with no mod to take them stop the start.
        plant("terrain.bin")
        with contextlib.redirect_stdout(io.StringIO()):
            start(instance, profile, modlist)
        assert os.path.isfile(os.path.join(mod, DATA, "terrain.bin")) and overwrite_files(instance) == []
        plant("terrain.bin")
        write_run_file(profile, tuned, instance, False, "c0ffee")
        start(instance, profile, modlist)
        assert os.path.isfile(os.path.join(other, DATA, "terrain.bin")) and not os.path.exists(run_file(profile))
        plant("terrain.bin")
        try:
            start(instance, profile, ["-" + plain.mod_name])
            raise AssertionError("files with no mod were taken")
        except Problem:
            pass
        os.remove(os.path.join(overwrite_folder(instance), "terrain.bin"))

        # RunLand: a timing run on a mod with no distant land is refused before any file is written (T-N1d); the
        # null: the mod with a version file passes. A read-only run says so in its head.
        fresh = Key("own-0badf00d", "own-0badf00d", "base")
        land = RunLand(instance, profile, fresh, True, "c0ffee")
        try:
            land.prepare(modlist, timing=True)
            raise AssertionError("a timing run took an empty mod")
        except Problem:
            assert not os.path.exists(run_file(profile))
        os.makedirs(os.path.join(land.mod, DATA))
        with open(os.path.join(land.mod, DATA, VERSION_FILE), "w") as file:
            file.write("22")
        head = land.prepare(modlist, timing=True)
        assert os.path.exists(run_file(profile)) and any("made without the run's files" in line for line in head), head
        assert land.switch(modlist) == ([plain.mod_name], fresh.mod_name)
        assert land.result()[1] and "not checked" in land.result()[0][0]
        land.end()
        assert land.result() == (["distant land: read-only, as before the run (none)"], False) and not os.path.exists(run_file(profile))
        # A read-only run that changed its mod's state is a fault, with the generator's cause.
        land.prepare(modlist)
        with open(os.path.join(land.mod, DATA, STATE_FILE), "w") as file:
            file.write("new")
        with open(os.path.join(land.mod, DATA, "generation_report.toml"), "w") as file:
            file.write('rebuild_cause = "meshes +0/-0/~61"\n')
        land.end()
        lines, fault = land.result()
        assert fault and "changed" in lines[0] and lines[-1] == "the generator's cause: meshes +0/-0/~61", lines
        os.remove(run_file(profile))
        # An archive whose helper has no option has a format of its own (the null); one with it takes the helper's key.
        old = lambda command, **kinds: subprocess.CompletedProcess(command, 1, "", "")  # noqa: E731
        assert archive_key("D:/a", "t.toml", [], run=old).form.startswith("archive-")
        assert archive_key("D:/a", "t.toml", [], run=old).text != archive_key("D:/b", "t.toml", [], run=old).text
        assert archive_key("D:/a", "t.toml", [], run=helper).text == "abcd1234-99887766"

        # adopt (N5): a dry run moves nothing; --apply moves and puts the mod on, the other marked mods off. The nulls:
        # an empty overwrite, and a key whose mod holds a distant land, are refused.
        listing = os.path.join(profile, "modlist.txt")
        with open(listing, "wb") as file:
            file.write(b"# by Mod Organizer\r\n+%s\r\n+Distant Lights OAAB patched and improved\r\n" % plain.mod_name.encode())
        resting = Key("own-12345678", "own-12345678", "base")
        try:
            adopt(instance, profile, resting, "c0ffee")
            raise AssertionError("an empty overwrite was adopted")
        except Problem as problem:
            assert "is empty" in str(problem)
        plant("version", "22")
        plant("statics/usage.data")
        dry = adopt(instance, profile, resting, "c0ffee")
        assert "2 files" in dry[0] and "dry run" in dry[-1] and overwrite_files(instance) == ["statics/usage.data", "version"], dry
        applied = adopt(instance, profile, resting, "c0ffee", apply=True)
        target = os.path.join(mods_folder(instance), resting.mod_name)
        assert has_data(target) and overwrite_files(instance) == [], applied
        with open(listing, "rb") as file:
            assert file.read() == ("# by Mod Organizer\r\n+%s\r\n-%s\r\n+Distant Lights OAAB patched and improved\r\n"
                                   % (resting.mod_name, plain.mod_name)).encode(), applied
        plant("version", "22")
        try:
            adopt(instance, profile, resting, "c0ffee")
            raise AssertionError("a second adopt into a full mod was taken")
        except Problem as problem:
            assert "holds a distant land already" in str(problem)
        os.remove(os.path.join(overwrite_folder(instance), "version"))
        # rest (deploy_rest.py): the key's mod on, the others off; a second call changes nothing.
        assert "would put" in rest(instance, profile, plain, "c0ffee", apply=False)[0]
        assert "on: " + plain.mod_name in rest(instance, profile, plain, "c0ffee")[0]
        assert rest(instance, profile, plain, "c0ffee")[0].endswith("mod list: as it was")
    print("distant_land self-test: ok")
    return 0


def main(argv=None):
    if argv is None:
        argv = sys.argv[1:]
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("command", nargs="?", choices=("adopt", "path"), help="adopt: the one-time move; path: print the key's mod folder")
    ap.add_argument("--instance", default="D:/Modlists/Morrowind75testing", help="the Mod Organizer instance")
    ap.add_argument("--profile", default="Default", help="its profile")
    ap.add_argument("--config-mod", default="Configuration", help="the mod of the instance with mgeXE.toml")
    ap.add_argument("--mge-build", help="the sealed set; without it the current set")
    ap.add_argument("--apply", action="store_true", help="adopt: move the files (without it, a dry run)")
    ap.add_argument("--self-test", action="store_true", help="the rules on a planted instance in a temporary folder")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    if not args.command:
        ap.print_help()
        return 2
    import clean_common
    import sets
    folder = args.mge_build or (os.path.join(clean_common.SETS, clean_common.current_set()) if clean_common.current_set() else None)
    if not folder:
        print("distant_land: no --mge-build and no current set")
        return 1
    try:
        manifest = sets.verify(folder)
        toml = os.path.join(args.instance, "mods", args.config_mod, "Root", "mgeXE.toml")
        key = key_of(folder, manifest, toml, [])
        if args.command == "path":
            mod = os.path.join(mods_folder(args.instance), key.mod_name)
            if not has_data(mod):
                raise Problem("%s is not there or has no version file: a plain run of that set makes it" % mod)
            print(os.path.join(mod, DATA))
            return 0
        import deploy_rest
        try:
            deploy_rest.check_nothing_runs()
        except deploy_rest.Refused as refused:
            raise Problem(str(refused))
        for line in adopt(args.instance, os.path.join(args.instance, "profiles", args.profile), key, tool_commit(), args.apply):
            print(line)
    except (Problem, sets.SetError) as problem:
        print("distant_land: %s" % problem)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
