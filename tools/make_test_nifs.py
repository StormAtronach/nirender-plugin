"""Makes the test NIFs of the flag support (flag inventory check, 2026-10-16): one file for
each flag value the engine supports and the game's content does not use, so that the tour
of the NIF test level shows the two paths of the renderer on it.

    python tools/make_test_nifs.py

Writes `mod/meshes/@_nifx/<name>.nif` and the list `mod/MWSE/mods/nirender/test_nifs.lua`
(the id and the mesh of each), which the tour step of the test mod places in the first
cell of the tour with `--tour-extra`. Each file is a root node with one tri shape (a box or
a cylinder), a lit material, a base texture of the level, and the property under test:

    fog_z        NiFogProperty on, function Z_LINEAR (the control of fog_range)
    fog_range    NiFogProperty on, function RANGE_SQ
    shade_smooth NiShadeProperty smooth (the control of shade_flat), a cylinder
    shade_flat   NiShadeProperty flat, the same cylinder
    specular     NiSpecularProperty on, a white specular colour, glossiness 40
    env_sphere   NiTextureEffect environment map, SPHERE_MAP coordinates, the cylinder
    env_parallel NiTextureEffect environment map, WORLD_PARALLEL coordinates (the box
                 spans half the texture)
    env_perspective NiTextureEffect environment map, WORLD_PERSPECTIVE coordinates
    fog_map      NiTextureEffect fog map, SPHERE_MAP coordinates, the cylinder
    node_billboard a box under a NiBillboardNode (always facing the camera), 80 units
                 off the node's origin, so its place turns with the node
    node_switch  a NiSwitchNode with index 1: child 0 a cylinder (never shown), child 1
                 the box
    node_lod     a NiLODNode: the box up to 350 units, the cylinder beyond; the tour
                 stands at 300 and comes from farther away, so the level changes on
                 arrival
    node_bsp     a NiBSPNode with the plane x = 0: the box west of it, the cylinder
                 east
    node_sortadjust a NiSortAdjustNode with the mode 0 (inherit) over two boxes side by
                 side; a batch sets MWSE's mode 64 (ordered subtree) at run time
    skin_soft    two skinned boxes with MWSE's software skinning flag, each beside a plain
                 box at the place and the size the engine's `NiSkinInstance::Deform` gives
                 it: the edges of each pair line up only with every influence and the raw
                 weights (`make_skinned`)
    skin_hard    the same file without the flag (the control): the edges do not line up
    turn_b<m>_c<n> a NiTextureEffect under a NiBillboardNode of the mode m (0 to 2), with the
                 coordinate mode n (0 to 4); the shape comes first in the walk, beside the
                 billboard (the effect and the coordinate modes: host/src/effect_turn.rs)
    turn_under_b<m> the same with the shape under the billboard, turning with the effect
    turn_two     two billboards above the effect, a plain node between, the shape beside them:
                 the second has no geometry under it and no display, so it does not turn
    turn_two_under the same with the shape under the second billboard: both turn
    turn_fog     a fog map under a billboard
    turn_hidden  a billboard with the hidden flag: no display, no turn
    turn_switched a billboard that a switch node does not select: no display, no turn
    turn_selected a billboard that a switch node selects, no geometry under the switch: it turns
    ctrl_b<m>_c<n> a keyframe controller on the effect, with the billboard mode m (n: no
                 billboard) and the coordinate mode n
    ctrl_above_b<m> the controller on the node above the billboard, which moves the shape too
    ctrl_between the controller on the node between two billboards, the shape under the second
    ctrl_under_b1 the controller on the effect, the shape under the billboard with it

The files are written with `nif40.py`, whose layouts are proven by a byte-identical read and
write of the level's own files. The texture effect is attached as the level's exhibits do:
a child of the node and in its `effects` list.
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import nif40  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(REPO, "mod", "meshes", "@_nifx")
LIST = os.path.join(REPO, "mod", "MWSE", "mods", "nirender", "test_nifs.lua")
# A white base: the shading shows on it. 1233.dds (a white triangle on black) is the one
# texture of the level with a pattern, for the effect; as a base it made the shapes black.
BASE_TEXTURE = "textures\\@_nif\\blankwhite.dds"
# The effect texture is a checker this script writes (`write_checker`): the one patterned
# texture of the level (1233.dds, a white triangle on black) shows as bands on a face of the
# box, which a position maps along one line of it.
TEXTURES = os.path.join(REPO, "mod", "textures", "@_nifx")
EFFECT_TEXTURE = "textures\\@_nifx\\checker.dds"
IDENTITY = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
ZERO3 = [0.0, 0.0, 0.0]
# The usual mapping of a reflection vector onto a sphere map: half of x and y, plus a half.
SPHERE_MATRIX = [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 1.0]
SPHERE_TRANSLATION = [0.5, 0.5, 0.0]


def object_net(name=""):
    return {"name": name, "extra_data": -1, "controller": -1}


def av_object(name="", flags=0):
    b = object_net(name)
    b.update({"flags": flags, "translation": list(ZERO3), "rotation": list(IDENTITY), "scale": 1.0, "velocity": list(ZERO3), "properties": [], "bounds": None})
    return b


def node(name="", flags=8):
    b = av_object(name, flags)
    b.update({"type": "NiNode", "children": [], "effects": []})
    return b


def box(size):
    """Six faces of four vertices, a normal for each face, the texture over each face. The
    triangles are counter-clockwise seen from outside (the cross product of their edges points
    along the face normal); written the other way round before 2026-10-17, the game showed the
    inner faces."""
    h = size / 2.0
    faces = [
        ([-h, -h, -h], [h, -h, -h], [h, -h, h], [-h, -h, h], [0, -1, 0]),
        ([h, h, -h], [-h, h, -h], [-h, h, h], [h, h, h], [0, 1, 0]),
        ([h, -h, -h], [h, h, -h], [h, h, h], [h, -h, h], [1, 0, 0]),
        ([-h, h, -h], [-h, -h, -h], [-h, -h, h], [-h, h, h], [-1, 0, 0]),
        ([-h, -h, h], [h, -h, h], [h, h, h], [-h, h, h], [0, 0, 1]),
        ([-h, h, -h], [h, h, -h], [h, -h, -h], [-h, -h, -h], [0, 0, -1]),
    ]
    vertices, normals, uvs, triangles = [], [], [], []
    for a, b, c, d, n in faces:
        first = len(vertices)
        vertices += [list(map(float, a)), list(map(float, b)), list(map(float, c)), list(map(float, d))]
        normals += [list(map(float, n))] * 4
        uvs += [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]
        triangles += [[first, first + 1, first + 2], [first, first + 2, first + 3]]
    return vertices, normals, uvs, triangles


def cylinder(radius, height, sides=12):
    """The side with shared vertices and radial normals (smooth), so that flat shading shows
    its facets; the caps with their own vertices."""
    vertices, normals, uvs, triangles = [], [], [], []
    h = height / 2.0
    for i in range(sides + 1):
        a = 2.0 * math.pi * i / sides
        x, y = math.cos(a), math.sin(a)
        for z, v in ((-h, 1.0), (h, 0.0)):
            vertices.append([radius * x, radius * y, z])
            normals.append([x, y, 0.0])
            uvs.append([i / float(sides), v])
    for i in range(sides):
        b = 2 * i
        triangles += [[b, b + 2, b + 1], [b + 1, b + 2, b + 3]]
    for z, nz in ((h, 1.0), (-h, -1.0)):
        centre = len(vertices)
        vertices.append([0.0, 0.0, z])
        normals.append([0.0, 0.0, nz])
        uvs.append([0.5, 0.5])
        ring = len(vertices)
        for i in range(sides):
            a = 2.0 * math.pi * i / sides
            vertices.append([radius * math.cos(a), radius * math.sin(a), z])
            normals.append([0.0, 0.0, nz])
            uvs.append([0.5 + 0.5 * math.cos(a), 0.5 + 0.5 * math.sin(a)])
        for i in range(sides):
            a, b = ring + i, ring + (i + 1) % sides
            triangles.append([centre, a, b] if nz > 0 else [centre, b, a])
    return vertices, normals, uvs, triangles


def check_winding(name, geometry):
    """Every triangle counter-clockwise seen from outside: the cross product of its edges
    points along the normals of its vertices."""
    vertices, normals, _, triangles = geometry
    for a, b, c in triangles:
        e1 = [vertices[b][k] - vertices[a][k] for k in range(3)]
        e2 = [vertices[c][k] - vertices[a][k] for k in range(3)]
        n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]]
        along = sum(n[k] * (normals[a][k] + normals[b][k] + normals[c][k]) for k in range(3))
        assert along > 0, (name, a, b, c)


def tri_shape_data(geometry):
    vertices, normals, uvs, triangles = geometry
    # The winding of the faces as the engine draws them: counter-clockwise faces are the
    # front with the default draw mode, as the level's exhibits (the triangle order of
    # `box` and `cylinder` is for that).
    cx = sum(v[0] for v in vertices) / len(vertices)
    cy = sum(v[1] for v in vertices) / len(vertices)
    cz = sum(v[2] for v in vertices) / len(vertices)
    radius = max(math.sqrt((v[0] - cx) ** 2 + (v[1] - cy) ** 2 + (v[2] - cz) ** 2) for v in vertices)
    return {"type": "NiTriShapeData", "tridata": {
        "has_vertices": 1, "vertices": vertices, "has_normals": 1, "normals": normals, "center": [cx, cy, cz], "radius": radius,
        "has_colors": 0, "colors": [], "uv_set_count": 1, "has_uv": 1, "uv_sets": [uvs],
        "triangle_points": 3 * len(triangles), "triangles": triangles, "match_groups": []}}


def material(specular=None, glossiness=0.0):
    b = object_net()
    b.update({"type": "NiMaterialProperty", "flags": 1, "ambient": [1.0, 1.0, 1.0], "diffuse": [1.0, 1.0, 1.0],
              "specular": specular or [0.0, 0.0, 0.0], "emissive": [0.0, 0.0, 0.0], "glossiness": glossiness, "alpha": 1.0})
    return b


def source_texture(file_name, pixel_layout=0, use_mipmaps=0, alpha_format=0):
    b = object_net()
    b.update({"type": "NiSourceTexture", "use_external": 1, "file_name": file_name, "pixel_layout": pixel_layout,
              "use_mipmaps": use_mipmaps, "alpha_format": alpha_format, "is_static": 1})
    return b


def texturing(source):
    b = object_net()
    maps = [None] * 7
    maps[0] = {"has": 1, "source": source, "clamp": 3, "filter": 2, "uv_set": 0, "ps2_l": 0, "ps2_k": -75, "unknown": 0}
    b.update({"type": "NiTexturingProperty", "flags": 0, "apply_mode": 2, "textures": maps})
    return b


def one_flag(kind, flags):
    b = object_net()
    b.update({"type": kind, "flags": flags})
    return b


def fog(function, depth=1.0, color=(1.0, 0.0, 0.0)):
    """Bit 1 fog on, bits 3-4 the function (0 Z_LINEAR, 1 RANGE_SQ), as the engine reads
    the flags."""
    b = object_net()
    b.update({"type": "NiFogProperty", "flags": 2 | (function << 3), "depth": depth, "color": list(color)})
    return b


def texture_effect(source, texture_type, coordinates, matrix=IDENTITY, translation=ZERO3):
    """As the exhibits of the level have it: flags 4, the texture filtered and wrapped;
    `affected` is a list of pointers in the file, 0 there. The projection (the
    matrix row by row, then the translation) maps the vector of the world (the position for
    a parallel or a perspective map, the reflection for a sphere map) to the texture
    coordinate: with the identity, a position in units repeats the texture every unit and
    the pattern averages to one colour (the first exhibits, 2026-10-16)."""
    b = av_object("", 4)
    # The level's exhibits carry a scale of 100 on the effect node; the engine folds the
    # node's world transform into the world projection, so the scale would divide the
    # projection by 100 (seen 2026-10-16: the scaled projections flattened to one colour).
    b.update({"type": "NiTextureEffect", "scale": 1.0, "affected": [0], "projection_matrix": list(matrix), "projection_translation": list(translation),
              "filtering": 2, "clamping": 3, "texture_type": texture_type, "coordinate_generation": coordinates, "source": source,
              "clipping_plane": 0, "plane_normal": [1.0, 0.0, 0.0], "plane_constant": 0.0, "ps2_l": 0, "ps2_k": -75, "unknown": 0})
    return b


def point_light(position, diffuse=(1.0, 1.0, 1.0)):
    """A white point light with no falloff (constant attenuation 1), as a NIF of the level
    carries one: a child of the root and in its effects list. The level itself lights
    almost nothing (its exhibits glow by their emissive colour), so a shading test brings
    its own light."""
    b = av_object("", 4)
    b.update({"type": "NiPointLight", "translation": list(position), "affected": [0], "dimmer": 1.0, "ambient": [0.0, 0.0, 0.0],
              "diffuse": list(diffuse), "specular": [1.0, 1.0, 1.0], "constant": 1.0, "linear": 0.0, "quadratic": 0.0})
    return b


def make(name, geometry, extra_properties=(), effect=None, material_block=None, light=None, base_texture=BASE_TEXTURE):
    """The blocks: 0 root, 1 shape, 2 data, 3 material, 4 texturing, 5 texture, then the
    extra properties, then the effect and its texture, then the light."""
    blocks = [node("@_nifx " + name), None, tri_shape_data(geometry), material_block or material(), texturing(5), source_texture(base_texture)]
    shape = av_object(name, 0)
    shape.update({"type": "NiTriShape", "data": 2, "skin": -1, "properties": [3, 4]})
    blocks[1] = shape
    for prop in extra_properties:
        shape["properties"].append(len(blocks))
        blocks.append(prop)
    blocks[0]["children"] = [1]
    if effect is not None:
        texture_type, coordinates, matrix, translation = effect
        # The shape and the effect move under a node of their own, which lists the effect:
        # the engine keeps the effect list of a reference's root node for the lights it
        # attaches, so an effect listed there is lost in the reference's scene (the loaded
        # model has it, the clone does not; seen through the bridge, 2026-10-16, with the
        # effect before and after the shape). An effect in a node of its own beside the
        # shape is kept but applies to nothing. The node that holds both works from the
        # file alone.
        sub = len(blocks)
        effect_at = sub + 1
        source = sub + 2
        blocks.append(node("sub"))
        blocks[sub]["children"] = [1, effect_at]
        blocks[sub]["effects"] = [effect_at]
        blocks.append(texture_effect(source, texture_type, coordinates, matrix, translation))
        blocks.append(source_texture(EFFECT_TEXTURE, 5, 2, 3))
        blocks[0]["children"] = [sub]
    if light is not None:
        at = len(blocks)
        blocks.append(light)
        blocks[0]["children"].append(at)
        blocks[0]["effects"].append(at)
    return write_file(name, blocks)


def make_tree(name, kind, children, fields=None, flags=8):
    """A root with one node of `kind` (with its own `fields`) whose children are shapes,
    one for each (geometry, translation). The blocks: 0 root, 1 the node, 2 material, 3
    texturing, 4 texture, then a shape and its data for each child. The node kinds of the
    retention rule are tested with these (plan step 5, 2026-10-16)."""
    mid = node(kind, flags)
    mid["type"] = kind
    mid.update(fields or {})
    blocks = [node("@_nifx " + name), mid, material(), texturing(4), source_texture(BASE_TEXTURE)]
    blocks[0]["children"] = [1]
    for i, (geometry, translation) in enumerate(children):
        shape = av_object("%s %d" % (name, i), 0)
        shape.update({"type": "NiTriShape", "data": len(blocks) + 1, "skin": -1, "properties": [2, 3], "translation": list(translation)})
        mid["children"].append(len(blocks))
        blocks.append(shape)
        blocks.append(tri_shape_data(geometry))
    return write_file(name, blocks)


def moved(geometry, by):
    """The geometry with its vertices moved by `by`."""
    vertices, normals, uvs, triangles = geometry
    return [[v[k] + by[k] for k in range(3)] for v in vertices], normals, uvs, triangles


def skin_blocks(blocks, skeleton, bones, weights):
    """Adds the skin of a shape: its bones have the identity from the skin, so a bone only moves
    a vertex by the bone's own place. `weights` gives (bone, weight) of a vertex for each vertex
    of the shape. The radius keeps the posed bound around each exhibit. Returns the block of
    the skin instance."""
    instance = len(blocks)
    records = []
    for bone in range(len(bones)):
        listed = [(vertex, weight) for vertex, influences in enumerate(weights) for b, weight in influences if b == bone]
        records.append({"rotation": list(IDENTITY), "translation": list(ZERO3), "scale": 1.0, "center": list(ZERO3), "radius": 500.0, "weights": listed})
    blocks.append({"type": "NiSkinInstance", "data": instance + 1, "skeleton_root": skeleton, "bones": list(bones)})
    blocks.append({"type": "NiSkinData", "skindata": {"rotation": list(IDENTITY), "translation": list(ZERO3), "scale": 1.0, "skin_partition": -1, "bones": records}})
    return instance


def make_skinned(name, flags):
    """A root, 6 bones, and two pairs of a skinned box (red) and a plain box (blue). `flags` are
    the flags of the two skinned shapes.

    With MWSE's flag the engine sums each influence with its raw weight, and its shape's place
    once (`NiSkinInstance::Deform` 0x6FA000, then the shape's world transform):

        place = shape(sum of weight * (vertex + place of the bone))

    The left pair, more than 4 influences that sum to 1: 6 bones, four at z 100 with the weight
    0.2 and two at z -200 with the weight 0.1. All of them give z 40 (0.8 * 100 + 0.2 * -200);
    the 4 largest, made to sum to 1, give z 100. The plain box is at z 40.

    The right pair, weights that do not sum to 1: 2 bones at x 150 with the weight 0.25. The box
    of 120 comes out with half its size at x 75. Made to sum to 1, it keeps its size at x 150,
    over the plain box of 60 at x 145. A palette blend that counts the shape's place with the
    weights puts it at half the shape's place in the world, far from the plain box."""
    red, blue = material(), material()
    red["emissive"] = [0.8, 0.25, 0.2]
    blue["emissive"] = [0.2, 0.3, 0.8]
    blocks = [node("@_nifx " + name), red, blue, texturing(4), source_texture(BASE_TEXTURE)]
    places = [[0.0, 0.0, 100.0]] * 4 + [[0.0, 0.0, -200.0]] * 2 + [[150.0, 0.0, 0.0]] * 2
    bones = []
    for i, place in enumerate(places):
        bone = node("bone %d" % i)
        bone["translation"] = list(place)
        bones.append(len(blocks))
        blocks[0]["children"].append(len(blocks))
        blocks.append(bone)
    many = [(b, 0.2 if b < 4 else 0.1) for b in range(6)]
    half = [(6, 0.25), (7, 0.25)]
    # Each pair: the skinned box, then the plain box at the place the engine gives it.
    exhibits = [
        ("many", moved(box(120.0), [-150.0, 0.0, 0.0]), many, box(120.0), [-280.0, 0.0, 40.0]),
        ("half", box(120.0), half, box(60.0), [145.0, 0.0, 0.0]),
    ]
    for label, skinned, influences, plain, plain_at in exhibits:
        shape = av_object("%s %s" % (name, label), flags)
        shape.update({"type": "NiTriShape", "data": len(blocks) + 1, "skin": len(blocks) + 2, "properties": [1, 3]})
        blocks[0]["children"].append(len(blocks))
        blocks.append(shape)
        blocks.append(tri_shape_data(skinned))
        skin_blocks(blocks, 0, bones, [influences] * len(skinned[0]))
        marker = av_object("%s %s plain" % (name, label), 0)
        marker.update({"type": "NiTriShape", "data": len(blocks) + 1, "skin": -1, "properties": [2, 3], "translation": list(plain_at)})
        blocks[0]["children"].append(len(blocks))
        blocks.append(marker)
        blocks.append(tri_shape_data(plain))
    return write_file(name, blocks)


def billboard(name, mode, translation):
    b = node(name, 8 | (mode << 5))
    b["type"] = "NiBillboardNode"
    b["translation"] = list(translation)
    return b


def keyframes(blocks, target, stop=4.0):
    """A controller that swings a block in place, in a loop of `stop` seconds, as the level's
    exhibits have it (flags 8): 40 units along x and a quarter turn about z and back."""
    controller = len(blocks)
    half = math.sqrt(0.5)
    blocks.append({"type": "NiKeyframeController", "next": -1, "flags": 8, "frequency": 1.0, "phase": 0.0, "start": 0.0, "stop": stop, "target": target, "data": controller + 1})
    blocks.append({"type": "NiKeyframeData",
                   "rotation_keys": {"interpolation": 1, "keys": [(0.0, 1.0, 0.0, 0.0, 0.0), (stop / 2, half, 0.0, 0.0, half), (stop, 1.0, 0.0, 0.0, 0.0)]},
                   "translation_keys": {"interpolation": 1, "keys": [(0.0, 0.0, 0.0, 0.0), (stop / 2, 40.0, 0.0, 0.0), (stop, 0.0, 0.0, 0.0)]},
                   "scale_keys": {"interpolation": 0, "keys": []}})
    blocks[target]["controller"] = controller


def make_turn(name, geometry, modes, effect, shape_under=False, hidden=False, switched=False, selected=False, controller=None):
    """A texture effect under turning billboards, one in each of `modes`, nested. The shape is
    a child of the node above the first billboard, before it in the walk, as in the level's
    103.nif: the shape displays before its effect turns. With `shape_under`, the shape is under
    the last billboard with the effect, and turns with it. The node above lists the effect.
    With `hidden` the first billboard has the hidden flag; with `switched` it is the child that a
    switch node does not select. Neither has a display, so the effect does not turn. With
    `selected`, the billboard is the child that a switch selects, with no geometry under the switch:
    a switch tests no geometry flag, so it has its display and turns.
    `controller` puts a keyframe controller on the effect ("effect"), on the node above the
    billboards ("above"), or on the node between two billboards ("between").
    The blocks: 0 root, 1 shape, 2 data, 3 material, 4 texturing, 5 texture, 6 the node above,
    then each billboard (a plain node between two), the effect and its texture."""
    blocks = [node("@_nifx " + name), None, tri_shape_data(geometry), material(), texturing(5), source_texture(BASE_TEXTURE)]
    shape = av_object(name, 0)
    shape.update({"type": "NiTriShape", "data": 2, "skin": -1, "properties": [3, 4]})
    blocks[1] = shape
    blocks[0]["children"] = [6]
    blocks.append(node("sub"))
    parent = 6
    blocks[6]["children"] = [] if shape_under else [1]
    if switched or selected:
        # With `switched` the switch selects an empty node, its child 0, and the billboard is its
        # child 1. With `selected` the billboard is its child 0.
        switch = node("switch")
        switch.update({"type": "NiSwitchNode", "index": 0})
        blocks.append(switch)
        blocks[parent]["children"].append(len(blocks) - 1)
        parent = len(blocks) - 1
    if switched:
        blocks.append(node("selected"))
        blocks[parent]["children"].append(len(blocks) - 1)
    for i, mode in enumerate(modes):
        if i > 0:
            between = node("between")
            between["translation"] = [10.0, 0.0, 20.0]
            blocks.append(between)
            blocks[parent]["children"].append(len(blocks) - 1)
            parent = len(blocks) - 1
        blocks.append(billboard("billboard %d" % i, mode, [40.0, 0.0, 0.0]))
        if hidden and i == 0:
            blocks[-1]["flags"] |= 1
        blocks[parent]["children"].append(len(blocks) - 1)
        parent = len(blocks) - 1
    if shape_under:
        blocks[parent]["children"].append(1)
    effect_at = len(blocks)
    blocks[parent]["children"].append(effect_at)
    blocks[6]["effects"] = [effect_at]
    texture_type, coordinates, matrix, translation = effect
    blocks.append(texture_effect(effect_at + 1, texture_type, coordinates, matrix, translation))
    blocks.append(source_texture(EFFECT_TEXTURE, 5, 2, 3))
    if controller is not None:
        # The engine runs the controllers of a static under an animated node, with these flags
        # as the level's exhibits have them (`BSAnimationManager::Update` 0x6EE690).
        blocks[0].update({"type": "NiBSAnimationNode", "flags": 0x6A})
    if controller == "effect":
        keyframes(blocks, effect_at)
    elif controller == "above":
        keyframes(blocks, 6)
    elif controller == "between":
        keyframes(blocks, next(i for i, block in enumerate(blocks) if block.get("name") == "between"))
    return write_file(name, blocks)


def write_file(name, blocks):
    """Writes the blocks with the root 0, proven by a read and a write back."""
    data = nif40.write(blocks, [0])
    back, roots = nif40.read(data)
    assert nif40.write(back, roots) == data, name
    path = os.path.join(OUT, name + ".nif")
    open(path, "wb").write(data)
    return path


def write_checker(path, size=64, cell=8):
    """An uncompressed DDS (A8R8G8B8, with its mipmaps) of a checker of white and dark blue
    cells, with one red cell at the origin: the texture of the effect exhibits, which both
    paths of the renderer sample through the generated coordinates. The mipmaps keep a far
    exhibit from aliasing differently on the two paths (seen 2026-10-16 without them)."""
    import struct
    level = []
    for y in range(size):
        for x in range(size):
            if x < cell and y < cell:
                level.append((0, 0, 255))
            elif (x // cell + y // cell) % 2 == 0:
                level.append((255, 255, 255))
            else:
                level.append((128, 32, 16))
    levels = [(size, level)]
    width = size
    while width > 1:
        half = width // 2
        smaller = []
        for y in range(half):
            for x in range(half):
                quad = [level[(2 * y + dy) * width + 2 * x + dx] for dy in (0, 1) for dx in (0, 1)]
                smaller.append(tuple(sum(p[k] for p in quad) // 4 for k in range(3)))
        level, width = smaller, half
        levels.append((width, level))
    # DDSD_CAPS, HEIGHT, WIDTH, PITCH, PIXELFORMAT and MIPMAPCOUNT; DDSCAPS_TEXTURE, COMPLEX
    # and MIPMAP.
    header = struct.pack("<4s7I11I8I5I", b"DDS ", 124, 0x100F | 0x20000, size, size, size * 4, 0, len(levels), *([0] * 11),
                         32, 0x41, 0, 32, 0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000, 0x1000 | 0x8 | 0x400000, 0, 0, 0, 0)
    assert len(header) == 128
    pixels = bytearray()
    for _, pixels_of_level in levels:
        for b, g, r in pixels_of_level:
            pixels += bytes((b, g, r, 255))
    open(path, "wb").write(header + pixels)


def bc7_pixel(level, x, y, size=64):
    """The pattern of the BC7 test textures, as a colour of level `level`: a gradient of the
    hue across, darker downwards, a dark line every 8 pixels of the top level, and a red cell
    at the origin, so that a turned, a mirrored or a wrongly decoded texture shows."""
    scale = 1 << level
    tx, ty = x * scale, y * scale
    if tx < 8 and ty < 8:
        return (255, 0, 0, 255)
    if tx % 8 == 0 or ty % 8 == 0:
        return (24, 24, 24, 255)
    u, v = tx / size, ty / size
    r = int(255 * max(0.0, 1.0 - 2.0 * u))
    g = int(255 * (1.0 - abs(2.0 * u - 1.0)))
    b = int(255 * max(0.0, 2.0 * u - 1.0))
    shade = 1.0 - 0.6 * v
    return (int(r * shade), int(g * shade), int(b * shade), 255)


def main():
    os.makedirs(OUT, exist_ok=True)
    os.makedirs(TEXTURES, exist_ok=True)
    write_checker(os.path.join(TEXTURES, "checker.dds"))
    # The BC7 textures (MGE XE's BC7, the DX10 header): the plain and the sRGB form, the
    # same blocks. Both draw as stored on both renderers (the BC7 design).
    import bc7
    bc7.write_dds(os.path.join(TEXTURES, "bc7.dds"), 64, bc7_pixel, bc7.DXGI_BC7)
    bc7.write_dds(os.path.join(TEXTURES, "bc7_srgb.dds"), 64, bc7_pixel, bc7.DXGI_BC7_SRGB)
    b = box(120.0)
    c = cylinder(60.0, 140.0)
    check_winding("box", b)
    check_winding("cylinder", c)
    files = [
        ("fog_z", make("fog_z", b, [fog(0)])),
        ("fog_range", make("fog_range", b, [fog(1)])),
        # A NiPointLight inside the file lights nothing in Morrowind (seen 2026-10-16 on both
        # renderers): the game takes its lights from light objects. The tour step places a
        # steady light object beside each entry marked `light`.
        ("shade_smooth", make("shade_smooth", c, [one_flag("NiShadeProperty", 1)])),
        ("shade_flat", make("shade_flat", c, [one_flag("NiShadeProperty", 0)])),
        ("specular", make("specular", b, [one_flag("NiSpecularProperty", 1)], material_block=material([1.0, 1.0, 1.0], 40.0))),
        # The sphere maps on the cylinder (the reflection turns around it; on a face of the
        # box it is one value) with the usual mapping of the reflection, half and offset by a
        # half. The parallel map scales the position so the box of 120 units spans half the
        # texture; the perspective map divides by a third coordinate that grows along z.
        ("env_sphere", make("env_sphere", c, effect=(2, 2, SPHERE_MATRIX, SPHERE_TRANSLATION))),
        ("env_parallel", make("env_parallel", b, effect=(2, 0, [1 / 240, 0.0, 0.0, 0.0, 1 / 240, 0.0, 0.0, 0.0, 1 / 240], [0.5, 0.5, 0.5]))),
        ("env_perspective", make("env_perspective", b, effect=(2, 1, [1 / 240, 0.0, 0.0, 0.0, 1 / 240, 0.0, 0.0, 0.0, 1 / 400], [0.5, 0.5, 1.0]))),
        ("fog_map", make("fog_map", c, effect=(3, 2, SPHERE_MATRIX, SPHERE_TRANSLATION))),
        # The node kinds of the retention rule. The billboard's mode is in bits 5 and 6 of
        # its flags: 0, always face the camera. The levels of detail need one range for
        # each child; the tour's stop is 300 units south and the stop before it farther,
        # so the far level shows first and the near one on arrival.
        ("node_billboard", make_tree("node_billboard", "NiBillboardNode", [(b, [80.0, 0.0, 0.0])])),
        ("node_switch", make_tree("node_switch", "NiSwitchNode", [(c, ZERO3), (b, ZERO3)], {"index": 1})),
        ("node_lod", make_tree("node_lod", "NiLODNode", [(b, ZERO3), (c, ZERO3)], {"index": 0, "lod_center": list(ZERO3), "lod_levels": [(0.0, 350.0), (350.0, 100000.0)]})),
        ("node_bsp", make_tree("node_bsp", "NiBSPNode", [(b, [-100.0, 0.0, 0.0]), (c, [100.0, 0.0, 0.0])], {"plane_normal": [1.0, 0.0, 0.0], "plane_constant": 0.0})),
        ("node_sortadjust", make_tree("node_sortadjust", "NiSortAdjustNode", [(b, [-80.0, 0.0, 0.0]), (b, [80.0, 0.0, 0.0])], {"sorting_mode": 0, "sub_sorter": -1})),
        # BC7 base textures: MGE XE's DDS reader patch takes them on both renderers.
        ("bc7", make("bc7", b, base_texture="textures\\@_nifx\\bc7.dds")),
        ("bc7_srgb", make("bc7_srgb", b, base_texture="textures\\@_nifx\\bc7_srgb.dds")),
        # The bit of MWSE's software skinning on a shape (`TriShapeFlags::SoftwareSkinningFlag`),
        # which its `TriShape::linkObject` copies to the geometry data at the load.
        ("skin_soft", make_skinned("skin_soft", 0x200)),
        ("skin_hard", make_skinned("skin_hard", 0)),
    ]
    # A texture effect under a turning billboard: each pair of the billboard's mode and the
    # coordinate mode, the shape turning with the effect, two billboards, a fog map.
    parallel = (2, 0, [1 / 240, 0.0, 0.0, 0.0, 1 / 240, 0.0, 0.0, 0.0, 1 / 240], [0.5, 0.5, 0.5])
    effects = [parallel, (2, 1, [1 / 240, 0.0, 0.0, 0.0, 1 / 240, 0.0, 0.0, 0.0, 1 / 400], [0.5, 0.5, 1.0]), (2, 2, SPHERE_MATRIX, SPHERE_TRANSLATION), (2, 3, SPHERE_MATRIX, SPHERE_TRANSLATION), (2, 4, SPHERE_MATRIX, SPHERE_TRANSLATION)]
    for mode in range(3):
        for coordinates, effect in enumerate(effects):
            name = "turn_b%d_c%d" % (mode, coordinates)
            files.append((name, make_turn(name, c if coordinates >= 2 else b, [mode], effect)))
        name = "turn_under_b%d" % mode
        files.append((name, make_turn(name, b, [mode], parallel, shape_under=True)))
    files.append(("turn_two", make_turn("turn_two", b, [1, 2], parallel)))
    files.append(("turn_two_under", make_turn("turn_two_under", b, [1, 2], parallel, shape_under=True)))
    files.append(("turn_fog", make_turn("turn_fog", c, [0], (3, 2, SPHERE_MATRIX, SPHERE_TRANSLATION))))
    files.append(("turn_hidden", make_turn("turn_hidden", b, [2], parallel, hidden=True)))
    files.append(("turn_switched", make_turn("turn_switched", b, [2], parallel, switched=True)))
    files.append(("turn_selected", make_turn("turn_selected", b, [2], parallel, selected=True)))
    # A keyframe controller on the effect: no billboard and each mode, with each coordinate mode;
    # then on a node above, on the node between two billboards, and with the shape under one.
    for mode, modes in [("n", []), ("0", [0]), ("1", [1]), ("2", [2])]:
        for coordinates, effect in enumerate(effects):
            name = "ctrl_b%s_c%d" % (mode, coordinates)
            files.append((name, make_turn(name, c if coordinates >= 2 else b, modes, effect, controller="effect")))
        name = "ctrl_above_b%s" % mode
        files.append((name, make_turn(name, b, modes, parallel, controller="above")))
    files.append(("ctrl_between", make_turn("ctrl_between", b, [1, 2], parallel, shape_under=True, controller="between")))
    files.append(("ctrl_under_b1", make_turn("ctrl_under_b1", b, [1], parallel, shape_under=True, controller="effect")))
    lines = ["-- The test NIFs of the flag support, made by tools/make_test_nifs.py (do not edit).",
             "-- The tour step places each in the first cell of the tour with spec.tourExtra.",
             "return {"]
    for name, path in files:
        light = ", light = true" if name in ("shade_smooth", "shade_flat", "specular") else ""
        lines.append('    { id = "@nif_x_%s", mesh = "@_nifx\\\\%s.nif"%s },' % (name, name, light))
        print("wrote", path, os.path.getsize(path), "bytes")
    lines.append("}")
    open(LIST, "w", newline="\n").write("\n".join(lines) + "\n")
    print("wrote", LIST)


main()
