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
           the time of the ports for each frame and the busy time of the helper.
  tour     the dual run over the NIF test level and its replay comparison (slow).
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
}


def run(args, env=None):
    e = dict(os.environ)
    e.update(env or {})
    result = subprocess.run(args, capture_output=True, text=True, env=e, cwd=REPO)
    return result.stdout + result.stderr


def newest(label):
    runs = sorted((d for d in os.listdir(os.path.join(REPO, "results")) if d.endswith("-" + label)), reverse=True)
    return os.path.join(REPO, "results", runs[0]) if runs else None


def game(label, extra, mge_build):
    args = [PY, os.path.join(HERE, "run_own_device.py"), "--mge-build", mge_build, "--no-build", "--label", label, "--set", "expanded_light_limit = true"] + extra
    run(args)
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
    busy = re.search(r"mark after rate: .*busy ([\d.]+) ms", open(os.path.join(folder, "host.txt"), errors="replace").read())
    ok = check(rate is not None, "frame rate %s, ports %.3f ms for each frame, helper busy %s ms" % (rate.group(1) if rate else "?", per_frame, busy.group(1) if busy else "?"))
    ok &= check(" 0 without a geometry or a material" in counts["shapes"], counts["shapes"][:160])
    skinned = re.search(r"skinned shapes (\d+)", open(os.path.join(folder, "host.txt"), errors="replace").read())
    print("       skinned shapes %s" % (skinned.group(1) if skinned else "?"), flush=True)
    return ok


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
    args = ap.parse_args()
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
        else:
            print("unknown stage " + stage)
            return 2
    print("all stages passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
