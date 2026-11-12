"""The patch sites of an MWSE.dll against the plugin's ports, before the DLL goes to a test install; and the probe of the
MWSE features that the plugin needs.

    python -I tools/check_mwse_sites.py <MWSE.dll>              # exit code 0 when every site is followed
    python -I tools/check_mwse_sites.py --features <MWSE.dll>   # exit code 0 when the DLL has every required feature
    python -I tools/check_mwse_sites.py --self-test             # the tests of the output filters: no cargo

The check of the sites runs the unit test `mwse_sites::tests::every_mwse_site_in_a_port_is_followed` of the plugin
(src/mwse_sites.rs) with NIRENDER_MWSE_DLL set to the DLL, and prints its lines: each place that
the DLL names in a function of the table, the failures, the stale lines of
tools/mwse_sites.baseline, the reach of the decode of the DLL's code, and a count. A failure names the site, for example "MWSE.dll names
0x6a9d83, NiDX8Renderer::deleting_dtor+0x3, push x1, in the stolen bytes of ...: the detour will
refuse the renderer". The rules and the limits: the header of src/mwse_sites.rs (Brief 22,
2026-11-10). It fails on any new MWSE patch in a ported function.

--features asks another question (W406): does this MWSE.dll have what this plugin needs? It runs the unit test
`mwse_sites::tests::every_required_mwse_feature_is_present` (the list MWSE_FEATURES of src/mwse_sites.rs) and prints one
line for each feature: "feature: <name>: present (...)", "MISSING: <name>: ...: the plugin needs it" for a required feature
that the DLL lacks, "absent: <name>: ..." for one that the plugin can run without; then "md5: <md5> <path>". Exit code 4
(MISSING_FEATURE_EXIT) when a required feature is absent: the plugin must not go into an install with this DLL. Any other
failure (a build error, no file, no PE file) is exit code 1.

Both run the plugin's tests, which compile when they are not built: a compile, so ask for a build place first (slot.py
take build).
"""
import argparse
import hashlib
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
TEST = "mwse_sites::tests::every_mwse_site_in_a_port_is_followed"
FEATURES_TEST = "mwse_sites::tests::every_required_mwse_feature_is_present"
# The lines of the test; cargo's own lines are left out. "reach: " is the line of the decode of the code: how much of
# .text it reached, the blocks dropped as data and the overlaps. A fall from one build to the next shows there.
MARKS = ("ok: ", "FAIL: ", "stale: ", "reach: ")
# The lines of the probe of the features, and the line of the DLL's md5 that this tool adds.
FEATURE_MARKS = ("feature: ", "MISSING: ", "absent: ")
MISSING = "MISSING: "
MD5 = "md5: "
# The exit code when the DLL lacks a feature that the plugin needs. merge_step.py and deploy_rest.py read it.
MISSING_FEATURE_EXIT = 4


def report_lines(output):
    """The lines of the check in the output of cargo test: the places, the stale lines, the reach and the count."""
    return [line for line in output.splitlines() if line.startswith(MARKS) or " places in ported or census functions, " in line]


def feature_lines(output):
    """The lines of the probe of the features in the output of cargo test."""
    return [line for line in output.splitlines() if line.startswith(FEATURE_MARKS)]


def features_exit(returncode, lines):
    """The exit code of --features: 0 when the test passed; MISSING_FEATURE_EXIT when it failed on a MISSING line; else 1
    (the test did not get to judge the features)."""
    if returncode == 0:
        return 0
    return MISSING_FEATURE_EXIT if any(line.startswith(MISSING) for line in lines) else 1


def md5_of(path):
    digest = hashlib.md5()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def self_test():
    output = "\n".join([
        "   Compiling nirender v0.3.6",
        "running 1 test",
        "ok: 0x6a9aed, NiDX8Renderer::ctor+0x7d, push x3 (BODY)",
        "FAIL: MWSE.dll names 0x6a9d83, NiDX8Renderer::deleting_dtor+0x3, push x1, in the stolen bytes of NiDX8Renderer::deleting_dtor: the detour will refuse the renderer",
        "stale: the baseline has 0x6aff07 mov-r32 x1, and the DLL does not",
        "reach: 18337040 of 19088896 bytes of .text (96.1 %, 536 of them weak), 1055138 blocks, 889 dropped as data, 11 overlaps",
        "x.dll: 105 places in ported or census functions, 1 failed, 1 stale baseline lines",
        "test mwse_sites::tests::every_mwse_site_in_a_port_is_followed ... FAILED",
    ])
    lines = report_lines(output)
    assert len(lines) == 5 and lines[1].startswith("FAIL: MWSE.dll names 0x6a9d83") and lines[3].startswith("reach: "), lines

    # The features: the lines of the probe; a required feature that is absent gives exit code 4, one that the plugin can
    # run without gives 0 with its line, and a failure with no MISSING line (a build error) gives 1.
    missing = "\n".join([
        "   Compiling nirender v0.3.6",
        "MISSING: count locks: 0 of 40 count lock sites in the data of MWSE.dll, missing 0x6fe39f, ...: the plugin needs it",
        "absent: b: 0 of 1 values in the data of MWSE.dll, missing 0x12345678: the plugin runs without it",
        "test mwse_sites::tests::every_required_mwse_feature_is_present ... FAILED",
    ])
    lines = feature_lines(missing)
    assert [line.split(":")[0] for line in lines] == ["MISSING", "absent"], lines
    assert features_exit(101, lines) == MISSING_FEATURE_EXIT
    optional = feature_lines("feature: count locks: present (40 of 40 count lock sites in the data of MWSE.dll)\n"
                             "absent: b: 0 of 1 values in the data of MWSE.dll, missing 0x12345678: the plugin runs without it\n")
    assert len(optional) == 2 and features_exit(0, optional) == 0, optional
    assert features_exit(101, feature_lines("error[E0425]: cannot find value `x`\n")) == 1, "a build error is no missing feature"
    assert features_exit(101, []) == 1
    print("check_mwse_sites self-test: ok")


def run_test(test, dll):
    env = dict(os.environ, NIRENDER_MWSE_DLL=dll)
    done = subprocess.run(["cargo", "test", "--release", "--lib", test, "--", "--exact", "--nocapture"], cwd=REPO, env=env,
                          capture_output=True, text=True, encoding="utf-8", errors="replace")
    return done.returncode, done.stdout + done.stderr


def check_sites(dll):
    code, output = run_test(TEST, dll)
    lines = report_lines(output)
    for line in lines:
        print(line)
    if code != 0 and not any(" places in ported or census functions, " in line for line in lines):
        # The test did not get to its count: a build error, or the DLL could not be read.
        print(output[-3000:])
    print("mwse sites: %s" % ("ok" if code == 0 else "FAILED"))
    return 0 if code == 0 else 1


def check_features(dll):
    code, output = run_test(FEATURES_TEST, dll)
    lines = feature_lines(output)
    for line in lines:
        print(line)
    print("%s%s %s" % (MD5, md5_of(dll), dll))
    result = features_exit(code, lines)
    if result == 1:
        # The test did not judge the features: a build error, or the DLL could not be read.
        print(output[-3000:])
    print("mwse features: %s" % {0: "ok", MISSING_FEATURE_EXIT: "MISSING"}.get(result, "FAILED"))
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dll", nargs="?", help="the MWSE.dll to check")
    ap.add_argument("--features", action="store_true", help="probe the MWSE features that the plugin needs, not the sites")
    ap.add_argument("--self-test", action="store_true", help="the tests of the output filters: no cargo")
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
    return check_features(dll) if args.features else check_sites(dll)


if __name__ == "__main__":
    sys.exit(main())
