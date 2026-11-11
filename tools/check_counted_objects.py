"""The GPU objects of our code are made through the counted calls of `host/src/objects.rs`
(`counted_bind_group` and so on, D288): this check fails on a `create_*` call of the device or of
a texture anywhere else, with its file and line, so that no new site escapes the count of the
objects that a steady frame makes.

    python -I tools/check_counted_objects.py                 # the host crate of this repository
    python -I tools/check_counted_objects.py --mge DIR       # the helper of an MGE XE tree (mgeHost64/src)
    python -I tools/check_counted_objects.py --self-test

A comment line is not code. objects.rs itself makes the objects. The exit code is 1 when a call is found."""
import re
import sys
from pathlib import Path

NAMES = [
    "bind_group_layout", "bind_group", "pipeline_layout", "buffer_init", "buffer", "texture_with_data",
    "texture", "sampler", "render_pipeline", "compute_pipeline", "shader_module", "query_set", "view",
]
# A method call (`device.create_buffer(`) or a call by the path of the method
# (`wgpu::util::DeviceExt::create_buffer_init(device, ...)`).
CALL = re.compile(r"(\.|::)create_(%s)\(" % "|".join(NAMES))
ALLOWED = {"objects.rs"}


def found(text):
    """The (line number, line) of each uncounted create call of a file's text."""
    out = []
    for number, line in enumerate(text.splitlines(), 1):
        code = line.split("//", 1)[0]
        if CALL.search(code):
            out.append((number, line.strip()))
    return out


def check(root):
    calls = []
    for path in sorted(root.rglob("*.rs")):
        if path.name in ALLOWED:
            continue
        for number, line in found(path.read_text(encoding="utf-8")):
            calls.append("%s:%d: %s" % (path.relative_to(root).as_posix(), number, line))
    return calls


def self_test():
    assert found("let g = device.create_bind_group(&d);\n") == [(1, "let g = device.create_bind_group(&d);")]
    assert found("let v = texture.create_view(&Default::default());") != []
    assert found("let b = device.create_buffer_init(&d);") != []
    # By the path of the method: the call of depth_merge.rs that the first version of the check missed.
    assert found("let b = wgpu::util::DeviceExt::create_buffer_init(") != []
    assert found("let g = wgpu::Device::create_bind_group(&device, &d);") != []
    # The counted calls, a command encoder, a comment and a doc line are no finding.
    assert found("let g = device.counted_bind_group(&d);\nlet e = device.create_command_encoder(&d);\n") == []
    assert found("// device.create_buffer(&d) was here\n/// `texture.create_view` makes a view\n") == []
    assert found("let x = 1; // then device.create_sampler(&d)\n") == []
    print("check_counted_objects: self-test passed")


def main(argv):
    if "--self-test" in argv:
        self_test()
        return 0
    if "--mge" in argv:
        root = Path(argv[argv.index("--mge") + 1]) / "mgeHost64" / "src"
    else:
        root = Path(__file__).resolve().parents[1] / "host" / "src"
    if not root.is_dir():
        print("counted objects: no folder %s" % root)
        return 1
    calls = check(root)
    for call in calls:
        print("counted objects: %s: make the object with the counted call of objects.rs (counted_*)" % call)
    if calls:
        return 1
    print("counted objects: no uncounted create call in %s" % root)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
