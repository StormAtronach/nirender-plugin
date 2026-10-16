"""Reads the records of a capture, for the root cause of a difference of a picture of the
retained pipeline. A capture from before the cut of the device path (step R7b) also has the
state packets, the stage states and the draws of the device; the helper does not read them.

    python -I tools/capture_dump.py <capture.nircap> sequence <frame>
    python -I tools/capture_dump.py <capture.nircap> materials <frame> [--geometry 0x...]
    python -I tools/capture_dump.py <capture.nircap> particles <frame>
    python -I tools/capture_dump.py <capture.nircap> calls

sequence   the records of one frame in their order: the objects, the shape records, the
           render state commands (in a capture from before the cut: those that the
           pipeline of the device sets for a draw, and the draws of the device).
materials  the material of each shape record of the frame, decoded, once for each
           material: the flags, the blend modes, the maps present, the vertex colour
           modes, the stencil, the effects. `--geometry` keeps one geometry.
particles  the particle records of the frame: the count, the radius, the camera axes,
           the first particles, the material, and the material of the device draw that
           follows (a capture from before the cut).
calls      the CALL and COMMAND records of the whole capture by their kind, for each
           frame: the calls of the frame (clear, viewport, targets), the view and the
           projection, and the render states of code outside the renderer.

The frames count from 0, as `replay.py --frame` does.
"""
import argparse
import collections
import struct
import sys

TAG_OBJECT, TAG_DRAW_STATE, TAG_COMMAND, TAG_CALL, TAG_FRAME_END = 6, 7, 8, 9, 10
TAG_SCENE_BEGIN, TAG_SCENE_END, TAG_GEOMETRY, TAG_MATERIAL, TAG_SHAPE = 3, 4, 18, 19, 20
TAG_INSTANCE, TAG_INSTANCE_DRAW, TAG_INSTANCE_DELETE = 23, 24, 25
RENDER_STATES = {7: "ZENABLE", 8: "FILLMODE", 14: "ZWRITEENABLE", 22: "CULLMODE", 28: "FOGENABLE", 34: "FOGCOLOR",
                 35: "FOGTABLEMODE", 36: "FOGSTART", 37: "FOGEND", 38: "FOGDENSITY", 48: "RANGEFOGENABLE",
                 52: "STENCILENABLE", 53: "STENCILFAIL", 54: "STENCILZFAIL", 55: "STENCILPASS", 56: "STENCILFUNC",
                 57: "STENCILREF", 58: "STENCILMASK", 59: "STENCILWRITEMASK", 140: "FOGVERTEXMODE",
                 147: "AMBIENTMATERIALSOURCE", 168: "COLORWRITEENABLE"}
CALLS = {1: "TestDevice", 2: "BeginScene", 3: "EndScene", 4: "Clear", 5: "SetViewport", 6: "Present", 7: "SetRenderTarget",
         8: "CreateVertexBuffer", 9: "CreateIndexBuffer", 10: "AddRef", 11: "Release", 12: "BufferLock", 13: "BufferUnlock",
         14: "BufferDescription", 15: "TextureLevel", 16: "CubeFace", 17: "SurfaceDescription", 18: "SurfaceLock",
         19: "SurfaceUnlock", 20: "CreateTexture", 21: "CreateDepthBuffer", 22: "CreateImageSurface", 23: "CopyPixels",
         24: "GetRenderTarget", 25: "GetDepthBuffer", 26: "DiscardBytes"}
COMMANDS = {1: "RenderState", 2: "StageState", 3: "Transform", 4: "DrawIndexed", 5: "Draw"}
# The offsets in a shape record (see `capture::tag::SHAPE` of the plugin).
SHAPE_HEAD_WORDS, SHAPE_WORLD, SHAPE_COLOURS, SHAPE_BONES = 36, 20, 84, 140
FLAG_EFFECTS, FLAG_FOG, FLAG_PARTICLES = 16, 32, 8
EFFECT_WORDS = 18


def records(path):
    """(frame, tag, body) for each record of the capture."""
    data = open(path, "rb").read()
    assert data[:8] == b"NIRCAP01", path
    at = 8
    frame = 0
    while at + 8 <= len(data):
        tag, length = struct.unpack_from("<II", data, at)
        body = data[at + 8:at + 8 + length]
        at += 8 + length
        if tag == TAG_FRAME_END:
            frame += 1
            continue
        yield frame, tag, body


def words(body, at, count):
    return struct.unpack_from("<%dI" % count, body, at)


def shape_tail(body):
    """The offset of the bones of a shape record, and the offsets of its effect stages and
    its fog when the record has them."""
    flags, lights, bones = words(body, 8, 1)[0], words(body, 12, 1)[0], words(body, SHAPE_BONES, 1)[0]
    at = SHAPE_HEAD_WORDS * 4 + lights * 4
    effects = at if flags & FLAG_EFFECTS else None
    if effects is not None:
        at += 2 * EFFECT_WORDS * 4
    fog = at if flags & FLAG_FOG else None
    if fog is not None:
        at += 16
    return at, at + bones * 64, effects, fog


def object_name(body):
    flags = words(body, 8, 1)[0]
    return body[24 if flags & 0x300 else 16:].decode("latin-1", errors="replace")


def sequence(path, wanted):
    for frame, tag, body in records(path):
        if frame > wanted:
            break
        if frame != wanted:
            continue
        if tag == TAG_SCENE_BEGIN:
            kind, flags = words(body, 4, 1)[0], words(body, 20, 1)[0]
            print("SCENE_BEGIN kind %d flags %#x%s" % (kind, flags, " sky hidden" if flags & 4 else ""))
        elif tag == TAG_SCENE_END:
            print("SCENE_END")
        elif tag == TAG_OBJECT:
            kind, cls, flags = words(body, 0, 3)
            print("OBJECT kind %d class %d flags %#x %r" % (kind, cls, flags, object_name(body)[:40]))
        elif tag == TAG_SHAPE:
            g, m, flags, lights = words(body, 0, 4)
            print("  SHAPE geometry %#x material %#x flags %#x lights %d%s" % (g, m, flags, lights, " twin" if flags & 0x200 else ""))
        elif tag == TAG_INSTANCE:
            i, cls, flags, g, m = words(body, 0, 5)
            name = body[160:]
            print("INSTANCE %d class %d flags %#x geometry %#x material %#x %r" % (i, cls, flags, g, m, name.decode("latin-1")[:40]))
        elif tag == TAG_INSTANCE_DRAW:
            i, ambient, flags = words(body, 0, 3)
            lights = words(body, 28 if flags & 32 else 12, 1)[0]
            print("  INSTANCE_DRAW %d flags %#x lights %d" % (i, flags, lights))
        elif tag == TAG_INSTANCE_DELETE:
            print("INSTANCE_DELETE %d" % words(body, 0, 1)[0])
        elif tag == TAG_COMMAND:
            command, a, b, c = words(body, 0, 4)
            if command == 1 and a in RENDER_STATES:
                print("  RS %s = %d" % (RENDER_STATES[a], b))
            elif command in (4, 5):
                print("  DRAW command %d" % command)
        elif tag == TAG_DRAW_STATE:
            flags, fvf = words(body, 4, 1)[0], words(body, 40, 1)[0]
            print("  DRAW_STATE flags %#x vertices %d format %#x stencil %s" % (flags, words(body, 20, 1)[0], fvf, words(body, 1532, 8)))


def materials(path, wanted, geometry):
    known = {}
    geometries = {}
    seen = set()
    for frame, tag, body in records(path):
        if frame > wanted:
            break
        if tag == TAG_MATERIAL:
            known[words(body, 0, 1)[0]] = words(body, 4, (len(body) - 4) // 4)
        elif tag == TAG_GEOMETRY:
            geometries[words(body, 0, 1)[0]] = (words(body, 12, 1)[0], words(body, 20, 1)[0])
        if frame != wanted or tag != TAG_SHAPE:
            continue
        g, m, flags = words(body, 0, 3)
        if geometry is not None and g != geometry:
            continue
        if m in seen:
            continue
        seen.add(m)
        fvf, vertices = geometries.get(g, (0, 0))
        w = known.get(m)
        if not w:
            print("material %#x (geometry %#x): no record" % (m, g))
            continue
        maps = [(i, w[8 + i * 5], w[9 + i * 5], w[10 + i * 5], w[11 + i * 5]) for i in range(8) if w[12 + i * 5]]
        print("material %#x (geometry %#x, format %#x, %d vertices, record flags %#x): flags %#x blend %d/%d test %d ref %d apply %d decals %d" % (
            m, g, fvf, vertices, flags, w[1], w[2], w[3], w[4], w[5], w[6], w[7]))
        print("  maps (slot, texture, clamp, filter, set): %s" % maps)
        print("  vertex colour source %d lighting %d; z function %d; stencil %s; fog mode %d; environment map (%#x, clamp %d, filter %d, mode %d); fog map (%#x, %d, %d, mode %d); projected %d/%d" % (
            w[68], w[69], w[70], w[71:78], w[80], w[81], w[82], w[83], w[84], w[85], w[86], w[87], w[88], w[89], w[90]))


def particles(path, wanted):
    known = {}
    last = None
    for frame, tag, body in records(path):
        if frame > wanted:
            break
        if tag == TAG_MATERIAL:
            known[words(body, 0, 1)[0]] = words(body, 4, (len(body) - 4) // 4)
        if frame != wanted:
            continue
        if tag == TAG_DRAW_STATE and last is not None:
            print("  device material %s; lighting %d fog %d diffuse source %d emissive source %d" % (
                tuple(round(v, 3) for v in struct.unpack_from("<17f", body, 168)), *words(body, 148, 4)))
            last = None
            continue
        if tag != TAG_SHAPE or words(body, 8, 1)[0] & FLAG_PARTICLES == 0:
            continue
        g, m, flags, lights = words(body, 0, 4)
        tail, _, effects, fog = shape_tail(body)
        count, radius = struct.unpack_from("<If", body, tail)
        axes = struct.unpack_from("<6f", body, tail + 8)
        colours = struct.unpack_from("<14f", body, SHAPE_COLOURS)
        print("particles key %#x material %#x flags %#x lights %d count %d radius %.3f right %s up %s" % (
            g, m, flags, lights, count, radius, tuple(round(a, 3) for a in axes[:3]), tuple(round(a, 3) for a in axes[3:])))
        print("  material colours ambient %s diffuse %s emissive %s alpha %.2f; effects %s; fog %s" % (
            colours[0:3], colours[3:6], colours[9:12], colours[13], "yes" if effects is not None else "no", words(body, fog, 4) if fog is not None else "no"))
        for i in range(min(count, 3)):
            p = struct.unpack_from("<4fI", body, tail + 32 + i * 20)
            print("  particle at %s size %.3f colour %#x" % (tuple(round(v, 1) for v in p[:3]), p[3], p[4]))
        last = m


def calls(path):
    """The first frame of a capture file has the snapshot of the device objects that exist
    at its start (buffers, textures, surfaces) as creation calls: it is counted apart."""
    frames = 0
    counts = collections.Counter()
    snapshot = collections.Counter()
    commands = collections.Counter()
    released = collections.Counter()
    origin = {}
    for frame, tag, body in records(path):
        frames = frame + 1
        if tag == TAG_CALL:
            call, result, handle, nargs = words(body, 0, 4)
            name = CALLS.get(call, call)
            (snapshot if frame == 0 else counts)[name] += 1
            if call in (8, 9, 15, 20, 21, 22, 24, 25):
                origin[result] = name
            if call == 11 and frame != 0:
                released[origin.get(handle, "unknown")] += 1
        elif tag == TAG_COMMAND and frame != 0:
            commands[COMMANDS.get(words(body, 0, 1)[0], words(body, 0, 1)[0])] += 1

    def per_frame(counter):
        return sorted(((k, round(v / max(frames - 1, 1), 1)) for k, v in counter.items()), key=lambda kv: -kv[1])

    print("frames", frames)
    print("the first frame, with the snapshot of the device objects:", sorted(snapshot.items(), key=lambda kv: -kv[1]))
    print("calls for each frame after it:", per_frame(counts))
    print("releases for each frame, by what made the handle:", per_frame(released))
    print("commands for each frame:", per_frame(commands))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("capture")
    ap.add_argument("what", choices=["sequence", "materials", "particles", "calls"])
    ap.add_argument("frame", nargs="?", type=int, default=0)
    ap.add_argument("--geometry", type=lambda v: int(v, 0), default=None)
    args = ap.parse_args()
    if args.what == "sequence":
        sequence(args.capture, args.frame)
    elif args.what == "materials":
        materials(args.capture, args.frame, args.geometry)
    elif args.what == "particles":
        particles(args.capture, args.frame)
    else:
        calls(args.capture)
    return 0


if __name__ == "__main__":
    sys.exit(main())
