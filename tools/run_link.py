"""Tests the render link between the plugin and MGE XE.

Starts the game through the testing instance and loads a save. In the place of the save and
in each cell of --cells, the game runs with the game code and then with the Rust code.
MGE XE counts, for each fact, how many times the packet of the plugin and its own inference
were equal or different.

    python tools/run_link.py --cells "Balmora;Seyda Neen" [--seconds 8]

With --facts, MGE XE also takes facts from the packets. In each place the game holds in menu
mode and saves three pictures of the frame: facts off, facts on, facts off. The picture with
the facts must be equal to a picture without. Then the game runs with the facts in use.

    python tools/run_link.py --facts all --cells "Balmora"
    python tools/run_link.py --facts "ui_scene,water_plane"

The d3d8.dll of the testing instance must have the render link host. Prints the tables and
writes results/<run>/link.json. The exit code is 0 when no fact was different and each
picture test was equal.
"""
import argparse
import json
import os
import re
import sys

import session

TAG = "[nirender.link] "
FACT = re.compile(r"fact (\w+): both_yes (\d+) both_no (\d+) host_only (\d+) packet_only (\d+)")
COLUMNS = ("both_yes", "both_no", "host_only", "packet_only")
TOTALS = re.compile(r"scenes (\d+) draws (\d+) draws_without_packet (\d+)")
# The index of each fact in the contract (MGE_LINK_FACT_*).
FACTS = ("light_radius", "sun_light", "moon_shadow", "water_plane", "ui_scene", "scene_kind", "sky_draw", "land_splat",
         "later_view", "state_draw", "state_buffers", "state_transform", "state_blend", "state_depth", "state_lighting",
         "state_material", "state_stages", "state_lights", "state_textures", "state_texture_transforms", "state_bones",
         "state_ambient", "texture_create", "state_depth_stencil", "state_samplers", "device_render_states",
         "device_stage_states", "device_textures", "device_buffers", "device_transforms", "device_lights",
         "client_camera_space")
# The facts that MGE XE can take from a packet.
USABLE = ("ui_scene", "water_plane", "sun_light", "sky_draw", "land_splat", "scene_kind", "moon_shadow", "state_draw",
          "device_render_states", "device_stage_states", "device_textures", "device_buffers", "device_transforms",
          "device_lights", "client_camera_space")
# The classes of the contract (MGE_LINK_CLASS_*), by index.
CLASSES = ("unknown", "sky_dome", "sky_night", "sky_clouds", "sky_sun", "sky_moon", "sky_moon_shadow", "sky_other",
           "sun_glare", "weather_particles", "water_plane", "landscape", "object", "shadow_volume", "shadow_overlay",
           "first_person", "ui", "water_ripple")
MODES = {"off": "game code", "live": "rust code", "facts": "game code, facts in use"}


def fact_mask(text):
    names = USABLE if text == "all" else [n.strip() for n in text.split(",") if n.strip()]
    mask = 0
    for name in names:
        if name not in FACTS:
            raise SystemExit("no fact '%s'; the facts are: %s" % (name, " ".join(FACTS)))
        mask |= 1 << FACTS.index(name)
    return mask


def different_bytes(a, b):
    """The number of bytes that differ between two files, or -1 when a file is missing."""
    if not (os.path.isfile(a) and os.path.isfile(b)):
        return -1
    x, y = open(a, "rb").read(), open(b, "rb").read()
    if x == y:
        return 0
    return abs(len(x) - len(y)) + sum(1 for i in range(min(len(x), len(y))) if x[i] != y[i])


def picture_result(shots):
    """The result line of the three pictures of one place, and True when it is a pass."""
    if not all(name in shots for name in ("off1", "on", "off2")):
        return "FAIL: pictures are missing", False
    still = different_bytes(shots["off1"], shots["off2"])
    before = different_bytes(shots["off1"], shots["on"])
    after = different_bytes(shots["on"], shots["off2"])
    if min(still, before, after) < 0:
        return "FAIL: a picture file is missing", False
    if before == 0 or after == 0:
        return "EQUAL: the frame with the facts is the same as a frame without", True
    if still == 0:
        return "DIFFERENT: %d bytes differ with the facts; the two frames without are equal" % before, False
    return ("NO RESULT: the scene moves in menu mode (bytes that differ: %d between the two frames without the "
            "facts, %d and %d between the frame with the facts and each of them)" % (still, before, after)), True


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cells", default="", help="cells to visit, separated by ';'")
    ap.add_argument("--save", default="quiksave.ess")
    ap.add_argument("--seconds", type=int, default=8, help="time in each mode in each place")
    ap.add_argument("--hour", type=float, help="set the game hour before the test, for example 0 for night")
    ap.add_argument("--facts", default="", help="'all', or names of facts separated by ','; MGE XE takes them from the packets")
    ap.add_argument("--sweep", type=float, help="turn the camera and look up during the test; the value is the sign "
                    "and size of the pitch, for example -1")
    ap.add_argument("--moon", choices=("masser", "secunda"), help="keep the camera on this moon during the test")
    ap.add_argument("--quick", action="store_true", help="only the Rust code, 2 seconds in each place, short waits: "
                    "for the state packets, which only the Rust code sends")
    ap.add_argument("--rust-only", action="store_true", help="skip the measurement with the game code; the waits "
                    "stay long, as the picture test needs: the eye adaptation shader changes the frame for some "
                    "seconds after a change of cell")
    ap.add_argument("--link-textures", action="store_true", help="the Rust code runs during the changes of cell and "
                    "makes the textures through the link")
    ap.add_argument("--camera-compare", action="store_true", help="the plugin makes the camera-relative world "
                    "matrices too and counts the differences from the matrices of MGE XE")
    ap.add_argument("--label", default="link")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    cells = [c.strip() for c in args.cells.split(";") if c.strip()]
    mask = fact_mask(args.facts) if args.facts else 0
    session.prepare(build=not args.no_build)
    run = session.Run(args.label)
    spec = run.spec("link", save=args.save, cells=cells, seconds=args.seconds, facts=mask,
                    shotDir=run.dir.replace("\\", "/"))
    if args.quick:
        spec.update(modes=["live"], seconds=2, settleSeconds=3, menuSeconds=3)
    if args.rust_only:
        spec.update(modes=["live"])
    # MGE XE takes the state of a draw from a packet that only the Rust code sends.
    spec["factsNeedRust"] = any(mask >> FACTS.index(n) & 1 for n in FACTS if n.startswith(("state_", "device_", "client_")))
    spec["linkTextures"] = args.link_textures
    spec["cameraCompare"] = args.camera_compare
    if args.hour is not None:
        spec["hour"] = args.hour
    if args.moon:
        spec["moon"] = args.moon
    if args.sweep is not None:
        spec["sweep"] = args.sweep
    per_place = (3 if mask else 2) * (args.seconds + 2) + (20 if mask else 12)
    done = run.launch(spec, 120 + (len(cells) + 1) * per_place, TAG)

    rows = []
    pictures = {}
    connected = False
    for line in run.progress_lines(TAG):
        line = re.sub(r"^\s*\d+s\s+", "", line)
        if line.startswith("link: "):
            connected = line.startswith("link: true")
            print(line)
        if line.startswith(("facts in use", "ERROR", "moon ")):
            print(line)
        if line.startswith("picture|"):
            _, where, place, name, path = line.split("|", 4)
            pictures.setdefault((int(place), where), {})[name] = path
        if not line.startswith("counters|"):
            continue
        _, where, mode, text = line.split("|", 3)
        totals = TOTALS.search(text)
        if totals:
            rows.append({"where": where, "mode": mode, "scenes": int(totals.group(1)), "draws": int(totals.group(2)),
                         "draws_without_packet": int(totals.group(3)), "facts": {}})
        if text.startswith("classes:") and rows:
            rows[-1]["classes"] = {CLASSES[i]: int(n) for i, n in enumerate(text.split()[1:]) if int(n)}
        fact = FACT.search(text)
        if fact and rows:
            rows[-1]["facts"][fact.group(1)] = dict(zip(COLUMNS, (int(n) for n in fact.groups()[1:])))

    different = 0
    for row in rows:
        print("\n%s, %s: %d scenes, %d draw packets, %d world draws without a packet" % (
            row["where"], MODES.get(row["mode"], row["mode"]), row["scenes"], row["draws"], row["draws_without_packet"]))
        for name, count in row["facts"].items():
            if any(count.values()):
                print("  %-14s both yes %8d  both no %8d  only MGE XE %8d  only packet %8d" % (
                    name, count["both_yes"], count["both_no"], count["host_only"], count["packet_only"]))
            # With the water plane or the moons from the packets, the mark is out of the
            # material. Then the inference of MGE XE finds nothing, and only the packet says yes.
            expected = row["mode"] == "facts" and name in ("water_plane", "moon_shadow") and mask >> FACTS.index(name) & 1
            # The texture counts are not a comparison: "only MGE XE" is a texture that the
            # device did not take.
            if name != "texture_create":
                different += count["host_only"] + (0 if expected else count["packet_only"])
        if row.get("classes"):
            print("  draw packets by class: " + ", ".join("%s %d" % item for item in row["classes"].items()))

    failed_pictures = 0
    picture_results = []
    if mask:
        print("\npicture test, facts: %s" % " ".join(n for i, n in enumerate(FACTS) if mask >> i & 1))
        for (place, where), shots in sorted(pictures.items()):
            text, ok = picture_result(shots)
            picture_results.append({"where": where, "result": text})
            print("  %s: %s" % (where, text))
            failed_pictures += 0 if ok else 1
        if not pictures:
            print("  FAIL: no pictures")
            failed_pictures += 1
    json.dump({"counters": rows, "pictures": picture_results, "fact_mask": mask},
              open(run.path("link.json"), "w", encoding="utf-8"), indent=1)

    problems = run.mge_problems()
    for problem in problems:
        print("FAIL: " + problem)
    if not done:
        print("FAIL: the scenario did not finish")
        return 1
    if not connected or not rows:
        print("FAIL: no connection to the render link, or no counters")
        return 1
    print("\n%s" % ("PASS: every compared fact was equal" if different == 0 else "DIFFERENT: %d comparisons" % different))
    return 0 if different == 0 and not problems and failed_pictures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
