"""The inventory of a capture, for the material model of the retained pipeline (R0).

    python tools/inventory.py <capture.nircap>...

For each capture: the draws and their flags, the distinct material keys (the state of a
draw without its buffers and matrices), the stage operations and texture coordinate
generators in use, the blend and alpha test and stencil states, the vertex formats, the
lights for each draw, and the buffers: those written once (static) and those written in
many frames (dynamic), with the names of the objects that draw with the dynamic ones.
"""
import collections
import struct
import sys

STAGE_STATES = [1, 2, 3, 4, 5, 6, 26, 27, 28, 11, 24, 7, 8, 9, 10, 22, 23]
STAGE_INDEX = {state: i for i, state in enumerate(STAGE_STATES)}
STAGES, STAGE_COUNT, SAMPLER_COUNT = 8, 17, 5
TAG = {1: "TEXTURE", 2: "BUFFER_DATA", 3: "SCENE_BEGIN", 4: "SCENE_END", 5: "FRAME_EVENT", 6: "OBJECT", 7: "DRAW_STATE",
       8: "COMMAND", 9: "CALL", 10: "FRAME_END", 11: "HEADER", 12: "TEXTURE_UPDATE", 13: "READ_PIXELS", 14: "SHOT",
       15: "DRAW_UP", 16: "EXTENSION", 17: "MARK"}
COLOROP = {1: "DISABLE", 2: "SELECTARG1", 3: "SELECTARG2", 4: "MODULATE", 5: "MODULATE2X", 6: "MODULATE4X", 7: "ADD",
           8: "ADDSIGNED", 9: "ADDSIGNED2X", 10: "SUBTRACT", 11: "ADDSMOOTH", 12: "BLENDDIFFUSEALPHA", 13: "BLENDTEXTUREALPHA",
           14: "BLENDFACTORALPHA", 15: "BLENDTEXTUREALPHAPM", 16: "BLENDCURRENTALPHA", 17: "PREMODULATE",
           18: "MODULATEALPHA_ADDCOLOR", 19: "MODULATECOLOR_ADDALPHA", 20: "MODULATEINVALPHA_ADDCOLOR",
           21: "MODULATEINVCOLOR_ADDALPHA", 22: "BUMPENVMAP", 23: "BUMPENVMAPLUMINANCE", 24: "DOTPRODUCT3",
           25: "MULTIPLYADD", 26: "LERP"}
ARG = {0: "DIFFUSE", 1: "CURRENT", 2: "TEXTURE", 3: "TFACTOR", 4: "SPECULAR", 5: "TEMP"}
BLEND = {1: "ZERO", 2: "ONE", 3: "SRCCOLOR", 4: "INVSRCCOLOR", 5: "SRCALPHA", 6: "INVSRCALPHA", 7: "DESTALPHA",
         8: "INVDESTALPHA", 9: "DESTCOLOR", 10: "INVDESTCOLOR", 11: "SRCALPHASAT"}
TCI = {0x00000: "set", 0x10000: "CAMERASPACENORMAL", 0x20000: "CAMERASPACEPOSITION", 0x30000: "CAMERASPACEREFLECTIONVECTOR"}

# The offsets in DrawStateV1 (1728 bytes).
O_FLAGS, O_PRIM, O_VCOUNT, O_PCOUNT, O_VB, O_STRIDE, O_FVF, O_IB = 4, 8, 20, 28, 32, 36, 40, 44
O_DEPTH_WRITE, O_CULL, O_VBLEND, O_BLEND, O_SRC, O_DST, O_ATEST, O_AFUNC, O_AREF, O_LIGHTING, O_FOG, O_DIFFSRC, O_EMISRC, O_AMBIENT = range(112, 168, 4)
O_MATERIAL, O_LIGHT_COUNT, O_STAGES, O_TEXTURES, O_TEXTURE_TRANSFORMS = 168, 236, 244, 980, 1012
O_DEPTH_ENABLE, O_DEPTH_FUNC, O_STENCIL, O_SAMPLERS, O_RELATIVE = 1524, 1528, 1532, 1564, 1724
PACKET = 1728


def words(data, at, count):
    return struct.unpack_from("<%dI" % count, data, at)


def arg_name(value):
    base = ARG.get(value & 0xF, str(value & 0xF))
    if value & 0x10:
        base += "~"
    if value & 0x20:
        base += ".a"
    return base


def inventory(path):
    data = open(path, "rb").read()
    assert data[:8] == b"NIRCAP01", path
    at = 8
    frames = 0
    tags = collections.Counter()
    draws = skinned = later = 0
    keys, pipeline_keys = collections.Counter(), collections.Counter()
    stage_counts = collections.Counter()
    ops = collections.Counter()
    tci = collections.Counter()
    transforms = 0
    bump = 0
    blends = collections.Counter()
    alpha_tests = collections.Counter()
    stencil_on = 0
    fvfs = collections.Counter()
    lights = collections.Counter()
    no_lighting = no_fog = 0
    vertex_blend = collections.Counter()
    buffer_frames = collections.defaultdict(set)
    buffer_bytes = collections.Counter()
    buffer_object = {}
    creates = collections.Counter()
    create_bytes = collections.Counter()
    textures = 0
    texture_bytes = 0
    objects = collections.Counter()
    classes = collections.Counter()
    current_object = ""
    while at + 8 <= len(data):
        tag, length = struct.unpack_from("<II", data, at)
        body = data[at + 8:at + 8 + length]
        at += 8 + length
        tags[TAG.get(tag, tag)] += 1
        if tag == 10:
            frames += 1
        elif tag == 6:
            kind, cls, flags = words(body, 0, 3)
            objects[kind] += 1
            classes[cls] += 1
            # With the flags 0x100 or 0x200 two more words (the geometry and material
            # handles) are before the name.
            current_object = body[24 if flags & 0x300 else 16:].decode("latin-1", errors="replace")
            # A draw without a geometry object has no name: the particles (kind 3), the
            # lines (4) and the screen polygons (5) are named by their kind.
            if not current_object:
                current_object = {3: "(particles)", 4: "(lines)", 5: "(screen polygon)"}.get(kind, "(kind %d)" % kind)
        elif tag == 1:
            textures += 1
            texture_bytes += length
        elif tag == 2:
            handle, offset, size = words(body, 0, 3)
            buffer_frames[handle].add(frames)
            buffer_bytes[handle] += size
        elif tag == 9:
            call, result, handle, nargs = words(body, 0, 4)
            if call in (8, 9, 20):
                creates[call] += 1
                if call in (8, 9):
                    create_bytes[call] += words(body, 16, 1)[0]
        elif tag == 7 and length >= PACKET:
            draws += 1
            flags = words(body, O_FLAGS, 1)[0]
            skinned += flags & 1 != 0
            later += flags & 2 != 0
            vb, ib = words(body, O_VB, 1)[0], words(body, O_IB, 1)[0]
            buffer_object.setdefault(vb, current_object)
            if ib:
                buffer_object.setdefault(ib, current_object + " (indexes)")
            fvfs[words(body, O_FVF, 1)[0]] += 1
            render = words(body, O_DEPTH_WRITE, 14)
            depth_write, cull, vblend, blend, src, dst, atest, afunc, aref, lighting, fog, diffsrc, emisrc, ambient = render
            vertex_blend[vblend] += 1
            if blend:
                blends[(BLEND.get(src, src), BLEND.get(dst, dst))] += 1
            if atest:
                alpha_tests[(afunc, aref)] += 1
            no_lighting += lighting == 0
            no_fog += fog == 0
            material = struct.unpack_from("<17f", body, O_MATERIAL)
            light_count = words(body, O_LIGHT_COUNT, 1)[0]
            lights[light_count] += 1
            stages = [words(body, O_STAGES + s * STAGE_COUNT * 4, STAGE_COUNT) for s in range(STAGES)]
            used = 0
            stage_key = []
            for s, st in enumerate(stages):
                colorop = st[STAGE_INDEX[1]]
                if colorop == 1 or colorop == 0x7FFFFFFF:
                    break
                used += 1
                ops[("color", COLOROP.get(colorop, colorop))] += 1
                ops[("alpha", COLOROP.get(st[STAGE_INDEX[4]], st[STAGE_INDEX[4]]))] += 1
                index = st[STAGE_INDEX[11]]
                if index != 0x7FFFFFFF:
                    tci[TCI.get(index & 0xFFFF0000, hex(index & 0xFFFF0000))] += 1
                flags_t = st[STAGE_INDEX[24]]
                if flags_t not in (0, 0x7FFFFFFF):
                    transforms += 1
                if colorop in (22, 23):
                    bump += 1
                stage_key.append((COLOROP.get(colorop, colorop), arg_name(st[STAGE_INDEX[2]]), arg_name(st[STAGE_INDEX[3]]),
                                  COLOROP.get(st[STAGE_INDEX[4]], st[STAGE_INDEX[4]]), arg_name(st[STAGE_INDEX[5]]), arg_name(st[STAGE_INDEX[6]]),
                                  index, flags_t))
            stage_counts[used] += 1
            depth_enable, depth_func = words(body, O_DEPTH_ENABLE, 2)
            stencil = words(body, O_STENCIL, 8)
            stencil_on += stencil[0] == 1
            samplers = tuple(words(body, O_SAMPLERS + s * SAMPLER_COUNT * 4, SAMPLER_COUNT) for s in range(used))
            pipeline = (tuple(stage_key), depth_write, depth_enable, depth_func, cull, blend, src, dst, atest, afunc, aref, lighting, fog,
                        diffsrc, emisrc, stencil, vblend)
            pipeline_keys[pipeline] += 1
            keys[(pipeline, samplers, tuple(round(v, 3) for v in material), ambient)] += 1
    print("== %s" % path)
    print("  frames %d, records %s" % (frames, dict(tags)))
    per = max(frames, 1)
    print("  draws %d (%.1f for each frame): skinned %d, later pass %d; lights for each draw %s" % (draws, draws / per, skinned, later, sorted(lights.items())))
    print("  distinct material keys %d, distinct pipeline keys (no material colours, no samplers) %d" % (len(keys), len(pipeline_keys)))
    print("  stages used %s; colour and alpha operations %s" % (sorted(stage_counts.items()), sorted(ops.items(), key=lambda kv: -kv[1])))
    print("  texture coordinates %s; texture transforms in use %d; bump stages %d" % (dict(tci), transforms, bump))
    print("  blends %s" % sorted(blends.items(), key=lambda kv: -kv[1]))
    print("  alpha tests (function, reference) %s; stencil on %d; lighting off %d; fog off %d; vertex blend %s" % (
        sorted(alpha_tests.items(), key=lambda kv: -kv[1]), stencil_on, no_lighting, no_fog, sorted(vertex_blend.items())))
    print("  vertex formats %s" % sorted(((hex(k), v) for k, v in fvfs.items()), key=lambda kv: -kv[1]))
    static = [h for h, f in buffer_frames.items() if len(f) <= 1]
    dynamic = [h for h, f in buffer_frames.items() if len(f) > 1]
    dynamic_bytes = sum(buffer_bytes[h] for h in dynamic)
    print("  buffers written: static %d (%.1f MB), dynamic %d (%.2f MB for each frame); made: vertex %d (%.1f MB), index %d (%.1f MB), textures %d (%d, %.1f MB)" % (
        len(static), sum(buffer_bytes[h] for h in static) / 1e6, len(dynamic), dynamic_bytes / 1e6 / per,
        creates[8], create_bytes[8] / 1e6, creates[9], create_bytes[9] / 1e6, creates[20], textures, texture_bytes / 1e6))
    names = collections.Counter()
    for h in dynamic:
        names[buffer_object.get(h, "?")] += buffer_bytes[h]
    print("  dynamic buffers by object (bytes in all): %s" % names.most_common(15))
    print("  objects by kind %s, by class %s" % (sorted(objects.items()), sorted(classes.items())))
    top = sorted(pipeline_keys.items(), key=lambda kv: -kv[1])[:12]
    print("  the most common pipeline keys:")
    for key, count in top:
        print("    %6d  stages %s depth(w %d, e %d, f %d) cull %d blend %d %s/%s atest %d/%d/%d light %d fog %d src %d/%d stencil %s vblend %d" % (
            count, key[0], key[1], key[2], key[3], key[4], key[5], BLEND.get(key[6], key[6]), BLEND.get(key[7], key[7]), key[8], key[9], key[10],
            key[11], key[12], key[13], key[14], key[15][0], key[16]))


if __name__ == "__main__":
    for path in sys.argv[1:]:
        inventory(path)
