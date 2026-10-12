"""The checks of the retained pipeline, one stage at a time: each stage prints its result at
once and a failed stage stops the run, so a problem shows after one game run (about two
minutes) and not after all of them.

    python tools/validate_retained.py [stage ...] [--mge-build DIR] [--places seyda,narsis] [--tour]

Stages (all by default, in this order):
  dual     a run with the shape records and the draws of the device both in the stream, at
           each place, with a capture; the capture replayed with the records and without
           them must give the same picture; the counts of the helper must show no geometry
           mismatch and no material difference.
  only     a run with the shape records alone, with the rate step: prints the frame rate,
           the time of the ports for each frame and the busy time of the helper. With
           --gpu-time the helper writes GPU time stamps (they cost it about 0.4 ms a frame).
  tour     the dual run over the NIF test level and its replay comparison (slow).
  clusters the newest dual capture of each place replayed with the clusters of lights and
           without (MGE_XE_CLUSTERS=0), compared; then the rate step with and without.
  weathers a run with the shape records alone that measures the frame rate of each of the
           ten weathers of the game in turn, at each place.

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


def stage_dual(place, mge_build):
    print("dual at %s" % place, flush=True)
    folder = game("v-dual-" + place, PLACES[place] + ["--steps", "exterior,capture"], mge_build)
    if not folder or not os.path.isfile(os.path.join(folder, "exterior.nircap")):
        return check(False, "no capture from the run")
    counts = host_counts(folder)
    ok = check("0 records that differ" in counts["geometry"], counts["geometry"][:120])
    different = re.search(r"(\d+) draws that differ", counts["material"])
    ok &= check(different is not None and int(different.group(1)) < 100, counts["material"][:160])
    capture = os.path.join(folder, "exterior.nircap")
    a, b = os.path.join(folder, "replay-shapes.png"), os.path.join(folder, "replay-device.png")
    out_a = run([PY, os.path.join(HERE, "replay.py"), capture, a, "--frame", "20"])
    run([PY, os.path.join(HERE, "replay.py"), capture, b, "--frame", "20"], {"MGE_XE_SHAPES": "0"})
    shapes = [line for line in out_a.splitlines() if line.startswith("shapes:")]
    ok &= check(shapes and " 0 without a geometry or a material" in shapes[0], shapes[0][:160] if shapes else "no shapes line")
    sys.path.insert(0, HERE)
    import picture_diff

    numbers = picture_diff.compare(picture_diff.load(a), picture_diff.load(b))
    text = "replay with the records against without: %s" % (numbers.get("problem") or "%.1f %% different, mean %.3f, largest %d" % (numbers["different"] * 100, numbers["mean"], numbers.get("max", 0)))
    ok &= check("problem" not in numbers and numbers["mean"] < 0.01, text)
    # The instances: the picture with them against the picture of one draw for each.
    c = os.path.join(folder, "replay-one-draw-each.png")
    run([PY, os.path.join(HERE, "replay.py"), capture, c, "--frame", "20"], {"MGE_XE_INSTANCING": "0"})
    numbers = picture_diff.compare(picture_diff.load(a), picture_diff.load(c))
    instanced = re.search(r"instanced away (\d+)", out_a)
    text = "replay with the instances against one draw for each: %s, %s instanced away" % (numbers.get("problem") or "%.2f %% different, mean %.3f" % (numbers["different"] * 100, numbers["mean"]), instanced.group(1) if instanced else "?")
    ok &= check("problem" not in numbers and numbers["mean"] < 0.01, text)
    return ok


def stage_only(place, mge_build):
    print("only at %s" % place, flush=True)
    folder = game("v-only-" + place, PLACES[place] + ["--steps", "exterior,rate", "--shapes", "2"], mge_build)
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
    counts = host_counts(folder)
    host = open(os.path.join(folder, "host.txt"), errors="replace").read()
    busy = re.search(r"mark after rate: .*busy ([\d.]+) ms", host)
    gpu = re.search(r"mark after rate:(?:.*\n)*?\s+gpu: ([\d.]+) ms in the passes, ([\d.]+) ms the frame", host)
    ok = check(rate is not None, "frame rate %s, ports %.3f ms for each frame, helper busy %s ms, GPU %s ms in the passes, %s ms the frame" % (
        rate.group(1) if rate else "?", per_frame, busy.group(1) if busy else "?", gpu.group(1) if gpu else "?", gpu.group(2) if gpu else "?"))
    ok &= check(" 0 without a geometry or a material" in counts["shapes"], counts["shapes"][:160])
    lights = re.search(r"lights: (\d+) records, (\d+) lights of a shape without a record", host)
    ok &= check(lights is not None and lights.group(2) == "0", lights.group(0) if lights else "no lights line")
    skinned = re.search(r"instanced away (\d+), skinned shapes (\d+)", open(os.path.join(folder, "host.txt"), errors="replace").read())
    print("       instanced away %s, skinned shapes %s" % ((skinned.group(1), skinned.group(2)) if skinned else ("?", "?")), flush=True)
    return ok


def stage_clusters(place, mge_build):
    print("clusters at %s" % place, flush=True)
    folder = newest("v-dual-" + place)
    capture = folder and os.path.join(folder, "exterior.nircap")
    if not capture or not os.path.isfile(capture):
        return check(False, "no dual capture of the place: run the dual stage first")
    a, b = os.path.join(folder, "replay-clusters.png"), os.path.join(folder, "replay-draw-lights.png")
    # The replays read the mgeXE.toml of the run (the fade of the lights is on in it).
    out = run([PY, os.path.join(HERE, "replay.py"), capture, a, "--frame", "20", "--config-root", folder])
    run([PY, os.path.join(HERE, "replay.py"), capture, b, "--frame", "20", "--config-root", folder], {"MGE_XE_CLUSTERS": "0"})
    sys.path.insert(0, HERE)
    import picture_diff

    used = re.search(r"clusters: ([\d.]+) lights and (\d+) entries for each frame, (\d+) frames without", out)
    ok = check(used is not None and used.group(3) == "0" and used.group(2) != "0", used.group(0) if used else "no clusters line in the replay")
    numbers = picture_diff.compare(picture_diff.load(a), picture_diff.load(b))
    text = "replay with the clusters against the lights of the draws: %s" % (numbers.get("problem") or "%.2f %% different, %.2f %% by more than 8, mean %.3f, largest %d" % (
        numbers["different"] * 100, numbers.get("over8", 0) * 100, numbers["mean"], numbers.get("largest", 0)))
    ok &= check("problem" not in numbers and numbers["mean"] < 0.5, text)
    for name, env in (("clusters", {}), ("draw-lights", {"MGE_XE_CLUSTERS": "0"})):
        label = "v-%s-%s" % (name, place)
        env = dict(env)
        env["MGE_XE_GPU_TIME"] = "1"
        run_folder = game(label, PLACES[place] + ["--steps", "exterior,rate", "--shapes", "2"], mge_build, env)
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
    folder = game("v-weathers-" + place, PLACES[place] + ["--steps", "exterior,weathers", "--shapes", "2", "--budget", "300"], mge_build)
    if not folder:
        return check(False, "no run")
    log = open(os.path.join(folder, "MWSE.log"), errors="replace").read()
    rates = {int(m.group(2)): float(m.group(1)) for m in re.finditer(r"rate: ([\d.]+) frames in a second at '[^']*' weather (\d+)", log)}
    for index, name in enumerate(WEATHERS):
        print("       %-9s %s" % (name, ("%.1f" % rates[index]) if index in rates else "?"), flush=True)
    return check(len(rates) == 10, "%d of 10 weathers measured" % len(rates))


def stage_tour(mge_build):
    print("tour of the test level", flush=True)
    folder = game("v-tour", ["--steps", "tour", "--tour-cells", "@nifTest_1;@nifTest_2;@nifTest_3", "--budget", "560"], mge_build)
    if not folder or len([f for f in os.listdir(folder) if f.endswith(".nircap")]) != 3:
        return check(False, "no three captures from the run")
    out_a = os.path.join(folder, "pictures-shapes")
    out_b = os.path.join(folder, "pictures-device")
    run([PY, os.path.join(HERE, "validate_tour.py"), folder, "--out", out_a, "--jobs", "4"])
    text = run([PY, os.path.join(HERE, "validate_tour.py"), folder, "--out", out_b, "--against", out_a, "--jobs", "4"], {"MGE_XE_SHAPES": "0"})
    equal = re.search(r"(\d+) of (\d+) equal", text)
    return check(equal is not None and equal.group(1) == equal.group(2), equal.group(0) if equal else text[-300:])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("stages", nargs="*", default=["dual", "only"])
    ap.add_argument("--mge-build", default=os.path.join(REPO, "..", "MGE-XE-G7-render-link", "bin", "MSVC-Release"))
    ap.add_argument("--places", default="seyda,narsis")
    ap.add_argument("--gpu-time", action="store_true", help="the helper writes GPU time stamps in the only stage")
    ap.add_argument("--disable-mod", default=None, help="a mod of the profile switched off for the runs")
    args = ap.parse_args()
    if args.disable_mod:
        RUNNER_ARGS.extend(["--disable-mod", args.disable_mod])
    if args.gpu_time:
        GPU_TIME["MGE_XE_GPU_TIME"] = "1"
    places = args.places.split(",")
    for stage in args.stages:
        if stage == "dual":
            for place in places:
                if not stage_dual(place, args.mge_build):
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
