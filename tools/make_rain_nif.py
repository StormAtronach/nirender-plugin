"""Makes placeable rain: a mesh that a mod author puts outside a window or a door, so that a player in
an interior sees rain outside. It does not replace the weather's rain (the owner, 2026-11-02).

    python tools/make_rain_nif.py --out <folder> [--box NAME WIDTH DEPTH HEIGHT] [--density N] ...

Writes `<folder>/meshes/rn/rain_<NAME>.nif` for each box and `<folder>/textures/rn/rain_streak.dds`.
Without `--box` it writes the sizes of `BOXES`. The rate follows the area of the box, so that all
sizes have the same look.

The rain does not depend on the frame rate. `NiParticleSystemController::UpdateParticles` 0x709710
keeps the fraction of a birth for the next frame, and `ParticleUpdate` 0x709B20 moves each particle by
its speed times the frame's time.

The place of the reference is the middle of the box's floor. The streaks start at the top of the box,
in a band of `EMIT_BAND` units, and fall to the floor. The file has no way to stop a streak:

    - Put the top of the box below the roof.
    - Put the floor on the ground or below the sill. A streak ends up to `EMIT_BAND` + `FLOOR_MARGIN`
      units below the floor, and the ground or the wall hides that part.

The file holds live particles at every age, so that the rain falls at the first look.

    root            NiNode "Rain"
      "Rain Emitter"  NiNode at the top of the box
      "Rain Drops"    NiBSParticleNode without the follow flag: the particles stay in the world
        "Rain Streaks"  NiRotatingParticles with a NiParticleSystemController

A particle is a square that faces the camera (`NiDX8Renderer::RenderPoints` 0x6AD0A0), so the streak
is in the texture: a thin line in the middle of a square of its height.
"""
import argparse
import math
import os
import random
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import nif40  # noqa: E402

IDENTITY = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
ZERO3 = [0.0, 0.0, 0.0]
# With the folder: without it the engine does not find a file below Textures (Warnings.txt,
# 2026-11-02: tex not found "rn\rain_streak.dds", and the quads drew plain white).
TEXTURE_NAME = "textures\\rn\\rain_streak.dds"

# name, width (x), depth (y), height (z). The depth counts: a shallow box shows few streaks.
BOXES = [("window", 256.0, 256.0, 512.0), ("door", 384.0, 384.0, 640.0), ("courtyard", 1024.0, 1024.0, 1024.0)]

# All particles of one frame start from the emitter's box and move by that frame's time. A band of
# this height hides the steps down to 9 frames a second (575 units a second / 64).
EMIT_BAND = 64.0
# A particle dies up to one frame early. This margin lets it reach the floor down to 18 frames a second.
FLOOR_MARGIN = 32.0

# Tri Raindrop 0 of Raindrop.nif, read with tools/nif40.py: the height of a quad, and the width of
# one of the 8 texels across it (the quad is 1.6 wide, its coordinates run from -0.143 to 1.143).
STREAK_HEIGHT = 32.4
SOURCE_TEXEL_WIDTH = 1.6 / (1.143 + 0.143) / 8.0
# Alpha by row of Tx_Raindrop_01.tga (8 x 32), from the bottom of the streak to its top: the column
# in the middle, and the mean of the columns on each side of it.
ALPHA_MIDDLE = [0, 20, 104, 99, 94, 93, 92, 90, 87, 84, 83, 81, 79, 78, 73, 66, 61, 54, 51, 47, 43, 38, 34, 30, 24, 22, 17, 15, 8, 1, 0, 0]
ALPHA_SIDE = [0, 10, 40, 41, 43, 40, 38, 38, 36, 34, 32, 31, 30, 26, 23, 18, 15, 10, 8, 5, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
# Samples of the streak in each texel, across it and along it.
TEXEL_SAMPLES = 8


def arguments():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, help="the folder that gets meshes/rn and textures/rn")
    ap.add_argument("--box", nargs=4, action="append", metavar=("NAME", "WIDTH", "DEPTH", "HEIGHT"),
                    help="a box to write, in world units; can repeat. Without it: %s" % ", ".join(b[0] for b in BOXES))
    ap.add_argument("--density", type=float, default=0.005,
                    help="the look: streaks a second for each square unit of the box's floor. "
                         "0.005 is near the weather's rain at 60 frames a second, 0.012 at 355")
    ap.add_argument("--fall-speed", type=float, default=575.0, help="units a second, constant: [Weather] Precip Gravity of Morrowind.ini")
    ap.add_argument("--wind", type=float, default=2.4, help="the sideways speed along +Y in units a second: [Weather Rain] Wind Speed times 8")
    ap.add_argument("--brightness", type=float, default=1.0, help="the colour of the streaks: 1 is the white of the weather's rain")
    ap.add_argument("--texture-size", type=int, default=256, help="the side of the streak's texture, a power of 2; at 128 a near streak is twice the width of the weather's")
    return ap.parse_args()


def object_net(kind, name=""):
    return {"type": kind, "name": name, "extra_data": -1, "controller": -1}


def av_object(kind, name, flags):
    block = object_net(kind, name)
    block.update({"flags": flags, "translation": list(ZERO3), "rotation": list(IDENTITY), "scale": 1.0, "velocity": list(ZERO3), "properties": [], "bounds": None})
    return block


def node(kind, name, flags, children=()):
    block = av_object(kind, name, flags)
    block.update({"children": list(children), "effects": []})
    return block


def one_flag(kind, flags):
    block = object_net(kind)
    block["flags"] = flags
    return block


def streak_alpha(across, row):
    """The game's streak at `across` of its texels from the middle, in a row of its texture."""
    if across <= 1.0:
        return ALPHA_MIDDLE[row] + (ALPHA_SIDE[row] - ALPHA_MIDDLE[row]) * across
    if across <= 2.0:
        return ALPHA_SIDE[row] * (2.0 - across)
    return 0.0


def streak_level(width):
    """One level of the texture, as rows of alpha. Row 0 is the bottom of the streak: a particle's
    quad has v = 0 at the bottom of the screen (`RenderPoints`).

    The streak has its middle in the middle of a texel, at each level. In the middle of the square it
    falls between two texels, and a far streak then has two columns at half the alpha, wider and
    softer than the weather's. The shift is half a texel of that level."""
    world_per_texel = STREAK_HEIGHT / width
    centre = (width // 2 + 0.5) * world_per_texel
    reach = 2.0 * SOURCE_TEXEL_WIDTH
    first = max(0, int((centre - reach) / world_per_texel))
    last = min(width - 1, int((centre + reach) / world_per_texel))

    level = [0.0] * (width * width)
    for y in range(width):
        rows = [min(int((y + (i + 0.5) / TEXEL_SAMPLES) / width * len(ALPHA_MIDDLE)), len(ALPHA_MIDDLE) - 1) for i in range(TEXEL_SAMPLES)]
        for x in range(first, last + 1):
            total = 0.0
            for i in range(TEXEL_SAMPLES):
                across = abs((x + (i + 0.5) / TEXEL_SAMPLES) * world_per_texel - centre) / SOURCE_TEXEL_WIDTH
                total += sum(streak_alpha(across, row) for row in rows)
            level[y * width + x] = total / TEXEL_SAMPLES ** 2
    return level


def streak_texture(size):
    """The top level first, then the mipmaps."""
    levels = []
    width = size
    while width >= 1:
        levels.append((width, streak_level(width)))
        width //= 2
    return levels


def write_dds(path, levels):
    """Uncompressed A8R8G8B8: white, the streak in the alpha, as Tx_Raindrop_01 has it."""
    size = levels[0][0]
    # DDSD_CAPS, HEIGHT, WIDTH, PITCH, PIXELFORMAT and MIPMAPCOUNT; DDSCAPS_TEXTURE, COMPLEX and MIPMAP.
    header = struct.pack("<4s7I11I8I5I", b"DDS ", 124, 0x100F | 0x20000, size, size, size * 4, 0, len(levels), *([0] * 11),
                         32, 0x41, 0, 32, 0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000, 0x1000 | 0x8 | 0x400000, 0, 0, 0, 0)
    pixels = bytearray()
    for _, alphas in levels:
        for alpha in alphas:
            pixels += bytes((255, 255, 255, min(255, int(round(alpha)))))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as fh:
        fh.write(header + pixels)


def rain_blocks(args, name, width, depth, height):
    """The blocks of the file. The order of the values follows Blizzard.nif of the game: emitter and
    particle node beside each other under the root."""
    streak_radius = STREAK_HEIGHT / 2.0
    # The speed and the declination from straight down: the wind along +Y plus the fall along -Z.
    speed = math.hypot(args.fall_speed, args.wind)
    declination = math.pi - math.atan2(args.wind, args.fall_speed)

    birth_rate = args.density * width * depth
    lifespan = (height + FLOOR_MARGIN) / args.fall_speed
    pool = int(math.ceil(birth_rate * lifespan)) + 2
    if pool > 65535:
        raise SystemExit("%d particles: a particle count is 16 bits" % pool)

    blocks = []

    def add(block):
        blocks.append(block)
        return len(blocks) - 1

    root = add(node("NiNode", "Rain", 10))
    emitter = add(node("NiNode", "Rain Emitter", 162))
    blocks[emitter]["translation"] = [0.0, 0.0, height - EMIT_BAND / 2.0]
    drops = add(node("NiBSParticleNode", "Rain Drops", 42))
    blocks[root]["children"] = [emitter, drops]
    # z test without a write, as `loadPrecipitationParticles` 0x43C030 gives the rain root.
    blocks[drops]["properties"] = [add(one_flag("NiZBufferProperty", 1))]

    shape = add(av_object("NiRotatingParticles", "Rain Streaks", 2))
    blocks[drops]["children"] = [shape]
    controller = add(dict(type="NiParticleSystemController", next=-1, flags=8, frequency=1.0, phase=0.0, low_key=0.0, high_key=10.0, target=shape))
    blocks[shape]["controller"] = controller
    blocks[controller].update({
        "speed": speed, "speed_variation": 0.0, "declination": declination, "declination_variation": 0.0,
        "planar_angle": math.pi / 2.0, "planar_angle_variation": 0.0, "initial_normal": [1.0, 0.0, 0.0],
        "initial_color": [1.0, 1.0, 1.0, 1.0], "initial_size": streak_radius, "emit_start": 0.0, "emit_stop": 12.0, "reset_system": 0,
        "birth_rate": birth_rate, "lifespan": lifespan, "lifespan_variance": 0.0, "use_birth_rate": 1, "spawn_on_death": 0,
        "emitter_width": width, "emitter_height": depth, "emitter_depth": EMIT_BAND, "emitter": emitter,
        "spawn_generations": 0, "spawn_percentage": 1.0, "spawn_multiplier": 1, "spawned_speed_chaos": 0.0, "spawned_direction_chaos": 0.0,
        "particles": {"active": 0, "items": [{"velocity": list(ZERO3), "rotation_axis": list(ZERO3), "age": 0.0, "lifespan": 0.0, "last_update": 0.0,
                                              "generation": 0, "index": 0} for _ in range(pool)]},
        "emitter_modifier": -1, "modifiers": -1, "colliders": -1, "static_bound": 0})

    # The rain's look: blended (237 is the alpha of Raindrop.nif), an emissive colour as
    # `loadPrecipitationParticles` gives the weather's rain, the vertex colours off and only the emissive lit.
    source = len(blocks) + 2
    alpha = add(one_flag("NiAlphaProperty", 237))
    blocks[alpha]["threshold"] = 0
    texturing = add(one_flag("NiTexturingProperty", 0))
    maps = [None] * 7
    maps[0] = {"has": 1, "source": source, "clamp": 3, "filter": 2, "uv_set": 0, "ps2_l": 0, "ps2_k": -75, "unknown": 0}
    blocks[texturing].update({"apply_mode": 2, "textures": maps})
    texture = add(object_net("NiSourceTexture"))
    blocks[texture].update({"use_external": 1, "file_name": TEXTURE_NAME, "pixel_layout": 5, "use_mipmaps": 1, "alpha_format": 3, "is_static": 1})
    material = add(one_flag("NiMaterialProperty", 1))
    emissive = [args.brightness] * 3
    blocks[material].update({"ambient": [1.0, 1.0, 1.0], "diffuse": [1.0, 1.0, 1.0], "specular": list(ZERO3), "emissive": emissive, "glossiness": 0.0, "alpha": 1.0})
    colors = add(one_flag("NiVertexColorProperty", 0))
    blocks[colors].update({"vertex_mode": 0, "lighting_mode": 0})
    blocks[shape]["properties"] = [alpha, texturing, material, colors]

    data = add({"type": "NiRotatingParticlesData", "particledata": {
        "count": pool, "has_vertices": 1, "vertices": [list(ZERO3)] * pool, "has_normals": 0, "normals": [], "center": list(ZERO3), "radius": 0.0,
        "has_colors": 0, "colors": [], "uv_set_count": 0, "has_uv": 0, "uv_sets": [], "triangle_count": pool, "particle_radius": streak_radius,
        "active": 0, "has_sizes": 0, "sizes": [], "has_rotations": 0, "rotations": []}})
    blocks[shape].update({"data": data, "skin": -1})

    # The bound before the first update: in an interior the system starts only when this bound
    # has been in view once (seen in Addamasartus, 2026-11-02). At the origin with no radius, a
    # window's rain did not start while its floor was out of view.
    particles = blocks[data]["particledata"]
    particles["center"] = [0.0, 0.0, height / 2.0]
    particles["radius"] = math.sqrt(width ** 2 + depth ** 2 + height ** 2) / 2.0

    fill_with_rain(blocks[controller]["particles"], particles, birth_rate, width, depth, height, lifespan, speed, declination, name)
    return blocks, root


def fill_with_rain(states, particles, birth_rate, width, depth, height, lifespan, speed, declination, seed):
    """Live particles in the file, at every age of their life: the rain falls at the first look."""
    pick = random.Random(seed)
    count = int(birth_rate * lifespan)
    down = speed * math.cos(declination)
    along = speed * math.sin(declination)
    for i in range(count):
        age = pick.random() * lifespan
        states["items"][i].update({"velocity": [0.0, along, down], "age": age, "lifespan": lifespan, "index": i})
        particles["vertices"][i] = [(pick.random() - 0.5) * width, (pick.random() - 0.5) * depth + along * age,
                                    height - pick.random() * EMIT_BAND + down * age]
    states["active"] = count
    particles["active"] = count


def write_mesh(args, name, width, depth, height):
    blocks, root = rain_blocks(args, name, width, depth, height)
    data = nif40.write(blocks, [root])
    back, roots = nif40.read(data)
    assert nif40.write(back, roots) == data

    mesh = os.path.join(args.out, "meshes", "rn", "rain_%s.nif" % name)
    os.makedirs(os.path.dirname(mesh), exist_ok=True)
    with open(mesh, "wb") as fh:
        fh.write(data)

    controller = next(b for b in blocks if b["type"] == "NiParticleSystemController")
    print("%-10s box %.0f x %.0f x %.0f  %.0f streaks a second  life %.3f s  %d particles  %d bytes  %s" % (
        name, width, depth, height, controller["birth_rate"], controller["lifespan"], len(controller["particles"]["items"]), len(data), mesh))


def main():
    args = arguments()
    boxes = [(b[0], float(b[1]), float(b[2]), float(b[3])) for b in args.box] if args.box else BOXES
    for name, width, depth, height in boxes:
        write_mesh(args, name, width, depth, height)

    texture = os.path.join(args.out, "textures", "rn", "rain_streak.dds")
    write_dds(texture, streak_texture(args.texture_size))
    assert len(TEXTURE_NAME) <= 31
    print("wrote", texture, os.path.getsize(texture), "bytes")


if __name__ == "__main__":
    main()
