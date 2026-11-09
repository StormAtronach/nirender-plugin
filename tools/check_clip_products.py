"""No new absolute clip product in the helper's WGSL: a matrix of the world times a position of the world is wrong far from the origin.

    python -I tools/check_clip_products.py                # the render-link worktree next to this tree
    python -I tools/check_clip_products.py --mge DIR      # another MGE XE tree (a worktree's hook)
    python -I tools/check_clip_products.py --self-test    # the search, with the planted fault

The fault (W49): `matrix * vec4f(position, 1.0)` with an absolute position in f32 rounds the rows of the product apart far from the
origin (0.004 to 0.008 units at 74,000 units, about 0.03 at 368,000). The grass of the statics showed it as a flicker (records entry 45).
The cure is the eye-relative form of `renderer/view.rs`: `clip_with(matrix, world)` or `clip_of(world)`, with a matrix made in f64
(`view::relative`, `view::relative_of_product`).

This check finds every product of a name or a parenthesised expression with a `vec4f(..., 1.0)` (also `vec4<f32>` and `vec4`, with the w written 1, 1., 1.0 or 1f), in either order, in
`mgeHost64/src` (the `.rs` files that hold the WGSL, and the `.wgsl` files). A product with a direction, `vec4f(..., 0.0)`, is no
position and not found. The product must be on the ALLOWED list below: a place of a known meaning that is not a clip product of an
absolute position. A new product fails the check; the fix is to use `clip_with`, or, when the product is not a clip product, to
add it to ALLOWED with the reason. The key of an entry is the file and the text of the product, not a line number, so that an edit
above it does not break the list. The limit: the check reads text. A product made by another spelling (a `mat4x4f * v4` with the
w added on another line, a function that wraps it) is not found; a reviewer reads those.

Two more rules close that gap (the stage rules of the far-origin fix, MGE 674c7a1):

- Every `out.position = ...;` of a vertex stage is `clip_of(...)` or `clip_with(...)`, or reads a local `clip` that the same stage
  made with one of them. A position of another form (a screen quad in clip space, a culled vertex) is on ALLOWED_POSITIONS with the
  reason. So a product that the first rule does not see (the w added on another line) still has to end in `clip_of`.
- No stage reads `u.view_projection`. That field is the absolute matrix of the view group. The stages use `clip_of`, which takes the
  matrix made relative to the eye (`view::relative`).
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

# (file below mgeHost64/src, value of `out.position = ...;` with the spaces collapsed): how many times, and why it is no world position.
ALLOWED_POSITIONS = {
    ("renderer/post.rs", "vec4f(x, y, 0.0, 1.0)"): (3, "a screen triangle in clip space (the post steps)"),
    ("renderer/post.rs", "vec4f(f32(i32(index & 1u) * 4 - 1), f32(i32(index >> 1u) * 4 - 1), 0.0, 1.0)"): (1, "the full screen triangle in clip space"),
    ("renderer/shadow.rs", "vec4f(x, y, 0.0, 1.0)"): (1, "a screen triangle in clip space (the shadow atlas picture)"),
    ("renderer/shadow.rs", "vec4f(-1.0 + corner.x * 0.5, -1.0 + corner.y * 0.25, 0.0, 1.0)"): (1, "the corner of the atlas picture in clip space"),
    ("renderer/statics.rs", "vec4f(0.0, 0.0, -10000.0, 0.0)"): (1, "a vertex of a place before the band: a point outside the clip volume"),
}
POSITION_ASSIGNMENT = re.compile(r"\bout\.position\s*=\s*(?P<value>[^;]+);")
CLIP_CALL = re.compile(r"clip_(?:of|with)\(")
CLIP_LOCAL = re.compile(r"\bvar clip\s*=\s*clip_(?:of|with)\(")
CLIP_NAME = re.compile(r"(?<![\w.])clip\b")
VIEW_PROJECTION_READ = re.compile(r"\bu\.view_projection\b")

# The spellings of WGSL: vec4f, vec4<f32>, vec4 (the type is inferred).
OPEN = r"vec4(?:f|<f32>)?\("
# The second operand of a product: vec4f(<anything without a semicolon>, 1.0), with the w written 1, 1., 1.0 or 1f (as 1.0f too).
POSITION = OPEN + r"[^;]*?,\s*1(?:\.0*)?f?\s*\)"
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


def positions_in(text):
    """The assignments of `out.position` in the text of a file, as (line number, value with the spaces collapsed, from clip).
    From clip: the value is a `clip_of` or `clip_with` call, or it reads a local `clip` that the vertex stage above it made with one."""
    found = []
    for hit in POSITION_ASSIGNMENT.finditer(text):
        value = " ".join(hit.group("value").split())
        stage = text[max(text.rfind("@vertex", 0, hit.start()), 0):hit.start()]
        from_clip = bool(CLIP_CALL.match(value)) or (bool(CLIP_NAME.search(value)) and bool(CLIP_LOCAL.search(stage)))
        found.append((text.count("\n", 0, hit.start()) + 1, value, from_clip))
    return found


def view_projection_reads(text):
    """The line numbers of the reads of `u.view_projection` in the text of a file."""
    return [text.count("\n", 0, hit.start()) + 1 for hit in VIEW_PROJECTION_READ.finditer(text)]


def check_tree(mge):
    """The problems of a render-link tree, as lines."""
    root = os.path.join(mge, "mgeHost64", "src")
    if not os.path.isdir(root):
        return ["no mgeHost64/src in %s" % mge]
    seen = {}
    seen_positions = {}
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
            for line, value, from_clip in positions_in(text):
                if from_clip:
                    continue
                key = (relative, value)
                seen_positions[key] = seen_positions.get(key, 0) + 1
                if key not in ALLOWED_POSITIONS:
                    problems.append("%s:%d: out.position = %s: a vertex stage must end in clip_of(world) or clip_with(matrix, world), or a local `clip` "
                                    "made with one. Use them, or add the value to ALLOWED_POSITIONS in tools/check_clip_products.py with its reason"
                                    % (relative, line, value))
            for line in view_projection_reads(text):
                problems.append("%s:%d: u.view_projection is the absolute matrix of the view group; far from the origin the f32 product rounds apart. "
                                "Use clip_of(world)" % (relative, line))
    for key, (count, _) in ALLOWED.items():
        if seen.get(key, 0) != count:
            problems.append("%s: the product `%s` is on the ALLOWED list %d time(s) and found %d time(s): update the list" % (key[0], key[1], count, seen.get(key, 0)))
    for key, (count, _) in ALLOWED_POSITIONS.items():
        if seen_positions.get(key, 0) != count:
            problems.append("%s: out.position = %s is on the ALLOWED_POSITIONS list %d time(s) and found %d time(s): update the list"
                            % (key[0], key[1], count, seen_positions.get(key, 0)))
    return problems


def self_test():
    # The forms: an absolute clip product in either order, with the spellings of the helper.
    assert products_in("out.position = s.clip * vec4f(out.world, 1.0);") == [(1, "s.clip * vec4f(out.world, 1.0)")]
    assert products_in("let s0 = g.shadow_layer0 * vec4f(world, 1.0);\n") == [(1, "g.shadow_layer0 * vec4f(world, 1.0)")]
    assert products_in("let c = lighting.shadow0 * vec4f(place, 1.0);") == [(1, "lighting.shadow0 * vec4f(place, 1.0)")]
    assert products_in("let c = vec4<f32>(p, 1.0) * m;") == [(1, "vec4<f32>(p, 1.0) * m")]
    assert products_in("let c = (u.view * u.proj) * vec4f(p, 1.0);") == [(1, "(u.view * u.proj) * vec4f(p, 1.0)")]
    assert products_in("let c = m[2] * vec4f(a.xyz, 1.0);") == [(1, "m[2] * vec4f(a.xyz, 1.0)")]
    # The other spellings of the same product: the w as 1, 1., 1f or 1.0f, and the type left to WGSL.
    assert products_in("out.position = s.clip * vec4f(out.world, 1);") == [(1, "s.clip * vec4f(out.world, 1)")]
    assert products_in("out.position = s.clip * vec4f(out.world, 1.);") == [(1, "s.clip * vec4f(out.world, 1.)")]
    assert products_in("out.position = s.clip * vec4f(out.world, 1f);") == [(1, "s.clip * vec4f(out.world, 1f)")]
    assert products_in("out.position = s.clip * vec4f(out.world, 1.0f);") == [(1, "s.clip * vec4f(out.world, 1.0f)")]
    assert products_in("out.position = s.clip * vec4(out.world, 1.0);") == [(1, "s.clip * vec4(out.world, 1.0)")]
    assert products_in("out.position = vec4(out.world, 1) * s.clip;") == [(1, "vec4(out.world, 1) * s.clip")]
    # A direction is no position, in any spelling of the zero; a w that only starts with 1 is no position either.
    for zero in ("0.0", "0", "0.", "0f", "0.0f"):
        assert products_in("let d = u.relative * vec4f(direction, %s);" % zero) == [], zero
    assert products_in("let d = u.relative * vec4(direction, 0);") == []
    for other in ("10", "1.5", "1.0e3", "10.0"):
        assert products_in("let d = m * vec4f(p, %s);" % other) == [], other
    # A direction is no position, the relative form is a call and not a product, and a line number counts from 1.
    assert products_in("let d = u.relative * vec4f(direction, 0.0);") == []
    assert products_in("out.position = clip_with(s.clip, out.world);") == []
    assert products_in("a\nb\nlet s0 = m * vec4f(w,\n 1.0);") == [(3, "m * vec4f(w, 1.0)")]

    # The stage rules: a position from clip_of or clip_with, from a local `clip` made with one, and the forms that are not.
    assert positions_in("out.position = clip_of(world);") == [(1, "clip_of(world)", True)]
    assert positions_in("out.position = clip_with(s.clip, out.world);") == [(1, "clip_with(s.clip, out.world)", True)]
    stage = "@vertex fn vs() -> Out {\n    var clip = clip_of(world);\n    clip.z = max(0.0, clip.z);\n    out.position = clip;\n"
    assert positions_in(stage) == [(4, "clip", True)]
    assert positions_in(stage.replace("clip_of(world)", "vec4f(world, 1.0)")) == [(4, "clip", False)], "a local not made by clip_of"
    assert positions_in(stage.replace("var clip", "let clip")) == [(4, "clip", False)], "the local must be made with var, as the stages do"
    assert positions_in("@vertex fn a() -> Out {\n    var clip = clip_of(p);\n}\n@vertex fn b() -> Out {\n    out.position = clip;\n") == [(5, "clip", False)], \
        "the local of another stage does not count"
    assert positions_in("out.position = s.clip * p;") == [(1, "s.clip * p", False)], "s.clip is no local clip"
    assert positions_in("@vertex fn vs() -> Out {\n    var clip = clip_of(world);\n    out.position = vec4f(clip.xy, clip.zw);\n") == [(3, "vec4f(clip.xy, clip.zw)", True)]
    assert positions_in("out.position = vec4f(x, y, 0.0, 1.0);") == [(1, "vec4f(x, y, 0.0, 1.0)", False)]
    assert view_projection_reads("let c = u.view_projection * p;\n\nlet d = g.view_projection;\nlet e = u.view_projection;") == [1, 4]
    assert view_projection_reads("view_projection: mat4x4f,\npub view_projection: [f32; 16],") == []

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
    # Each position on ALLOWED_POSITIONS, as often as the list says.
    for (relative, value), (count, _) in ALLOWED_POSITIONS.items():
        clean[relative] = clean.get(relative, "") + "@vertex fn vs() -> Out {\n    var out: Out;\n" + "    out.position = %s;\n" % value * count + "    return out;\n}\n"
    clean["renderer/statics.rs"] = clean.get("renderer/statics.rs", "") + (
        "@vertex fn vs(in: In) -> Out {\n    var out: Out;\n    var clip = clip_of(world);\n    clip.z = max(0.0, clip.z);\n    out.position = clip;\n    return out;\n}\n")
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
        assert len(problems) == 2 and problems[0].startswith("renderer/water/volume.rs:3: s.clip * vec4f(out.world, 1.0)"), problems
        assert problems[1].startswith("renderer/water/volume.rs:3: out.position = s.clip * vec4f(out.world, 1.0)"), "the stage rule sees it too: %s" % problems

        # The planted fault that the product rule cannot see: the w added on another line, with the absolute matrix of the view group.
        split = dict(clean)
        split["renderer/water.rs"] = "@vertex fn vs() -> Out {\n    var out: Out;\n    let p = vec4f(world, 1.0);\n    out.position = u.view_projection * p;\n    return out;\n}\n"
        far = os.path.join(temp, "far")
        tree(far, split)
        problems = check_tree(far)
        assert len(problems) == 2, problems
        assert problems[0].startswith("renderer/water.rs:4: out.position = u.view_projection * p"), problems
        assert problems[1].startswith("renderer/water.rs:4: u.view_projection is the absolute matrix"), problems

        # A local `clip` that is no clip_of result fails; the same local made by clip_of passes (the clean tree has it).
        local = dict(clean)
        local["renderer/statics.rs"] = clean["renderer/statics.rs"].replace("var clip = clip_of(world);", "var clip = vec4f(world, 1.0);")
        wrong = os.path.join(temp, "local")
        tree(wrong, local)
        problems = check_tree(wrong)
        assert len(problems) == 1 and "out.position = clip:" in problems[0], problems

        # A position on ALLOWED_POSITIONS loses its entry when it goes, and one that comes twice is found.
        screen = dict(clean)
        screen["renderer/shadow.rs"] = clean["renderer/shadow.rs"].replace("    out.position = vec4f(x, y, 0.0, 1.0);\n", "", 1)
        missing = os.path.join(temp, "screen")
        tree(missing, screen)
        assert any("ALLOWED_POSITIONS list 1 time(s) and found 0 time(s)" in problem for problem in check_tree(missing)), check_tree(missing)
        new_form = dict(clean)
        new_form["renderer/post.rs"] = clean["renderer/post.rs"] + "@vertex fn vs() -> Out {\n    out.position = vec4f(x, y, 0.5, 1.0);\n}\n"
        extra = os.path.join(temp, "extra")
        tree(extra, new_form)
        assert any("renderer/post.rs:" in problem and "out.position = vec4f(x, y, 0.5, 1.0)" in problem for problem in check_tree(extra)), check_tree(extra)

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
