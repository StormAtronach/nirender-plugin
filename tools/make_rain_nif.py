"""Makes a particle system that replicates the rain of the game (the owner, 2026-11-02).

    python tools/make_rain_nif.py --out <folder> [--streaks-per-second N] [--lifespan S] ...

Writes `<folder>/meshes/rn/rain.nif` and `<folder>/textures/rn/rain_streak.dds`. Every value is an
option, with the default the engine or the shipped Morrowind.ini gives.

The engine makes no particle system for rain. `WeatherParticleRaindrop::update` 0x451D60 moves each
drop by hand, and each drop is a clone of the one shape of Raindrop.nif: 40 vertical quads, 1.6 wide
and 32.4 high, spread about the drop's axis. This file gives each streak its own particle:

    root            NiNode "Rain"
      "Rain Emitter"  NiNode at the height of the rain. A mod moves this node to make the rain
                      follow something.
      "Rain Drops"    NiBSParticleNode without the follow flag: the particles stay in the world
        "Rain Streaks"  NiRotatingParticles with a NiParticleSystemController

A particle is a square that faces the camera (`NiDX8Renderer::RenderPoints` 0x6AD0A0), so the streak
is in the texture: a thin line in the middle of a square of its height.
"""
import argparse
import math
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import nif40  # noqa: E402

IDENTITY = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
ZERO3 = [0.0, 0.0, 0.0]
# With the folder: without it the engine does not find a file below Textures (Warnings.txt,
# 2026-11-02: tex not found "rn\rain_streak.dds", and the quads drew plain white).
TEXTURE_NAME = "textures\\rn\\rain_streak.dds"

# Tri Raindrop 0 of Raindrop.nif, read with tools/nif40.py: the height of a quad, and the width of
# one of the 8 texels across it (the quad is 1.6 wide, its coordinates run from -0.143 to 1.143).
STREAK_HEIGHT = 32.4
SOURCE_TEXEL_WIDTH = 1.6 / (1.143 + 0.143) / 8.0
# The centres of the 40 quads above the drop's origin, and their spread across it (root mean square).
CLUSTER_CENTRE_LOW = 46.4
CLUSTER_CENTRE_HIGH = 226.2
CLUSTER_RMS = (53.7, 60.7)
# Alpha by row of Tx_Raindrop_01.tga (8 x 32), from the bottom of the streak to its top: the column
# in the middle, and the mean of the columns on each side of it.
ALPHA_MIDDLE = [0, 20, 104, 99, 94, 93, 92, 90, 87, 84, 83, 81, 79, 78, 73, 66, 61, 54, 51, 47, 43, 38, 34, 30, 24, 22, 17, 15, 8, 1, 0, 0]
ALPHA_SIDE = [0, 10, 40, 41, 43, 40, 38, 38, 36, 34, 32, 31, 30, 26, 23, 18, 15, 10, 8, 5, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]


def arguments():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, help="the folder that gets meshes/rn and textures/rn")
    ap.add_argument("--rain-diameter", type=float, default=600.0, help="[Weather Rain] Rain Diameter of Morrowind.ini")
    ap.add_argument("--rain-height-min", type=float, default=200.0, help="[Weather Rain] Rain Height Min")
    ap.add_argument("--rain-height-max", type=float, default=700.0, help="[Weather Rain] Rain Height Max")
    ap.add_argument("--precip-gravity", type=float, default=575.0, help="[Weather] Precip Gravity: the fall in units a second, constant")
    ap.add_argument("--wind", type=float, default=2.4, help="the sideways speed along +Y in units a second: [Weather Rain] Wind Speed times 8")
    ap.add_argument("--streaks-per-second", type=float, default=1714.0,
                    help="the birth rate of the streaks: 40 for each drop the engine starts in a second, which depends on the frame rate")
    ap.add_argument("--lifespan", type=float, default=2.0, help="the seconds of a streak; the depth test hides what falls through the ground")
    ap.add_argument("--texture-size", type=int, default=256, help="the side of the streak's texture, a power of 2; at 128 a near streak is twice the engine's width")
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


def streak_texture(size):
    """The top level first, then the mipmaps, each as rows of alpha. Row 0 is the bottom of the
    streak: a particle's quad has v = 0 at the bottom of the screen (`RenderPoints`)."""
    world_per_texel = STREAK_HEIGHT / size
    level = []
    for y in range(size):
        row = (y + 0.5) / size * len(ALPHA_MIDDLE)
        low = min(int(row), len(ALPHA_MIDDLE) - 1)
        for x in range(size):
            # The distance from the middle of the streak in texels of the game's texture.
            across = abs((x + 0.5) * world_per_texel - STREAK_HEIGHT / 2.0) / SOURCE_TEXEL_WIDTH
            if across <= 1.0:
                alpha = ALPHA_MIDDLE[low] + (ALPHA_SIDE[low] - ALPHA_MIDDLE[low]) * across
            elif across <= 2.0:
                alpha = ALPHA_SIDE[low] * (2.0 - across)
            else:
                alpha = 0.0
            level.append(alpha)
    levels = [(size, level)]
    width = size
    while width > 1:
        half = width // 2
        smaller = []
        for y in range(half):
            for x in range(half):
                smaller.append(sum(level[(2 * y + dy) * width + 2 * x + dx] for dy in (0, 1) for dx in (0, 1)) / 4.0)
        level, width = smaller, half
        levels.append((width, level))
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


def rain_blocks(args):
    """The blocks of the file. The order of the values follows Blizzard.nif of the game, the
    nearest thing it has: emitter and particle node beside each other under the root."""
    streak_radius = STREAK_HEIGHT / 2.0
    pool = int(math.ceil(args.streaks_per_second * args.lifespan)) + 2
    if pool > 65535:
        raise SystemExit("%d particles: a particle count is 16 bits" % pool)

    # A drop starts at its height above the player; a streak is its place in the cluster above that.
    lowest = args.rain_height_min + CLUSTER_CENTRE_LOW
    highest = args.rain_height_max + CLUSTER_CENTRE_HIGH
    # The drops are even in a square, and the cluster adds its spread (variances add).
    width_x = math.sqrt(args.rain_diameter ** 2 + 12.0 * CLUSTER_RMS[0] ** 2)
    width_y = math.sqrt(args.rain_diameter ** 2 + 12.0 * CLUSTER_RMS[1] ** 2)

    # The speed and the declination from straight down: the engine's velocity is the wind along +Y
    # plus the gravity along -Z, set again each frame, so a drop does not speed up.
    speed = math.hypot(args.precip_gravity, args.wind)
    declination = math.pi - math.atan2(args.wind, args.precip_gravity)

    blocks = []

    def add(block):
        blocks.append(block)
        return len(blocks) - 1

    root = add(node("NiNode", "Rain", 10))
    emitter = add(node("NiNode", "Rain Emitter", 162))
    blocks[emitter]["translation"] = [0.0, 0.0, (lowest + highest) / 2.0]
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
        "birth_rate": args.streaks_per_second, "lifespan": args.lifespan, "lifespan_variance": 0.0, "use_birth_rate": 1, "spawn_on_death": 0,
        "emitter_width": width_x, "emitter_height": width_y, "emitter_depth": highest - lowest, "emitter": emitter,
        "spawn_generations": 0, "spawn_percentage": 1.0, "spawn_multiplier": 1, "spawned_speed_chaos": 0.0, "spawned_direction_chaos": 0.0,
        "particles": {"active": 0, "items": [{"velocity": list(ZERO3), "rotation_axis": list(ZERO3), "age": 0.0, "lifespan": 0.0, "last_update": 0.0,
                                              "generation": 0, "index": 0} for _ in range(pool)]},
        "emitter_modifier": -1, "modifiers": -1, "colliders": -1, "static_bound": 0})

    # The rain's look: blended (237 is the alpha of Raindrop.nif), the emissive white of
    # `loadPrecipitationParticles`, the vertex colours off and only the emissive lit.
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
    blocks[material].update({"ambient": [1.0, 1.0, 1.0], "diffuse": [1.0, 1.0, 1.0], "specular": list(ZERO3), "emissive": [1.0, 1.0, 1.0], "glossiness": 0.0, "alpha": 1.0})
    colors = add(one_flag("NiVertexColorProperty", 0))
    blocks[colors].update({"vertex_mode": 0, "lighting_mode": 0})
    blocks[shape]["properties"] = [alpha, texturing, material, colors]

    data = add({"type": "NiRotatingParticlesData", "particledata": {
        "count": pool, "has_vertices": 1, "vertices": [list(ZERO3)] * pool, "has_normals": 0, "normals": [], "center": list(ZERO3), "radius": 0.0,
        "has_colors": 0, "colors": [], "uv_set_count": 0, "has_uv": 0, "uv_sets": [], "triangle_count": pool, "particle_radius": streak_radius,
        "active": 0, "has_sizes": 0, "sizes": [], "has_rotations": 0, "rotations": []}})
    blocks[shape].update({"data": data, "skin": -1})
    return blocks, root


def main():
    args = arguments()
    blocks, root = rain_blocks(args)
    data = nif40.write(blocks, [root])
    back, roots = nif40.read(data)
    assert nif40.write(back, roots) == data

    mesh = os.path.join(args.out, "meshes", "rn", "rain.nif")
    os.makedirs(os.path.dirname(mesh), exist_ok=True)
    with open(mesh, "wb") as fh:
        fh.write(data)
    texture = os.path.join(args.out, "textures", "rn", "rain_streak.dds")
    write_dds(texture, streak_texture(args.texture_size))
    assert len(TEXTURE_NAME) <= 31

    controller = next(b for b in blocks if b["type"] == "NiParticleSystemController")
    print("wrote", mesh, os.path.getsize(mesh), "bytes,", len(controller["particles"]["items"]), "particles")
    print("wrote", texture, os.path.getsize(texture), "bytes")
    print("emitter %.0f x %.0f x %.0f at height %.0f, speed %.1f, declination %.4f" % (
        controller["emitter_width"], controller["emitter_height"], controller["emitter_depth"],
        blocks[controller["emitter"]]["translation"][2], controller["speed"], controller["declination"]))


if __name__ == "__main__":
    main()
