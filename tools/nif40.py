"""A reader and writer of NetImmerse 4.0.0.2 files (the NIF of Morrowind) for the blocks the
test assets need: nodes, tri shapes and their data, the properties, source textures, texture
effects and string extra data. Written 2026-10-16 for the test NIFs of the flag support
(flag inventory check). The layouts are those of nif.xml for 4.0.0.2; the proof of each one
is `python tools/nif40.py check <folder>`: every file of the folder whose blocks are all
known must read and write back byte for byte.

A file is a list of blocks (dicts with a "type" and the fields of the layout) and the roots.
References are block indexes, -1 for none, as in the file.
"""
import os
import struct
import sys

VERSION_LINE = b"NetImmerse File Format, Version 4.0.0.2\n"
VERSION = 0x04000002

# The layouts: a list of (field, kind). Kinds: u8 u16 u32 i16 i32 f v3 m33 str ref, and the
# compound kinds handled in code ("refs" a u32 count then refs, "bounds", "tridata", ...).
OBJECT_NET = [("name", "str"), ("extra_data", "ref"), ("controller", "ref")]
AV_OBJECT = OBJECT_NET + [("flags", "u16"), ("translation", "v3"), ("rotation", "m33"), ("scale", "f"), ("velocity", "v3"),
                          ("properties", "refs"), ("bounds", "bounds")]
TEX_DESC = [("source", "ref"), ("clamp", "u32"), ("filter", "u32"), ("uv_set", "u32"), ("ps2_l", "i16"), ("ps2_k", "i16"), ("unknown", "u16")]
NODE = AV_OBJECT + [("children", "refs"), ("effects", "refs")]
LAYOUTS = {
    "NiNode": NODE,
    "NiCollisionSwitch": NODE,
    "NiBillboardNode": NODE,
    "NiBSAnimationNode": NODE,
    "NiBSParticleNode": NODE,
    "RootCollisionNode": NODE,
    "NiSwitchNode": NODE + [("index", "u32")],
    # A switch node with the levels of detail: the centre, then (near, far) for each level.
    "NiLODNode": NODE + [("index", "u32"), ("lod_center", "v3"), ("lod_levels", "lodlevels")],
    "NiSortAdjustNode": NODE + [("sorting_mode", "u32"), ("sub_sorter", "ref")],
    "NiTriShape": AV_OBJECT + [("data", "ref"), ("skin", "ref")],
    "NiTriShapeData": [("tridata", "tridata")],
    "NiMaterialProperty": OBJECT_NET + [("flags", "u16"), ("ambient", "v3"), ("diffuse", "v3"), ("specular", "v3"), ("emissive", "v3"),
                                        ("glossiness", "f"), ("alpha", "f")],
    "NiAlphaProperty": OBJECT_NET + [("flags", "u16"), ("threshold", "u8")],
    "NiZBufferProperty": OBJECT_NET + [("flags", "u16")],
    "NiStencilProperty": OBJECT_NET + [("flags", "u16"), ("enabled", "u8"), ("function", "u32"), ("reference", "u32"), ("mask", "u32"),
                                       ("fail", "u32"), ("z_fail", "u32"), ("pass", "u32"), ("draw_mode", "u32")],
    "NiFogProperty": OBJECT_NET + [("flags", "u16"), ("depth", "f"), ("color", "v3")],
    "NiShadeProperty": OBJECT_NET + [("flags", "u16")],
    "NiSpecularProperty": OBJECT_NET + [("flags", "u16")],
    "NiWireframeProperty": OBJECT_NET + [("flags", "u16")],
    "NiDitherProperty": OBJECT_NET + [("flags", "u16")],
    "NiVertexColorProperty": OBJECT_NET + [("flags", "u16"), ("vertex_mode", "u32"), ("lighting_mode", "u32")],
    "NiTexturingProperty": OBJECT_NET + [("flags", "u16"), ("apply_mode", "u32"), ("textures", "textures")],
    "NiSourceTexture": OBJECT_NET + [("use_external", "u8"), ("file_name", "str"), ("pixel_layout", "u32"), ("use_mipmaps", "u32"),
                                     ("alpha_format", "u32"), ("is_static", "u8")],
    "NiTextureEffect": AV_OBJECT + [("affected", "u32list"), ("projection_matrix", "m33"), ("projection_translation", "v3"),
                                    ("filtering", "u32"), ("clamping", "u32"), ("texture_type", "u32"), ("coordinate_generation", "u32"),
                                    ("source", "ref"), ("clipping_plane", "u8"), ("plane_normal", "v3"), ("plane_constant", "f"),
                                    ("ps2_l", "i16"), ("ps2_k", "i16"), ("unknown", "u16")],
    "NiStringExtraData": [("next", "ref"), ("bytes_remaining", "u32"), ("string", "str")],
    # A light: a dynamic effect (the affected list) with its colours, then the attenuation.
    "NiPointLight": AV_OBJECT + [("affected", "u32list"), ("dimmer", "f"), ("ambient", "v3"), ("diffuse", "v3"), ("specular", "v3"),
                                 ("constant", "f"), ("linear", "f"), ("quadratic", "f")],
    "NiSpotLight": AV_OBJECT + [("affected", "u32list"), ("dimmer", "f"), ("ambient", "v3"), ("diffuse", "v3"), ("specular", "v3"),
                                ("constant", "f"), ("linear", "f"), ("quadratic", "f"), ("cutoff", "f"), ("exponent", "f")],
}


class Reader:
    def __init__(self, data):
        self.data = data
        self.at = 0

    def take(self, fmt):
        size = struct.calcsize(fmt)
        value = struct.unpack_from(fmt, self.data, self.at)
        self.at += size
        return value[0] if len(value) == 1 else list(value)

    def string(self):
        n = self.take("<I")
        s = self.data[self.at:self.at + n]
        self.at += n
        return s.decode("latin-1")

    def refs(self):
        n = self.take("<I")
        return [self.take("<i") for _ in range(n)]

    def field(self, kind):
        if kind == "u8":
            return self.take("<B")
        if kind == "u16":
            return self.take("<H")
        if kind == "u32":
            return self.take("<I")
        if kind == "i16":
            return self.take("<h")
        if kind in ("i32", "ref"):
            return self.take("<i")
        if kind == "f":
            return self.take("<f")
        if kind == "v3":
            return self.take("<3f")
        if kind == "m33":
            return self.take("<9f")
        if kind == "str":
            return self.string()
        if kind == "refs":
            return self.refs()
        if kind == "u32list":
            n = self.take("<I")
            return [self.take("<I") for _ in range(n)]
        if kind == "lodlevels":
            n = self.take("<I")
            return [self.take("<2f") for _ in range(n)]
        if kind == "bounds":
            has = self.take("<I")
            if has == 0:
                return None
            return {"has": has, "unknown": self.take("<I"), "translation": self.take("<3f"), "rotation": self.take("<9f"), "radius": self.take("<3f")}
        if kind == "textures":
            count = self.take("<I")
            maps = []
            for i in range(count):
                has = self.take("<I")
                if has == 0:
                    maps.append(None)
                    continue
                # The word is "has a texture": any value but 0 in the files of the game.
                m = {"has": has}
                m.update({name: self.field(k) for name, k in TEX_DESC})
                if i == 5:
                    m["luma_scale"] = self.take("<f")
                    m["luma_offset"] = self.take("<f")
                    m["bump_matrix"] = self.take("<4f")
                maps.append(m)
            return maps
        if kind == "tridata":
            d = {}
            n = self.take("<H")
            d["has_vertices"] = self.take("<I")
            d["vertices"] = [self.take("<3f") for _ in range(n)] if d["has_vertices"] else []
            d["has_normals"] = self.take("<I")
            d["normals"] = [self.take("<3f") for _ in range(n)] if d["has_normals"] else []
            d["center"] = self.take("<3f")
            d["radius"] = self.take("<f")
            d["has_colors"] = self.take("<I")
            d["colors"] = [self.take("<4f") for _ in range(n)] if d["has_colors"] else []
            sets = self.take("<H")
            d["has_uv"] = self.take("<I")
            d["uv_sets"] = [[self.take("<2f") for _ in range(n)] for _ in range(sets)] if d["has_uv"] else []
            d["uv_set_count"] = sets
            t = self.take("<H")
            d["triangle_points"] = self.take("<I")
            d["triangles"] = [self.take("<3H") for _ in range(t)]
            g = self.take("<H")
            d["match_groups"] = []
            for _ in range(g):
                c = self.take("<H")
                d["match_groups"].append([self.take("<H") for _ in range(c)])
            return d
        raise ValueError(kind)


class Writer:
    def __init__(self):
        self.parts = []

    def put(self, fmt, *values):
        self.parts.append(struct.pack(fmt, *values))

    def string(self, s):
        b = s.encode("latin-1")
        self.put("<I", len(b))
        self.parts.append(b)

    def field(self, kind, v):
        if kind == "u8":
            self.put("<B", v)
        elif kind == "u16":
            self.put("<H", v)
        elif kind == "u32":
            self.put("<I", v)
        elif kind == "i16":
            self.put("<h", v)
        elif kind in ("i32", "ref"):
            self.put("<i", v)
        elif kind == "f":
            self.put("<f", v)
        elif kind == "v3":
            self.put("<3f", *v)
        elif kind == "m33":
            self.put("<9f", *v)
        elif kind == "str":
            self.string(v)
        elif kind == "refs":
            self.put("<I", len(v))
            for r in v:
                self.put("<i", r)
        elif kind == "u32list":
            self.put("<I", len(v))
            for r in v:
                self.put("<I", r)
        elif kind == "lodlevels":
            self.put("<I", len(v))
            for near, far in v:
                self.put("<2f", near, far)
        elif kind == "bounds":
            if v is None:
                self.put("<I", 0)
            else:
                self.put("<I", v.get("has", 1))
                self.put("<I", v["unknown"])
                self.put("<3f", *v["translation"])
                self.put("<9f", *v["rotation"])
                self.put("<3f", *v["radius"])
        elif kind == "textures":
            self.put("<I", len(v))
            for i, m in enumerate(v):
                if m is None:
                    self.put("<I", 0)
                    continue
                self.put("<I", m.get("has", 1))
                for name, k in TEX_DESC:
                    self.field(k, m[name])
                if i == 5:
                    self.put("<f", m["luma_scale"])
                    self.put("<f", m["luma_offset"])
                    self.put("<4f", *m["bump_matrix"])
        elif kind == "tridata":
            d = v
            n = len(d["vertices"]) if d["has_vertices"] else len(d["normals"]) if d["has_normals"] else len(d["colors"])
            if d["has_uv"] and d["uv_sets"]:
                n = len(d["uv_sets"][0])
            self.put("<H", n)
            self.put("<I", d["has_vertices"])
            for p in d["vertices"]:
                self.put("<3f", *p)
            self.put("<I", d["has_normals"])
            for p in d["normals"]:
                self.put("<3f", *p)
            self.put("<3f", *d["center"])
            self.put("<f", d["radius"])
            self.put("<I", d["has_colors"])
            for c in d["colors"]:
                self.put("<4f", *c)
            self.put("<H", d["uv_set_count"])
            self.put("<I", d["has_uv"])
            for uvs in d["uv_sets"]:
                for uv in uvs:
                    self.put("<2f", *uv)
            self.put("<H", len(d["triangles"]))
            self.put("<I", d["triangle_points"])
            for t in d["triangles"]:
                self.put("<3H", *t)
            self.put("<H", len(d["match_groups"]))
            for g in d["match_groups"]:
                self.put("<H", len(g))
                for x in g:
                    self.put("<H", x)
        else:
            raise ValueError(kind)

    def bytes(self):
        return b"".join(self.parts)


class Unknown(Exception):
    pass


def read(data):
    """The blocks and the roots of a file. Raises Unknown at the first block of a type that
    has no layout here (the rest of the file cannot be read then)."""
    if not data.startswith(VERSION_LINE):
        raise ValueError("not a 4.0.0.2 file")
    r = Reader(data)
    r.at = len(VERSION_LINE)
    if r.take("<I") != VERSION:
        raise ValueError("version word")
    count = r.take("<I")
    blocks = []
    for i in range(count):
        kind = r.string()
        if kind not in LAYOUTS:
            raise Unknown("block %d: %s" % (i, kind))
        block = {"type": kind}
        for name, k in LAYOUTS[kind]:
            block[name] = r.field(k)
        blocks.append(block)
    roots = r.refs()
    if r.at != len(data):
        raise ValueError("%d bytes left after the roots" % (len(data) - r.at))
    return blocks, roots


def write(blocks, roots):
    w = Writer()
    w.parts.append(VERSION_LINE)
    w.put("<I", VERSION)
    w.put("<I", len(blocks))
    for block in blocks:
        w.string(block["type"])
        for name, k in LAYOUTS[block["type"]]:
            w.field(k, block[name])
    w.field("refs", roots)
    return w.bytes()


def check(folder):
    """Every file whose blocks are all known must come back byte for byte."""
    same = unknown = bad = 0
    unknown_types = {}
    for name in sorted(os.listdir(folder)):
        path = os.path.join(folder, name)
        if not name.lower().endswith(".nif") or not os.path.isfile(path):
            continue
        data = open(path, "rb").read()
        try:
            blocks, roots = read(data)
        except Unknown as e:
            unknown += 1
            unknown_types[str(e).split(": ")[-1]] = unknown_types.get(str(e).split(": ")[-1], 0) + 1
            continue
        except Exception as e:
            bad += 1
            print("  %s: cannot read: %s" % (name, e))
            continue
        out = write(blocks, roots)
        if out == data:
            same += 1
        else:
            bad += 1
            at = next((i for i in range(min(len(out), len(data))) if out[i] != data[i]), min(len(out), len(data)))
            print("  %s: differs at byte %d of %d (written %d)" % (name, at, len(data), len(out)))
    print("%s: %d files byte-identical after read and write, %d with a block of an unknown type, %d wrong" % (folder, same, unknown, bad))
    if unknown_types:
        print("  unknown types (files stopped by each):", ", ".join("%s %d" % (k, n) for k, n in sorted(unknown_types.items(), key=lambda x: -x[1])))
    return bad == 0


if __name__ == "__main__":
    if sys.argv[1:2] == ["check"]:
        sys.exit(0 if check(sys.argv[2]) else 1)
    print(__doc__)
