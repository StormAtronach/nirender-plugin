"""Compares the dumps of two game runs. `tools/run_test.py --dump` makes the dumps.

    python tools/compare_runs.py <run A> <run B>

A run is a folder under results/. Give the name or a unique part of it. The dumps are
state.txt and frame.txt. state.txt has the renderer and the objects that it owns, with a
marker in place of each address. frame.txt has the device calls of one frame after the save
loaded, with addresses as numbers of first appearance.

Use it to test the code that runs one time at the start of the game. Compare a run that
started with the game functions (--start off) with a run that started with the Rust functions
(--start live). Compare two "off" runs first. The port does not cause the differences between
those runs.
"""
import glob
import os
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def find_run(name):
    if os.path.isdir(name):
        return name
    matches = sorted(glob.glob(os.path.join(REPO, "results", "*%s*" % name)))
    matches = [m for m in matches if os.path.isdir(m)]
    if len(matches) != 1:
        raise SystemExit("'%s' names %d runs" % (name, len(matches)))
    return matches[0]


def read_state(path):
    """{object: {offset: word}}"""
    objects, current = {}, None
    for line in open(path, encoding="utf-8"):
        if line.startswith("object "):
            current = objects.setdefault(line.split()[1], {})
        elif ":" in line and current is not None:
            offset, words = line.strip().split(":")
            for i, word in enumerate(words.split()):
                current[int(offset, 16) + 4 * i] = word
    return objects


def same_word(a, b):
    """Two words of a state dump are the same when the values are the same, or when the two
    words are addresses. An address has the mark * and is a different number in each run."""
    if a is None or b is None:
        return a == b
    return a.lstrip("*") == b.lstrip("*") or (a.startswith("*") and b.startswith("*"))


def compare_state(a, b, ignore):
    differences = []
    for name in sorted(set(a) | set(b)):
        words_a, words_b = a.get(name, {}), b.get(name, {})
        for offset in sorted(set(words_a) | set(words_b)):
            if (name, offset) in ignore:
                continue
            if not same_word(words_a.get(offset), words_b.get(offset)):
                differences.append((name, offset, words_a.get(offset), words_b.get(offset)))
    return differences


def compare_frame(path_a, path_b):
    a = open(path_a, encoding="utf-8").read().splitlines()
    b = open(path_b, encoding="utf-8").read().splitlines()
    differences = [(i, x, y) for i, (x, y) in enumerate(zip(a, b)) if x != y]
    return len(a), len(b), differences


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) < 2:
        raise SystemExit(__doc__)
    run_a, run_b = find_run(args[0]), find_run(args[1])
    # This is a third run of the same kind as A. The differences between A and it are noise
    # of the game.
    baseline = find_run(args[2]) if len(args) > 2 else None
    print("A: %s\nB: %s" % (os.path.basename(run_a), os.path.basename(run_b)))

    different = False
    for dump in ("state_start.txt", "state.txt"):
        if not all(os.path.isfile(os.path.join(run, dump)) for run in (run_a, run_b)):
            continue
        state_a = read_state(os.path.join(run_a, dump))
        ignore = set()
        if baseline and os.path.isfile(os.path.join(baseline, dump)):
            noise = compare_state(state_a, read_state(os.path.join(baseline, dump)), set())
            ignore = {(name, offset) for name, offset, _, _ in noise}
        state = compare_state(state_a, read_state(os.path.join(run_b, dump)), ignore)
        total = sum(len(words) for words in state_a.values())
        print("\n%s: %d words in %d objects, %d differ (%d words left out as noise of the game)" % (
            dump, total, len(state_a), len(state), len(ignore)))
        for name, offset, word_a, word_b in state[:40]:
            print("  %s+0x%x: A %s  B %s" % (name, offset, word_a, word_b))
        different = different or bool(state)

    # The scene after a load is not the same in two runs of the game, because actors move with
    # the clock. So the frame is information only. The verdict is the state.
    count_a, count_b, frame = compare_frame(os.path.join(run_a, "frame.txt"), os.path.join(run_b, "frame.txt"))
    print("\nframe (information): A %d calls, B %d calls, %d lines differ" % (count_a, count_b, len(frame)))

    print("\n" + ("STATE DIFFERENT" if different else "STATE IDENTICAL"))
    return 1 if different else 0


if __name__ == "__main__":
    sys.exit(main())
