"""The checks of the retained pipeline, one stage at a time: each stage prints its result at
once and a failed stage stops the run, so a problem shows after one game run (about two
minutes) and not after all of them.

    python tools/validate_retained.py [stage ...] [--mge-build DIR] [--places seyda,narsis]

The helper draws from the records only, so a picture has no second path to compare with on
the same capture. The reference of a change of the helper is a baseline: captures that are
kept, each with the picture that was accepted for it.

Stages (capture, baseline and only by default, in this order):
  capture  a run at each place with a capture; the counts of the helper must show no shape
           without a geometry or a material and no light without a record, and the plugin
           no draw without a record; the capture replayed with the sort and the instances
           (MGE_XE_SORT=1, off by default) and with one draw for each must give the same
           picture.
  baseline replays each capture of the baseline folder (--baseline, results/baseline-records by
           default: `<name>.nircap` files, frame 1) in the order of the game, as a player
           has them (MGE_XE_SORT=0, the default) and compares the picture
           with `accepted/<name>.png` there. --accept
           writes the pictures instead: for a new baseline, or after a change that is
           meant to change them. A tour run in `tour/` of the baseline folder is replayed
           and compared the same way (`tour/accepted`).
  only     a run with the rate step: prints the frame rate, the time of the ports for each
           frame and the busy time of the helper. With --gpu-time the helper writes GPU
           time stamps (they cost it about 0.4 ms a frame).
  tour     a run over the NIF test level with a capture for each cell; its counts. Copy
           the run folder to `tour/` of the baseline folder to make it the baseline.
  clusters the newest capture of each place replayed with the clusters of lights
           (MGE_XE_CLUSTERS=1; they are off by default) and without, compared; then the
           rate step with and without.
  weathers a run that measures the frame rate of each of the ten weathers of the game in
           turn, at each place.

--disable-mod NAME switches a mod of the profile off for the runs (an A/B of a mod).
"""
import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
PY = sys.executable
PLACES = {
    "seyda": ["--exterior", "Seyda Neen", "--position=-12288,-69632,74", "--yaw", "0"],
    "narsis": ["--exterior", "Narsis"],
    "council": ["--exterior", "Narsis, Council Quarter", "--position=53000,-412000", "--yaw", "270"],
    # Narsis at night, with the light of each pixel and its fade: the lights matter.
    "narsisnight": ["--exterior", "Narsis", "--hour", "0", "--set", "[distant_land] per_pixel_lighting = true", "--set", "[distant_land] per_pixel_light_fade = true"],
}


def run(args, env=None):
    e = dict(os.environ)
    # The folder of the interpreter: an isolated interpreter (-I) finds the DLL of Pillow there.
    e["PATH"] = os.path.dirname(PY) + os.pathsep + e.get("PATH", "")
    e.update(env or {})
    result = subprocess.run(args, capture_output=True, text=True, env=e, cwd=REPO)
    return result.stdout + result.stderr


def newest(label):
    runs = sorted((d for d in os.listdir(os.path.join(REPO, "results")) if d.endswith("-" + label)), reverse=True)
    return os.path.join(REPO, "results", runs[0]) if runs else None


GPU_TIME = {}
RUNNER_ARGS = []


def game(label, extra, mge_build, env=None):
    args = [PY, os.path.join(HERE, "run_own_device.py"), "--mge-build", mge_build, "--no-build", "--label", label, "--set", "expanded_light_limit = true"] + RUNNER_ARGS + extra
    e = dict(GPU_TIME)
    e.update(env or {})
    run(args, e)
    return newest(label)


def host_counts(folder):
    text = open(os.path.join(folder, "host.txt"), errors="replace").read()
    lines = {k: "" for k in ("geometry", "material", "shapes")}
    for line in text.splitlines():
        for key in lines:
            if line.startswith(key + ":"):
                lines[key] = line
    return lines


def check(condition, text):
    print(("  ok   " if condition else "  FAIL ") + text, flush=True)
    return condition


def plugin_report(folder):
    """The line of the shape records in the log of the plugin, or ""."""
    path = os.path.join(folder, "nirender.log")
    lines = [line for line in open(path, errors="replace").read().splitlines() if "shapes: " in line and "draws without a record" in line] if os.path.isfile(path) else []
    return lines[-1] if lines else ""


def check_counts(folder):
    """The counts of a run: each shape drew, each light had its record, each draw of the game
    had a record."""
    counts = host_counts(folder)
    host = open(os.path.join(folder, "host.txt"), errors="replace").read()
    ok = check(" 0 without a geometry or a material" in counts["shapes"], counts["shapes"][:160] or "no shapes line of the helper")
    ok &= check(" 0 draws of more index entries" in counts["geometry"], counts["geometry"][:160] or "no geometry line of the helper")
    lights = re.search(r"lights: (\d+) records, (\d+) lights of a shape without a record", host)
    ok &= check(lights is not None and lights.group(2) == "0", lights.group(0) if lights else "no lights line")
    report = plugin_report(folder)
    # Since the skin weights of NiSkinData (1f01765) the line also counts the skinned draws
    # with no palette and those skinned in software; each count must be 0.
    without = re.search(r"draws without a record: (\d+) with no geometry, (\d+) with no material(?:, (\d+) skinned with no palette, (\d+) skinned in software)?; (\d+) draws of an empty geometry|draws without a record: (\d+) with no geometry, (\d+) with no material;", report)
    ok &= check(without is not None and {g for g in without.groups() if g is not None} == {"0"}, report[-170:] or "no shapes line of the plugin")
    return ok


def stage_capture(place, mge_build):
    print("capture at %s" % place, flush=True)
    folder = game("v-capture-" + place, PLACES[place] + ["--steps", "exterior,capture"], mge_build)
    if not folder or not os.path.isfile(os.path.join(folder, "exterior.nircap")):
        return check(False, "no capture from the run")
    ok = check_counts(folder)
    capture = os.path.join(folder, "exterior.nircap")
    a = os.path.join(folder, "replay.png")
    out_a = run([PY, os.path.join(HERE, "replay.py"), capture, a, "--frame", "20"], {"MGE_XE_SORT": "1"})
    sys.path.insert(0, HERE)
    import picture_diff

    # The instances: the picture with them against the picture of one draw for each.
    c = os.path.join(folder, "replay-one-draw-each.png")
    run([PY, os.path.join(HERE, "replay.py"), capture, c, "--frame", "20"], {"MGE_XE_SORT": "1", "MGE_XE_INSTANCING": "0"})
    numbers = picture_diff.compare(picture_diff.load(a), picture_diff.load(c))
    instanced = re.search(r"instanced away (\d+)", out_a)
    text = "replay with the instances against one draw for each: %s, %s instanced away" % (numbers.get("problem") or "%.2f %% different, mean %.3f" % (numbers["different"] * 100, numbers["mean"]), instanced.group(1) if instanced else "?")
    ok &= check("problem" not in numbers and numbers["mean"] < 0.01, text)
    return ok


def stage_baseline(baseline, accept, mge_build):
    """The captures of the baseline folder against their accepted pictures, in the order of
    the draws of the game, as a player has them."""
    print("baseline %s" % baseline, flush=True)
    captures = sorted(f for f in os.listdir(baseline) if f.endswith(".nircap")) if os.path.isdir(baseline) else []
    if not captures:
        return check(False, "no capture in the baseline folder")
    env = {"MGE_XE_SORT": "0"}
    accepted = os.path.join(baseline, "accepted")
    out = accepted if accept else os.path.join(baseline, "now")
    os.makedirs(out, exist_ok=True)
    for capture in captures:
        run([PY, os.path.join(HERE, "replay.py"), os.path.join(baseline, capture), os.path.join(out, capture[:-7] + ".png"), "--frame", "1", "--mge-build", mge_build], env)
    tour = os.path.join(baseline, "tour")
    tour_out = os.path.join(tour, "accepted" if accept else "now")
    if os.path.isdir(tour):
        run([PY, os.path.join(HERE, "validate_tour.py"), tour, "--out", tour_out, "--jobs", "4", "--mge-build", mge_build], env)
    if accept:
        return check(True, "%d pictures accepted%s" % (len(captures), ", and the tour" if os.path.isdir(tour) else ""))
    sys.path.insert(0, HERE)
    import picture_diff

    ok = True
    for a, b in [(accepted, out)] + ([(os.path.join(tour, "accepted"), tour_out)] if os.path.isdir(tour) else []):
        names = sorted(f for f in os.listdir(a) if f.endswith(".png")) if os.path.isdir(a) else []
        equal = 0
        for name in names:
            if not os.path.isfile(os.path.join(b, name)):
                print("       %-40s no picture now" % name, flush=True)
                continue
            numbers = picture_diff.compare(picture_diff.load(os.path.join(a, name)), picture_diff.load(os.path.join(b, name)))
            if "problem" not in numbers and numbers["different"] == 0:
                equal += 1
            else:
                print("       %-40s %s" % (name, numbers.get("problem") or "%.2f %% different, mean %.3f, largest %d" % (numbers["different"] * 100, numbers["mean"], numbers.get("max", 0))), flush=True)
        ok &= check(names != [] and equal == len(names), "%d of %d pictures equal to the accepted ones in %s" % (equal, len(names), b))
    return ok


def stage_only(place, mge_build):
    print("only at %s" % place, flush=True)
    folder = game("v-only-" + place, PLACES[place] + ["--steps", "exterior,rate"], mge_build)
    if not folder:
        return check(False, "no run")
    log = open(os.path.join(folder, "MWSE.log"), errors="replace").read()
    rate = re.search(r"rate: ([\d.]+) frames", log)
    lines = [l for l in log.splitlines() if "[nirender.owndevice] after " in l]
    ports = []
    for line in lines:
        m = re.search(r"frames (\d+) ms_per_frame [\d.]+ ms stream ([\d.]+) ports ([\d.]+)", line)
        if m:
            ports.append((int(m.group(1)), float(m.group(3))))
    per_frame = (ports[-1][1] - ports[-2][1]) / max(ports[-1][0] - ports[-2][0], 1) if len(ports) >= 2 else float("nan")
    host = open(os.path.join(folder, "host.txt"), errors="replace").read()
    busy = re.search(r"mark after rate: .*busy ([\d.]+) ms", host)
    gpu = re.search(r"mark after rate:(?:.*\n)*?\s+gpu: ([\d.]+) ms in the passes, ([\d.]+) ms the frame", host)
    ok = check(rate is not None, "frame rate %s, ports %.3f ms for each frame, helper busy %s ms, GPU %s ms in the passes, %s ms the frame" % (
        rate.group(1) if rate else "?", per_frame, busy.group(1) if busy else "?", gpu.group(1) if gpu else "?", gpu.group(2) if gpu else "?"))
    ok &= check_counts(folder)
    skinned = re.search(r"instanced away (\d+), skinned shapes (\d+), particle systems (\d+)", host)
    print("       instanced away %s, skinned shapes %s, particle systems %s" % (skinned.groups() if skinned else ("?", "?", "?")), flush=True)
    return ok


def stage_clusters(place, mge_build):
    print("clusters at %s" % place, flush=True)
    folder = newest("v-capture-" + place)
    capture = folder and os.path.join(folder, "exterior.nircap")
    if not capture or not os.path.isfile(capture):
        return check(False, "no capture of the place: run the capture stage first")
    a, b = os.path.join(folder, "replay-clusters.png"), os.path.join(folder, "replay-draw-lights.png")
    # The replays read the mgeXE.toml of the run (the fade of the lights is on in it).
    out = run([PY, os.path.join(HERE, "replay.py"), capture, a, "--frame", "20", "--config-root", folder], {"MGE_XE_CLUSTERS": "1"})
    run([PY, os.path.join(HERE, "replay.py"), capture, b, "--frame", "20", "--config-root", folder], {"MGE_XE_CLUSTERS": "0"})
    sys.path.insert(0, HERE)
    import picture_diff

    used = re.search(r"clusters: ([\d.]+) lights and (\d+) entries for each frame, (\d+) frames without", out)
    ok = check(used is not None and used.group(3) == "0" and used.group(2) != "0", used.group(0) if used else "no clusters line in the replay")
    numbers = picture_diff.compare(picture_diff.load(a), picture_diff.load(b))
    text = "replay with the clusters against the lights of the draws: %s" % (numbers.get("problem") or "%.2f %% different, %.2f %% by more than 8, mean %.3f, largest %d" % (
        numbers["different"] * 100, numbers.get("over8", 0) * 100, numbers["mean"], numbers.get("largest", 0)))
    ok &= check("problem" not in numbers and numbers["mean"] < 0.5, text)
    for name, env in (("clusters", {"MGE_XE_CLUSTERS": "1"}), ("draw-lights", {"MGE_XE_CLUSTERS": "0"})):
        label = "v-%s-%s" % (name, place)
        env = dict(env)
        env["MGE_XE_GPU_TIME"] = "1"
        run_folder = game(label, PLACES[place] + ["--steps", "exterior,rate"], mge_build, env)
        host = open(os.path.join(run_folder, "host.txt"), errors="replace").read() if run_folder else ""
        rate = re.search(r"rate: ([\d.]+) frames", open(os.path.join(run_folder, "MWSE.log"), errors="replace").read()) if run_folder else None
        gpu = re.search(r"mark after rate:(?:.*\n)*?\s+gpu: ([\d.]+) ms in the passes, ([\d.]+) ms the frame", host)
        busy = re.search(r"mark after rate: .*busy ([\d.]+) ms", host)
        clusters = re.search(r"clusters: .*", host)
        ok &= check(gpu is not None, "%s: frame rate %s, helper busy %s ms, GPU %s ms in the passes, %s ms the frame" % (name, rate.group(1) if rate else "?", busy.group(1) if busy else "?", gpu.group(1) if gpu else "?", gpu.group(2) if gpu else "?"))
        if clusters:
            print("       " + clusters.group(0)[:150], flush=True)
    return ok


WEATHERS = ["clear", "cloudy", "foggy", "overcast", "rain", "thunder", "ash", "blight", "snow", "blizzard"]


def stage_weathers(place, mge_build):
    print("weathers at %s" % place, flush=True)
    folder = game("v-weathers-" + place, PLACES[place] + ["--steps", "exterior,weathers", "--budget", "300"], mge_build)
    if not folder:
        return check(False, "no run")
    log = open(os.path.join(folder, "MWSE.log"), errors="replace").read()
    rates = weather_rates(log)
    print("       %-9s %6s %6s %6s" % ("", "pass 1", "pass 2", "mean"), flush=True)
    for index, name in enumerate(WEATHERS):
        print("       %-9s %s" % (name, weather_line(rates, index)), flush=True)
    return check(len(rates) == 20, "%d of 20 measurements (10 weathers, two passes)" % len(rates))


def weather_rates(log):
    """The frame rate of each (weather, pass) of the step "weathers" in a log: the second
    pass goes from 9 to 0, so a trend of the run shows between the passes."""
    return {(int(m.group(2)), int(m.group(3))): float(m.group(1)) for m in re.finditer(r"rate: ([\d.]+) frames in a second at '[^']*' weather (\d+) pass (\d+)", log)}


def weather_line(rates, index):
    passes = [rates.get((index, p)) for p in (1, 2)]
    mean = sum(p for p in passes if p is not None) / max(sum(p is not None for p in passes), 1)
    return "%6s %6s %6.1f" % tuple([("%.1f" % p) if p is not None else "?" for p in passes] + [mean])


def stage_tour(mge_build):
    """A run over the test level with a capture for each cell, and its counts. The pictures
    of the stops are compared in the baseline stage, when the run is the baseline tour."""
    print("tour of the test level", flush=True)
    folder = game("v-tour", ["--steps", "tour", "--tour-cells", "@nifTest_1;@nifTest_2;@nifTest_3", "--budget", "560"], mge_build)
    if not folder or len([f for f in os.listdir(folder) if f.endswith(".nircap")]) != 3:
        return check(False, "no three captures from the run")
    print("       " + folder, flush=True)
    return check_counts(folder)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("stages", nargs="*", default=["capture", "baseline", "only"])
    ap.add_argument("--mge-build", default=os.path.join(REPO, "..", "MGE-XE-G7-render-link", "bin", "MSVC-Release"))
    ap.add_argument("--places", default="seyda,narsis")
    ap.add_argument("--baseline", default=os.path.join(REPO, "results", "baseline-records"), help="the folder of the baseline stage")
    ap.add_argument("--accept", action="store_true", help="the baseline stage writes the accepted pictures and compares nothing")
    ap.add_argument("--gpu-time", action="store_true", help="the helper writes GPU time stamps in the only stage")
    ap.add_argument("--disable-mod", default=None, help="a mod of the profile switched off for the runs")
    ap.add_argument("--weather", type=int, default=None, help="the weather of the runs, 0 clear to 9 blizzard (the game's without this)")
    args = ap.parse_args()
    if args.disable_mod:
        RUNNER_ARGS.extend(["--disable-mod", args.disable_mod])
    if args.weather is not None:
        RUNNER_ARGS.extend(["--weather", str(args.weather)])
    if args.gpu_time:
        GPU_TIME["MGE_XE_GPU_TIME"] = "1"
    places = args.places.split(",")
    for stage in args.stages:
        if stage == "capture":
            for place in places:
                if not stage_capture(place, args.mge_build):
                    return 1
        elif stage == "baseline":
            if not stage_baseline(args.baseline, args.accept, args.mge_build):
                return 1
        elif stage == "only":
            for place in places:
                if not stage_only(place, args.mge_build):
                    return 1
        elif stage == "tour":
            if not stage_tour(args.mge_build):
                return 1
        elif stage == "clusters":
            for place in places:
                if not stage_clusters(place, args.mge_build):
                    return 1
        elif stage == "weathers":
            for place in places:
                if not stage_weathers(place, args.mge_build):
                    return 1
        else:
            print("unknown stage " + stage)
            return 2
    print("all stages passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
