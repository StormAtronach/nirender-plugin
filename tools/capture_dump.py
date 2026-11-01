"""Reads the records of a capture, for the root cause of a difference of a picture of the
retained pipeline. A capture from before the cut of the device path (step R7b) also has the
state packets, the stage states and the draws of the device; the helper does not read them.

    python -I tools/capture_dump.py <capture.nircap> sequence <frame>
    python -I tools/capture_dump.py <capture.nircap> materials <frame> [--geometry 0x...]
    python -I tools/capture_dump.py <capture.nircap> particles <frame>
    python -I tools/capture_dump.py <capture.nircap> calls
    python -I tools/capture_dump.py <capture.nircap> ui <frame> [--box x0 y0 x1 y1]
    python -I tools/capture_dump.py <capture.nircap> ui-geometry <frame> --shapes 3,5
    python -I tools/capture_dump.py <capture.nircap> ui-textures <frame>
    python -I tools/capture_dump.py <capture.nircap> first-draw

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
ui         the interface shapes of the frame in draw order, each with its number, name,
           rectangle on the screen, texture and state, for a picture that a menu lacks. The
           rectangle: the geometry's vertices through the world matrix; the screen's x is
           X + 960 and its y is 1140 - Z (the interface camera of a 1920 x 1080 picture).
           `--box` keeps the shapes that touch a rectangle of the screen.
ui-geometry  the shapes of those numbers: the world matrix, and each GEOMETRY record of
           their geometries in the capture, with the vertices.
ui-textures  the TEXTURE records by frame, and the interface shapes of the frame by their
           base texture, or "NOT IN THE CAPTURE" for a handle with no record.
first-draw the frames whose first draw into the back buffer comes before a colour clear of
           it: such a frame draws over the picture of the frame before.

The frames count from 0, as `replay.py --frame` does.
"""
import argparse
import collections
import struct
import sys

TAG_OBJECT, TAG_DRAW_STATE, TAG_COMMAND, TAG_CALL, TAG_FRAME_END = 6, 7, 8, 9, 10
TAG_SCENE_BEGIN, TAG_SCENE_END, TAG_GEOMETRY, TAG_MATERIAL, TAG_SHAPE = 3, 4, 18, 19, 20
TAG_INSTANCE, TAG_INSTANCE_DRAW, TAG_INSTANCE_DELETE = 23, 24, 25
TAG_INSTANCE_LIGHTS, TAG_INSTANCE_STATE, TAG_RETAINED_SCENE = 26, 27, 28
TAG_LIGHT = 21
TAG_TEXTURE, TAG_INSTANCE_PARTICLES = 1, 35
# SCENE_BEGIN's kind of the interface, and the calls `first-draw` follows.
SCENE_INTERFACE = 7
CALL_CLEAR, CALL_RENDER_TARGET, CALL_TEXTURE_LEVEL, CALL_CREATE_TEXTURE = 4, 7, 15, 20
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
            # Flag 1: the engine's sorted queue stays open after the scene. An older capture has no word.
            flags = words(body, 0, 1)[0] if len(body) >= 4 else 0
            print("SCENE_END%s" % (" queue open" if flags & 1 else ""))
        elif tag == TAG_OBJECT:
            kind, cls, flags = words(body, 0, 3)
            print("OBJECT kind %d class %d flags %#x %r" % (kind, cls, flags, object_name(body)[:40]))
        elif tag == TAG_SHAPE:
            g, m, flags, lights = words(body, 0, 4)
            print("  SHAPE geometry %#x material %#x flags %#x lights %d%s" % (g, m, flags, lights, " twin" if flags & 0x200 else ""))
        elif tag == TAG_INSTANCE:
            i, cls, flags, g, m = words(body, 0, 5)
            name = body[176:]
            print("INSTANCE %d class %d flags %#x geometry %#x material %#x %r" % (i, cls, flags, g, m, name.decode("latin-1")[:40]))
        elif tag == TAG_INSTANCE_LIGHTS:
            i, lights = words(body, 0, 2)
            print("  INSTANCE_LIGHTS %d lights %d" % (i, lights))
        elif tag == TAG_INSTANCE_STATE:
            i, visible = words(body, 0, 2)
            print("INSTANCE_STATE %d visible %d" % (i, visible))
        elif tag == TAG_RETAINED_SCENE:
            ambient, flags = words(body, 0, 2)
            print("RETAINED_SCENE ambient %#x flags %#x" % (ambient, flags))
        elif tag == TAG_INSTANCE_DRAW:
            i, ambient, flags = words(body, 0, 3)
            lights = words(body, 12 + (144 if flags & 16 else 0) + (16 if flags & 32 else 0), 1)[0]
            print("  INSTANCE_DRAW %d flags %#x lights %d" % (i, flags, lights))
        elif tag == TAG_INSTANCE_DELETE:
            print("INSTANCE_DELETE %d" % words(body, 0, 1)[0])
        elif tag == TAG_LIGHT:
            key = words(body, 0, 1)[0]
            values = words(body, 4, 26)
            print("LIGHT %d values %08x" % (key, hash(values) & 0xFFFFFFFF))
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


def interface_shapes(path, wanted):
    """(number, body, name) for each interface shape of the frame, from 1, in draw order, and
    the tables of the records up to the frame's end: the textures (width, height, levels,
    format), the materials and the geometries, each as the frame last had it."""
    shapes, textures, material_words, geometries = [], {}, {}, {}
    kind, name = None, ""
    for frame, tag, body in records(path):
        if frame > wanted:
            break
        if tag == TAG_TEXTURE and len(body) >= 24:
            textures[words(body, 0, 1)[0]] = words(body, 8, 4)
        elif tag == TAG_SCENE_BEGIN and len(body) >= 8:
            kind = words(body, 4, 1)[0]
        elif tag == TAG_SCENE_END:
            kind = None
        elif tag == TAG_OBJECT and len(body) >= 12:
            name = object_name(body).split("\0")[0]
        elif tag == TAG_GEOMETRY and len(body) >= 32:
            fvf, stride, vertices, _entries, strips = words(body, 12, 5)
            vertex_at = 32 + ((strips * 2 + 3) & ~3)
            points = [struct.unpack_from("<3f", body, vertex_at + i * stride) for i in range(vertices) if vertex_at + i * stride + 12 <= len(body)]
            geometries[words(body, 0, 1)[0]] = points
        elif tag == TAG_MATERIAL and len(body) >= 4 + 70 * 4:
            material_words[words(body, 0, 1)[0]] = words(body, 4, 70)
        elif tag == TAG_SHAPE and kind == SCENE_INTERFACE and frame == wanted and len(body) >= 140:
            shapes.append((len(shapes) + 1, body, name))
    return shapes, textures, material_words, geometries


def base_texture(material_words):
    """The handle of the material's base map, or None without one."""
    return material_words[8] if material_words and material_words[12] else None


def ui(path, wanted, box):
    shapes, textures, material_words, geometries = interface_shapes(path, wanted)
    for number, body, name in shapes:
        geometry, material, flags = words(body, 0, 3)
        world = struct.unpack_from("<16f", body, SHAPE_WORLD)
        points = geometries.get(geometry, [])
        xs = [p[0] * world[0] + p[1] * world[4] + p[2] * world[8] + world[12] for p in points]
        zs = [p[0] * world[2] + p[1] * world[6] + p[2] * world[10] + world[14] for p in points]
        rect = (min(xs) + 960, 1140 - max(zs), max(xs) + 960, 1140 - min(zs)) if xs else (0, 0, 0, 0)
        if box and (rect[2] < box[0] or rect[0] > box[2] or rect[3] < box[1] or rect[1] > box[3]):
            continue
        state = material_words.get(material)
        base = base_texture(state)
        size = textures.get(base)
        texture = "none" if base is None else "0x%x %s" % (base, "%dx%d L%d fmt 0x%x" % size if size else "NOT IN THE CAPTURE")
        look = "flags 0x%x blend %d/%d apply %d vert %d light %d" % (state[1], state[2], state[3], state[6], state[68], state[69]) if state else "no material"
        alpha = struct.unpack_from("<14f", body, SHAPE_COLOURS)[13]
        print("#%d %-28s [%.0f,%.0f - %.0f,%.0f] verts %d shape 0x%x tex %s %s alpha %.3f" % (number, name[:28], *rect, len(points), flags, texture, look, alpha))


def ui_geometry(path, wanted, numbers):
    shapes = interface_shapes(path, wanted)[0]
    handles = {}
    for number, body, _name in shapes:
        if number in numbers:
            geometry, material = words(body, 0, 2)
            handles[geometry] = number
            w = struct.unpack_from("<16f", body, SHAPE_WORLD)
            print("shape #%d geometry 0x%x material 0x%x world x %.3f %.3f %.3f y %.3f %.3f %.3f z %.3f %.3f %.3f t %.1f %.1f %.1f" % (number, geometry, material, w[0], w[1], w[2], w[4], w[5], w[6], w[8], w[9], w[10], w[12], w[13], w[14]))
    for frame, tag, body in records(path):
        if tag == TAG_GEOMETRY and len(body) >= 32 and words(body, 0, 1)[0] in handles:
            geometry = words(body, 0, 1)[0]
            fvf, stride, vertices, entries, strips = words(body, 12, 5)
            print("frame %d GEOMETRY 0x%x (shape #%d) fvf 0x%x stride %d vertices %d entries %d strips %d" % (frame, geometry, handles[geometry], fvf, stride, vertices, entries, strips))
            vertex_at = 32 + ((strips * 2 + 3) & ~3)
            for i in range(vertices):
                vertex = struct.unpack_from("<%df" % (stride // 4), body, vertex_at + i * stride)
                print("   ", " ".join("%.3f" % value for value in vertex))


def ui_textures(path, wanted):
    by_frame = collections.Counter(frame for frame, tag, body in records(path) if tag == TAG_TEXTURE and len(body) >= 24)
    print("TEXTURE records by frame:", dict(sorted(by_frame.items())))
    shapes, textures, material_words, _geometries = interface_shapes(path, wanted)
    print("interface shapes of frame %d: %d" % (wanted, len(shapes)))
    groups = {}
    for _number, body, _name in shapes:
        geometry, material = words(body, 0, 2)
        base = base_texture(material_words.get(material))
        size = textures.get(base)
        key = "no base map" if base is None else "texture 0x%x %s" % (base, "%dx%d levels %d format 0x%x" % size if size else "NOT IN THE CAPTURE")
        groups.setdefault(key, []).append((geometry, struct.unpack_from("<16f", body, SHAPE_WORLD)))
    for key, group in sorted(groups.items(), key=lambda item: -len(item[1])):
        geometry, world = group[0]
        print("  %4d x %s; first geometry 0x%x at %.1f, %.1f scale %.2f" % (len(group), key, geometry, world[12], world[13], world[0]))


def first_draw(path):
    surfaces, targets = {}, set()
    back_buffer = True
    first = {}
    for frame, tag, body in records(path):
        if tag == TAG_CALL and len(body) >= 16:
            call, _result, handle = words(body, 0, 3)
            arguments = body[16:]
            if call == CALL_CREATE_TEXTURE and len(arguments) >= 16 and handle and words(arguments, 12, 1)[0] & 1:
                targets.add(handle)
            elif call == CALL_TEXTURE_LEVEL and len(arguments) >= 8 and handle and words(arguments, 4, 1)[0] == 0:
                surfaces[handle] = words(arguments, 0, 1)[0]
            elif call == CALL_RENDER_TARGET and len(arguments) >= 4:
                back_buffer = surfaces.get(words(arguments, 0, 1)[0]) not in targets
            elif call == CALL_CLEAR and back_buffer and len(arguments) >= 12 and words(arguments, 8, 1)[0] & 1:
                first.setdefault(frame, "clear")
        elif tag in (TAG_SHAPE, TAG_INSTANCE_DRAW, TAG_INSTANCE_PARTICLES) and back_buffer:
            first.setdefault(frame, "draw")
    drawn_first = sorted(frame for frame, what in first.items() if what == "draw")
    print("%d frames with a clear or a draw, %d start with a draw into the back buffer%s" % (len(first), len(drawn_first), ": frames " + ", ".join(map(str, drawn_first[:12])) if drawn_first else ""))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("capture")
    ap.add_argument("what", choices=["sequence", "materials", "particles", "calls", "ui", "ui-geometry", "ui-textures", "first-draw"])
    ap.add_argument("frame", nargs="?", type=int, default=0)
    ap.add_argument("--geometry", type=lambda v: int(v, 0), default=None)
    ap.add_argument("--box", nargs=4, type=float, default=None, metavar=("X0", "Y0", "X1", "Y1"))
    ap.add_argument("--shapes", default="")
    args = ap.parse_args()
    if args.what == "sequence":
        sequence(args.capture, args.frame)
    elif args.what == "materials":
        materials(args.capture, args.frame, args.geometry)
    elif args.what == "particles":
        particles(args.capture, args.frame)
    elif args.what == "ui":
        ui(args.capture, args.frame, args.box)
    elif args.what == "ui-geometry":
        ui_geometry(args.capture, args.frame, {int(number) for number in args.shapes.split(",") if number})
    elif args.what == "ui-textures":
        ui_textures(args.capture, args.frame)
    elif args.what == "first-draw":
        first_draw(args.capture)
    else:
        calls(args.capture)
    return 0


if __name__ == "__main__":
    sys.exit(main())
