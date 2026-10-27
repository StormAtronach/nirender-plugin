"""Corrupts a capture file at seeded random places and replays each mutant: the reader of the
helper must refuse the damage, never abort.

    python -I tools/fuzz_capture.py <capture.nircap> --host <nirender-host.exe> [--seed N]
        [--mutants N] [--frame N] [--backend vulkan|dx12] [--out DIR] [--keep]
    python -I tools/fuzz_capture.py --self-test    # the pure parts, no replay

A capture is the magic NIRCAP01, then records: a tag word, a length word, the bytes. The
mutants, in turn by the seed (`random.Random(seed)`; each mutation is printed with its
offsets, so a run is repeated with its seed):

- flip      some bytes anywhere in the file (most land in a record's bytes: a texture or a
            draw with wrong data, which the reader takes or refuses);
- head      a byte of a record's head (its tag or its length): a record of a wrong tag, or
            of a length that does not match the file;
- cut       the file cut inside a record: the last record is short;
- boundary  the file cut between two records: a shorter capture that is still whole.

Each mutant is replayed with the host program (`nirender-host.exe <mutant> <picture> --frame N
--backend ...`), its output kept in the out folder, and judged:

- an ABORT is an end by a crash or a panic: an exit code that is not 0 (a replay that drew)
  and not 2 (the usage), or an output without the host's last lines;
- a REFUSAL is a line of the reader that names the damage: a short record "refused", or a
  record of a tag the build does not know "skipped" (or the report's count of either above
  0; the report prints the short-record count on a clean replay too, with 0);
- a flip, a head change or a boundary cut passes with or without a refusal (the bytes may be
  valid data; a tag changed into another known tag, on a record its fixed part fits, is a
  valid record of the format that no length check can see; a boundary cut is a whole file);
  a cut inside a record must end with a refusal of the short record.

The run passes (exit 0) when no mutant aborted and every cut inside a record was refused.
The mutant files are deleted at the end unless `--keep`. Standard library only.
"""
import argparse
import os
import random
import re
import subprocess
import sys
import time

MAGIC = b"NIRCAP01"
KINDS = ("flip", "head", "cut", "boundary")
FLIPS = 8
REPLAY_SECONDS = 180


def records_of(data):
    """The records of a capture as (offset of the head, tag, length), as the host parses them:
    from the magic to the first head that the file cannot hold whole."""
    if data[:8] != MAGIC:
        raise ValueError("not a capture file (no NIRCAP01 magic)")
    records = []
    at = 8
    while at + 8 <= len(data):
        tag = int.from_bytes(data[at:at + 4], "little")
        length = int.from_bytes(data[at + 4:at + 8], "little")
        if at + 8 + length > len(data):
            break
        records.append((at, tag, length))
        at += 8 + length
    return records


def mutate(data, records, kind, rng):
    """One mutant of `data` of the given kind, and the text that says what changed."""
    data = bytearray(data)
    if kind == "flip":
        places = sorted(rng.sample(range(8, len(data)), min(FLIPS, len(data) - 8)))
        changes = []
        for place in places:
            old = data[place]
            data[place] = old ^ (1 << rng.randrange(8))
            changes.append("%d: %#04x -> %#04x" % (place, old, data[place]))
        return bytes(data), "flip of %d bytes (%s)" % (len(places), ", ".join(changes))
    if kind == "head":
        offset, tag, length = rng.choice(records)
        place = offset + rng.randrange(8)
        old = data[place]
        data[place] = old ^ (1 << rng.randrange(8))
        word = "tag" if place < offset + 4 else "length"
        return bytes(data), "head of the record at %d (tag %d, %d bytes): the %s byte %d: %#04x -> %#04x" % (offset, tag, length, word, place, old, data[place])
    if kind == "cut":
        offset, tag, length = rng.choice([record for record in records if record[2] > 0] or records)
        end = offset + 8 + max(length, 1)
        place = rng.randrange(offset + 8, end) if length > 0 else offset + rng.randrange(1, 8)
        return bytes(data[:place]), "cut at %d, inside the record at %d (tag %d, %d bytes)" % (place, offset, tag, length)
    if kind == "boundary":
        offset, tag, length = rng.choice(records)
        return bytes(data[:offset]), "cut at %d, before the record at %d (tag %d): the file ends whole" % (offset, offset, tag)
    raise ValueError(kind)


def mutants(data, seed, count):
    """`count` mutants of `data` from the seed, the kinds in turn: (kind, bytes, text)."""
    rng = random.Random(seed)
    records = records_of(data)
    if not records:
        raise ValueError("the capture has no whole record")
    out = []
    for index in range(count):
        kind = KINDS[index % len(KINDS)]
        mutant, text = mutate(data, records, kind, rng)
        out.append((kind, mutant, text))
    return out


def refusals(output):
    """Whether the host saw damage and said so: the reader's refusal of a short record ("...:
    N bytes, M needed: refused") or its skip of a record of a tag it does not know ("which
    this build does not know: skipped"), or a count above 0 in the report's lines of the two
    (a clean replay prints the short-record count too, with 0)."""
    if re.search(r"needed: refused|which this build does not know: skipped", output):
        return True
    counts = re.findall(r"(\d+) records of any tag shorter than their tag needs, refused", output)
    counts += re.findall(r"records of a tag this build does not know: (\d+), skipped", output)
    return any(int(count) > 0 for count in counts)


def judge(kind, exit_code, output):
    """The verdict of one replay: (passed, the reason)."""
    refused = refusals(output)
    ended = "records," in output or refused or exit_code == 2
    if exit_code not in (0, 2) or not ended:
        return False, "ABORT: exit code %d%s" % (exit_code, "" if ended else ", the host did not get to its report")
    if kind == "cut" and not refused:
        return False, "no refusal of a cut inside a record: the short record was not seen"
    return True, "refused" if refused else "no refusal (taken as valid)"


def self_test():
    """The pure parts: the record walk, each mutation and the verdicts."""
    record = lambda tag, body: tag.to_bytes(4, "little") + len(body).to_bytes(4, "little") + body
    data = MAGIC + record(1, b"abcdef") + record(7, b"") + record(5, b"xyz")
    assert records_of(data) == [(8, 1, 6), (22, 7, 0), (30, 5, 3)], records_of(data)
    assert records_of(data[:-1]) == [(8, 1, 6), (22, 7, 0)]
    assert records_of(data[:12]) == []
    try:
        records_of(b"NOPE" + data)
        raise AssertionError("no magic check")
    except ValueError:
        pass
    rng = random.Random(3)
    records = records_of(data)
    flipped, text = mutate(data, records, "flip", rng)
    assert len(flipped) == len(data) and flipped != data and flipped[:8] == MAGIC and text.startswith("flip of")
    headed, text = mutate(data, records, "head", rng)
    assert len(headed) == len(data) and sum(a != b for a, b in zip(headed, data)) == 1 and "head of the record" in text
    cut, text = mutate(data, records, "cut", rng)
    assert len(cut) < len(data) and cut == data[:len(cut)] and "inside the record" in text
    whole, text = mutate(data, records, "boundary", rng)
    assert records_of(whole) == records[:len(records_of(whole))] and "ends whole" in text
    kinds = [kind for kind, _, _ in mutants(data, 1, 6)]
    assert kinds == ["flip", "head", "cut", "boundary", "flip", "head"]
    assert mutants(data, 9, 3) == mutants(data, 9, 3), "a seed repeats its mutants"
    assert judge("flip", 0, "3 records, 1 frames\nreport\n") == (True, "no refusal (taken as valid)")
    # A clean replay prints the report's count with 0: that is no refusal.
    clean = "23477 records, 30 frames\nmaterial: 198 records; 0 records of any tag shorter than their tag needs, refused\n"
    assert not refusals(clean)
    assert judge("cut", 0, clean)[1].startswith("no refusal of a cut inside a record")
    assert refusals(clean.replace("; 0 records", "; 2 records"))
    # A record of a tag the build does not know is skipped and said: the damage was seen.
    assert refusals(clean + "record of the tag 148, which this build does not know: skipped (the plugin is another build)\n")
    assert refusals(clean + "records of a tag this build does not know: 1, skipped; the first tag 148\n")
    # A head change passes without a refusal (a tag changed into another known tag is a
    # valid record); a cut inside a record does not.
    assert judge("head", 0, "3 records, 1 frames\n") == (True, "no refusal (taken as valid)")
    assert judge("cut", 0, "3 records, 1 frames\n")[1].startswith("no refusal of a cut inside a record")
    assert judge("cut", 0, "record of the tag 5 (x): 1 bytes, 4 needed: refused\n3 records,") == (True, "refused")
    assert judge("boundary", 0, "2 records, 1 frames\n") == (True, "no refusal (taken as valid)")
    assert judge("head", 3, "") == (False, "ABORT: exit code 3, the host did not get to its report")
    assert judge("flip", -1073741819, "3 records,")[1].startswith("ABORT")
    print("fuzz_capture self-test: ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("capture", nargs="?", help="the capture file to corrupt")
    ap.add_argument("--host", help="the nirender-host.exe that replays the mutants")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--mutants", type=int, default=8)
    ap.add_argument("--frame", type=int, default=1, help="the frame each replay draws (a small one keeps a run short)")
    ap.add_argument("--backend", default="vulkan")
    ap.add_argument("--out", help="the folder of the mutants, their pictures and their outputs (results/fuzz-<time> without this)")
    ap.add_argument("--keep", action="store_true", help="keep the mutant files (they are the size of the capture)")
    ap.add_argument("--self-test", action="store_true", help="the tests of the pure parts: no replay")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.capture or not args.host:
        ap.error("the capture and --host are required")
    data = open(args.capture, "rb").read()
    out = args.out or os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "results", time.strftime("%Y%m%d-%H%M%S") + "-fuzz")
    os.makedirs(out, exist_ok=True)
    print("fuzz of %s (%d bytes, %d records), seed %d, %d mutants, the host %s" % (args.capture, len(data), len(records_of(data)), args.seed, args.mutants, args.host))
    failures = 0
    for index, (kind, mutant, text) in enumerate(mutants(data, args.seed, args.mutants)):
        name = "mutant-%02d-%s" % (index, kind)
        path = os.path.join(out, name + ".nircap")
        open(path, "wb").write(mutant)
        print("%s: %s" % (name, text), flush=True)
        command = [args.host, path, os.path.join(out, name + ".png"), "--frame", str(args.frame), "--backend", args.backend]
        started = time.time()
        try:
            run = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=REPLAY_SECONDS)
            exit_code, output = run.returncode, run.stdout + run.stderr
        except subprocess.TimeoutExpired as expired:
            exit_code, output = -1, (expired.stdout or b"").decode("utf-8", "replace") + "\n(no end in %d s)\n" % REPLAY_SECONDS
        open(os.path.join(out, name + ".txt"), "w", encoding="utf-8").write(output)
        passed, reason = judge(kind, exit_code, output)
        failures += not passed
        print("  %s in %.1f s: %s (exit %d)" % ("pass" if passed else "FAIL", time.time() - started, reason, exit_code), flush=True)
        if not args.keep:
            os.remove(path)
    print("fuzz: %d of %d mutants passed; the outputs are in %s" % (args.mutants - failures, args.mutants, out))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
