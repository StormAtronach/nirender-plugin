"""The patch sites of an MWSE.dll against the plugin's ports, before the DLL goes to a test install.

    python -I tools/check_mwse_sites.py <MWSE.dll>     # exit code 0 when every site is followed
    python -I tools/check_mwse_sites.py --self-test    # the tests of the output filter: no cargo

It runs the unit test `mwse_sites::tests::every_mwse_site_in_a_port_is_followed` of the plugin
(src/mwse_sites.rs) with NIRENDER_MWSE_DLL set to the DLL, and prints its lines: each place that
the DLL names in a function of the table, the failures, the stale lines of
tools/mwse_sites.baseline, the reach of the decode of the DLL's code, and a count. A failure names the site, for example "MWSE.dll names
0x6a9d83, NiDX8Renderer::deleting_dtor+0x3, push x1, in the stolen bytes of ...: the detour will
refuse the renderer". The rules and the limits: the header of src/mwse_sites.rs (Brief 22,
2026-11-10). The test compiles the plugin's tests when they are not built: a compile, so ask
for a build place first (slot.py take build).
"""
import argparse
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
TEST = "mwse_sites::tests::every_mwse_site_in_a_port_is_followed"
# The lines of the test; cargo's own lines are left out. "reach: " is the line of the decode of the code: how much of
# .text it reached, the blocks dropped as data and the overlaps. A fall from one build to the next shows there.
MARKS = ("ok: ", "FAIL: ", "stale: ", "reach: ")


def report_lines(output):
    """The lines of the check in the output of cargo test: the places, the stale lines, the reach and the count."""
    return [line for line in output.splitlines() if line.startswith(MARKS) or " places in ported or census functions, " in line]


def self_test():
    output = "\n".join([
        "   Compiling nirender v0.3.6",
        "running 1 test",
        "ok: 0x6a9aed, NiDX8Renderer::ctor+0x7d, push x3 (BODY)",
        "FAIL: MWSE.dll names 0x6a9d83, NiDX8Renderer::deleting_dtor+0x3, push x1, in the stolen bytes of NiDX8Renderer::deleting_dtor: the detour will refuse the renderer",
        "stale: the baseline has 0x6aff07 mov-r32 x1, and the DLL does not",
        "reach: 18340791 of 19088896 bytes of .text (96.1 %), 1055138 blocks, 889 dropped as data, 8 overlaps",
        "x.dll: 105 places in ported or census functions, 1 failed, 1 stale baseline lines",
        "test mwse_sites::tests::every_mwse_site_in_a_port_is_followed ... FAILED",
    ])
    lines = report_lines(output)
    assert len(lines) == 5 and lines[1].startswith("FAIL: MWSE.dll names 0x6a9d83") and lines[3].startswith("reach: "), lines
    print("check_mwse_sites self-test: ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dll", nargs="?", help="the MWSE.dll to check")
    ap.add_argument("--self-test", action="store_true", help="the tests of the output filter: no cargo")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.dll:
        ap.error("name the MWSE.dll")
    dll = os.path.abspath(args.dll)
    if not os.path.isfile(dll):
        print("no file %s" % dll)
        return 2
    env = dict(os.environ, NIRENDER_MWSE_DLL=dll)
    done = subprocess.run(["cargo", "test", "--release", "--lib", TEST, "--", "--exact", "--nocapture"], cwd=REPO, env=env,
                          capture_output=True, text=True, encoding="utf-8", errors="replace")
    lines = report_lines(done.stdout + done.stderr)
    for line in lines:
        print(line)
    if done.returncode != 0 and not any(" places in ported or census functions, " in line for line in lines):
        # The test did not get to its count: a build error, or the DLL could not be read.
        print((done.stdout + done.stderr)[-3000:])
    print("mwse sites: %s" % ("ok" if done.returncode == 0 else "FAILED"))
    return 0 if done.returncode == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
