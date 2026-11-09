"""No new absolute clip product in the helper's WGSL: a matrix of the world times a position of the world is wrong far from the origin.

    python -I tools/check_clip_products.py                # the render-link worktree next to this tree
    python -I tools/check_clip_products.py --mge DIR      # another MGE XE tree (a worktree's hook)
    python -I tools/check_clip_products.py --self-test    # the search, with the planted fault

The fault (W49): `matrix * vec4f(position, 1.0)` with an absolute position in f32 rounds the rows of the product apart far from the
origin (0.004 to 0.008 units at 74,000 units, about 0.03 at 368,000). The grass of the statics showed it as a flicker (records entry 45).
The cure is the eye-relative form of `renderer/view.rs`: `clip_with(matrix, world)` or `clip_of(world)`, with a matrix made in f64
(`view::relative`, `view::relative_of_product`).

This check finds every product of a name or a parenthesised expression with a `vec4f(..., 1.0)` (or `vec4<f32>`), in either order, in
`mgeHost64/src` (the `.rs` files that hold the WGSL, and the `.wgsl` files). A product with a direction, `vec4f(..., 0.0)`, is no
position and not found. The product must be on the ALLOWED list below: a place of a known meaning that is not a clip product of an
absolute position. A new product fails the check; the fix is to use `clip_with`, or, when the product is not a clip product, to
add it to ALLOWED with the reason. The key of an entry is the file and the text of the product, not a line number, so that an edit
above it does not break the list. The limit: the check reads text. A product made by another spelling (a `mat4x4f * v4` with the
w added on another line, a function that wraps it) is not found; a reviewer reads those.
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
MGE = os.path.join(os.path.dirname(os.path.dirname(HERE)), "MGE-XE-G7-render-link")

# (file below mgeHost64/src, product with its spaces collapsed): how many times, and why it is no absolute clip product.
ALLOWED = {
    ("renderer/view.rs", "relative * vec4f(world - u.eye.xyz, 1.0)"): (1, "clip_with itself: the position less the eye, with a matrix relative to the eye"),
    ("renderer/water/volume.rs", "s.world * vec4f(local, 1.0)"): (1, "model to world, with the matrix of the mesh (vs)"),
    ("renderer/water/volume.rs", "s.world * vec4f(position.xyz, 1.0)"): (1, "model to world, with the matrix of the mesh (vs_far)"),
    ("renderer.rs", "draw.view * vec4f(in.world_position, 1.0)"): (3, "world to view for the depth of a draw, with the view of that draw (view_z, view_position)"),
}

OPEN = r"vec4(?:f|<f32>)\("
# The second operand of a product: vec4f(<anything without a semicolon>, 1.0).
POSITION = OPEN + r"[^;]*?,\s*1\.0\s*\)"
FIRST = r"(?:[A-Za-z_][\w.]*(?:\[[^\]]*\])?|\([^()]*\))"
PRODUCTS = (
    re.compile(r"(?P<text>(?P<matrix>%s)\s*\*\s*(?P<vector>%s))" % (FIRST, POSITION)),
    re.compile(r"(?P<text>(?P<vector>%s)\s*\*\s*(?P<matrix>%s))" % (POSITION, FIRST)),
)


def products_in(text):
    """The products of a matrix with a position in the text of a file, as (line number, text with the spaces collapsed)."""
    found = []
    for pattern in PRODUCTS:
        for hit in pattern.finditer(text):
            # A vector in the first form must not itself be the end of another product's first operand: `a * b * vec4f(...)` reads as
            # `b * vec4f(...)`, which is the product that WGSL evaluates last, so it counts as one product.
            line = text.count("\n", 0, hit.start()) + 1
            found.append((line, " ".join(hit.group("text").split())))
    return sorted(set(found))


def check_tree(mge):
    """The problems of a render-link tree, as lines."""
    root = os.path.join(mge, "mgeHost64", "src")
    if not os.path.isdir(root):
        return ["no mgeHost64/src in %s" % mge]
    seen = {}
    problems = []
    for folder, _, names in os.walk(root):
        for name in sorted(names):
            if not name.endswith((".rs", ".wgsl")):
                continue
            path = os.path.join(folder, name)
            relative = os.path.relpath(path, root).replace(os.sep, "/")
            with open(path, encoding="utf-8", errors="replace") as file:
                text = file.read()
            for line, product in products_in(text):
                key = (relative, product)
                seen[key] = seen.get(key, 0) + 1
                if key not in ALLOWED:
                    problems.append("%s:%d: %s: a matrix times a position of the world; far from the origin the f32 product rounds apart. "
                                    "Use clip_with(matrix, world) with a matrix relative to the eye, or add the product to ALLOWED in "
                                    "tools/check_clip_products.py with its reason" % (relative, line, product))
    for key, (count, _) in ALLOWED.items():
        if seen.get(key, 0) != count:
            problems.append("%s: the product `%s` is on the ALLOWED list %d time(s) and found %d time(s): update the list" % (key[0], key[1], count, seen.get(key, 0)))
    return problems


def self_test():
    # The forms: an absolute clip product in either order, with the spellings of the helper.
    assert products_in("out.position = s.clip * vec4f(out.world, 1.0);") == [(1, "s.clip * vec4f(out.world, 1.0)")]
    assert products_in("let s0 = g.shadow_layer0 * vec4f(world, 1.0);\n") == [(1, "g.shadow_layer0 * vec4f(world, 1.0)")]
    assert products_in("let c = lighting.shadow0 * vec4f(place, 1.0);") == [(1, "lighting.shadow0 * vec4f(place, 1.0)")]
    assert products_in("let c = vec4<f32>(p, 1.0) * m;") == [(1, "vec4<f32>(p, 1.0) * m")]
    assert products_in("let c = (u.view * u.proj) * vec4f(p, 1.0);") == [(1, "(u.view * u.proj) * vec4f(p, 1.0)")]
    assert products_in("let c = m[2] * vec4f(a.xyz, 1.0);") == [(1, "m[2] * vec4f(a.xyz, 1.0)")]
    # A direction is no position, the relative form is a call and not a product, and a line number counts from 1.
    assert products_in("let d = u.relative * vec4f(direction, 0.0);") == []
    assert products_in("out.position = clip_with(s.clip, out.world);") == []
    assert products_in("a\nb\nlet s0 = m * vec4f(w,\n 1.0);") == [(3, "m * vec4f(w, 1.0)")]

    import tempfile

    def tree(root, files):
        for relative, text in files.items():
            path = os.path.join(root, "mgeHost64", "src", *relative.split("/"))
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as file:
                file.write(text)

    clean = {
        "renderer/view.rs": "fn clip_with(relative: mat4x4f, world: vec3f) -> vec4f { return relative * vec4f(world - u.eye.xyz, 1.0); }\n",
        "renderer/water/volume.rs": "out.world = (s.world * vec4f(local, 1.0)).xyz;\nout.world = (s.world * vec4f(position.xyz, 1.0)).xyz;\n"
                                    "out.position = clip_with(s.clip, out.world);\n",
        "renderer.rs": "let a = draw.view * vec4f(in.world_position, 1.0);\nlet b = (draw.view * vec4f(in.world_position, 1.0)).z;\n"
                       "let c = (draw.view * vec4f(in.world_position, 1.0)).z;\n",
    }
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp:
        good = os.path.join(temp, "good")
        tree(good, clean)
        assert check_tree(good) == [], check_tree(good)

        # The planted fault: one site put back to the absolute form fails the check, and the report names the file and the line.
        planted = dict(clean)
        planted["renderer/water/volume.rs"] = clean["renderer/water/volume.rs"].replace("clip_with(s.clip, out.world)", "s.clip * vec4f(out.world, 1.0)")
        bad = os.path.join(temp, "bad")
        tree(bad, planted)
        problems = check_tree(bad)
        assert len(problems) == 1 and problems[0].startswith("renderer/water/volume.rs:3: s.clip * vec4f(out.world, 1.0)"), problems

        # A product that is on the list loses its entry when it goes (the list is kept true), and one that comes twice is found.
        gone = dict(clean)
        gone["renderer.rs"] = clean["renderer.rs"].replace("let a = draw.view * vec4f(in.world_position, 1.0);\n", "")
        bare = os.path.join(temp, "gone")
        tree(bare, gone)
        assert any("found 2 time(s)" in problem for problem in check_tree(bare)), check_tree(bare)
        assert check_tree(os.path.join(temp, "nothing"))[0].startswith("no mgeHost64/src")
    print("check_clip_products self-test: ok")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--mge", default=MGE, help="the MGE XE render-link tree (default: the one next to this repository)")
    parser.add_argument("--self-test", action="store_true", help="the search and the planted fault, on trees made in a temporary folder")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    problems = check_tree(args.mge)
    for problem in problems:
        print(problem)
    print("clip products (%s): %d problems" % (args.mge, len(problems)))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
