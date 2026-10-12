"""Compares the pictures of two folders by name, with `picture_diff`.

    python -I tools/compare_pictures.py <folder a> <folder b> [--most N]

Prints the number of pictures that are equal, then the pictures that differ, the largest
mean difference first: the mean, the part of the pixels that differ, the largest
difference of a colour. The gate of the tour (`validate_retained.py tour`) reads the
first line. Run with -I: the Pillow of the user site cannot load its DLL.
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import picture_diff  # noqa: E402


def compare_folders(a, b):
    """[(mean, name, text)] for the pictures of both folders, the largest mean first."""
    names = sorted(n for n in os.listdir(a) if n.endswith(".png") and os.path.isfile(os.path.join(b, n)))
    rows = []
    for name in names:
        numbers = picture_diff.compare(picture_diff.load(os.path.join(a, name)), picture_diff.load(os.path.join(b, name)))
        if "problem" in numbers:
            rows.append((float("inf"), name, numbers["problem"]))
        else:
            rows.append((numbers["mean"], name, "mean %.4f, %.2f %% different, largest %d" % (numbers["mean"], numbers["different"] * 100, numbers["largest"])))
    rows.sort(reverse=True)
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--most", type=int, default=12, help="the pictures to list")
    args = ap.parse_args()
    rows = compare_folders(args.a, args.b)
    print("%d of %d equal" % (sum(1 for mean, _, _ in rows if mean == 0), len(rows)))
    for mean, name, text in rows[: args.most]:
        if mean != 0:
            print("  %-40s %s" % (name, text))
    return 0


if __name__ == "__main__":
    sys.exit(main())
