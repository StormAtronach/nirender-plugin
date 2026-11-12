"""The census of the frames of a capture: each scene by kind, with its target, its view, its
records by tag and its draws by class, the names of its shapes, and where the main view's
lists stand against the swap and the reads. A page (the frame map) can cite a line of it by
the name of the capture and the line number: the lines come in one order for one capture and
one set of options.

    python -I tools/frame_census.py <capture.nircap> [--frames N] [--top N | --names]
    python -I tools/frame_census.py --self-test    # planted captures in a temporary folder

The lines, each with its key first:

    census: <file> format <F> build <id> frames <N>; snapshot copies <n>
    frame <f> scene <s> kind <k> <name> flags <0x..> target <back buffer | texture 0x..> view <slot | none>: records <TAG> <n>, ...; shapes <n> (class <c> <name> <n>, ...); table draws <n> (...); draw commands <n>; up draws <n>
    frame <f> scene <s> names: '<name>' <n>, ... (<shown> of <distinct>)
    frame <f> shown <i>: ids <n> (class ...); <in scene <s> kind <k> | outside the scenes, after scene <s> kind <k> | before the first scene>; reads before it <n>, after it <n>; the swap <after it | before it | not in the frame>
    kind <k> <name>: <scenes> scenes in <frames> of <N> frames, per frame <min> to <max>; targets back buffer <n>, texture <n>; views <slot> <n>, ...; records ...; shapes ...; table draws ...; draw commands <n>; up draws <n>
    kind <k> <name> names: '<name>' <n>, ... (<shown> of <distinct>)
    shown: <sends> sends in <frames> of <N> frames, <n> frames with more than one; <n> in a scene; <n> after a read; <n> after the swap

The frame lines come first, in the order of the capture; then one summary line for each kind
that the capture has, and always one for the kinds 4 (the shadow volumes) and 5 (the shadow
casters), with 0 when the capture has none; then the shown line.

What the lines count:
- a scene runs from SCENE_BEGIN to SCENE_END. Its target is the back buffer when its flags have
  SCENE_BACK_BUFFER (0x8), else the texture of its render_target word; its view is the view slot
  of the SceneV2 `view` word (the low byte minus 1), "none" for 0.
- records: the records of the tags in COUNTED_TAGS inside the scene.
- a SHAPE record is a draw on the record path; its class is the class of the OBJECT record
  before it in the scene (the object of the draws that follow), "no object" without one, and
  its name is that object's name.
- a table draw is an INSTANCE_DRAW, INSTANCE_QUEUED or INSTANCE_WALK record; its class and its
  name are those of the INSTANCE record of its id (the last one before it in the capture),
  "unknown instance" for an id with no record.
- a draw command is a COMMAND record DrawIndexed or Draw; an up draw is a DRAW_UP record.
- an INSTANCE_SHOWN record is a list of the main view (form D). The swap is the CALL record
  Present of its frame; a read is a READ_PIXELS record (in a stream only) or a CALL record
  CopyPixels. The snapshot of the device objects at the start of a capture file (frame 0,
  before its first scene) has CopyPixels calls of its own: those are counted as snapshot
  copies, not as reads.
Records outside a scene are not counted in a scene.

The record layouts are those of the stream formats in READS_FORMATS. A capture of another
format, or with no HEADER record first, is refused with a line and the exit code 2: a guess of
a layout would give numbers that look right. A file that cannot be read gets the same. The names of the scene kinds and of the classes
come from src/link.rs (`SCENE_*` and `CLASS_*`); without the file the lines give the numbers.
"""
import argparse
import collections
import os
import re
import struct
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import capture_dump as cd  # noqa: E402

# The stream formats whose layouts of HEADER, SCENE_BEGIN (SceneV2), OBJECT, SHAPE, INSTANCE,
# INSTANCE_DRAW, INSTANCE_QUEUED, INSTANCE_WALK, INSTANCE_SHOWN, COMMAND and CALL this tool
# reads (src/stream_format.rs, STREAM_FORMAT). The self-test fails when the plugin writes a
# format outside this list.
READS_FORMATS = (10,)
TAG_HEADER, TAG_READ_PIXELS, TAG_DRAW_UP = 11, 13, 15
TAG_INSTANCE_QUEUED, TAG_INSTANCE_WALK = 31, 32
# The HEADER record (src/stream_format.rs: HEADER_FORMAT_AT, HEADER_BUILD_ID_AT, HEADER_BUILD_ID_BYTES).
HEADER_FORMAT_AT, HEADER_BUILD_ID_AT, HEADER_BUILD_ID_BYTES = 16, 20, 64
# The words of SceneV2 (src/link.rs) that the lines show, as byte offsets.
SCENE_KIND_AT, SCENE_TARGET_AT, SCENE_FLAGS_AT, SCENE_VIEW_AT = 4, 16, 20, 48
SCENE_BACK_BUFFER = 0x8
# The words of an INSTANCE record before its name (src/capture.rs, INSTANCE_HEAD_WORDS).
INSTANCE_HEAD_WORDS = 44
# The COMMAND records that draw (capture_dump.COMMANDS), and the CALL records of the swap
# and of a read (capture_dump.CALLS).
DRAW_COMMANDS = (4, 5)
CALL_PRESENT, CALL_COPY_PIXELS = 6, 23
# The records counted in each scene, in the order of the lines.
COUNTED_TAGS = (("SHAPE", cd.TAG_SHAPE), ("OBJECT", cd.TAG_OBJECT), ("INSTANCE_SHOWN", cd.TAG_INSTANCE_SHOWN),
                ("INSTANCE_QUEUED", TAG_INSTANCE_QUEUED), ("INSTANCE_WALK", TAG_INSTANCE_WALK),
                ("INSTANCE_DRAW", cd.TAG_INSTANCE_DRAW))
COUNTED = frozenset(tag for _, tag in COUNTED_TAGS)
TABLE_DRAWS = (cd.TAG_INSTANCE_DRAW, TAG_INSTANCE_QUEUED, TAG_INSTANCE_WALK)
# The kinds that always get a summary line: the scenes of the shadow camera.
SHADOW_KINDS = (4, 5)
# The names shown for each scene and each kind without --names.
TOP_NAMES = 10
NO_OBJECT, UNKNOWN_INSTANCE = "no object", "unknown instance"
LINK_RS = os.path.join(REPO, "src", "link.rs")


class Refused(Exception):
    """The capture is not one this tool reads."""


def link_names(path=LINK_RS):
    """The names of the scene kinds and of the classes, {number: name}, from the decimal
    `SCENE_*` and `CLASS_*` constants of src/link.rs; empty when the file is missing. The flag
    constants of the same prefix are written in hex and are not read."""
    kinds, classes = {}, {}
    try:
        text = open(path, encoding="utf-8").read()
    except OSError:
        return kinds, classes
    for prefix, name, value in re.findall(r"pub const (SCENE|CLASS)_(\w+): u32 = (\d+);", text):
        (kinds if prefix == "SCENE" else classes).setdefault(int(value), name.lower())
    return kinds, classes


def header(path):
    """The stream format and the build id of a capture, from its HEADER record. Raises Refused."""
    try:
        with open(path, "rb") as f:
            head = f.read(16 + HEADER_BUILD_ID_AT + HEADER_BUILD_ID_BYTES)
    except OSError as error:
        raise Refused("%s cannot be read: %s" % (path, error.strerror))
    if head[:8] != b"NIRCAP01":
        raise Refused("%s is not a capture: no NIRCAP01 at its start" % path)
    if len(head) < 16:
        raise Refused("%s has no record" % path)
    tag, length = struct.unpack_from("<II", head, 8)
    if tag != TAG_HEADER or length < HEADER_FORMAT_AT + 4:
        raise Refused("%s has no HEADER record first (tag %d, %d bytes): its stream format is not known" % (path, tag, length))
    body = head[16:16 + length]
    stream_format = struct.unpack_from("<I", body, HEADER_FORMAT_AT)[0]
    if stream_format not in READS_FORMATS:
        raise Refused("%s has the stream format %d; this tool reads the formats %s: check the record layouts "
                      "against src/capture.rs and src/link.rs, and add the format to READS_FORMATS" % (
                          path, stream_format, list(READS_FORMATS)))
    build = body[HEADER_BUILD_ID_AT:HEADER_BUILD_ID_AT + HEADER_BUILD_ID_BYTES].split(b"\0")[0].decode("ascii", "replace")
    return stream_format, build or "none"


class Counts:
    """The counts of one scene, or of all the scenes of one kind."""

    def __init__(self):
        self.tags = collections.Counter()
        self.shapes = collections.Counter()
        self.table_draws = collections.Counter()
        self.names = collections.Counter()
        self.draw_commands = 0
        self.up_draws = 0
        self.targets = collections.Counter()
        self.views = collections.Counter()

    def add(self, other):
        for name in ("tags", "shapes", "table_draws", "names", "targets", "views"):
            getattr(self, name).update(getattr(other, name))
        self.draw_commands += other.draw_commands
        self.up_draws += other.up_draws


class Scene:
    def __init__(self, frame, index, body):
        self.frame, self.index = frame, index
        self.kind, self.flags = cd.words(body, SCENE_KIND_AT, 1)[0], cd.words(body, SCENE_FLAGS_AT, 1)[0]
        target = cd.words(body, SCENE_TARGET_AT, 1)[0]
        self.target = "back buffer" if self.flags & SCENE_BACK_BUFFER else "texture %#x" % target
        view = cd.words(body, SCENE_VIEW_AT, 1)[0] & 0xFF if len(body) >= SCENE_VIEW_AT + 4 else 0
        self.view = str(view - 1) if view else "none"
        self.counts = Counts()
        self.counts.targets["back buffer" if self.flags & SCENE_BACK_BUFFER else "texture"] += 1
        self.counts.views[self.view] += 1
        self.object = None


class Shown:
    """An INSTANCE_SHOWN record and its place in its frame."""

    def __init__(self, frame, index, classes, scene, last_scene, reads_before):
        self.frame, self.index, self.classes = frame, index, classes
        self.scene, self.last_scene, self.reads_before = scene, last_scene, reads_before
        self.reads_after = 0
        self.swap = "not in the frame"


def census(path, frames=None):
    """The scenes and the shown lists of a capture: (format, build, the frames read, scenes, shown)."""
    stream_format, build = header(path)
    scenes, shown = [], []
    instances = {}
    current = None
    frame_scenes, frame_shown, reads, swapped = [], [], 0, False
    read = 0
    snapshot_copies = 0

    def end_frame():
        for entry in frame_shown:
            entry.reads_after = reads - entry.reads_before
            if entry.swap == "not in the frame" and swapped:
                entry.swap = "after it"

    last_frame = 0
    for frame, tag, body in cd.records(path):
        if frames is not None and frame >= frames:
            break
        if frame != last_frame:
            end_frame()
            frame_scenes, frame_shown, reads, swapped, current = [], [], 0, False, None
            last_frame = frame
        read = max(read, frame + 1)
        if tag == cd.TAG_INSTANCE and len(body) >= 8:
            instance, cls = cd.words(body, 0, 2)
            instances[instance] = (cls, body[4 * INSTANCE_HEAD_WORDS:].split(b"\0")[0].decode("latin-1"))
        elif tag == TAG_READ_PIXELS or (tag == cd.TAG_CALL and len(body) >= 4 and cd.words(body, 0, 1)[0] == CALL_COPY_PIXELS):
            if frame == 0 and not scenes:
                snapshot_copies += 1
            else:
                reads += 1
        elif tag == cd.TAG_CALL and len(body) >= 4 and cd.words(body, 0, 1)[0] == CALL_PRESENT:
            swapped = True
            for entry in frame_shown:
                if entry.swap == "not in the frame":
                    entry.swap = "after it"
        if tag == cd.TAG_INSTANCE_SHOWN and len(body) >= 4:
            count = cd.words(body, 0, 1)[0]
            ids = cd.words(body, 4, count) if len(body) >= 4 + 4 * count else ()
            classes = collections.Counter(instances.get(i, (UNKNOWN_INSTANCE, ""))[0] for i in ids)
            entry = Shown(frame, len(frame_shown), classes, current, frame_scenes[-1] if frame_scenes else None, reads)
            if swapped:
                entry.swap = "before it"
            frame_shown.append(entry)
            shown.append(entry)
        if tag == cd.TAG_SCENE_BEGIN and len(body) >= SCENE_FLAGS_AT + 4:
            current = Scene(frame, len(frame_scenes), body)
            frame_scenes.append(current)
            scenes.append(current)
            continue
        if tag == cd.TAG_SCENE_END:
            current = None
            continue
        if current is None:
            continue
        counts = current.counts
        if tag in COUNTED:
            counts.tags[tag] += 1
        if tag == cd.TAG_OBJECT and len(body) >= 12:
            current.object = (cd.words(body, 4, 1)[0], cd.object_name(body).split("\0")[0])
        elif tag == cd.TAG_SHAPE:
            cls, name = current.object or (NO_OBJECT, "")
            counts.shapes[cls] += 1
            counts.names[name] += 1
        elif tag in TABLE_DRAWS and len(body) >= 4:
            cls, name = instances.get(cd.words(body, 0, 1)[0], (UNKNOWN_INSTANCE, ""))
            counts.table_draws[cls] += 1
            counts.names[name] += 1
        elif tag == cd.TAG_COMMAND and len(body) >= 4 and cd.words(body, 0, 1)[0] in DRAW_COMMANDS:
            counts.draw_commands += 1
        elif tag == TAG_DRAW_UP:
            counts.up_draws += 1
    end_frame()
    return stream_format, build, read, scenes, shown, snapshot_copies


def by_class(counter, classes):
    """`<n> (class <c> <name> <n>, ...)`, the classes in their number order, the others after."""
    def key(cls):
        return (0, cls, "") if isinstance(cls, int) else (1, 0, cls)

    parts = []
    for cls in sorted(counter, key=key):
        label = "class %d %s" % (cls, classes.get(cls, "?")) if isinstance(cls, int) else cls
        parts.append("%s %d" % (label, counter[cls]))
    return "%d (%s)" % (sum(counter.values()), ", ".join(parts))


def names_text(names, top):
    """The names by count and then by name; `top` None gives all of them."""
    ordered = sorted(names.items(), key=lambda kv: (-kv[1], kv[0]))
    shown = ordered if top is None else ordered[:top]
    return "%s (%d of %d)" % (", ".join("%r %d" % (name, n) for name, n in shown), len(shown), len(ordered))


def counts_text(counts, classes):
    tags = ", ".join("%s %d" % (name, counts.tags[tag]) for name, tag in COUNTED_TAGS)
    return "records %s; shapes %s; table draws %s; draw commands %d; up draws %d" % (
        tags, by_class(counts.shapes, classes), by_class(counts.table_draws, classes), counts.draw_commands, counts.up_draws)


def scene_name(scene, kinds):
    return "scene %d kind %d %s" % (scene.index, scene.kind, kinds.get(scene.kind, "?"))


def lines(path, frames=None, top=TOP_NAMES, names=None):
    """The lines of the census of a capture. Raises Refused."""
    kinds, classes = names if names is not None else link_names()
    stream_format, build, read, scenes, shown, snapshot_copies = census(path, frames)
    out = ["census: %s format %d build %s frames %d; snapshot copies %d" % (
        os.path.basename(path), stream_format, build, read, snapshot_copies)]
    shown_of = {}
    for entry in shown:
        shown_of.setdefault(entry.frame, []).append(entry)
    by_kind = collections.defaultdict(Counts)
    per_frame = collections.defaultdict(collections.Counter)
    frames_seen = sorted(set(s.frame for s in scenes) | set(shown_of))
    scenes_of = collections.defaultdict(list)
    for scene in scenes:
        scenes_of[scene.frame].append(scene)
    for frame in frames_seen:
        for scene in scenes_of[frame]:
            head = "frame %d scene %d" % (frame, scene.index)
            out.append("%s kind %d %s flags %#x target %s view %s: %s" % (
                head, scene.kind, kinds.get(scene.kind, "?"), scene.flags, scene.target, scene.view, counts_text(scene.counts, classes)))
            if scene.counts.names:
                out.append("%s names: %s" % (head, names_text(scene.counts.names, top)))
            by_kind[scene.kind].add(scene.counts)
            per_frame[scene.kind][frame] += 1
        for entry in shown_of.get(frame, []):
            if entry.scene is not None:
                place = "in " + scene_name(entry.scene, kinds)
            elif entry.last_scene is not None:
                place = "outside the scenes, after " + scene_name(entry.last_scene, kinds)
            else:
                place = "before the first scene"
            out.append("frame %d shown %d: ids %s; %s; reads before it %d, after it %d; the swap %s" % (
                frame, entry.index, by_class(entry.classes, classes), place, entry.reads_before, entry.reads_after, entry.swap))
    for kind in sorted(set(by_kind) | set(SHADOW_KINDS)):
        counts = [per_frame[kind][f] for f in range(read)]
        with_kind = [n for n in counts if n]
        spread = "%d to %d" % (min(with_kind), max(with_kind)) if with_kind else "0 to 0"
        total = by_kind[kind]
        views = ", ".join("%s %d" % (v, n) for v, n in sorted(total.views.items())) or "none"
        head = "kind %d %s" % (kind, kinds.get(kind, "?"))
        out.append("%s: %d scenes in %d of %d frames, per frame %s; targets back buffer %d, texture %d; views %s; %s" % (
            head, sum(counts), len(with_kind), read, spread, total.targets["back buffer"], total.targets["texture"], views,
            counts_text(total, classes)))
        if total.names:
            out.append("%s names: %s" % (head, names_text(total.names, top)))
    more = sum(1 for f in shown_of if len(shown_of[f]) > 1)
    out.append("shown: %d sends in %d of %d frames, %d frames with more than one; %d in a scene; %d after a read; %d after the swap" % (
        len(shown), len(shown_of), read, more, sum(1 for e in shown if e.scene is not None),
        sum(1 for e in shown if e.reads_before), sum(1 for e in shown if e.swap == "before it")))
    return out


def record(tag, body):
    return struct.pack("<II", tag, len(body)) + body


def plant(path, stream_format=READS_FORMATS[0], header_first=True, shadow=True, lists=True):
    """A small capture: 2 frames. Instance 7 (class 0, 'Tri Body') and a snapshot copy
    (CopyPixels) come in frame 0 before the scenes. Each frame: a shape outside any scene; a world scene into the back buffer with an
    object of class 12 ('Tri Wall') and 2 of its shapes, a draw and a queued place of instance 7,
    and a DrawIndexed command; with `shadow` a kind 5 scene into texture 0x50 of view slot 2
    with 3 shapes of an object of class 0 ('Tri Head'); a CopyPixels call; the list of the main
    view with instance 7; the swap. Frame 1 has a second list after the swap. Without `lists`
    frame 1 has no list."""
    def words(*values):
        return struct.pack("<%dI" % len(values), *values)

    def scene(kind, flags, target=0, view=0):
        return record(cd.TAG_SCENE_BEGIN, words(88, kind, 0, 0, target, flags, 0, 0, 0, 0, 0, 0, view, 0) + bytes(32))

    def obj(cls, name):
        return record(cd.TAG_OBJECT, words(1, cls, 0x300, 0, 0, 0) + name)

    def call(number):
        return record(cd.TAG_CALL, words(number, 0, 0, 0))

    shape = record(cd.TAG_SHAPE, words(1, 2, 0, 0))
    head = words(1920, 1080, 0, 0, stream_format) + b"test build".ljust(HEADER_BUILD_ID_BYTES, b"\0")
    data = b"NIRCAP01"
    if header_first:
        data += record(TAG_HEADER, head)
    data += record(cd.TAG_INSTANCE, words(7, 0, *([0] * (INSTANCE_HEAD_WORDS - 2))) + b"Tri Body\0")
    # The snapshot of the device objects: a copy before the first scene of frame 0.
    data += call(CALL_COPY_PIXELS)
    for frame in range(2):
        data += shape
        data += scene(1, 0x69) + obj(12, b"Tri Wall") + shape + shape
        data += record(cd.TAG_INSTANCE_DRAW, words(7, 0, 0, 0)) + record(TAG_INSTANCE_QUEUED, words(7))
        data += record(cd.TAG_COMMAND, words(4, 0, 0, 0)) + record(cd.TAG_SCENE_END, words(0))
        if shadow:
            data += scene(5, 0x20, 0x50, 3) + obj(0, b"Tri Head") + shape + shape + shape + record(cd.TAG_SCENE_END, words(1))
        data += call(CALL_COPY_PIXELS)
        if lists or frame == 0:
            data += record(cd.TAG_INSTANCE_SHOWN, words(1, 7))
        data += call(CALL_PRESENT)
        if frame == 1 and lists:
            data += record(cd.TAG_INSTANCE_SHOWN, words(1, 7))
        data += record(cd.TAG_FRAME_END, b"")
    with open(path, "wb") as f:
        f.write(data)


def refused_with(path, names):
    try:
        lines(path, names=names)
    except Refused as refusal:
        return str(refusal)
    raise AssertionError(path + " was read")


def self_test():
    names = ({1: "world", 4: "shadow_volume", 5: "shadow_casters"}, {0: "unknown", 12: "object"})
    none = "records SHAPE 0, OBJECT 0, INSTANCE_SHOWN 0, INSTANCE_QUEUED 0, INSTANCE_WALK 0, INSTANCE_DRAW 0"
    with tempfile.TemporaryDirectory() as folder:
        good = os.path.join(folder, "good.nircap")
        plant(good)
        out = lines(good, names=names)
        assert out[0] == "census: good.nircap format 10 build test build frames 2; snapshot copies 1", out[0]
        assert out[1] == ("frame 0 scene 0 kind 1 world flags 0x69 target back buffer view none: records SHAPE 2, OBJECT 1, "
                          "INSTANCE_SHOWN 0, INSTANCE_QUEUED 1, INSTANCE_WALK 0, INSTANCE_DRAW 1; shapes 2 (class 12 object 2); "
                          "table draws 2 (class 0 unknown 2); draw commands 1; up draws 0"), out[1]
        assert out[2] == "frame 0 scene 0 names: 'Tri Body' 2, 'Tri Wall' 2 (2 of 2)", out[2]
        assert out[3] == ("frame 0 scene 1 kind 5 shadow_casters flags 0x20 target texture 0x50 view 2: records SHAPE 3, OBJECT 1, "
                          "INSTANCE_SHOWN 0, INSTANCE_QUEUED 0, INSTANCE_WALK 0, INSTANCE_DRAW 0; shapes 3 (class 0 unknown 3); "
                          "table draws 0 (); draw commands 0; up draws 0"), out[3]
        assert out[5] == ("frame 0 shown 0: ids 1 (class 0 unknown 1); outside the scenes, after scene 1 kind 5 shadow_casters; "
                          "reads before it 1, after it 0; the swap after it"), out[5]
        frame1 = [line for line in out if line.startswith("frame 1 shown")]
        assert frame1[1] == ("frame 1 shown 1: ids 1 (class 0 unknown 1); outside the scenes, after scene 1 kind 5 shadow_casters; "
                             "reads before it 1, after it 0; the swap before it"), frame1
        summary = {line.split(":")[0]: line for line in out if line.startswith("kind ")}
        assert summary["kind 5 shadow_casters"] == (
            "kind 5 shadow_casters: 2 scenes in 2 of 2 frames, per frame 1 to 1; targets back buffer 0, texture 2; views 2 2; "
            "records SHAPE 6, OBJECT 2, INSTANCE_SHOWN 0, INSTANCE_QUEUED 0, INSTANCE_WALK 0, INSTANCE_DRAW 0; shapes 6 "
            "(class 0 unknown 6); table draws 0 (); draw commands 0; up draws 0"), summary
        assert summary["kind 4 shadow_volume"] == (
            "kind 4 shadow_volume: 0 scenes in 0 of 2 frames, per frame 0 to 0; targets back buffer 0, texture 0; views none; "
            + none + "; shapes 0 (); table draws 0 (); draw commands 0; up draws 0"), summary
        assert "kind 4 shadow_volume names" not in summary
        # The shape outside a scene is not counted: 2 world shapes in each frame.
        assert "shapes 4 (class 12 object 4)" in summary["kind 1 world"], summary
        assert out[-1] == "shown: 3 sends in 2 of 2 frames, 1 frames with more than one; 0 in a scene; 3 after a read; 1 after the swap", out[-1]
        # --frames 1 reads one frame; the same lines come for one capture each time.
        assert lines(good, frames=1, names=names)[0].endswith("frames 1; snapshot copies 1")
        assert lines(good, names=names) == out
        assert lines(good, top=1, names=names)[2] == "frame 0 scene 0 names: 'Tri Body' 2 (1 of 2)"
        # The null: a capture with no kind 5 scene gives 0 for kind 5.
        plain = os.path.join(folder, "plain.nircap")
        plant(plain, shadow=False, lists=False)
        plain_lines = lines(plain, names=names)
        summary = [line for line in plain_lines if line.startswith("kind 5")]
        # A frame with no list is not a frame with a list.
        assert plain_lines[-1] == "shown: 1 sends in 1 of 2 frames, 0 frames with more than one; 0 in a scene; 1 after a read; 0 after the swap", plain_lines[-1]
        assert summary == ["kind 5 shadow_casters: 0 scenes in 0 of 2 frames, per frame 0 to 0; targets back buffer 0, texture 0; "
                           "views none; " + none + "; shapes 0 (); table draws 0 (); draw commands 0; up draws 0"], summary
        # Refusals: another format, no HEADER first, not a capture.
        other = os.path.join(folder, "other.nircap")
        plant(other, stream_format=11)
        assert "has the stream format 11" in refused_with(other, names)
        headless = os.path.join(folder, "headless.nircap")
        plant(headless, header_first=False)
        assert "has no HEADER record first" in refused_with(headless, names)
        text = os.path.join(folder, "text.nircap")
        with open(text, "wb") as f:
            f.write(b"not a capture")
        assert "no NIRCAP01" in refused_with(text, names)
        assert "cannot be read" in refused_with(os.path.join(folder, "missing.nircap"), names)
    # The names of src/link.rs: the kinds 4 and 5 and the classes 0 and 13.
    kinds, classes = link_names()
    if kinds:
        assert kinds[4] == "shadow_volume" and kinds[5] == "shadow_casters", kinds
        assert classes[0] == "unknown" and classes[13] == "shadow_volume", classes
    # The plugin's format is one that this tool reads.
    stream_format = open(os.path.join(REPO, "src", "stream_format.rs"), encoding="utf-8").read()
    written = int(re.search(r"pub const STREAM_FORMAT: u32 = (\d+);", stream_format).group(1))
    assert written in READS_FORMATS, "the plugin writes the stream format %d; frame_census.py reads %s: check its layouts" % (
        written, list(READS_FORMATS))
    print("frame_census self-test: ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("capture", nargs="?")
    ap.add_argument("--frames", type=int, help="read the first N frames only")
    group = ap.add_mutually_exclusive_group()
    group.add_argument("--top", type=int, default=TOP_NAMES, help="the names shown for each scene and kind (default %d)" % TOP_NAMES)
    group.add_argument("--names", action="store_true", help="all the names")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.capture:
        ap.error("give a capture, or --self-test")
    try:
        for line in lines(args.capture, args.frames, None if args.names else args.top):
            print(line)
    except Refused as refusal:
        print("REFUSED %s" % refusal)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
