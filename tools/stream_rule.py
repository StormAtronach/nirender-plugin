"""The stream rule of the replay gate: the cameras of one frame have one field of view.

    python -I tools/stream_rule.py <capture.nircap> [...]
    python -I tools/stream_rule.py --self-test    # the tests of the rule, no capture, no game

In each frame of a capture, each scene of the kinds 2 to 6 must have the projection terms _11 and _22
of the world scene (kind 1) of the same frame, within 1e-4. The world, the water, the sorted alpha,
the shadow camera (kinds 4 and 5) and the first-person arms are all views of the player: MGE XE gives
them one field of view and one zoom. Kind 7 (the interface, the menu camera) is left out by name.
Other kinds (the preview, the local map, a mod's camera into a texture) are not views of the player.

The gate compares mode gpu with mode off, and both take the projection from the stream. A wrong
projection in the stream is wrong in both, so the replay gate could not see risk 76 (the actors that
the game shadows were drawn at 75 degrees in a world of 91.3). The reference of this rule is the
world scene of the same frame, not the helper.

The projection of a scene is the first projection command after its scene record: the camera's own
(`SetCameraData`). A later projection in the scene (a screen polygon) is not the camera's.

A capture with no scene of kind 5 that was checked gives NOT RUN, not a pass: it holds no actor in
shadow range, or its kind 5 scenes had no world scene with a projection in their frame, so the rule
compared none of the draws that risk 76 spoiled. The exit code is 0 for a pass, 1 for a failure,
5 for NOT RUN. The end of a scene ends its wait for a projection: a projection between scenes is
no scene's.
"""
import contextlib
import io
import mmap
import os
import struct
import sys
import tempfile

TAG_SCENE_BEGIN, TAG_SCENE_END, TAG_COMMAND, TAG_FRAME_END = 3, 4, 8, 10
COMMAND_TRANSFORM, D3DTS_PROJECTION = 3, 3
SCENE_WORLD = 1
# The scenes whose projection must be that of the world scene: the sorted alpha, the water, the two
# scenes of the shadow camera and the first-person arms.
FOLLOWING_KINDS = (2, 3, 4, 5, 6)
SCENE_SHADOW_CASTERS = 5
TOLERANCE = 1e-4
PASS, FAIL, NOT_RUN = "pass", "fail", "not run"
EXIT_CODES = {PASS: 0, FAIL: 1, NOT_RUN: 5}


def scene_projections(data):
    """(frame, kind, _11, _22) for each scene of the capture that has a projection command. `data`
    is the content of the file (bytes or a map). The frame of the first records is 0."""
    at, frame, kind, waiting = 8, 0, None, False
    size = len(data)
    if bytes(data[:8]) != b"NIRCAP01":
        raise ValueError("not a capture file")
    while at + 8 <= size:
        tag, length = struct.unpack_from("<II", data, at)
        body = at + 8
        at = body + length
        if tag == TAG_FRAME_END:
            frame += 1
        elif tag == TAG_SCENE_BEGIN and length >= 8:
            kind, waiting = struct.unpack_from("<I", data, body + 4)[0], True
        elif tag == TAG_SCENE_END:
            # A projection after the end of a scene is not its camera's.
            waiting = False
        elif tag == TAG_COMMAND and waiting and length >= 80:
            command, state = struct.unpack_from("<II", data, body)
            if command == COMMAND_TRANSFORM and state == D3DTS_PROJECTION:
                matrix = struct.unpack_from("<16f", data, body + 16)
                waiting = False
                yield frame, kind, matrix[0], matrix[5]


def check_scenes(scenes):
    """The verdict of the rule for scenes from `scene_projections`: (status, report), with the
    report a dict of the counts and the list `faults` of (frame, kind, term, value, world)."""
    world = {}
    others = []
    for frame, kind, m11, m22 in scenes:
        if kind == SCENE_WORLD:
            world.setdefault(frame, (m11, m22))
        elif kind in FOLLOWING_KINDS:
            others.append((frame, kind, m11, m22))
    faults, checked, shadow = [], 0, 0
    for frame, kind, m11, m22 in others:
        reference = world.get(frame)
        if reference is None:
            continue
        checked += 1
        if kind == SCENE_SHADOW_CASTERS:
            shadow += 1
        for term, value, expected in (("_11", m11, reference[0]), ("_22", m22, reference[1])):
            if abs(value - expected) > TOLERANCE:
                faults.append((frame, kind, term, value, expected))
    report = {"world_scenes": len(world), "checked": checked, "shadow_scenes": shadow, "faults": faults}
    if faults:
        return FAIL, report
    if shadow == 0:
        return NOT_RUN, report
    return PASS, report


def check_capture(path):
    """The verdict of the rule for a capture file."""
    with open(path, "rb") as file, mmap.mmap(file.fileno(), 0, access=mmap.ACCESS_READ) as data:
        return check_scenes(scene_projections(data))


def describe(path, status, report):
    """The lines for the result of one capture: the first line says the verdict."""
    name = os.path.basename(path)
    head = "%s: %s: %d scenes checked against %d world scenes, %d scenes of kind 5" % (
        name, status.upper(), report["checked"], report["world_scenes"], report["shadow_scenes"])
    lines = [head]
    if status == NOT_RUN:
        lines.append("  no scene of kind 5: the capture holds no actor in shadow range")
    faults = report["faults"]
    for frame, kind, term, value, expected in faults[:6]:
        lines.append("  frame %d kind %d %s %.4f, the world scene of the frame %.4f" % (frame, kind, term, value, expected))
    if len(faults) > 6:
        frames = sorted({f[0] for f in faults})
        kinds = sorted({f[1] for f in faults})
        lines.append("  %d faults in all, in kinds %s, frames %d to %d" % (len(faults), kinds, frames[0], frames[-1]))
    return lines


def plant(frames):
    """The bytes of a small capture, for the tests. `frames` is a list of frames, each a list of
    (kind, _11, _22) for the scenes in it. Each scene has its scene record, a projection command
    (none when _11 is None) and its end record."""
    out = bytearray(b"NIRCAP01")

    def record(tag, body):
        out.extend(struct.pack("<II", tag, len(body)))
        out.extend(body)

    for scenes in frames:
        for kind, m11, m22 in scenes:
            record(TAG_SCENE_BEGIN, struct.pack("<II", 0, kind) + bytes(16))
            if m11 is not None:
                matrix = [0.0] * 16
                matrix[0], matrix[5], matrix[10], matrix[11], matrix[14] = m11, m22, 1.0001, 1.0, -1.0001
                record(TAG_COMMAND, struct.pack("<IIII", COMMAND_TRANSFORM, D3DTS_PROJECTION, 0, 0) + struct.pack("<16f", *matrix))
            record(TAG_SCENE_END, bytes(4))
        record(TAG_FRAME_END, bytes(4))
    return bytes(out)


def self_test():
    """The tests of the rule on planted captures: the fault of risk 76 itself, and the passes."""
    world, shadow, menu = (0.9774, 1.7376), (1.3032, 2.3168), (1.3032, 2.3168)

    def verdict(frames):
        return check_scenes(scene_projections(plant(frames)))

    ok_frame = [(1,) + world, (3,) + world, (4,) + world, (5,) + world, (6,) + world, (7,) + menu]
    status, report = verdict([ok_frame, ok_frame])
    assert status == PASS and report["checked"] == 8 and report["shadow_scenes"] == 2, (status, report)
    # The fault of risk 76: the shadow camera's scenes at 75 degrees in a world of 91.3.
    bad_frame = [(1,) + world, (3,) + world, (4,) + shadow, (5,) + shadow, (6,) + world, (7,) + menu]
    status, report = verdict([ok_frame, bad_frame, bad_frame])
    assert status == FAIL and {f[1] for f in report["faults"]} == {4, 5} and {f[0] for f in report["faults"]} == {1, 2}, report
    assert len(report["faults"]) == 8, report["faults"]
    # Each term is tested: a wrong _22 alone fails.
    status, report = verdict([[(1,) + world, (5, world[0], shadow[1])]])
    assert status == FAIL and [f[2] for f in report["faults"]] == ["_22"], report
    # The tolerance: 5e-5 passes, 2e-4 fails.
    assert verdict([[(1,) + world, (5, world[0] + 5e-5, world[1])]])[0] == PASS
    assert verdict([[(1,) + world, (5, world[0] + 2e-4, world[1])]])[0] == FAIL
    # No scene of kind 5: NOT RUN, not a pass. A fault is a failure whatever else is missing.
    assert verdict([[(1,) + world, (3,) + world, (6,) + world, (7,) + menu]])[0] == NOT_RUN
    assert verdict([[(1,) + world, (4,) + shadow]])[0] == FAIL
    # Kind 7 and the kinds that are not views of the player are left out by name.
    assert verdict([[(1,) + world, (5,) + world, (7,) + shadow, (11,) + shadow, (0,) + shadow]])[0] == PASS
    # A frame with no world scene (a menu) has nothing to compare with: its scenes are not checked.
    status, report = verdict([[(5,) + shadow], [(1,) + world, (5,) + world]])
    assert status == PASS and report["checked"] == 1 and report["shadow_scenes"] == 1, report
    # A pass needs a kind 5 scene that was CHECKED against a world scene of its frame (B1: kind 5 in
    # a capture with no world scene; B2: a world scene with no projection command, kind 5 at 75 degrees).
    status, report = verdict([[(5,) + shadow], [(5,) + shadow, (7,) + menu]])
    assert status == NOT_RUN and report["checked"] == 0 and report["shadow_scenes"] == 0, (status, report)
    status, report = verdict([[(1, None, None), (5,) + shadow]])
    assert status == NOT_RUN and report["checked"] == 0 and report["shadow_scenes"] == 0, (status, report)
    # B3: a projection between scenes is not credited to the scene that had none. Kind 5 with no
    # projection of its own is not checked; the stray one (a screen polygon after its end) is no scene's.
    data = bytearray(plant([[(1,) + world, (5, None, None)]]))
    stray = struct.pack("<II", TAG_COMMAND, 16 + 64) + struct.pack("<IIII", COMMAND_TRANSFORM, D3DTS_PROJECTION, 0, 0) + struct.pack("<16f", *([0.0] * 5 + [2.0] + [0.0] * 10))
    end = len(data) - 12
    data[end:end] = stray
    status, report = check_scenes(scene_projections(bytes(data)))
    assert status == NOT_RUN and report["checked"] == 0 and not report["faults"], (status, report)
    # The world scene of the frame is the reference, not the first scene of the capture.
    zoomed = (1.9548, 3.4752)
    assert verdict([[(1,) + world, (5,) + world], [(1,) + zoomed, (5,) + zoomed]])[0] == PASS
    assert verdict([[(1,) + world, (5,) + world], [(1,) + zoomed, (5,) + world]])[0] == FAIL
    # Kind 2 (the sorted alpha scene) follows the world scene like the others. No capture has one today.
    assert verdict([[(1,) + world, (2,) + world, (5,) + world]])[0] == PASS
    status, report = verdict([[(1,) + world, (2,) + shadow, (5,) + world]])
    assert status == FAIL and {f[1] for f in report["faults"]} == {2}, report
    # A frame with two world scenes: the first is the reference, not the last.
    assert verdict([[(1,) + world, (5,) + world, (1,) + zoomed]])[0] == PASS
    assert verdict([[(1,) + world, (5,) + zoomed, (1,) + zoomed]])[0] == FAIL
    # Only the first projection of a scene counts: a screen polygon's projection after it is not
    # the camera's.
    data = bytearray(plant([[(1,) + world, (5,) + world]]))
    poly = [0.0] * 16
    poly[0], poly[5] = 2.0, 2.0
    extra = struct.pack("<II", TAG_COMMAND, 16 + 64) + struct.pack("<IIII", COMMAND_TRANSFORM, D3DTS_PROJECTION, 0, 0) + struct.pack("<16f", *poly)
    end = len(data) - 12
    # Inside the last scene: before its end record (12 bytes) and the frame end (12 bytes).
    end = len(data) - 24
    data[end:end] = extra
    assert check_scenes(scene_projections(bytes(data)))[0] == PASS
    # A file and a file that is not a capture.
    with tempfile.TemporaryDirectory() as folder:
        path = os.path.join(folder, "plant.nircap")
        with open(path, "wb") as file:
            file.write(plant([ok_frame, bad_frame]))
        status, report = check_capture(path)
        assert status == FAIL and "FAIL" in describe(path, status, report)[0] and "kind 4" in describe(path, status, report)[1]
        junk = os.path.join(folder, "junk.nircap")
        with open(junk, "wb") as file:
            file.write(b"NOTACAPTURE-NOTACAPTURE")
        try:
            check_capture(junk)
        except ValueError:
            pass
        else:
            raise AssertionError("a file that is not a capture must be refused")
        # The exit codes of main, which a caller reads: 0 pass, 1 fail, 5 not run; a failure beats a not run, and a
        # not run beats a pass, whatever the order of the files.
        paths = {}
        for name, frames in (("right", [[(1,) + world, (5,) + world]]), ("none", [[(1,) + world, (3,) + world]]), ("wrong", [[(1,) + world, (5,) + shadow]])):
            paths[name] = os.path.join(folder, name + ".nircap")
            with open(paths[name], "wb") as file:
                file.write(plant(frames))
        with contextlib.redirect_stdout(io.StringIO()):
            for names, wanted in ((("right",), 0), (("none",), 5), (("wrong",), 1), (("right", "none"), 5), (("none", "right"), 5),
                                  (("none", "wrong"), 1), (("wrong", "none"), 1), (("right", "wrong", "none"), 1)):
                assert main([paths[name] for name in names]) == wanted, (names, wanted)
    print("stream_rule self-test: ok")


def main(argv):
    if argv == ["--self-test"]:
        self_test()
        return 0
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    worst = PASS
    for path in argv:
        status, report = check_capture(path)
        for line in describe(path, status, report):
            print(line)
        if status == FAIL or (status == NOT_RUN and worst == PASS):
            worst = status
    return EXIT_CODES[worst]


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
