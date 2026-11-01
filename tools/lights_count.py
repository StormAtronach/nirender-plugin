"""Option B's count (notes/designs-2026-10/next-feature-step.md, section 4): what clustered distant
lights and a 3D light volume would hold, from lights.data alone. No game and no build.

    python -I lights_count.py <lights.data> [--view-range 7168]

For each view and draw distance: the sources in a 90-degree view past the handoff and within the
draw distance, with the mesh flag; the sources in each cell of a 16 x 9 x 24 grid of the view, with
log depth slices from the handoff to the draw distance; the same with the reach of L2 bounded. Then
the worst place over the lit cells, and the bricks of the 3D volume (shadows-and-lights-design.md
3.5: a voxel every 1024 units, 8 x 8 a cell, 4 layers, 24 bytes a voxel).

The format: formats/lights.rs. The reach: 1.5 times the radius, as the bake (`BAKE_REACH`, generation/lights.rs).
The handoff: where a glow starts, view range - 1800 - 512 (`before_world`, renderer.rs).

[I] in the numbers: the view is 90 degrees across at 16:9 and level; a source goes into every cell
of the box around its sphere, so a cell's count is high by a little; the bricks count from z = 0,
not from the land under each cell.
"""
import math
import struct
import sys

import numpy as np

CELL = 8192.0
GRID = (16, 9, 24)
CAP = 64

# Name, eye, heading (0 north, 90 east). Balmora and Vivec: wgpu-distant-lights/batch-night-towns.lua.
# Ald'ruhn [I]: no known view; the same shape as Balmora's, east of the town at (-2, 6).
VIEWS = [
    ("Vivec", (62000.0, -92000.0, 3500.0), 270.0),
    ("Balmora from the hill", (2000.0, -12000.0, 7000.0), 270.0),
    ("Ald'ruhn", (-12288.0 + 22000.0, 53248.0, 7000.0), 270.0),
]
DRAW_DISTANCES = (7.0, 20.0)


def read_sources(path):
    with open(path, "rb") as file:
        head = file.read(40)
        assert head[:8] == b"XELIGHT1", head[:8]
        version, sources, tiles, side = struct.unpack_from("<IIII", head, 8)
        assert version == 2, version
        data = np.frombuffer(file.read(32 * sources), dtype=[("pos", "<3f4"), ("radius", "<f4"), ("colour", "u1", 4), ("flags", "<u4"), ("zero", "<2u4")])
    return data, tiles


def camera(eye, heading):
    h = math.radians(heading)
    forward = np.array([math.sin(h), math.cos(h), 0.0])
    right = np.array([math.cos(h), -math.sin(h), 0.0])
    up = np.array([0.0, 0.0, 1.0])
    return np.array(eye), right, up, forward


def to_view(pos, eye, right, up, forward):
    relative = pos - eye
    return relative @ right, relative @ up, relative @ forward


TAN_X = 1.0
TAN_Y = 9.0 / 16.0


def in_view(pos, reach, handoff, far, frame):
    x, y, z = to_view(pos, *frame)
    distance = np.sqrt(x * x + y * y + z * z)
    range_ok = (distance > handoff) & (distance < far)
    centre = range_ok & (z > 0) & (np.abs(x) <= z * TAN_X) & (np.abs(y) <= z * TAN_Y)
    across = (x + reach >= -(z + reach) * TAN_X) & (x - reach <= (z + reach) * TAN_X)
    high = (y + reach >= -(z + reach) * TAN_Y) & (y - reach <= (z + reach) * TAN_Y)
    touch = (distance + reach > handoff) & (distance - reach < far) & (z + reach > 0) & across & high
    return centre, touch, x, y, z


def grid_counts(x, y, z, reach, near, far):
    counts = np.zeros(GRID, dtype=np.int32)
    span = math.log(far / near)

    def tile(value, zmin, zmax, tan, cells):
        scale = zmin if value >= 0 else zmax
        return (value / (max(scale, 1.0) * tan) * 0.5 + 0.5) * cells

    for xi, yi, zi, ri in zip(x, y, z, reach):
        low, high = max(zi - ri, near), min(zi + ri, far)
        if high <= low:
            continue
        first = int(math.log(low / near) / span * GRID[2])
        last = min(int(math.log(high / near) / span * GRID[2]), GRID[2] - 1)
        for slice_index in range(first, last + 1):
            zmin = max(low, near * math.exp(span * slice_index / GRID[2]))
            zmax = min(high, near * math.exp(span * (slice_index + 1) / GRID[2]))
            x0 = max(int(tile(xi - ri, zmax, zmin, TAN_X, GRID[0])), 0)
            x1 = min(int(tile(xi + ri, zmin, zmax, TAN_X, GRID[0])), GRID[0] - 1)
            y0 = max(int(tile(yi - ri, zmax, zmin, TAN_Y, GRID[1])), 0)
            y1 = min(int(tile(yi + ri, zmin, zmax, TAN_Y, GRID[1])), GRID[1] - 1)
            if x0 <= x1 and y0 <= y1:
                counts[x0:x1 + 1, y0:y1 + 1, slice_index] += 1
    return counts


def describe(counts):
    used = counts[counts > 0]
    if used.size == 0:
        return "no cell holds a source"
    return "%d of %d cells hold a source; a used cell: mean %.1f, median %d, 99th %d, most %d; %d cells over the crate's %d, %d over 100" % (
        used.size, counts.size, used.mean(), np.median(used), np.percentile(used, 99), used.max(), (counts > CAP).sum(), CAP, (counts > 100).sum())


def main():
    path = sys.argv[1]
    view_range = float(sys.argv[sys.argv.index("--view-range") + 1]) if "--view-range" in sys.argv else 7168.0
    handoff = view_range - 1800.0 - 512.0
    data, tiles = read_sources(path)
    pos = data["pos"].astype(np.float64)
    radius = data["radius"].astype(np.float64)
    reach = radius * 1.5
    mesh = data["colour"][:, 3] == 255

    print("file %s: %d sources, %d tiles" % (path, len(data), tiles))
    print("handoff %.0f units (view range %.0f [I: the game's far plane]), reach 1.5 x radius" % (handoff, view_range))
    print("with a mesh %d, without %d" % (mesh.sum(), (~mesh).sum()))
    print("radius: median %.0f, 90th %.0f, 99th %.0f, most %.0f; reach over a cell: %d; over 2 cells: %d" % (
        np.median(radius), np.percentile(radius, 90), np.percentile(radius, 99), radius.max(), (reach > CELL).sum(), (reach > 2 * CELL).sum()))
    print("z: min %.0f, median %.0f, max %.0f" % (pos[:, 2].min(), np.median(pos[:, 2]), pos[:, 2].max()))

    for name, eye, heading in VIEWS:
        frame = camera(eye, heading)
        print("\n%s, eye %s, heading %.0f" % (name, eye, heading))
        for cells in DRAW_DISTANCES:
            far = cells * CELL
            centre, touch, x, y, z = in_view(pos, reach, handoff, far, frame)
            print("  draw distance %.0f cells: %d sources in view past the handoff (%d with a mesh, %d without); %d reach into the view" % (
                cells, centre.sum(), (centre & mesh).sum(), (centre & ~mesh).sum(), touch.sum()))
            print("    the grid: " + describe(grid_counts(x[touch], y[touch], z[touch], reach[touch], handoff, far)))
        # L2: a reach for the clustered lights, past it the baked tile and the glows.
        for cells in (2.0, 4.0, 8.0):
            far = cells * CELL
            centre, touch, x, y, z = in_view(pos, reach, handoff, far, frame)
            print("  L2's reach %.0f cells: %d in view; the grid: %s" % (cells, centre.sum(), describe(grid_counts(x[touch], y[touch], z[touch], reach[touch], handoff, far))))

    # The worst place: an eye over each cell with a source, in four headings, level, at z 3500.
    cells_with = np.unique(np.floor(pos[:, :2] / CELL).astype(np.int64), axis=0)
    print("\nthe worst place, over %d cells with a source, 4 headings:" % len(cells_with))
    for far_cells in DRAW_DISTANCES:
        far = far_cells * CELL
        best = (0, None, None)
        counts = []
        for cx, cy in cells_with:
            eye = np.array([(cx + 0.5) * CELL, (cy + 0.5) * CELL, 3500.0])
            relative = pos - eye
            for heading in (0.0, 90.0, 180.0, 270.0):
                _eye, right, up, forward = camera(eye, heading)
                xv, yv, zv = relative @ right, relative @ up, relative @ forward
                distance = np.sqrt(xv * xv + yv * yv + zv * zv)
                n = int(((distance > handoff) & (distance < far) & (zv > 0) & (np.abs(xv) <= zv) & (np.abs(yv) <= zv * TAN_Y)).sum())
                counts.append(n)
                if n > best[0]:
                    best = (n, (int(cx), int(cy)), heading)
        counts = np.array(counts)
        print("  draw distance %.0f cells: most %d at cell %s heading %.0f; over all: median %d, 90th %d, 99th %d" % (
            far_cells, best[0], best[1], best[2], np.median(counts), np.percentile(counts, 90), np.percentile(counts, 99)))
        eye = np.array([(best[1][0] + 0.5) * CELL, (best[1][1] + 0.5) * CELL, 3500.0])
        centre, touch, x, y, z = in_view(pos, reach, handoff, far, camera(eye, best[2]))
        print("    its grid: " + describe(grid_counts(x[touch], y[touch], z[touch], reach[touch], handoff, far)))

    # The bricks of the 3D volume.
    for spacing, across, layers in ((1024.0, 8, 4), (512.0, 16, 8)):
        height = spacing * layers
        bricks = set()
        for (px, py, pz), r in zip(pos, reach):
            for cx in range(int(math.floor((px - r) / CELL)), int(math.floor((px + r) / CELL)) + 1):
                for cy in range(int(math.floor((py - r) / CELL)), int(math.floor((py + r) / CELL)) + 1):
                    for layer in range(int(math.floor((pz - r) / height)), int(math.floor((pz + r) / height)) + 1):
                        bricks.add((cx, cy, layer))
        cells = {(b[0], b[1]) for b in bricks}
        voxels = across * across * layers
        print("\nthe volume, a voxel every %.0f units: %d bricks in %d cells, %.1f bricks a cell, %d voxels a brick; %.1f MB at 24 bytes a voxel; the index of the cells beside it" % (
            spacing, len(bricks), len(cells), len(bricks) / max(len(cells), 1), voxels, len(bricks) * voxels * 24 / 1048576.0))


main()
